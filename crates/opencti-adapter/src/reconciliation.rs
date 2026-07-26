// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! Bounded provider-to-canonical reconciliation planning and safe repair contract.

use std::collections::{BTreeMap, BTreeSet};

use graph_core::{Graph, NodeId, RelationshipId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{MappedRecord, OpenCtiAdapter};

/// Hard reconciliation request limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReconciliationLimits {
    /// Maximum canonical/reference records inspected by one command.
    pub max_records: usize,
    /// Maximum serialized reference payload bytes accepted by one command.
    pub max_payload_bytes: usize,
}

impl Default for ReconciliationLimits {
    fn default() -> Self {
        Self {
            max_records: 10_000,
            max_payload_bytes: 32 * 1024 * 1024,
        }
    }
}

/// Whether a command only reports or also applies safe changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconciliationMode {
    /// Compute an exact plan without changing canonical data or projections.
    DryRun,
    /// Apply safe declared changes and verify the resulting parity.
    Repair,
}

/// Deterministic bounded selection for targeted, range or full repair.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "scope", rename_all = "snake_case")]
pub enum ReconciliationScope {
    /// Exact canonical record IDs.
    Records {
        /// Exact IDs to inspect; duplicates and blanks are rejected.
        record_ids: Vec<String>,
    },
    /// Lexicographic half-open canonical-ID range.
    Range {
        /// First canonical ID included in the range.
        start_inclusive: String,
        /// First canonical ID excluded from the range.
        end_exclusive: String,
        /// Hard selected-record bound.
        max_records: usize,
    },
    /// Stable hash partition for parallel bounded repair.
    Partition {
        /// Zero-based stable hash partition.
        partition: u32,
        /// Total number of stable hash partitions.
        partition_count: u32,
        /// Hard selected-record bound.
        max_records: usize,
    },
    /// Full resynchronization with an explicit hard record bound.
    Full {
        /// Hard selected-record bound.
        max_records: usize,
    },
}

/// One provider snapshot and repair policy.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OpenCtiReconciliationCommand {
    /// Stable idempotency and WAL identity.
    pub command_id: String,
    /// Dry-run or repair behavior.
    pub mode: ReconciliationMode,
    /// Records/range/partition selected for comparison.
    pub scope: ReconciliationScope,
    /// Lossless reference-provider records for the selected scope.
    pub reference_records: Vec<Value>,
    /// Whether extra canonical records may be tombstoned; otherwise quarantine.
    pub allow_extra_deletion: bool,
}

impl OpenCtiReconciliationCommand {
    /// Construct a command; deep scope and payload validation occurs before scan.
    pub fn new(
        command_id: impl Into<String>,
        mode: ReconciliationMode,
        scope: ReconciliationScope,
        reference_records: Vec<Value>,
        allow_extra_deletion: bool,
    ) -> Result<Self, ReconciliationError> {
        let command_id = command_id.into();
        if command_id.trim().is_empty() {
            return Err(ReconciliationError::InvalidInput(
                "command_id cannot be blank".to_owned(),
            ));
        }
        Ok(Self {
            command_id,
            mode,
            scope,
            reference_records,
            allow_extra_deletion,
        })
    }
}

/// Stable mismatch dimensions required by issue #51.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DivergenceKind {
    /// Reference record is absent from Corrobore.
    Missing,
    /// Corrobore record is absent from the authoritative reference scope.
    Extra,
    /// Non-security object properties differ.
    PropertyDivergent,
    /// Relationship type or endpoints differ.
    RelationshipDivergent,
    /// Marking, organization or tenant policy differs.
    PermissionDivergent,
    /// A derived lookup/search projection is not current.
    StaleIndex,
}

/// Planned or completed action for one difference.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairAction {
    /// Create a missing canonical record.
    PlannedCreate,
    /// Replace a divergent canonical record.
    PlannedReplace,
    /// Tombstone an explicitly authorized extra record.
    PlannedDelete,
    /// Rebuild derived projections for an unchanged canonical record.
    PlannedProjectionRebuild,
    /// Safe declared repair was applied.
    Applied,
    /// Unsafe change requires operator policy.
    Quarantined,
}

/// Payload-free exact change record returned by dry-run and repair.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReconciliationDifference {
    /// Canonical record ID.
    pub record_id: String,
    /// Mismatched dimension.
    pub kind: DivergenceKind,
    /// Planned or completed action.
    pub action: RepairAction,
    /// Actionable content-free diagnostic.
    pub diagnostic: String,
}

/// Observable command report persisted beside its WAL receipt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReconciliationReport {
    /// Stable command ID.
    pub command_id: String,
    /// Requested mode.
    pub mode: ReconciliationMode,
    /// Whether canonical state changed.
    pub mutated: bool,
    /// Whether post-repair comparison found no unresolved differences.
    pub parity_verified: bool,
    /// Stable ID/dimension ordered differences.
    pub differences: Vec<ReconciliationDifference>,
    /// Unsafe records requiring operator policy.
    pub quarantined_record_ids: Vec<String>,
    /// Canonical IDs requiring targeted projection repair.
    pub projection_rebuild_ids: Vec<String>,
}

/// Prepared graph plus its report; the host owns atomic commit and index repair.
#[derive(Clone, Debug)]
pub struct OpenCtiReconciliationOutcome {
    /// Input graph for dry-run or repaired clone for repair mode.
    pub graph: Graph,
    /// Exact reconciliation evidence.
    pub report: ReconciliationReport,
}

/// Reconciliation validation or planning failure.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ReconciliationError {
    /// Malformed scope or record payload.
    #[error("invalid reconciliation input: {0}")]
    InvalidInput(String),
    /// Configured record or byte bound exceeded.
    #[error("reconciliation limit exceeded: {0}")]
    LimitExceeded(String),
    /// Graph or mapping operation failed.
    #[error("reconciliation graph failure: {0}")]
    Graph(String),
    /// Phase-2 API shape exists but behavior is not implemented yet.
    #[error("reconciliation implementation is not available")]
    NotImplemented,
}

/// Stateless diff and safe-repair planner.
#[derive(Clone, Debug)]
pub struct OpenCtiReconciler {
    limits: ReconciliationLimits,
}

impl OpenCtiReconciler {
    /// Configure bounded comparison and payload processing.
    pub const fn new(limits: ReconciliationLimits) -> Self {
        Self { limits }
    }

    /// Validate the bounded scope, canonicalize provider records, compare every
    /// required dimension, and apply only safe declared changes on a graph clone.
    /// Dry-run returns the same exact plan but always returns the input image.
    pub fn execute(
        &self,
        graph: &Graph,
        command: &OpenCtiReconciliationCommand,
        stale_index_ids: &[String],
    ) -> Result<OpenCtiReconciliationOutcome, ReconciliationError> {
        self.validate(command)?;
        let adapter = OpenCtiAdapter::pinned();
        let expected = expected_records(&adapter, &command.reference_records)?;
        let actual = ActualIndex::build(graph, &adapter)?;
        let selected = selected_ids(command, &expected, &actual, stale_index_ids, self.limits)?;
        let stale = stale_index_ids.iter().cloned().collect::<BTreeSet<_>>();
        let mut differences = compare(&selected, &expected, &actual, &stale, command);
        quarantine_unsafe_node_deletions(graph, &adapter, &actual, &mut differences)?;
        let mut next = graph.clone();
        let mut repair_index = ActualIndex::build(&next, &adapter)?;
        let mut mutated = false;
        let mut projection_rebuild_ids = Vec::new();
        let mut quarantined_record_ids = differences
            .iter()
            .filter(|item| item.action == RepairAction::Quarantined)
            .map(|item| item.record_id.clone())
            .collect::<Vec<_>>();

        if command.mode == ReconciliationMode::Repair {
            for relationship_phase in [false, true] {
                for difference in &mut differences {
                    if difference.action == RepairAction::Quarantined
                        || difference.kind == DivergenceKind::Extra
                        || difference.kind == DivergenceKind::StaleIndex
                    {
                        continue;
                    }
                    let Some(reference) = expected.get(&difference.record_id) else {
                        continue;
                    };
                    if reference.is_relationship() != relationship_phase {
                        continue;
                    }
                    apply_reference(&mut next, &adapter, &mut repair_index, reference)?;
                    difference.action = RepairAction::Applied;
                    mutated = true;
                }
            }
            for relationship_phase in [true, false] {
                for difference in &mut differences {
                    if difference.kind == DivergenceKind::Extra
                        && difference.action == RepairAction::PlannedDelete
                        && actual
                            .records
                            .get(&difference.record_id)
                            .is_some_and(|record| record.is_relationship() == relationship_phase)
                    {
                        apply_delete(&mut next, &actual, &difference.record_id)?;
                        difference.action = RepairAction::Applied;
                        mutated = true;
                    }
                }
            }
            projection_rebuild_ids.extend(
                differences
                    .iter()
                    .filter(|difference| difference.kind == DivergenceKind::StaleIndex)
                    .map(|difference| difference.record_id.clone()),
            );
        }
        projection_rebuild_ids.sort();
        projection_rebuild_ids.dedup();
        quarantined_record_ids.sort();
        quarantined_record_ids.dedup();

        let parity_verified = if command.mode == ReconciliationMode::DryRun {
            differences.is_empty()
        } else if !quarantined_record_ids.is_empty() || !projection_rebuild_ids.is_empty() {
            false
        } else {
            let repaired = ActualIndex::build(&next, &adapter)?;
            compare(&selected, &expected, &repaired, &BTreeSet::new(), command).is_empty()
        };
        Ok(OpenCtiReconciliationOutcome {
            graph: if command.mode == ReconciliationMode::DryRun {
                graph.clone()
            } else {
                next
            },
            report: ReconciliationReport {
                command_id: command.command_id.clone(),
                mode: command.mode,
                mutated,
                parity_verified,
                differences,
                quarantined_record_ids,
                projection_rebuild_ids,
            },
        })
    }

    fn validate(&self, command: &OpenCtiReconciliationCommand) -> Result<(), ReconciliationError> {
        let payload_bytes = serde_json::to_vec(&command.reference_records)
            .map_err(|error| ReconciliationError::InvalidInput(error.to_string()))?
            .len();
        if payload_bytes > self.limits.max_payload_bytes {
            return Err(ReconciliationError::LimitExceeded(format!(
                "max_payload_bytes is {}, received {payload_bytes}",
                self.limits.max_payload_bytes
            )));
        }
        match &command.scope {
            ReconciliationScope::Records { record_ids } => {
                let unique = record_ids.iter().collect::<BTreeSet<_>>();
                if unique.len() != record_ids.len()
                    || record_ids
                        .iter()
                        .any(|record_id| record_id.trim().is_empty())
                {
                    return Err(ReconciliationError::InvalidInput(
                        "record scope IDs must be non-blank and unique".to_owned(),
                    ));
                }
            }
            ReconciliationScope::Range {
                start_inclusive,
                end_exclusive,
                max_records,
            } => {
                if start_inclusive >= end_exclusive || *max_records == 0 {
                    return Err(ReconciliationError::InvalidInput(
                        "range scope must be non-empty and bounded".to_owned(),
                    ));
                }
            }
            ReconciliationScope::Partition {
                partition,
                partition_count,
                max_records,
            } => {
                if *partition_count == 0 || partition >= partition_count || *max_records == 0 {
                    return Err(ReconciliationError::InvalidInput(
                        "partition scope and bound are invalid".to_owned(),
                    ));
                }
            }
            ReconciliationScope::Full { max_records } if *max_records == 0 => {
                return Err(ReconciliationError::InvalidInput(
                    "full scope must have a positive bound".to_owned(),
                ));
            }
            ReconciliationScope::Full { .. } => {}
        }
        Ok(())
    }
}

fn quarantine_unsafe_node_deletions(
    graph: &Graph,
    adapter: &OpenCtiAdapter,
    actual: &ActualIndex,
    differences: &mut [ReconciliationDifference],
) -> Result<(), ReconciliationError> {
    let planned_deletions = differences
        .iter()
        .filter(|difference| difference.action == RepairAction::PlannedDelete)
        .map(|difference| difference.record_id.clone())
        .collect::<BTreeSet<_>>();
    if planned_deletions.is_empty() {
        return Ok(());
    }
    let relationships = graph.list_relationships().map_err(graph_error)?;
    for difference in differences {
        if difference.action != RepairAction::PlannedDelete {
            continue;
        }
        let Some(ActualRecord::Object { node_id, .. }) = actual.records.get(&difference.record_id)
        else {
            continue;
        };
        let has_undeclared_relationship = relationships.iter().any(|relationship| {
            if relationship.source() != node_id && relationship.target() != node_id {
                return false;
            }
            adapter
                .restore_relationship(relationship)
                .map(|record| !planned_deletions.contains(record.record_ref().canonical_id()))
                .unwrap_or(true)
        });
        if has_undeclared_relationship {
            difference.action = RepairAction::Quarantined;
            difference.diagnostic =
                "extra node has relationships outside the declared deletion scope".to_owned();
        }
    }
    Ok(())
}

#[derive(Clone)]
enum ExpectedRecord {
    Object(Value),
    Relationship(Value),
}

impl ExpectedRecord {
    fn raw(&self) -> &Value {
        match self {
            Self::Object(value) | Self::Relationship(value) => value,
        }
    }

    const fn is_relationship(&self) -> bool {
        matches!(self, Self::Relationship(_))
    }
}

#[derive(Clone)]
enum ActualRecord {
    Object {
        node_id: NodeId,
        raw: Value,
    },
    Relationship {
        relationship_id: RelationshipId,
        raw: Value,
    },
}

impl ActualRecord {
    fn raw(&self) -> &Value {
        match self {
            Self::Object { raw, .. } | Self::Relationship { raw, .. } => raw,
        }
    }

    const fn is_relationship(&self) -> bool {
        matches!(self, Self::Relationship { .. })
    }
}

struct ActualIndex {
    records: BTreeMap<String, ActualRecord>,
    nodes: BTreeMap<String, NodeId>,
}

impl ActualIndex {
    fn build(graph: &Graph, adapter: &OpenCtiAdapter) -> Result<Self, ReconciliationError> {
        let mut records = BTreeMap::new();
        let mut nodes = BTreeMap::new();
        for node in graph.list_nodes().map_err(graph_error)? {
            let mapped = adapter.restore_node(&node).map_err(mapping_error)?;
            let canonical_id = mapped.record_ref().canonical_id().to_owned();
            records.insert(
                canonical_id.clone(),
                ActualRecord::Object {
                    node_id: node.id().clone(),
                    raw: mapped.raw().clone(),
                },
            );
            nodes.insert(canonical_id, node.id().clone());
            for identifier in mapped.identifiers() {
                nodes
                    .entry(identifier.value().to_owned())
                    .or_insert_with(|| node.id().clone());
            }
        }
        for relationship in graph.list_relationships().map_err(graph_error)? {
            let mapped = adapter
                .restore_relationship(&relationship)
                .map_err(mapping_error)?;
            records.insert(
                mapped.record_ref().canonical_id().to_owned(),
                ActualRecord::Relationship {
                    relationship_id: relationship.id().clone(),
                    raw: mapped.raw().clone(),
                },
            );
        }
        Ok(Self { records, nodes })
    }
}

fn expected_records(
    adapter: &OpenCtiAdapter,
    values: &[Value],
) -> Result<BTreeMap<String, ExpectedRecord>, ReconciliationError> {
    let mut records = BTreeMap::new();
    for value in values {
        let mapped = adapter.map(value.clone()).map_err(mapping_error)?;
        let canonical_id = mapped.record_ref().canonical_id().to_owned();
        let expected = match mapped {
            MappedRecord::Object(_) => ExpectedRecord::Object(value.clone()),
            MappedRecord::Relationship(_) => ExpectedRecord::Relationship(value.clone()),
        };
        if records.insert(canonical_id.clone(), expected).is_some() {
            return Err(ReconciliationError::InvalidInput(format!(
                "duplicate reference record {canonical_id}"
            )));
        }
    }
    Ok(records)
}

fn selected_ids(
    command: &OpenCtiReconciliationCommand,
    expected: &BTreeMap<String, ExpectedRecord>,
    actual: &ActualIndex,
    stale_index_ids: &[String],
    limits: ReconciliationLimits,
) -> Result<Vec<String>, ReconciliationError> {
    let universe = expected
        .keys()
        .chain(actual.records.keys())
        .chain(stale_index_ids.iter())
        .cloned()
        .collect::<BTreeSet<_>>();
    let (selected, scope_limit) = match &command.scope {
        ReconciliationScope::Records { record_ids } => (
            record_ids.iter().cloned().collect::<BTreeSet<_>>(),
            limits.max_records,
        ),
        ReconciliationScope::Range {
            start_inclusive,
            end_exclusive,
            max_records,
        } => (
            universe
                .into_iter()
                .filter(|id| id >= start_inclusive && id < end_exclusive)
                .collect(),
            *max_records,
        ),
        ReconciliationScope::Partition {
            partition,
            partition_count,
            max_records,
        } => (
            universe
                .into_iter()
                .filter(|id| stable_partition(id, *partition_count) == *partition)
                .collect(),
            *max_records,
        ),
        ReconciliationScope::Full { max_records } => (universe, *max_records),
    };
    let effective_limit = scope_limit.min(limits.max_records);
    if selected.len() > effective_limit {
        return Err(ReconciliationError::LimitExceeded(format!(
            "max_records is {effective_limit}, selected {}",
            selected.len()
        )));
    }
    Ok(selected.into_iter().collect())
}

fn compare(
    selected: &[String],
    expected: &BTreeMap<String, ExpectedRecord>,
    actual: &ActualIndex,
    stale: &BTreeSet<String>,
    command: &OpenCtiReconciliationCommand,
) -> Vec<ReconciliationDifference> {
    let adapter = OpenCtiAdapter::pinned();
    let mut differences = Vec::new();
    for record_id in selected {
        match (expected.get(record_id), actual.records.get(record_id)) {
            (Some(_), None) => differences.push(difference(
                record_id,
                DivergenceKind::Missing,
                RepairAction::PlannedCreate,
                "reference record is missing from canonical storage",
            )),
            (None, Some(_)) => differences.push(difference(
                record_id,
                DivergenceKind::Extra,
                if command.allow_extra_deletion {
                    RepairAction::PlannedDelete
                } else {
                    RepairAction::Quarantined
                },
                "canonical record is absent from the authoritative reference scope",
            )),
            (Some(expected), Some(actual)) => {
                let kind = classify_difference(&adapter, expected, actual);
                if let Some(kind) = kind {
                    let unsafe_kind = expected.is_relationship() != actual.is_relationship();
                    differences.push(difference(
                        record_id,
                        kind,
                        if unsafe_kind {
                            RepairAction::Quarantined
                        } else {
                            RepairAction::PlannedReplace
                        },
                        if unsafe_kind {
                            "record category conflict requires operator policy"
                        } else {
                            "reference and canonical record dimensions differ"
                        },
                    ));
                }
            }
            (None, None) => {}
        }
        if stale.contains(record_id) {
            differences.push(difference(
                record_id,
                DivergenceKind::StaleIndex,
                RepairAction::PlannedProjectionRebuild,
                "derived projection does not match canonical generation",
            ));
        }
    }
    differences.sort_by(|left, right| {
        left.record_id
            .cmp(&right.record_id)
            .then(left.kind.cmp(&right.kind))
    });
    differences
}

fn classify_difference(
    adapter: &OpenCtiAdapter,
    expected: &ExpectedRecord,
    actual: &ActualRecord,
) -> Option<DivergenceKind> {
    if expected.raw() == actual.raw() && expected.is_relationship() == actual.is_relationship() {
        return None;
    }
    let expected_mapped = adapter.map(expected.raw().clone()).ok()?;
    let actual_mapped = adapter.map(actual.raw().clone()).ok()?;
    match (&expected_mapped, &actual_mapped) {
        (MappedRecord::Relationship(expected), MappedRecord::Relationship(actual))
            if expected.source_ref() != actual.source_ref()
                || expected.target_ref() != actual.target_ref()
                || expected.relationship_type() != actual.relationship_type() =>
        {
            Some(DivergenceKind::RelationshipDivergent)
        }
        _ if expected_mapped.access() != actual_mapped.access() => {
            Some(DivergenceKind::PermissionDivergent)
        }
        _ => Some(DivergenceKind::PropertyDivergent),
    }
}

fn apply_reference(
    graph: &mut Graph,
    adapter: &OpenCtiAdapter,
    index: &mut ActualIndex,
    reference: &ExpectedRecord,
) -> Result<(), ReconciliationError> {
    let mapped = adapter
        .map(reference.raw().clone())
        .map_err(mapping_error)?;
    let canonical_id = mapped.record_ref().canonical_id().to_owned();
    match mapped {
        MappedRecord::Object(object) => {
            let identifiers = object
                .identifiers
                .iter()
                .map(|identifier| identifier.value().to_owned())
                .collect::<Vec<_>>();
            let node_id = if let Some(ActualRecord::Object { node_id, .. }) =
                index.records.get(&canonical_id)
            {
                graph
                    .replace_node(node_id, object.to_node_input())
                    .map_err(graph_error)?
            } else {
                graph
                    .create_node(object.to_node_input())
                    .map_err(graph_error)?
            };
            index.nodes.insert(canonical_id.clone(), node_id.clone());
            for identifier in identifiers {
                index
                    .nodes
                    .entry(identifier)
                    .or_insert_with(|| node_id.clone());
            }
            index.records.insert(
                canonical_id,
                ActualRecord::Object {
                    node_id,
                    raw: reference.raw().clone(),
                },
            );
        }
        MappedRecord::Relationship(relationship) => {
            let source = index
                .nodes
                .get(relationship.source_ref())
                .cloned()
                .ok_or_else(|| {
                    ReconciliationError::Graph(format!(
                        "relationship source {} is unavailable",
                        relationship.source_ref()
                    ))
                })?;
            let target = index
                .nodes
                .get(relationship.target_ref())
                .cloned()
                .ok_or_else(|| {
                    ReconciliationError::Graph(format!(
                        "relationship target {} is unavailable",
                        relationship.target_ref()
                    ))
                })?;
            let input = relationship
                .to_relationship_input(source, target)
                .map_err(mapping_error)?;
            let relationship_id = if let Some(ActualRecord::Relationship {
                relationship_id, ..
            }) = index.records.get(&canonical_id)
            {
                graph
                    .replace_relationship(relationship_id, input)
                    .map_err(graph_error)?
            } else {
                graph.create_relationship(input).map_err(graph_error)?
            };
            index.records.insert(
                canonical_id,
                ActualRecord::Relationship {
                    relationship_id,
                    raw: reference.raw().clone(),
                },
            );
        }
    }
    Ok(())
}

fn apply_delete(
    graph: &mut Graph,
    actual: &ActualIndex,
    record_id: &str,
) -> Result<(), ReconciliationError> {
    match actual.records.get(record_id) {
        Some(ActualRecord::Object { node_id, .. }) => {
            graph.tombstone_node(node_id).map_err(graph_error)?;
        }
        Some(ActualRecord::Relationship {
            relationship_id, ..
        }) => {
            graph
                .tombstone_relationship(relationship_id)
                .map_err(graph_error)?;
        }
        None => {}
    }
    Ok(())
}

fn difference(
    record_id: &str,
    kind: DivergenceKind,
    action: RepairAction,
    diagnostic: &str,
) -> ReconciliationDifference {
    ReconciliationDifference {
        record_id: record_id.to_owned(),
        kind,
        action,
        diagnostic: diagnostic.to_owned(),
    }
}

fn stable_partition(record_id: &str, partition_count: u32) -> u32 {
    let digest = Sha256::digest(record_id.as_bytes());
    let value = u64::from_be_bytes(digest[..8].try_into().unwrap_or_default());
    (value % u64::from(partition_count)) as u32
}

fn graph_error(error: graph_core::GraphError) -> ReconciliationError {
    ReconciliationError::Graph(error.to_string())
}

fn mapping_error(error: crate::MappingError) -> ReconciliationError {
    ReconciliationError::InvalidInput(error.to_string())
}

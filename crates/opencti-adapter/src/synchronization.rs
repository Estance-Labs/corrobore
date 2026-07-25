// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! Ordered, replay-safe OpenCTI snapshot and mutation synchronization.
//!
//! The protocol keeps source ordering and checkpoint semantics outside the
//! transport layer. A caller may persist [`SyncCheckpoint`] beside the graph
//! transaction acknowledgement and safely replay the same batch after a crash.

use std::collections::BTreeMap;

use graph_core::{Graph, NodeId, PropertyValue, RelationshipId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{MappedRecord, MappedRelationship, OpenCtiAdapter};

/// Synchronization lifecycle phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncPhase {
    /// Consistent source export bounded by a captured high-water mark.
    Snapshot,
    /// Ordered mutation replay from the snapshot high-water mark.
    CatchUp,
    /// Continuous ordered replication after parity validation.
    SteadyState,
}

/// Source mutation semantic class.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MutationClass {
    /// Create or replace an object record.
    Upsert,
    /// Tombstone an object record.
    Delete,
    /// Create or replace a relationship record.
    RelationshipUpsert,
    /// Tombstone a relationship record.
    RelationshipDelete,
    /// Replace an object or relationship after an access-policy change.
    AccessPolicyUpdate,
}

/// Bounded bulk-ingestion limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BulkLimits {
    /// Maximum operations accepted in one request.
    pub max_operations: usize,
    /// Maximum serialized mutation payload bytes accepted in one request.
    pub max_payload_bytes: usize,
    /// Maximum recent operation fingerprints retained for conflict detection.
    pub max_replay_identities: usize,
}

impl Default for BulkLimits {
    fn default() -> Self {
        Self {
            max_operations: 512,
            max_payload_bytes: 8 * 1024 * 1024,
            max_replay_identities: 4_096,
        }
    }
}

/// One ordered source mutation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OpenCtiMutation {
    /// Stable replay identity assigned by the source connector.
    pub operation_id: String,
    /// Monotonic source sequence.
    pub sequence: u64,
    /// Mutation semantic class.
    pub class: MutationClass,
    /// Lossless OpenCTI record or tombstone descriptor.
    pub record: Value,
}

impl OpenCtiMutation {
    /// Construct a validated source mutation.
    pub fn new(
        operation_id: impl Into<String>,
        sequence: u64,
        class: MutationClass,
        record: Value,
    ) -> Result<Self, SyncError> {
        let operation_id = operation_id.into();
        if operation_id.trim().is_empty() {
            return Err(SyncError::InvalidInput(
                "operation_id cannot be blank".to_owned(),
            ));
        }
        if sequence == 0 {
            return Err(SyncError::InvalidInput(
                "sequence must be greater than zero".to_owned(),
            ));
        }
        if !record.is_object() {
            return Err(SyncError::InvalidInput(
                "record must be a JSON object".to_owned(),
            ));
        }
        Ok(Self {
            operation_id,
            sequence,
            class,
            record,
        })
    }
}

/// One bounded synchronization batch.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OpenCtiSyncBatch {
    /// Stable source instance identity.
    pub source_id: String,
    /// Consistent source snapshot identity.
    pub snapshot_id: String,
    /// Lifecycle phase represented by the batch.
    pub phase: SyncPhase,
    /// Highest source sequence known when the batch was emitted.
    pub high_water_mark: u64,
    /// Whether this batch closes the consistent snapshot export.
    pub snapshot_complete: bool,
    /// Ordered operations.
    pub operations: Vec<OpenCtiMutation>,
}

impl OpenCtiSyncBatch {
    /// Construct a validated synchronization batch.
    pub fn new(
        source_id: impl Into<String>,
        snapshot_id: impl Into<String>,
        phase: SyncPhase,
        high_water_mark: u64,
        snapshot_complete: bool,
        operations: Vec<OpenCtiMutation>,
    ) -> Result<Self, SyncError> {
        let source_id = source_id.into();
        let snapshot_id = snapshot_id.into();
        if source_id.trim().is_empty() || snapshot_id.trim().is_empty() {
            return Err(SyncError::InvalidInput(
                "source_id and snapshot_id cannot be blank".to_owned(),
            ));
        }
        Ok(Self {
            source_id,
            snapshot_id,
            phase,
            high_water_mark,
            snapshot_complete,
            operations,
        })
    }
}

/// Per-operation synchronization result.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationStatus {
    /// Canonical state changed.
    Applied,
    /// Equivalent acknowledged operation was replayed.
    Duplicate,
    /// A transient dependency or ordering condition requires retry.
    Retryable,
    /// Invalid source data was durably rejected.
    PermanentlyRejected,
    /// A conflicting replay identity was isolated for operator review.
    Quarantined,
}

/// Detailed result for one input operation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationResult {
    /// Stable input operation identity.
    pub operation_id: String,
    /// Source sequence.
    pub sequence: u64,
    /// Classified outcome.
    pub status: OperationStatus,
    /// Stable diagnostic suitable for dead-letter inspection.
    pub diagnostic: Option<String>,
}

/// Bounded durable diagnostic for an operation that cannot be applied.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeadLetterRecord {
    /// Stable input operation identity.
    pub operation_id: String,
    /// Source sequence.
    pub sequence: u64,
    /// Permanent or quarantined outcome.
    pub status: OperationStatus,
    /// Stable operator-facing reason.
    pub diagnostic: String,
}

/// Result of one bounded batch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncBatchResult {
    /// Stable graph transaction identity derived from the source boundary.
    pub transaction_id: String,
    /// Per-operation results in input order.
    pub operations: Vec<OperationResult>,
    /// Highest durably acknowledged contiguous source sequence.
    pub acknowledged_sequence: u64,
    /// Remaining retryable queue depth.
    pub queue_depth: u64,
}

/// Durable synchronization progress and observability counters.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncCheckpoint {
    /// Stable source identity.
    pub source_id: String,
    /// Active snapshot identity.
    pub snapshot_id: String,
    /// Current lifecycle phase.
    pub phase: SyncPhase,
    /// Highest contiguous acknowledged source sequence.
    pub last_acknowledged_sequence: u64,
    /// Latest observed source high-water mark.
    pub high_water_mark: u64,
    /// Operations currently blocked for retry.
    pub queue_depth: u64,
    /// Cumulative retryable results.
    pub retry_count: u64,
    /// Cumulative permanently rejected operations.
    pub rejected_operations: u64,
    /// Cumulative quarantined operations.
    pub quarantined_operations: u64,
    /// Recent operation identity fingerprints, oldest first.
    pub replay_identities: Vec<(String, String)>,
    /// Recent permanently rejected or quarantined operations, oldest first.
    #[serde(default)]
    pub dead_letters: Vec<DeadLetterRecord>,
}

impl SyncCheckpoint {
    /// Start synchronization before the first snapshot operation.
    pub fn new(source_id: impl Into<String>, snapshot_id: impl Into<String>) -> Self {
        Self {
            source_id: source_id.into(),
            snapshot_id: snapshot_id.into(),
            phase: SyncPhase::Snapshot,
            last_acknowledged_sequence: 0,
            high_water_mark: 0,
            queue_depth: 0,
            retry_count: 0,
            rejected_operations: 0,
            quarantined_operations: 0,
            replay_identities: Vec::new(),
            dead_letters: Vec::new(),
        }
    }
}

/// Deterministic canonical and projection parity digest.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphDigest {
    /// Active object count.
    pub object_count: u64,
    /// Active relationship count.
    pub relationship_count: u64,
    /// Checksum of the active canonical identity set.
    pub canonical_checksum: String,
    /// Checksum of lossless source properties by canonical identity.
    #[serde(default)]
    pub property_checksum: String,
    /// Checksum of every indexed identifier.
    pub identifier_checksum: String,
    /// Checksum of relationship endpoints and types.
    pub relation_checksum: String,
    /// Checksum of access-policy inputs.
    pub access_policy_checksum: String,
}

/// Current divergence state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DivergenceStatus {
    /// No comparison has completed.
    Unknown,
    /// One or more canonical/projection dimensions differ.
    Diverged,
    /// Every required dimension matches.
    InSync,
}

/// Validation result used to gate shadow reads.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncValidationReport {
    /// Overall parity status.
    pub divergence: DivergenceStatus,
    /// Stable names of mismatched dimensions.
    pub differences: Vec<String>,
    /// Whether every required check permits shadow reads.
    pub shadow_reads_enabled: bool,
    /// Actual target digest.
    pub actual: GraphDigest,
}

/// Synchronization failures that prevent a batch-level acknowledgement.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SyncError {
    /// Input or lifecycle validation failed.
    #[error("invalid synchronization input: {0}")]
    InvalidInput(String),
    /// Configured bulk boundary was exceeded.
    #[error("bulk limit exceeded: {0}")]
    BulkLimitExceeded(String),
    /// Graph access or transition failed.
    #[error("graph synchronization failed: {0}")]
    Graph(String),
}

/// Applies ordered OpenCTI batches to a canonical graph.
#[derive(Clone, Debug)]
pub struct OpenCtiSynchronizer {
    limits: BulkLimits,
    adapter: OpenCtiAdapter,
}

impl OpenCtiSynchronizer {
    /// Create a synchronizer with explicit memory/backpressure limits.
    pub fn new(limits: BulkLimits) -> Self {
        Self {
            limits,
            adapter: OpenCtiAdapter::pinned(),
        }
    }

    /// Apply a bounded contiguous prefix and advance the checkpoint only after
    /// the graph transition is ready for durable commit.
    pub fn apply_batch(
        &self,
        graph: &mut Graph,
        checkpoint: &mut SyncCheckpoint,
        batch: OpenCtiSyncBatch,
    ) -> Result<SyncBatchResult, SyncError> {
        self.validate_batch(checkpoint, &batch)?;
        let payload_bytes = serde_json::to_vec(&batch.operations)
            .map_err(|error| SyncError::InvalidInput(error.to_string()))?
            .len();
        if batch.operations.len() > self.limits.max_operations {
            return Err(SyncError::BulkLimitExceeded(format!(
                "max_operations is {}, received {}",
                self.limits.max_operations,
                batch.operations.len()
            )));
        }
        if payload_bytes > self.limits.max_payload_bytes {
            return Err(SyncError::BulkLimitExceeded(format!(
                "max_payload_bytes is {}, received {payload_bytes}",
                self.limits.max_payload_bytes
            )));
        }

        let mut working_graph = graph.clone();
        let mut working_checkpoint = checkpoint.clone();
        working_checkpoint.high_water_mark = working_checkpoint
            .high_water_mark
            .max(batch.high_water_mark)
            .max(
                batch
                    .operations
                    .iter()
                    .map(|operation| operation.sequence)
                    .max()
                    .unwrap_or(0),
            );
        working_checkpoint.queue_depth = 0;
        let mut indexes = GraphIndexes::from_graph(&working_graph, &self.adapter)?;
        let mut results = Vec::with_capacity(batch.operations.len());
        let mut retry_blocked = false;

        for operation in &batch.operations {
            let fingerprint = operation_fingerprint(operation)?;
            if retry_blocked {
                results.push(operation_result(
                    operation,
                    OperationStatus::Retryable,
                    "blocked by an earlier retryable source sequence",
                ));
                continue;
            }
            let applied_fingerprint = working_checkpoint
                .replay_identities
                .iter()
                .find(|(operation_id, _)| operation_id == &operation.operation_id)
                .map(|(_, fingerprint)| fingerprint.clone());
            if operation.sequence <= working_checkpoint.last_acknowledged_sequence {
                if applied_fingerprint
                    .as_ref()
                    .is_some_and(|applied| applied != &fingerprint)
                {
                    let diagnostic = "operation identity was replayed with a different payload";
                    working_checkpoint.quarantined_operations =
                        working_checkpoint.quarantined_operations.saturating_add(1);
                    remember_dead_letter(
                        &mut working_checkpoint,
                        operation,
                        OperationStatus::Quarantined,
                        diagnostic,
                        self.limits.max_replay_identities,
                    );
                    results.push(operation_result(
                        operation,
                        OperationStatus::Quarantined,
                        diagnostic,
                    ));
                    continue;
                }
                results.push(operation_result(
                    operation,
                    OperationStatus::Duplicate,
                    "operation sequence is already acknowledged",
                ));
                continue;
            }
            let expected = working_checkpoint
                .last_acknowledged_sequence
                .saturating_add(1);
            if operation.sequence != expected {
                retry_blocked = true;
                results.push(operation_result(
                    operation,
                    OperationStatus::Retryable,
                    &format!(
                        "source sequence gap: expected {expected}, received {}",
                        operation.sequence
                    ),
                ));
                continue;
            }
            if applied_fingerprint.is_some() {
                let diagnostic = "operation identity was reused for a new source sequence";
                working_checkpoint.quarantined_operations =
                    working_checkpoint.quarantined_operations.saturating_add(1);
                working_checkpoint.last_acknowledged_sequence = operation.sequence;
                remember_dead_letter(
                    &mut working_checkpoint,
                    operation,
                    OperationStatus::Quarantined,
                    diagnostic,
                    self.limits.max_replay_identities,
                );
                results.push(operation_result(
                    operation,
                    OperationStatus::Quarantined,
                    diagnostic,
                ));
                continue;
            }

            let (status, diagnostic) =
                match apply_operation(&mut working_graph, &indexes, &self.adapter, operation) {
                    Ok(ApplyOperation::Applied) => {
                        indexes = GraphIndexes::from_graph(&working_graph, &self.adapter)?;
                        (OperationStatus::Applied, None)
                    }
                    Ok(ApplyOperation::Duplicate) => (
                        OperationStatus::Duplicate,
                        Some("canonical payload is already current".to_owned()),
                    ),
                    Ok(ApplyOperation::Retryable(reason)) => {
                        retry_blocked = true;
                        (OperationStatus::Retryable, Some(reason))
                    }
                    Err(error) => {
                        working_checkpoint.rejected_operations =
                            working_checkpoint.rejected_operations.saturating_add(1);
                        (
                            OperationStatus::PermanentlyRejected,
                            Some(error.to_string()),
                        )
                    }
                };
            if status != OperationStatus::Retryable {
                working_checkpoint.last_acknowledged_sequence = operation.sequence;
                remember_replay_identity(
                    &mut working_checkpoint,
                    operation.operation_id.clone(),
                    fingerprint,
                    self.limits.max_replay_identities,
                );
            }
            if matches!(
                status,
                OperationStatus::PermanentlyRejected | OperationStatus::Quarantined
            ) {
                remember_dead_letter(
                    &mut working_checkpoint,
                    operation,
                    status,
                    diagnostic.as_deref().unwrap_or("operation was rejected"),
                    self.limits.max_replay_identities,
                );
            }
            results.push(OperationResult {
                operation_id: operation.operation_id.clone(),
                sequence: operation.sequence,
                status,
                diagnostic,
            });
        }

        let retryable = results
            .iter()
            .filter(|result| result.status == OperationStatus::Retryable)
            .count() as u64;
        working_checkpoint.queue_depth = retryable;
        working_checkpoint.retry_count = working_checkpoint.retry_count.saturating_add(retryable);
        if batch.phase == SyncPhase::Snapshot
            && batch.snapshot_complete
            && working_checkpoint.queue_depth == 0
        {
            working_checkpoint.phase = SyncPhase::CatchUp;
        }

        *graph = working_graph;
        *checkpoint = working_checkpoint;
        Ok(SyncBatchResult {
            transaction_id: format!(
                "tx--opencti-sync-{}-{}-{}",
                stable_component(&batch.source_id),
                stable_component(&batch.snapshot_id),
                checkpoint.last_acknowledged_sequence
            ),
            operations: results,
            acknowledged_sequence: checkpoint.last_acknowledged_sequence,
            queue_depth: checkpoint.queue_depth,
        })
    }

    /// Compute deterministic canonical, identifier, relation, and access-policy
    /// checksums without depending on graph allocation order.
    pub fn digest(&self, graph: &Graph) -> Result<GraphDigest, SyncError> {
        let mut canonical = Vec::new();
        let mut properties = Vec::new();
        let mut identifiers = Vec::new();
        let mut relations = Vec::new();
        let mut access = Vec::new();
        let mut object_count = 0_u64;
        let mut relationship_count = 0_u64;

        for node in graph.list_nodes().map_err(graph_error)? {
            let Ok(mapped) = self.adapter.restore_node(&node) else {
                continue;
            };
            object_count = object_count.saturating_add(1);
            push_digest_parts(
                &mapped,
                &mut canonical,
                &mut properties,
                &mut identifiers,
                &mut access,
            )?;
        }
        for relationship in graph.list_relationships().map_err(graph_error)? {
            let Ok(mapped) = self.adapter.restore_relationship(&relationship) else {
                continue;
            };
            relationship_count = relationship_count.saturating_add(1);
            push_digest_parts(
                &mapped,
                &mut canonical,
                &mut properties,
                &mut identifiers,
                &mut access,
            )?;
            if let Some(mapped_relationship) = mapped.as_relationship() {
                relations.push(format!(
                    "{}\u{0}{}\u{0}{}\u{0}{}",
                    mapped.record_ref().canonical_id(),
                    mapped_relationship.source_ref(),
                    mapped_relationship.relationship_type(),
                    mapped_relationship.target_ref()
                ));
            }
        }
        canonical.sort();
        properties.sort();
        identifiers.sort();
        relations.sort();
        access.sort();
        Ok(GraphDigest {
            object_count,
            relationship_count,
            canonical_checksum: hash_parts(&canonical),
            property_checksum: hash_parts(&properties),
            identifier_checksum: hash_parts(&identifiers),
            relation_checksum: hash_parts(&relations),
            access_policy_checksum: hash_parts(&access),
        })
    }

    /// Compare every required parity dimension and gate shadow reads only when
    /// canonical data and derived projections are simultaneously current.
    pub fn validate(
        &self,
        graph: &Graph,
        expected: &GraphDigest,
        projections_fresh: bool,
    ) -> Result<SyncValidationReport, SyncError> {
        let actual = self.digest(graph)?;
        let mut differences = Vec::new();
        if actual.object_count != expected.object_count
            || actual.relationship_count != expected.relationship_count
            || actual.canonical_checksum != expected.canonical_checksum
        {
            differences.push("records".to_owned());
        }
        if actual.property_checksum != expected.property_checksum {
            differences.push("properties".to_owned());
        }
        if actual.identifier_checksum != expected.identifier_checksum {
            differences.push("identifiers".to_owned());
        }
        if actual.relation_checksum != expected.relation_checksum {
            differences.push("relations".to_owned());
        }
        if actual.access_policy_checksum != expected.access_policy_checksum {
            differences.push("access_policy".to_owned());
        }
        if !projections_fresh {
            differences.push("projections".to_owned());
        }
        let shadow_reads_enabled = differences.is_empty();
        Ok(SyncValidationReport {
            divergence: if shadow_reads_enabled {
                DivergenceStatus::InSync
            } else {
                DivergenceStatus::Diverged
            },
            differences,
            shadow_reads_enabled,
            actual,
        })
    }

    fn validate_batch(
        &self,
        checkpoint: &SyncCheckpoint,
        batch: &OpenCtiSyncBatch,
    ) -> Result<(), SyncError> {
        if batch.source_id != checkpoint.source_id || batch.snapshot_id != checkpoint.snapshot_id {
            return Err(SyncError::InvalidInput(
                "batch source_id and snapshot_id must match the durable checkpoint".to_owned(),
            ));
        }
        let replay_only = batch
            .operations
            .iter()
            .all(|operation| operation.sequence <= checkpoint.last_acknowledged_sequence);
        let phase_valid = replay_only
            || checkpoint.phase == batch.phase
            || (checkpoint.phase == SyncPhase::CatchUp && batch.phase == SyncPhase::SteadyState);
        if !phase_valid {
            return Err(SyncError::InvalidInput(format!(
                "phase {:?} cannot follow checkpoint phase {:?}",
                batch.phase, checkpoint.phase
            )));
        }
        if batch.operations.is_empty() {
            return Err(SyncError::InvalidInput(
                "synchronization batch cannot be empty".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug)]
struct GraphIndexes {
    node_by_identifier: BTreeMap<String, NodeId>,
    relationship_by_identifier: BTreeMap<String, RelationshipId>,
}

impl GraphIndexes {
    fn from_graph(graph: &Graph, adapter: &OpenCtiAdapter) -> Result<Self, SyncError> {
        let mut node_by_identifier = BTreeMap::new();
        let mut relationship_by_identifier = BTreeMap::new();
        for node in graph.list_nodes().map_err(graph_error)? {
            let Ok(mapped) = adapter.restore_node(&node) else {
                continue;
            };
            node_by_identifier.insert(
                mapped.record_ref().canonical_id().to_owned(),
                node.id().clone(),
            );
            for identifier in mapped.identifiers() {
                node_by_identifier.insert(identifier.value().to_owned(), node.id().clone());
            }
        }
        for relationship in graph.list_relationships().map_err(graph_error)? {
            let Ok(mapped) = adapter.restore_relationship(&relationship) else {
                continue;
            };
            relationship_by_identifier.insert(
                mapped.record_ref().canonical_id().to_owned(),
                relationship.id().clone(),
            );
            for identifier in mapped.identifiers() {
                relationship_by_identifier
                    .insert(identifier.value().to_owned(), relationship.id().clone());
            }
        }
        Ok(Self {
            node_by_identifier,
            relationship_by_identifier,
        })
    }
}

enum ApplyOperation {
    Applied,
    Duplicate,
    Retryable(String),
}

fn apply_operation(
    graph: &mut Graph,
    indexes: &GraphIndexes,
    adapter: &OpenCtiAdapter,
    operation: &OpenCtiMutation,
) -> Result<ApplyOperation, SyncError> {
    match operation.class {
        MutationClass::Upsert | MutationClass::AccessPolicyUpdate => {
            let mapped = adapter
                .map(operation.record.clone())
                .map_err(|error| SyncError::InvalidInput(error.to_string()))?;
            match mapped {
                MappedRecord::Object(object) => {
                    let existing = indexes
                        .node_by_identifier
                        .get(object.record_ref.canonical_id())
                        .cloned()
                        .or_else(|| {
                            object.identifiers.iter().find_map(|identifier| {
                                indexes.node_by_identifier.get(identifier.value()).cloned()
                            })
                        });
                    if let Some(node_id) = existing {
                        let current =
                            graph
                                .get_node(&node_id)
                                .map_err(graph_error)?
                                .ok_or_else(|| {
                                    SyncError::Graph("indexed node is not current".to_owned())
                                })?;
                        if current.property("opencti.raw")
                            == Some(&PropertyValue::Json(object.raw.clone()))
                        {
                            return Ok(ApplyOperation::Duplicate);
                        }
                        graph
                            .replace_node(&node_id, object.to_node_input())
                            .map_err(graph_error)?;
                    } else {
                        graph
                            .create_node(object.to_node_input())
                            .map_err(graph_error)?;
                    }
                    Ok(ApplyOperation::Applied)
                }
                MappedRecord::Relationship(relationship)
                    if operation.class == MutationClass::AccessPolicyUpdate =>
                {
                    apply_relationship(graph, indexes, relationship)
                }
                MappedRecord::Relationship(_) => Err(SyncError::InvalidInput(
                    "object mutation class cannot contain a relationship".to_owned(),
                )),
            }
        }
        MutationClass::RelationshipUpsert => {
            let mapped = adapter
                .map(operation.record.clone())
                .map_err(|error| SyncError::InvalidInput(error.to_string()))?;
            let MappedRecord::Relationship(relationship) = mapped else {
                return Err(SyncError::InvalidInput(
                    "relationship mutation class requires a relationship record".to_owned(),
                ));
            };
            apply_relationship(graph, indexes, relationship)
        }
        MutationClass::Delete => {
            let canonical_id = canonical_id(&operation.record)?;
            let Some(node_id) = indexes.node_by_identifier.get(canonical_id) else {
                return Ok(ApplyOperation::Duplicate);
            };
            graph.tombstone_node(node_id).map_err(graph_error)?;
            Ok(ApplyOperation::Applied)
        }
        MutationClass::RelationshipDelete => {
            let canonical_id = canonical_id(&operation.record)?;
            let Some(relationship_id) = indexes.relationship_by_identifier.get(canonical_id) else {
                return Ok(ApplyOperation::Duplicate);
            };
            graph
                .tombstone_relationship(relationship_id)
                .map_err(graph_error)?;
            Ok(ApplyOperation::Applied)
        }
    }
}

fn apply_relationship(
    graph: &mut Graph,
    indexes: &GraphIndexes,
    relationship: MappedRelationship,
) -> Result<ApplyOperation, SyncError> {
    let Some(source) = indexes
        .node_by_identifier
        .get(relationship.source_ref())
        .cloned()
    else {
        return Ok(ApplyOperation::Retryable(format!(
            "relationship source {} is not available",
            relationship.source_ref()
        )));
    };
    let Some(target) = indexes
        .node_by_identifier
        .get(relationship.target_ref())
        .cloned()
    else {
        return Ok(ApplyOperation::Retryable(format!(
            "relationship target {} is not available",
            relationship.target_ref()
        )));
    };
    let input = relationship
        .to_relationship_input(source, target)
        .map_err(|error| SyncError::InvalidInput(error.to_string()))?;
    if let Some(relationship_id) = indexes
        .relationship_by_identifier
        .get(relationship.record_ref.canonical_id())
    {
        let current = graph
            .get_relationship(relationship_id)
            .map_err(graph_error)?
            .ok_or_else(|| SyncError::Graph("indexed relationship is not current".to_owned()))?;
        if current.property("opencti.raw") == Some(&PropertyValue::Json(relationship.raw.clone())) {
            return Ok(ApplyOperation::Duplicate);
        }
        graph
            .replace_relationship(relationship_id, input)
            .map_err(graph_error)?;
    } else {
        graph.create_relationship(input).map_err(graph_error)?;
    }
    Ok(ApplyOperation::Applied)
}

fn canonical_id(record: &Value) -> Result<&str, SyncError> {
    record
        .get("internal_id")
        .or_else(|| record.get("id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            SyncError::InvalidInput("delete record requires internal_id or id".to_owned())
        })
}

fn operation_fingerprint(operation: &OpenCtiMutation) -> Result<String, SyncError> {
    let bytes = serde_json::to_vec(&(
        operation.sequence,
        operation.class,
        canonicalize_json(&operation.record),
    ))
    .map_err(|error| SyncError::InvalidInput(error.to_string()))?;
    Ok(hash_bytes(&bytes))
}

fn remember_replay_identity(
    checkpoint: &mut SyncCheckpoint,
    operation_id: String,
    fingerprint: String,
    limit: usize,
) {
    checkpoint
        .replay_identities
        .retain(|(candidate, _)| candidate != &operation_id);
    checkpoint
        .replay_identities
        .push((operation_id, fingerprint));
    if checkpoint.replay_identities.len() > limit {
        let excess = checkpoint.replay_identities.len() - limit;
        checkpoint.replay_identities.drain(0..excess);
    }
}

fn remember_dead_letter(
    checkpoint: &mut SyncCheckpoint,
    operation: &OpenCtiMutation,
    status: OperationStatus,
    diagnostic: &str,
    limit: usize,
) {
    checkpoint.dead_letters.push(DeadLetterRecord {
        operation_id: operation.operation_id.clone(),
        sequence: operation.sequence,
        status,
        diagnostic: diagnostic.to_owned(),
    });
    if checkpoint.dead_letters.len() > limit {
        let excess = checkpoint.dead_letters.len() - limit;
        checkpoint.dead_letters.drain(0..excess);
    }
}

fn operation_result(
    operation: &OpenCtiMutation,
    status: OperationStatus,
    diagnostic: &str,
) -> OperationResult {
    OperationResult {
        operation_id: operation.operation_id.clone(),
        sequence: operation.sequence,
        status,
        diagnostic: Some(diagnostic.to_owned()),
    }
}

fn push_digest_parts(
    mapped: &MappedRecord,
    canonical: &mut Vec<String>,
    properties: &mut Vec<String>,
    identifiers: &mut Vec<String>,
    access: &mut Vec<String>,
) -> Result<(), SyncError> {
    let record_ref = mapped.record_ref();
    canonical.push(record_ref.canonical_id().to_owned());
    properties.push(format!(
        "{}\u{0}{}",
        record_ref.canonical_id(),
        serde_json::to_string(&canonicalize_json(mapped.raw()))
            .map_err(|error| SyncError::InvalidInput(error.to_string()))?
    ));
    for identifier in mapped.identifiers() {
        identifiers.push(format!(
            "{}\u{0}{:?}\u{0}{}",
            record_ref.canonical_id(),
            identifier.kind(),
            identifier.value()
        ));
    }
    access.push(format!(
        "{}\u{0}{}",
        record_ref.canonical_id(),
        serde_json::to_string(mapped.access())
            .map_err(|error| SyncError::InvalidInput(error.to_string()))?
    ));
    Ok(())
}

fn canonicalize_json(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut entries = object.iter().collect::<Vec<_>>();
            entries.sort_by_key(|(key, _)| *key);
            let mut canonical = serde_json::Map::with_capacity(entries.len());
            for (key, value) in entries {
                canonical.insert(key.clone(), canonicalize_json(value));
            }
            Value::Object(canonical)
        }
        Value::Array(values) => Value::Array(values.iter().map(canonicalize_json).collect()),
        other => other.clone(),
    }
}

fn hash_parts(parts: &[String]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    format_digest(hasher.finalize().as_slice())
}

fn hash_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format_digest(digest.as_slice())
}

fn format_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn stable_component(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect()
}

fn graph_error(error: impl ToString) -> SyncError {
    SyncError::Graph(error.to_string())
}

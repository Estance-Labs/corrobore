// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! Deterministic OpenCTI survivor merge and graph-edge deduplication contract.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use graph_core::{Graph, Node, NodeId, Relationship, RelationshipId};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use thiserror::Error;

use crate::{MappedRecord, OpenCtiAdapter};

/// Hard bounds checked before a merge scans or rewrites graph relationships.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeLimits {
    /// Maximum duplicate records accepted by one merge.
    pub max_sources: usize,
    /// Maximum current relationships inspected by one merge.
    pub max_relationships: usize,
}

impl Default for MergeLimits {
    fn default() -> Self {
        Self {
            max_sources: 64,
            max_relationships: 100_000,
        }
    }
}

/// Complete deterministic merge command.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenCtiMergeRequest {
    /// Caller-stable identity used by the durable WAL receipt.
    pub merge_id: String,
    /// Canonical record that survives.
    pub target_id: String,
    /// Canonical duplicate records that are tombstoned after rewiring.
    pub source_ids: Vec<String>,
    /// Optional optimistic revision preconditions keyed by canonical ID.
    pub expected_revisions: BTreeMap<String, u64>,
}

impl OpenCtiMergeRequest {
    /// Validate command identity, survivor/source separation and source uniqueness.
    pub fn new(
        merge_id: impl Into<String>,
        target_id: impl Into<String>,
        source_ids: Vec<String>,
        expected_revisions: BTreeMap<String, u64>,
    ) -> Result<Self, MergeError> {
        let merge_id = merge_id.into();
        let target_id = target_id.into();
        if merge_id.trim().is_empty() || target_id.trim().is_empty() || source_ids.is_empty() {
            return Err(MergeError::InvalidInput(
                "merge_id, target_id and source_ids are required".to_owned(),
            ));
        }
        let mut unique = BTreeSet::new();
        for source_id in &source_ids {
            if source_id.trim().is_empty() {
                return Err(MergeError::InvalidInput(
                    "source_ids cannot contain blank values".to_owned(),
                ));
            }
            if source_id == &target_id {
                return Err(MergeError::InvalidInput(
                    "target_id cannot also be a source_id".to_owned(),
                ));
            }
            if !unique.insert(source_id.as_str()) {
                return Err(MergeError::InvalidInput(format!(
                    "duplicate source_id {source_id}"
                )));
            }
        }
        Ok(Self {
            merge_id,
            target_id,
            source_ids,
            expected_revisions,
        })
    }
}

/// One deterministic scalar conflict resolved in favor of the survivor.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MergeConflict {
    /// JSON path of the conflicting value.
    pub path: String,
    /// Canonical source whose value was retained.
    pub retained_from: String,
    /// Canonical source whose value remains available in provenance history.
    pub discarded_from: String,
    /// Retained value.
    pub retained_value: Value,
    /// Discarded value.
    pub discarded_value: Value,
}

/// Prepared atomic graph transition and observable merge evidence.
#[derive(Clone, Debug)]
pub struct OpenCtiMergeOutcome {
    /// Complete next graph; the caller commits it through one WAL transaction.
    pub graph: Graph,
    /// Whether this invocation prepared a new logical mutation.
    pub applied: bool,
    /// Canonical survivor ID.
    pub target_id: String,
    /// Survivor revision after merge.
    pub target_revision: u64,
    /// Tombstoned duplicate canonical IDs.
    pub deleted_source_ids: Vec<String>,
    /// Relationship canonical IDs whose endpoints changed.
    pub redirected_relationship_ids: Vec<String>,
    /// Object canonical IDs whose embedded STIX references changed.
    pub redirected_reference_ids: Vec<String>,
    /// Relationship canonical IDs tombstoned as duplicate edges.
    pub deduplicated_relationship_ids: Vec<String>,
    /// Deterministic target-wins scalar conflicts.
    pub conflicts: Vec<MergeConflict>,
}

/// Merge planning failures that must not mutate the caller graph.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum MergeError {
    /// Malformed command or incompatible record families.
    #[error("invalid merge input: {0}")]
    InvalidInput(String),
    /// Optimistic revision mismatch or unsafe identity conflict.
    #[error("merge conflict: {0}")]
    Conflict(String),
    /// Configured source or supernode bound exceeded.
    #[error("merge limit exceeded: {0}")]
    LimitExceeded(String),
    /// Graph or mapping operation failed.
    #[error("merge graph failure: {0}")]
    Graph(String),
    /// Phase-2 API shape exists but behavior is not implemented yet.
    #[error("merge implementation is not available")]
    NotImplemented,
}

/// Stateless merge planner; durability and idempotent receipts belong to the host.
#[derive(Clone, Debug)]
pub struct OpenCtiMergeExecutor {
    limits: MergeLimits,
}

impl OpenCtiMergeExecutor {
    /// Configure bounded duplicate and relationship processing.
    pub const fn new(limits: MergeLimits) -> Self {
        Self { limits }
    }

    /// Plan the full merge on a clone. The implementation validates revisions,
    /// unions identifier/security/provenance arrays, rewires all affected edges,
    /// removes deterministic duplicate edges, tombstones sources, and returns a
    /// single graph image suitable for one WAL-backed atomic commit.
    pub fn apply(
        &self,
        graph: &Graph,
        request: &OpenCtiMergeRequest,
    ) -> Result<OpenCtiMergeOutcome, MergeError> {
        if request.source_ids.len() > self.limits.max_sources {
            return Err(MergeError::LimitExceeded(format!(
                "max_sources is {}, received {}",
                self.limits.max_sources,
                request.source_ids.len()
            )));
        }
        let relationships = graph.list_relationships().map_err(graph_error)?;
        if relationships.len() > self.limits.max_relationships {
            return Err(MergeError::LimitExceeded(format!(
                "max_relationships is {}, received {}",
                self.limits.max_relationships,
                relationships.len()
            )));
        }

        let adapter = OpenCtiAdapter::pinned();
        let index = RecordIndex::build(graph, &adapter)?;
        let target = index.node(request.target_id.as_str())?;
        validate_revision(request, &target.canonical_id, target.node.version())?;
        let mut sources = request
            .source_ids
            .iter()
            .map(|source_id| index.node(source_id))
            .collect::<Result<Vec<_>, _>>()?;
        sources.sort_by(|left, right| left.canonical_id.cmp(&right.canonical_id));
        for source in &sources {
            validate_revision(request, &source.canonical_id, source.node.version())?;
            if source.entity_type != target.entity_type {
                return Err(MergeError::InvalidInput(format!(
                    "source {} has entity type {}, expected {}",
                    source.canonical_id, source.entity_type, target.entity_type
                )));
            }
        }

        let mut next = graph.clone();
        let mut merged_raw = target.raw.clone();
        let mut conflicts = Vec::new();
        let mut source_history = Vec::with_capacity(sources.len());
        for source in &sources {
            merge_json_object(
                &mut merged_raw,
                &source.raw,
                "",
                &target.canonical_id,
                &source.canonical_id,
                &mut conflicts,
            );
            source_history.push(json!({
                "id": source.canonical_id,
                "revision": source.node.version(),
                "record": source.raw,
            }));
        }
        let source_canonical_ids = sources
            .iter()
            .map(|source| source.canonical_id.clone())
            .collect::<HashSet<_>>();
        redirect_reference_values(&mut merged_raw, &source_canonical_ids, &target.canonical_id);
        let source_ids = sources
            .iter()
            .map(|source| source.canonical_id.clone())
            .collect::<Vec<_>>();
        union_string_field(&mut merged_raw, "x_opencti_stix_ids", &source_ids)?;
        if let Some(object) = merged_raw.as_object_mut() {
            object.insert(
                "x_corrobore_merged_sources".to_owned(),
                Value::Array(source_history),
            );
            object.insert(
                "x_corrobore_merge_conflicts".to_owned(),
                serde_json::to_value(&conflicts)
                    .map_err(|error| MergeError::Graph(error.to_string()))?,
            );
            object.insert("id".to_owned(), Value::String(target.canonical_id.clone()));
        }
        let mapped_target = adapter.map(merged_raw).map_err(mapping_error)?;
        let MappedRecord::Object(mapped_target) = mapped_target else {
            return Err(MergeError::InvalidInput(
                "merge survivor must remain an object".to_owned(),
            ));
        };
        next.replace_node(&target.node_id, mapped_target.to_node_input())
            .map_err(graph_error)?;

        let mut redirected_reference_ids = redirect_object_references(
            &mut next,
            &index,
            &adapter,
            &source_canonical_ids,
            &target.canonical_id,
        )?;

        let source_graph_ids = sources
            .iter()
            .map(|source| source.node_id.clone())
            .collect::<HashSet<_>>();
        let relationship_plans = plan_relationships(
            &relationships,
            &adapter,
            &source_graph_ids,
            &target.node_id,
            request.target_id.as_str(),
            &mut conflicts,
        )?;
        let mut redirected_relationship_ids = Vec::new();
        let mut deduplicated_relationship_ids = Vec::new();
        for plan in relationship_plans {
            if plan.duplicate {
                next.tombstone_relationship(&plan.relationship_id)
                    .map_err(graph_error)?;
                deduplicated_relationship_ids.push(plan.canonical_id);
            } else if let Some(input) = plan.replacement {
                next.replace_relationship(&plan.relationship_id, input)
                    .map_err(graph_error)?;
                redirected_relationship_ids.push(plan.canonical_id);
            }
        }
        for source in &sources {
            next.tombstone_node(&source.node_id).map_err(graph_error)?;
        }

        redirected_relationship_ids.sort();
        redirected_reference_ids.sort();
        deduplicated_relationship_ids.sort();
        let target_revision = next
            .get_node(&target.node_id)
            .map_err(graph_error)?
            .ok_or_else(|| MergeError::Graph("merged survivor is not current".to_owned()))?
            .version();
        Ok(OpenCtiMergeOutcome {
            graph: next,
            applied: true,
            target_id: target.canonical_id,
            target_revision,
            deleted_source_ids: source_ids,
            redirected_relationship_ids,
            redirected_reference_ids,
            deduplicated_relationship_ids,
            conflicts,
        })
    }
}

#[derive(Clone)]
struct IndexedNode {
    canonical_id: String,
    entity_type: String,
    raw: Value,
    node_id: NodeId,
    node: Node,
}

struct RecordIndex {
    nodes: BTreeMap<String, IndexedNode>,
}

impl RecordIndex {
    fn build(graph: &Graph, adapter: &OpenCtiAdapter) -> Result<Self, MergeError> {
        let mut nodes = BTreeMap::new();
        for node in graph.list_nodes().map_err(graph_error)? {
            let mapped = adapter.restore_node(&node).map_err(mapping_error)?;
            let canonical_id = mapped.record_ref().canonical_id().to_owned();
            let entity_type = mapped
                .raw()
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let indexed = IndexedNode {
                canonical_id: canonical_id.clone(),
                entity_type,
                raw: mapped.raw().clone(),
                node_id: node.id().clone(),
                node,
            };
            nodes.insert(canonical_id, indexed.clone());
            for identifier in mapped.identifiers() {
                nodes
                    .entry(identifier.value().to_owned())
                    .or_insert_with(|| indexed.clone());
            }
        }
        Ok(Self { nodes })
    }

    fn node(&self, identifier: &str) -> Result<IndexedNode, MergeError> {
        self.nodes.get(identifier).cloned().ok_or_else(|| {
            MergeError::InvalidInput(format!("merge record {identifier} was not found"))
        })
    }
}

struct RelationshipPlan {
    relationship_id: RelationshipId,
    canonical_id: String,
    replacement: Option<graph_core::RelationshipInput>,
    duplicate: bool,
}

fn plan_relationships(
    relationships: &[Relationship],
    adapter: &OpenCtiAdapter,
    source_graph_ids: &HashSet<NodeId>,
    target_graph_id: &NodeId,
    target_canonical_id: &str,
    conflicts: &mut Vec<MergeConflict>,
) -> Result<Vec<RelationshipPlan>, MergeError> {
    struct Candidate {
        relationship_id: RelationshipId,
        canonical_id: String,
        key: (String, String, String),
        source: NodeId,
        target: NodeId,
        raw: Value,
        changed: bool,
    }
    let mut candidates = Vec::with_capacity(relationships.len());
    for relationship in relationships {
        let mapped = adapter
            .restore_relationship(relationship)
            .map_err(mapping_error)?;
        let canonical_id = mapped.record_ref().canonical_id().to_owned();
        let source_changed = source_graph_ids.contains(relationship.source());
        let target_changed = source_graph_ids.contains(relationship.target());
        let source = if source_changed {
            target_graph_id.clone()
        } else {
            relationship.source().clone()
        };
        let target = if target_changed {
            target_graph_id.clone()
        } else {
            relationship.target().clone()
        };
        let mut raw = mapped.raw().clone();
        if let Some(object) = raw.as_object_mut() {
            if source_changed {
                object.insert(
                    "source_ref".to_owned(),
                    Value::String(target_canonical_id.to_owned()),
                );
            }
            if target_changed {
                object.insert(
                    "target_ref".to_owned(),
                    Value::String(target_canonical_id.to_owned()),
                );
            }
        }
        candidates.push(Candidate {
            relationship_id: relationship.id().clone(),
            canonical_id,
            key: (
                source.as_str().to_owned(),
                target.as_str().to_owned(),
                relationship.rel_type().as_str().to_owned(),
            ),
            source,
            target,
            raw,
            changed: source_changed || target_changed,
        });
    }
    candidates.sort_by(|left, right| left.canonical_id.cmp(&right.canonical_id));
    let mut groups = BTreeMap::<(String, String, String), Vec<Candidate>>::new();
    for candidate in candidates {
        groups
            .entry(candidate.key.clone())
            .or_default()
            .push(candidate);
    }
    let mut plans = Vec::new();
    for (_, mut group) in groups {
        if !group.iter().any(|candidate| candidate.changed) {
            plans.extend(group.into_iter().map(|candidate| RelationshipPlan {
                relationship_id: candidate.relationship_id,
                canonical_id: candidate.canonical_id,
                replacement: None,
                duplicate: false,
            }));
            continue;
        }

        let retained = group.remove(0);
        let mut merged_raw = retained.raw.clone();
        for duplicate in &group {
            merge_json_object(
                &mut merged_raw,
                &duplicate.raw,
                "",
                &retained.canonical_id,
                &duplicate.canonical_id,
                conflicts,
            );
        }
        if let Some(object) = merged_raw.as_object_mut() {
            object.insert(
                "id".to_owned(),
                Value::String(retained.canonical_id.clone()),
            );
        }
        let mapped = adapter.map(merged_raw).map_err(mapping_error)?;
        let MappedRecord::Relationship(mapped) = mapped else {
            return Err(MergeError::Graph(
                "rewired relationship mapped as object".to_owned(),
            ));
        };
        plans.push(RelationshipPlan {
            relationship_id: retained.relationship_id,
            canonical_id: retained.canonical_id,
            replacement: Some(
                mapped
                    .to_relationship_input(retained.source, retained.target)
                    .map_err(mapping_error)?,
            ),
            duplicate: false,
        });
        plans.extend(group.into_iter().map(|candidate| RelationshipPlan {
            relationship_id: candidate.relationship_id,
            canonical_id: candidate.canonical_id,
            replacement: None,
            duplicate: true,
        }));
    }
    plans.sort_by(|left, right| left.canonical_id.cmp(&right.canonical_id));
    Ok(plans)
}

fn redirect_object_references(
    graph: &mut Graph,
    index: &RecordIndex,
    adapter: &OpenCtiAdapter,
    source_ids: &HashSet<String>,
    target_id: &str,
) -> Result<Vec<String>, MergeError> {
    let mut seen = HashSet::new();
    let mut redirected = Vec::new();
    for indexed in index.nodes.values() {
        if !seen.insert(indexed.node_id.clone())
            || indexed.canonical_id == target_id
            || source_ids.contains(&indexed.canonical_id)
        {
            continue;
        }
        let mut raw = indexed.raw.clone();
        if !redirect_reference_values(&mut raw, source_ids, target_id) {
            continue;
        }
        let mapped = adapter.map(raw).map_err(mapping_error)?;
        let MappedRecord::Object(mapped) = mapped else {
            return Err(MergeError::Graph(
                "reference-bearing object mapped as relationship".to_owned(),
            ));
        };
        graph
            .replace_node(&indexed.node_id, mapped.to_node_input())
            .map_err(graph_error)?;
        redirected.push(indexed.canonical_id.clone());
    }
    Ok(redirected)
}

fn redirect_reference_values(
    value: &mut Value,
    source_ids: &HashSet<String>,
    target_id: &str,
) -> bool {
    match value {
        Value::Object(object) => {
            let mut changed = false;
            for (key, value) in object {
                if key.ends_with("_ref") {
                    if value
                        .as_str()
                        .is_some_and(|reference| source_ids.contains(reference))
                    {
                        *value = Value::String(target_id.to_owned());
                        changed = true;
                    }
                } else if key.ends_with("_refs") {
                    if let Some(references) = value.as_array_mut() {
                        for reference in references {
                            if reference
                                .as_str()
                                .is_some_and(|reference| source_ids.contains(reference))
                            {
                                *reference = Value::String(target_id.to_owned());
                                changed = true;
                            }
                        }
                    }
                } else if key != "x_corrobore_merged_sources" {
                    changed |= redirect_reference_values(value, source_ids, target_id);
                }
            }
            changed
        }
        Value::Array(values) => values.iter_mut().fold(false, |changed, value| {
            changed | redirect_reference_values(value, source_ids, target_id)
        }),
        _ => false,
    }
}

fn validate_revision(
    request: &OpenCtiMergeRequest,
    canonical_id: &str,
    actual: u64,
) -> Result<(), MergeError> {
    if request
        .expected_revisions
        .get(canonical_id)
        .is_some_and(|expected| *expected != actual)
    {
        return Err(MergeError::Conflict(format!(
            "expected revision for {canonical_id} does not match current revision {actual}"
        )));
    }
    Ok(())
}

fn merge_json_object(
    target: &mut Value,
    source: &Value,
    path: &str,
    target_id: &str,
    source_id: &str,
    conflicts: &mut Vec<MergeConflict>,
) {
    let (Value::Object(target), Value::Object(source)) = (target, source) else {
        return;
    };
    for (key, source_value) in source {
        if matches!(key.as_str(), "id" | "internal_id" | "type" | "parent_types") {
            continue;
        }
        let child_path = if path.is_empty() {
            format!("/{key}")
        } else {
            format!("{path}/{key}")
        };
        match target.get_mut(key) {
            None => {
                target.insert(key.clone(), source_value.clone());
            }
            Some(target_value) if target_value.is_null() => {
                *target_value = source_value.clone();
            }
            Some(Value::Array(target_values)) if source_value.is_array() => {
                let source_values = source_value.as_array().expect("checked array");
                union_json_arrays(target_values, source_values);
            }
            Some(Value::Object(target_object)) if source_value.is_object() => {
                let mut nested_target = Value::Object(std::mem::take(target_object));
                merge_json_object(
                    &mut nested_target,
                    source_value,
                    &child_path,
                    target_id,
                    source_id,
                    conflicts,
                );
                *target_object = nested_target.as_object().cloned().unwrap_or_default();
            }
            Some(target_value) if target_value != source_value => {
                conflicts.push(MergeConflict {
                    path: child_path,
                    retained_from: target_id.to_owned(),
                    discarded_from: source_id.to_owned(),
                    retained_value: target_value.clone(),
                    discarded_value: source_value.clone(),
                });
            }
            Some(_) => {}
        }
    }
}

fn union_json_arrays(target: &mut Vec<Value>, source: &[Value]) {
    target.extend(source.iter().cloned());
    target.sort_by_key(canonical_json);
    target.dedup_by(|left, right| canonical_json(left) == canonical_json(right));
}

fn union_string_field(
    target: &mut Value,
    field: &str,
    values: &[String],
) -> Result<(), MergeError> {
    let object = target
        .as_object_mut()
        .ok_or_else(|| MergeError::InvalidInput("merge records must be objects".to_owned()))?;
    let target_values = object
        .entry(field.to_owned())
        .or_insert_with(|| Value::Array(Vec::new()));
    let array = target_values
        .as_array_mut()
        .ok_or_else(|| MergeError::InvalidInput(format!("merge field {field} must be an array")))?;
    union_json_arrays(
        array,
        &values
            .iter()
            .cloned()
            .map(Value::String)
            .collect::<Vec<_>>(),
    );
    Ok(())
}

fn canonical_json(value: &Value) -> String {
    fn canonicalize(value: &Value) -> Value {
        match value {
            Value::Object(object) => {
                let mut entries = object.iter().collect::<Vec<_>>();
                entries.sort_by(|left, right| left.0.cmp(right.0));
                Value::Object(
                    entries
                        .into_iter()
                        .map(|(key, value)| (key.clone(), canonicalize(value)))
                        .collect::<Map<_, _>>(),
                )
            }
            Value::Array(values) => Value::Array(values.iter().map(canonicalize).collect()),
            scalar => scalar.clone(),
        }
    }
    serde_json::to_string(&canonicalize(value)).unwrap_or_default()
}

fn graph_error(error: graph_core::GraphError) -> MergeError {
    MergeError::Graph(error.to_string())
}

fn mapping_error(error: crate::MappingError) -> MergeError {
    MergeError::InvalidInput(error.to_string())
}

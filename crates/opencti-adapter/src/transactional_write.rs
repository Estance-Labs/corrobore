// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! Transactional OpenCTI create, update, delete, relationship, access-policy,
//! and bounded bulk mutation planning over the generic graph model.

use std::collections::{BTreeMap, BTreeSet};

use graph_core::{Graph, NodeId, RelationshipId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::{MappedRecord, OpenCtiAdapter};

/// Hard request limits applied before any graph mutation is attempted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteLimits {
    /// Maximum ordered items accepted by one transaction.
    pub max_operations: usize,
    /// Maximum serialized operation bytes accepted by one transaction.
    pub max_payload_bytes: usize,
}

impl Default for WriteLimits {
    fn default() -> Self {
        Self {
            max_operations: 512,
            max_payload_bytes: 8 * 1024 * 1024,
        }
    }
}

/// One direct mutation class independent of any Elasticsearch naming.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum OpenCtiWriteOperationKind {
    /// Insert a new object or relationship.
    Create {
        /// Lossless OpenCTI record.
        record: Value,
    },
    /// Merge a provider-neutral patch into one current record.
    Update {
        /// Stable OpenCTI identifier.
        id: String,
        /// Required current revision when supplied.
        expected_revision: Option<u64>,
        /// JSON object patch merged into the lossless record.
        patch: Value,
    },
    /// Tombstone one current object or relationship.
    Delete {
        /// Stable OpenCTI identifier.
        id: String,
        /// Required current revision when supplied.
        expected_revision: Option<u64>,
    },
}

/// One ordered item inside a transactional write batch.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OpenCtiWriteOperation {
    /// Caller-stable identity used in per-item results and audit.
    pub operation_id: String,
    /// Typed mutation payload.
    pub mutation: OpenCtiWriteOperationKind,
}

impl OpenCtiWriteOperation {
    /// Construct a create item.
    pub fn create(operation_id: impl Into<String>, record: Value) -> Self {
        Self {
            operation_id: operation_id.into(),
            mutation: OpenCtiWriteOperationKind::Create { record },
        }
    }

    /// Construct an update item.
    pub fn update(
        operation_id: impl Into<String>,
        id: impl Into<String>,
        expected_revision: Option<u64>,
        patch: Value,
    ) -> Self {
        Self {
            operation_id: operation_id.into(),
            mutation: OpenCtiWriteOperationKind::Update {
                id: id.into(),
                expected_revision,
                patch,
            },
        }
    }

    /// Construct a delete item.
    pub fn delete(
        operation_id: impl Into<String>,
        id: impl Into<String>,
        expected_revision: Option<u64>,
    ) -> Self {
        Self {
            operation_id: operation_id.into(),
            mutation: OpenCtiWriteOperationKind::Delete {
                id: id.into(),
                expected_revision,
            },
        }
    }
}

/// Ordered transaction envelope with explicit atomic or partial policy.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OpenCtiWriteBatch {
    /// Stable transaction identity derived from a hashed idempotency key.
    pub transaction_id: String,
    /// Whether any item failure aborts every otherwise valid item.
    pub atomic: bool,
    /// Ordered mutation items.
    pub operations: Vec<OpenCtiWriteOperation>,
}

impl OpenCtiWriteBatch {
    /// Validate envelope identity and non-empty operation content.
    pub fn new(
        transaction_id: impl Into<String>,
        atomic: bool,
        operations: Vec<OpenCtiWriteOperation>,
    ) -> Result<Self, WriteError> {
        let transaction_id = transaction_id.into();
        if transaction_id.trim().is_empty() {
            return Err(WriteError::InvalidInput(
                "transaction_id cannot be blank".to_owned(),
            ));
        }
        if operations.is_empty() {
            return Err(WriteError::InvalidInput(
                "write batch cannot be empty".to_owned(),
            ));
        }
        Ok(Self {
            transaction_id,
            atomic,
            operations,
        })
    }
}

/// Stable per-item outcome taxonomy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteOperationStatus {
    /// The canonical graph changed.
    Applied,
    /// The operation lost an optimistic concurrency race.
    Conflict,
    /// The payload is permanently invalid.
    Rejected,
    /// A dependency such as a relationship endpoint is not available yet.
    Retryable,
    /// A valid item was rolled back because another atomic item failed.
    Aborted,
}

/// Observable result for one ordered mutation item.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteOperationOutcome {
    /// Caller-stable operation identity.
    pub operation_id: String,
    /// Stable record identifier when it could be resolved safely.
    pub id: Option<String>,
    /// Classified outcome.
    pub status: WriteOperationStatus,
    /// Revision observed before evaluation.
    pub before_revision: Option<u64>,
    /// Revision produced by an applied mutation.
    pub after_revision: Option<u64>,
    /// Payload-free deterministic diagnostic.
    pub diagnostic: Option<String>,
}

/// Prepared graph transition and ordered results for one transaction.
#[derive(Clone, Debug)]
pub struct OpenCtiWriteBatchOutcome {
    /// Stable transaction identity.
    pub transaction_id: String,
    /// Whether at least one mutation is ready for durable commit.
    pub committed: bool,
    /// Complete next graph or unchanged input graph after atomic abort.
    pub graph: Graph,
    /// Ordered per-item results.
    pub operations: Vec<WriteOperationOutcome>,
}

/// Direct transactional mutation planning failure.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum WriteError {
    /// Malformed envelope or operation.
    #[error("invalid write input: {0}")]
    InvalidInput(String),
    /// Configured request bound was exceeded.
    #[error("write limit exceeded: {0}")]
    LimitExceeded(String),
    /// Graph mapping or mutation failed unexpectedly.
    #[error("write graph failure: {0}")]
    Graph(String),
}

/// Stateless transactional planner using the pinned OpenCTI mapping contract.
#[derive(Clone, Debug)]
pub struct OpenCtiWriteExecutor {
    limits: WriteLimits,
}

impl OpenCtiWriteExecutor {
    /// Create an executor with explicit bulk and memory limits.
    pub const fn new(limits: WriteLimits) -> Self {
        Self { limits }
    }

    /// Validate and apply operations to a clone, preserving input state on
    /// atomic failure. The implementation rebuilds identifier lookups after
    /// each applied item so relationship endpoints and revisions observe the
    /// transaction's ordered prefix.
    pub fn apply(
        &self,
        graph: &Graph,
        batch: &OpenCtiWriteBatch,
    ) -> Result<OpenCtiWriteBatchOutcome, WriteError> {
        self.validate_batch(batch)?;
        let original = graph.clone();
        let mut working = graph.clone();
        let mut outcomes = Vec::with_capacity(batch.operations.len());
        let mut atomic_failed = false;

        for operation in &batch.operations {
            if atomic_failed {
                outcomes.push(aborted(operation, "atomic transaction already failed"));
                continue;
            }
            // Each graph primitive validates before replacing its immutable
            // current version, so one rejected item cannot leave a partial
            // mutation. The transaction-level original clone remains the
            // rollback image for atomic batches without an O(items * graph)
            // cloning penalty.
            let outcome = apply_one(&mut working, operation);
            if outcome.status != WriteOperationStatus::Applied && batch.atomic {
                atomic_failed = true;
            }
            outcomes.push(outcome);
        }

        if atomic_failed {
            for outcome in &mut outcomes {
                if outcome.status == WriteOperationStatus::Applied {
                    outcome.status = WriteOperationStatus::Aborted;
                    outcome.after_revision = None;
                    outcome.diagnostic = Some("atomic transaction was rolled back".to_owned());
                }
            }
            return Ok(OpenCtiWriteBatchOutcome {
                transaction_id: batch.transaction_id.clone(),
                committed: false,
                graph: original,
                operations: outcomes,
            });
        }

        let committed = outcomes
            .iter()
            .any(|outcome| outcome.status == WriteOperationStatus::Applied);
        Ok(OpenCtiWriteBatchOutcome {
            transaction_id: batch.transaction_id.clone(),
            committed,
            graph: working,
            operations: outcomes,
        })
    }

    fn validate_batch(&self, batch: &OpenCtiWriteBatch) -> Result<(), WriteError> {
        if batch.operations.len() > self.limits.max_operations {
            return Err(WriteError::LimitExceeded(format!(
                "max_operations is {}, received {}",
                self.limits.max_operations,
                batch.operations.len()
            )));
        }
        let payload_bytes = serde_json::to_vec(&batch.operations)
            .map_err(|error| WriteError::InvalidInput(error.to_string()))?
            .len();
        if payload_bytes > self.limits.max_payload_bytes {
            return Err(WriteError::LimitExceeded(format!(
                "max_payload_bytes is {}, received {payload_bytes}",
                self.limits.max_payload_bytes
            )));
        }
        let mut operation_ids = BTreeSet::new();
        for operation in &batch.operations {
            if operation.operation_id.trim().is_empty() {
                return Err(WriteError::InvalidInput(
                    "operation_id cannot be blank".to_owned(),
                ));
            }
            if !operation_ids.insert(operation.operation_id.as_str()) {
                return Err(WriteError::InvalidInput(format!(
                    "duplicate operation_id {}",
                    operation.operation_id
                )));
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
enum RecordLocation {
    Node(NodeId),
    Relationship(RelationshipId),
}

struct RecordIndex {
    by_identifier: BTreeMap<String, RecordLocation>,
}

impl RecordIndex {
    fn build(graph: &Graph, adapter: &OpenCtiAdapter) -> Result<Self, WriteError> {
        let mut by_identifier = BTreeMap::new();
        for node in graph.list_nodes().map_err(graph_error)? {
            let mapped = adapter.restore_node(&node).map_err(mapping_error)?;
            let location = RecordLocation::Node(node.id().clone());
            by_identifier.insert(
                mapped.record_ref().canonical_id().to_owned(),
                location.clone(),
            );
            for identifier in mapped.identifiers() {
                by_identifier.insert(identifier.value().to_owned(), location.clone());
            }
        }
        for relationship in graph.list_relationships().map_err(graph_error)? {
            let mapped = adapter
                .restore_relationship(&relationship)
                .map_err(mapping_error)?;
            let location = RecordLocation::Relationship(relationship.id().clone());
            by_identifier.insert(
                mapped.record_ref().canonical_id().to_owned(),
                location.clone(),
            );
            for identifier in mapped.identifiers() {
                by_identifier.insert(identifier.value().to_owned(), location.clone());
            }
        }
        Ok(Self { by_identifier })
    }

    fn get(&self, id: &str) -> Option<&RecordLocation> {
        self.by_identifier.get(id)
    }
}

fn apply_one(graph: &mut Graph, operation: &OpenCtiWriteOperation) -> WriteOperationOutcome {
    let adapter = OpenCtiAdapter::pinned();
    let index = match RecordIndex::build(graph, &adapter) {
        Ok(index) => index,
        Err(error) => return rejected(operation, None, error.to_string()),
    };
    match &operation.mutation {
        OpenCtiWriteOperationKind::Create { record } => {
            apply_create(graph, &index, &adapter, operation, record)
        }
        OpenCtiWriteOperationKind::Update {
            id,
            expected_revision,
            patch,
        } => apply_update(
            graph,
            &index,
            &adapter,
            operation,
            id,
            *expected_revision,
            patch,
        ),
        OpenCtiWriteOperationKind::Delete {
            id,
            expected_revision,
        } => apply_delete(graph, &index, operation, id, *expected_revision),
    }
}

fn apply_create(
    graph: &mut Graph,
    index: &RecordIndex,
    adapter: &OpenCtiAdapter,
    operation: &OpenCtiWriteOperation,
    record: &Value,
) -> WriteOperationOutcome {
    let mapped = match adapter.map(record.clone()) {
        Ok(mapped) => mapped,
        Err(error) => return rejected(operation, canonical_id(record), error.to_string()),
    };
    let id = mapped.record_ref().canonical_id().to_owned();
    if index.get(&id).is_some()
        || mapped
            .identifiers()
            .iter()
            .any(|identifier| index.get(identifier.value()).is_some())
    {
        return conflict(operation, Some(id), None, "record already exists");
    }
    match mapped {
        MappedRecord::Object(object) => match graph.create_node(object.to_node_input()) {
            Ok(node_id) => match current_node_revision(graph, &node_id) {
                Ok(revision) => applied(operation, id, None, revision),
                Err(error) => rejected(operation, Some(id), error.to_string()),
            },
            Err(error) => rejected(operation, Some(id), error.to_string()),
        },
        MappedRecord::Relationship(relationship) => {
            let Some(RecordLocation::Node(source)) = index.get(relationship.source_ref()) else {
                return retryable(
                    operation,
                    Some(id),
                    format!(
                        "relationship source {} is not available",
                        relationship.source_ref()
                    ),
                );
            };
            let Some(RecordLocation::Node(target)) = index.get(relationship.target_ref()) else {
                return retryable(
                    operation,
                    Some(id),
                    format!(
                        "relationship target {} is not available",
                        relationship.target_ref()
                    ),
                );
            };
            let input = match relationship.to_relationship_input(source.clone(), target.clone()) {
                Ok(input) => input,
                Err(error) => return rejected(operation, Some(id), error.to_string()),
            };
            match graph.create_relationship(input) {
                Ok(relationship_id) => match current_relationship_revision(graph, &relationship_id)
                {
                    Ok(revision) => applied(operation, id, None, revision),
                    Err(error) => rejected(operation, Some(id), error.to_string()),
                },
                Err(error) => rejected(operation, Some(id), error.to_string()),
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_update(
    graph: &mut Graph,
    index: &RecordIndex,
    adapter: &OpenCtiAdapter,
    operation: &OpenCtiWriteOperation,
    id: &str,
    expected_revision: Option<u64>,
    patch: &Value,
) -> WriteOperationOutcome {
    if id.trim().is_empty() || !patch.is_object() {
        return rejected(
            operation,
            canonical_id_from_str(id),
            "update requires a non-blank id and JSON object patch".to_owned(),
        );
    }
    let Some(location) = index.get(id).cloned() else {
        return rejected(
            operation,
            Some(id.to_owned()),
            "record was not found".to_owned(),
        );
    };
    let (before, raw, relationship_endpoints) = match &location {
        RecordLocation::Node(node_id) => {
            let node = match graph.get_node(node_id) {
                Ok(Some(node)) => node,
                Ok(None) => {
                    return rejected(
                        operation,
                        Some(id.to_owned()),
                        "record was not current".to_owned(),
                    );
                }
                Err(error) => {
                    return rejected(operation, Some(id.to_owned()), error.to_string());
                }
            };
            let mapped = match adapter.restore_node(&node) {
                Ok(mapped) => mapped,
                Err(error) => return rejected(operation, Some(id.to_owned()), error.to_string()),
            };
            (node.version(), mapped.raw().clone(), None)
        }
        RecordLocation::Relationship(relationship_id) => {
            let relationship = match graph.get_relationship(relationship_id) {
                Ok(Some(relationship)) => relationship,
                Ok(None) => {
                    return rejected(
                        operation,
                        Some(id.to_owned()),
                        "record was not current".to_owned(),
                    );
                }
                Err(error) => {
                    return rejected(operation, Some(id.to_owned()), error.to_string());
                }
            };
            let mapped = match adapter.restore_relationship(&relationship) {
                Ok(mapped) => mapped,
                Err(error) => return rejected(operation, Some(id.to_owned()), error.to_string()),
            };
            let Some(mapped_relationship) = mapped.as_relationship() else {
                return rejected(
                    operation,
                    Some(id.to_owned()),
                    "stored relationship restored as an object".to_owned(),
                );
            };
            (
                relationship.version(),
                mapped.raw().clone(),
                Some((
                    mapped_relationship.source_ref().to_owned(),
                    mapped_relationship.target_ref().to_owned(),
                )),
            )
        }
    };
    if expected_revision.is_some_and(|expected| expected != before) {
        return conflict(
            operation,
            Some(id.to_owned()),
            Some(before),
            "expected_revision does not match current revision",
        );
    }
    let mut next_raw = raw;
    merge_patch(&mut next_raw, patch);
    if let Some(object) = next_raw.as_object_mut() {
        object.insert("id".to_owned(), Value::String(id.to_owned()));
    }
    let mapped = match adapter.map(next_raw) {
        Ok(mapped) => mapped,
        Err(error) => return rejected(operation, Some(id.to_owned()), error.to_string()),
    };
    match (location, mapped) {
        (RecordLocation::Node(node_id), MappedRecord::Object(object)) => {
            match graph.replace_node(&node_id, object.to_node_input()) {
                Ok(node_id) => match current_node_revision(graph, &node_id) {
                    Ok(revision) => applied(operation, id.to_owned(), Some(before), revision),
                    Err(error) => rejected(operation, Some(id.to_owned()), error.to_string()),
                },
                Err(error) => rejected(operation, Some(id.to_owned()), error.to_string()),
            }
        }
        (
            RecordLocation::Relationship(relationship_id),
            MappedRecord::Relationship(relationship),
        ) => {
            if relationship_endpoints.as_ref()
                != Some(&(
                    relationship.source_ref().to_owned(),
                    relationship.target_ref().to_owned(),
                ))
            {
                return rejected(
                    operation,
                    Some(id.to_owned()),
                    "relationship endpoint movement is delivered by issue #51".to_owned(),
                );
            }
            let Some(RecordLocation::Node(source)) = index.get(relationship.source_ref()) else {
                return retryable(
                    operation,
                    Some(id.to_owned()),
                    "relationship source is not available".to_owned(),
                );
            };
            let Some(RecordLocation::Node(target)) = index.get(relationship.target_ref()) else {
                return retryable(
                    operation,
                    Some(id.to_owned()),
                    "relationship target is not available".to_owned(),
                );
            };
            let input = match relationship.to_relationship_input(source.clone(), target.clone()) {
                Ok(input) => input,
                Err(error) => {
                    return rejected(operation, Some(id.to_owned()), error.to_string());
                }
            };
            match graph.replace_relationship(&relationship_id, input) {
                Ok(relationship_id) => {
                    match current_relationship_revision(graph, &relationship_id) {
                        Ok(revision) => applied(operation, id.to_owned(), Some(before), revision),
                        Err(error) => rejected(operation, Some(id.to_owned()), error.to_string()),
                    }
                }
                Err(error) => rejected(operation, Some(id.to_owned()), error.to_string()),
            }
        }
        _ => rejected(
            operation,
            Some(id.to_owned()),
            "record category cannot change during update".to_owned(),
        ),
    }
}

fn apply_delete(
    graph: &mut Graph,
    index: &RecordIndex,
    operation: &OpenCtiWriteOperation,
    id: &str,
    expected_revision: Option<u64>,
) -> WriteOperationOutcome {
    let Some(location) = index.get(id) else {
        return rejected(
            operation,
            Some(id.to_owned()),
            "record was not found".to_owned(),
        );
    };
    let before = match location {
        RecordLocation::Node(node_id) => match graph.get_node(node_id) {
            Ok(Some(node)) => node.version(),
            Ok(None) => {
                return rejected(
                    operation,
                    Some(id.to_owned()),
                    "record was not current".to_owned(),
                );
            }
            Err(error) => return rejected(operation, Some(id.to_owned()), error.to_string()),
        },
        RecordLocation::Relationship(relationship_id) => {
            match graph.get_relationship(relationship_id) {
                Ok(Some(relationship)) => relationship.version(),
                Ok(None) => {
                    return rejected(
                        operation,
                        Some(id.to_owned()),
                        "record was not current".to_owned(),
                    );
                }
                Err(error) => return rejected(operation, Some(id.to_owned()), error.to_string()),
            }
        }
    };
    if expected_revision.is_some_and(|expected| expected != before) {
        return conflict(
            operation,
            Some(id.to_owned()),
            Some(before),
            "expected_revision does not match current revision",
        );
    }
    let result = match location {
        RecordLocation::Node(node_id) => graph
            .tombstone_node(node_id)
            .and_then(|node_id| latest_node_revision(graph, &node_id)),
        RecordLocation::Relationship(relationship_id) => graph
            .tombstone_relationship(relationship_id)
            .and_then(|relationship_id| latest_relationship_revision(graph, &relationship_id)),
    };
    match result {
        Ok(after) => applied(operation, id.to_owned(), Some(before), after),
        Err(error) => rejected(operation, Some(id.to_owned()), error.to_string()),
    }
}

fn current_node_revision(graph: &Graph, node_id: &NodeId) -> Result<u64, graph_core::GraphError> {
    graph
        .get_node(node_id)?
        .map(|node| node.version())
        .ok_or_else(|| graph_core::GraphError::NodeNotFound(node_id.clone()))
}

fn current_relationship_revision(
    graph: &Graph,
    relationship_id: &RelationshipId,
) -> Result<u64, graph_core::GraphError> {
    graph
        .get_relationship(relationship_id)?
        .map(|relationship| relationship.version())
        .ok_or_else(|| graph_core::GraphError::RelationshipNotFound(relationship_id.clone()))
}

fn latest_node_revision(graph: &Graph, node_id: &NodeId) -> Result<u64, graph_core::GraphError> {
    graph
        .list_node_versions(node_id)?
        .into_iter()
        .find(|node| node.is_current())
        .map(|node| node.version())
        .ok_or_else(|| graph_core::GraphError::NodeNotFound(node_id.clone()))
}

fn latest_relationship_revision(
    graph: &Graph,
    relationship_id: &RelationshipId,
) -> Result<u64, graph_core::GraphError> {
    graph
        .list_relationship_versions(relationship_id)?
        .into_iter()
        .find(|relationship| relationship.is_current())
        .map(|relationship| relationship.version())
        .ok_or_else(|| graph_core::GraphError::RelationshipNotFound(relationship_id.clone()))
}

fn merge_patch(target: &mut Value, patch: &Value) {
    let Value::Object(patch) = patch else {
        *target = patch.clone();
        return;
    };
    if !target.is_object() {
        *target = Value::Object(Default::default());
    }
    let target = target
        .as_object_mut()
        .expect("target was normalized to object");
    for (key, value) in patch {
        if value.is_null() {
            target.remove(key);
        } else {
            merge_patch(target.entry(key.clone()).or_insert(Value::Null), value);
        }
    }
}

fn canonical_id(record: &Value) -> Option<String> {
    record
        .get("internal_id")
        .or_else(|| record.get("id"))
        .and_then(Value::as_str)
        .and_then(canonical_id_from_str)
}

fn canonical_id_from_str(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn applied(
    operation: &OpenCtiWriteOperation,
    id: String,
    before_revision: Option<u64>,
    after_revision: u64,
) -> WriteOperationOutcome {
    WriteOperationOutcome {
        operation_id: operation.operation_id.clone(),
        id: Some(id),
        status: WriteOperationStatus::Applied,
        before_revision,
        after_revision: Some(after_revision),
        diagnostic: None,
    }
}

fn conflict(
    operation: &OpenCtiWriteOperation,
    id: Option<String>,
    before_revision: Option<u64>,
    diagnostic: &str,
) -> WriteOperationOutcome {
    WriteOperationOutcome {
        operation_id: operation.operation_id.clone(),
        id,
        status: WriteOperationStatus::Conflict,
        before_revision,
        after_revision: None,
        diagnostic: Some(diagnostic.to_owned()),
    }
}

fn rejected(
    operation: &OpenCtiWriteOperation,
    id: Option<String>,
    diagnostic: String,
) -> WriteOperationOutcome {
    WriteOperationOutcome {
        operation_id: operation.operation_id.clone(),
        id,
        status: WriteOperationStatus::Rejected,
        before_revision: None,
        after_revision: None,
        diagnostic: Some(diagnostic),
    }
}

fn retryable(
    operation: &OpenCtiWriteOperation,
    id: Option<String>,
    diagnostic: String,
) -> WriteOperationOutcome {
    WriteOperationOutcome {
        operation_id: operation.operation_id.clone(),
        id,
        status: WriteOperationStatus::Retryable,
        before_revision: None,
        after_revision: None,
        diagnostic: Some(diagnostic),
    }
}

fn aborted(operation: &OpenCtiWriteOperation, diagnostic: &str) -> WriteOperationOutcome {
    WriteOperationOutcome {
        operation_id: operation.operation_id.clone(),
        id: None,
        status: WriteOperationStatus::Aborted,
        before_revision: None,
        after_revision: None,
        diagnostic: Some(diagnostic.to_owned()),
    }
}

fn graph_error(error: graph_core::GraphError) -> WriteError {
    WriteError::Graph(error.to_string())
}

fn mapping_error(error: crate::MappingError) -> WriteError {
    WriteError::InvalidInput(error.to_string())
}

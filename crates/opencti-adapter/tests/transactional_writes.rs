// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
#![allow(clippy::unwrap_used)]

use graph_core::{Graph, PropertyValue};
use opencti_adapter::{
    OpenCtiWriteBatch, OpenCtiWriteExecutor, OpenCtiWriteOperation, WriteLimits,
    WriteOperationStatus,
};
use serde_json::{Value, json};

fn object(id: &str, name: &str) -> Value {
    json!({
        "id": id,
        "type": "indicator",
        "name": name,
        "object_marking_refs": ["marking-definition--clear"]
    })
}

fn relationship(id: &str, source: &str, target: &str) -> Value {
    json!({
        "id": id,
        "type": "relationship",
        "relationship_type": "indicates",
        "source_ref": source,
        "target_ref": target
    })
}

fn executor() -> OpenCtiWriteExecutor {
    OpenCtiWriteExecutor::new(WriteLimits {
        max_operations: 8,
        max_payload_bytes: 16 * 1024,
    })
}

fn raw_by_id(graph: &Graph, id: &str) -> Option<Value> {
    graph
        .list_nodes()
        .unwrap()
        .into_iter()
        .find(|node| {
            node.property("opencti.canonical_id") == Some(&PropertyValue::String(id.to_owned()))
        })
        .and_then(|node| node.property("opencti.raw").cloned())
        .and_then(|value| match value {
            PropertyValue::Json(value) => Some(value),
            _ => None,
        })
}

fn revision_by_id(graph: &Graph, id: &str) -> Option<u64> {
    graph
        .list_nodes()
        .unwrap()
        .into_iter()
        .find(|node| {
            node.property("opencti.canonical_id") == Some(&PropertyValue::String(id.to_owned()))
        })
        .map(|node| node.version())
}

#[test]
fn create_update_delete_relationship_and_access_policy_match_reference_semantics() {
    let initial = OpenCtiWriteBatch::new(
        "tx--write-1",
        true,
        vec![
            OpenCtiWriteOperation::create("create-indicator", object("indicator--1", "one")),
            OpenCtiWriteOperation::create("create-malware", object("malware--1", "malware")),
            OpenCtiWriteOperation::create(
                "create-relationship",
                relationship("relationship--1", "indicator--1", "malware--1"),
            ),
        ],
    )
    .unwrap();
    let created = executor().apply(&Graph::new(), &initial).unwrap();

    assert!(created.committed);
    assert!(
        created
            .operations
            .iter()
            .all(|item| item.status == WriteOperationStatus::Applied)
    );
    assert_eq!(revision_by_id(&created.graph, "indicator--1"), Some(1));
    assert_eq!(created.graph.list_relationships().unwrap().len(), 1);

    let update = OpenCtiWriteBatch::new(
        "tx--write-2",
        true,
        vec![OpenCtiWriteOperation::update(
            "update-indicator",
            "indicator--1",
            Some(1),
            json!({
                "name": "updated",
                "object_marking_refs": ["marking-definition--restricted"]
            }),
        )],
    )
    .unwrap();
    let updated = executor().apply(&created.graph, &update).unwrap();
    assert_eq!(revision_by_id(&updated.graph, "indicator--1"), Some(2));
    assert_eq!(
        raw_by_id(&updated.graph, "indicator--1").unwrap()["name"],
        "updated"
    );

    let delete = OpenCtiWriteBatch::new(
        "tx--write-3",
        true,
        vec![OpenCtiWriteOperation::delete(
            "delete-relationship",
            "relationship--1",
            Some(1),
        )],
    )
    .unwrap();
    let deleted = executor().apply(&updated.graph, &delete).unwrap();
    assert!(deleted.committed);
    assert!(deleted.graph.list_relationships().unwrap().is_empty());
}

#[test]
fn optimistic_preconditions_reject_stale_concurrent_updates_without_lost_writes() {
    let create = OpenCtiWriteBatch::new(
        "tx--create",
        true,
        vec![OpenCtiWriteOperation::create(
            "create",
            object("indicator--1", "original"),
        )],
    )
    .unwrap();
    let created = executor().apply(&Graph::new(), &create).unwrap();
    let first = executor()
        .apply(
            &created.graph,
            &OpenCtiWriteBatch::new(
                "tx--winner",
                true,
                vec![OpenCtiWriteOperation::update(
                    "winner",
                    "indicator--1",
                    Some(1),
                    json!({"name": "winner"}),
                )],
            )
            .unwrap(),
        )
        .unwrap();
    let stale = executor()
        .apply(
            &first.graph,
            &OpenCtiWriteBatch::new(
                "tx--stale",
                true,
                vec![OpenCtiWriteOperation::update(
                    "stale",
                    "indicator--1",
                    Some(1),
                    json!({"name": "lost"}),
                )],
            )
            .unwrap(),
        )
        .unwrap();

    assert!(!stale.committed);
    assert_eq!(stale.operations[0].status, WriteOperationStatus::Conflict);
    assert_eq!(stale.operations[0].before_revision, Some(2));
    assert_eq!(
        raw_by_id(&stale.graph, "indicator--1").unwrap()["name"],
        "winner"
    );
}

#[test]
fn atomic_and_partial_bulk_policies_have_deterministic_per_item_results() {
    let operations = vec![
        OpenCtiWriteOperation::create("valid", object("indicator--1", "one")),
        OpenCtiWriteOperation::create("invalid", json!({"type": "indicator"})),
        OpenCtiWriteOperation::create("after", object("indicator--2", "two")),
    ];
    let atomic = executor()
        .apply(
            &Graph::new(),
            &OpenCtiWriteBatch::new("tx--atomic", true, operations.clone()).unwrap(),
        )
        .unwrap();
    assert!(!atomic.committed);
    assert!(atomic.graph.list_nodes().unwrap().is_empty());
    assert_eq!(atomic.operations[0].status, WriteOperationStatus::Aborted);
    assert_eq!(atomic.operations[1].status, WriteOperationStatus::Rejected);
    assert_eq!(atomic.operations[2].status, WriteOperationStatus::Aborted);

    let partial = executor()
        .apply(
            &Graph::new(),
            &OpenCtiWriteBatch::new("tx--partial", false, operations).unwrap(),
        )
        .unwrap();
    assert!(partial.committed);
    assert_eq!(partial.operations[0].status, WriteOperationStatus::Applied);
    assert_eq!(partial.operations[1].status, WriteOperationStatus::Rejected);
    assert_eq!(partial.operations[2].status, WriteOperationStatus::Applied);
    assert_eq!(partial.graph.list_nodes().unwrap().len(), 2);
}

#[test]
fn bulk_limits_reject_operation_and_byte_overflow_before_mutation() {
    let limited = OpenCtiWriteExecutor::new(WriteLimits {
        max_operations: 1,
        max_payload_bytes: 64,
    });
    let too_many = OpenCtiWriteBatch::new(
        "tx--too-many",
        true,
        vec![
            OpenCtiWriteOperation::create("one", object("indicator--1", "one")),
            OpenCtiWriteOperation::create("two", object("indicator--2", "two")),
        ],
    )
    .unwrap();
    assert!(
        limited
            .apply(&Graph::new(), &too_many)
            .unwrap_err()
            .to_string()
            .contains("max_operations")
    );

    let too_large = OpenCtiWriteBatch::new(
        "tx--too-large",
        true,
        vec![OpenCtiWriteOperation::create(
            "large",
            object("indicator--1", &"x".repeat(512)),
        )],
    )
    .unwrap();
    assert!(
        limited
            .apply(&Graph::new(), &too_large)
            .unwrap_err()
            .to_string()
            .contains("max_payload_bytes")
    );
}

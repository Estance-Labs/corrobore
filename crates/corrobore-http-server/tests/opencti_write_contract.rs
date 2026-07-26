// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
#![allow(clippy::unwrap_used)]

use std::{
    collections::BTreeMap,
    fs,
    sync::atomic::{AtomicU64, Ordering},
};

use corrobore_engine::{
    AccessContext, BulkRequest, ConsistencyLevel, CreateRequest, DeleteRequest,
    KnowledgeDataErrorCode, KnowledgeDataOperation, KnowledgeDataResponse, MergeRequest,
    RequestContext, UpdateRequest,
};
use corrobore_http_server::opencti_write::{
    DualWriteOutcome, OpenCtiWriteRuntime, ReconciliationStatus,
};
use graph_storage::{
    CanonicalEngineStore, CanonicalProjectionRequest, CanonicalStoreOptions, GraphId,
    PersistedRecordId, RecordFormat, StorageManifest, StorageTimestamp, StorageVersion,
    create_storage_root,
};
use opencti_adapter::WriteLimits;
use serde_json::json;

static NEXT_ROOT: AtomicU64 = AtomicU64::new(1);

fn root() -> graph_storage::StorageRoot {
    let path = std::env::temp_dir().join(format!(
        "corrobore-opencti-write-runtime-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT_ROOT.fetch_add(1, Ordering::Relaxed)
    ));
    let root = create_storage_root(
        path,
        StorageManifest {
            storage_version: StorageVersion::V1,
            graph_id: GraphId {
                value: "graph--opencti-write-runtime".to_owned(),
            },
            created_at: StorageTimestamp {
                value: "2026-07-25T00:00:00Z".to_owned(),
            },
            updated_at: StorageTimestamp {
                value: "2026-07-25T00:00:00Z".to_owned(),
            },
            record_format: RecordFormat::JsonLinesV1,
        },
    )
    .unwrap();
    for relative in [
        "nodes/node_records.log",
        "relationships/relationship_records.log",
        "adjacency/outgoing_adjacency.log",
        "adjacency/incoming_adjacency.log",
    ] {
        let path = root.path().join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"").unwrap();
    }
    root
}

fn context(key: &str) -> RequestContext {
    RequestContext {
        request_id: "request--write".to_owned(),
        correlation_id: "correlation--write".to_owned(),
        idempotency_key: Some(key.to_owned()),
        deadline_unix_ms: Some(4_102_444_800_000),
        cancellation_id: None,
        access: AccessContext {
            subject_id: "identity--writer".to_owned(),
            roles: vec!["system".to_owned()],
            attributes: BTreeMap::from([("source_offset".to_owned(), "offset--7".to_owned())]),
            ..AccessContext::default()
        },
        consistency: ConsistencyLevel::ReadYourWrites,
    }
}

fn create() -> KnowledgeDataOperation {
    KnowledgeDataOperation::Create(CreateRequest {
        record: json!({"id": "indicator--1", "type": "indicator", "name": "one"}),
    })
}

#[test]
fn idempotent_outcome_and_audit_survive_restart_without_duplicate_mutation() {
    let root = root();
    let state_path = root.path().join("runtime/opencti-write-state.json");
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    let mut runtime =
        OpenCtiWriteRuntime::open(Some(state_path.clone()), WriteLimits::default(), 128).unwrap();
    let first = runtime
        .apply(&mut store, &create(), &context("secret-key"))
        .unwrap();
    assert!(matches!(first, KnowledgeDataResponse::Write(_)));
    drop(runtime);
    drop(store);

    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    let mut runtime =
        OpenCtiWriteRuntime::open(Some(state_path), WriteLimits::default(), 128).unwrap();
    let replay = runtime
        .apply(&mut store, &create(), &context("secret-key"))
        .unwrap();
    assert_eq!(replay, first);
    assert_eq!(
        store
            .load_projection(CanonicalProjectionRequest::all_nodes())
            .unwrap()
            .list_nodes()
            .unwrap()
            .len(),
        1
    );
    let audits = runtime.audit_records();
    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0].source_offset.as_deref(), Some("offset--7"));
    assert_eq!(audits[0].before_revision, None);
    assert_eq!(audits[0].after_revision, Some(1));
    assert!(!audits[0].idempotency_key_hash.contains("secret-key"));
    assert!(
        !fs::read_to_string(root.path().join("transactions/audit_events.log"))
            .unwrap()
            .contains("secret-key")
    );
    fs::remove_dir_all(root.path()).unwrap();
}

#[test]
fn reused_idempotency_key_with_different_payload_is_a_durable_conflict() {
    let root = root();
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    let mut runtime = OpenCtiWriteRuntime::open(
        Some(root.path().join("runtime/opencti-write-state.json")),
        WriteLimits::default(),
        128,
    )
    .unwrap();
    runtime
        .apply(&mut store, &create(), &context("same-key"))
        .unwrap();
    let changed = KnowledgeDataOperation::Create(CreateRequest {
        record: json!({"id": "indicator--2", "type": "indicator", "name": "two"}),
    });
    let error = runtime
        .apply(&mut store, &changed, &context("same-key"))
        .unwrap_err();
    assert_eq!(error.code, KnowledgeDataErrorCode::Conflict);
    fs::remove_dir_all(root.path()).unwrap();
}

#[test]
fn typed_create_update_delete_and_bulk_return_stable_contract_results() {
    let root = root();
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    let mut runtime = OpenCtiWriteRuntime::open(None, WriteLimits::default(), 128).unwrap();
    runtime
        .apply(&mut store, &create(), &context("create"))
        .unwrap();
    let updated = runtime
        .apply(
            &mut store,
            &KnowledgeDataOperation::Update(UpdateRequest {
                id: "indicator--1".to_owned(),
                expected_revision: Some(1),
                patch: json!({"name": "updated"}),
            }),
            &context("update"),
        )
        .unwrap();
    assert!(matches!(updated, KnowledgeDataResponse::Write(ref result) if result.revision == 2));

    let bulk = runtime
        .apply(
            &mut store,
            &KnowledgeDataOperation::Bulk(BulkRequest {
                atomic: false,
                operations: vec![
                    json!({"operation": "create", "record": {"id": "indicator--2", "type": "indicator", "name": "two"}}),
                    json!({"operation": "update", "id": "missing--1", "expected_revision": 1, "patch": {"name": "missing"}}),
                ],
            }),
            &context("bulk"),
        )
        .unwrap();
    assert!(matches!(bulk, KnowledgeDataResponse::Bulk(ref result) if result.results.len() == 2));

    let deleted = runtime
        .apply(
            &mut store,
            &KnowledgeDataOperation::Delete(DeleteRequest {
                id: "indicator--1".to_owned(),
                expected_revision: Some(2),
            }),
            &context("delete"),
        )
        .unwrap();
    assert!(matches!(deleted, KnowledgeDataResponse::Write(ref result) if result.revision == 3));
    fs::remove_dir_all(root.path()).unwrap();
}

#[test]
fn partial_dual_write_is_persisted_and_not_reported_reconciled() {
    let root = root();
    let state_path = root.path().join("runtime/opencti-write-state.json");
    let mut runtime =
        OpenCtiWriteRuntime::open(Some(state_path.clone()), WriteLimits::default(), 2).unwrap();
    runtime
        .record_dual_write(DualWriteOutcome {
            idempotency_key_hash: "sha256:one".to_owned(),
            correlation_id: "correlation--one".to_owned(),
            reference_applied: true,
            corrobore_applied: false,
            diagnostic: Some("corrobore unavailable".to_owned()),
        })
        .unwrap();
    assert_eq!(runtime.status().pending_reconciliation, 1);
    assert_eq!(
        runtime.reconciliation_records()[0].status,
        ReconciliationStatus::Pending
    );

    drop(runtime);
    let reopened = OpenCtiWriteRuntime::open(Some(state_path), WriteLimits::default(), 2).unwrap();
    assert_eq!(reopened.status().pending_reconciliation, 1);
    assert!(!reopened.status().fully_reconciled);
    fs::remove_dir_all(root.path()).unwrap();
}

#[test]
fn merge_receipt_replays_after_restart_without_duplicate_versions_or_relationships() {
    let root = root();
    let state_path = root.path().join("runtime/opencti-write-state.json");
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    let mut runtime =
        OpenCtiWriteRuntime::open(Some(state_path.clone()), WriteLimits::default(), 128).unwrap();
    for (key, operation) in [
        (
            "merge-target",
            KnowledgeDataOperation::Create(CreateRequest {
                record: json!({"id": "indicator--target", "type": "indicator", "name": "target"}),
            }),
        ),
        (
            "merge-source",
            KnowledgeDataOperation::Create(CreateRequest {
                record: json!({"id": "indicator--source", "type": "indicator", "name": "source"}),
            }),
        ),
        (
            "merge-malware",
            KnowledgeDataOperation::Create(CreateRequest {
                record: json!({"id": "malware--one", "type": "malware", "name": "one"}),
            }),
        ),
        (
            "merge-relationship",
            KnowledgeDataOperation::Create(CreateRequest {
                record: json!({
                    "id": "relationship--source",
                    "type": "relationship",
                    "relationship_type": "indicates",
                    "source_ref": "indicator--source",
                    "target_ref": "malware--one"
                }),
            }),
        ),
    ] {
        runtime
            .apply(&mut store, &operation, &context(key))
            .unwrap();
    }
    let merge = KnowledgeDataOperation::Merge(MergeRequest {
        target_id: "indicator--target".to_owned(),
        source_ids: vec!["indicator--source".to_owned()],
        expected_revisions: BTreeMap::from([
            ("indicator--target".to_owned(), 1),
            ("indicator--source".to_owned(), 1),
        ]),
    });
    let first = runtime
        .apply(&mut store, &merge, &context("merge-command"))
        .unwrap();
    drop(runtime);
    drop(store);

    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    let mut runtime =
        OpenCtiWriteRuntime::open(Some(state_path), WriteLimits::default(), 128).unwrap();
    let replay = runtime
        .apply(&mut store, &merge, &context("merge-command"))
        .unwrap();
    assert_eq!(replay, first);
    let graph = store
        .load_projection(CanonicalProjectionRequest::all())
        .unwrap();
    assert_eq!(graph.list_nodes().unwrap().len(), 2);
    assert_eq!(graph.list_relationships().unwrap().len(), 1);
    let target = graph
        .list_nodes()
        .unwrap()
        .into_iter()
        .find(|node| {
            node.property("opencti.canonical_id")
                == Some(&graph_core::PropertyValue::String(
                    "indicator--target".to_owned(),
                ))
        })
        .unwrap();
    assert!(store.catalog().historical_records.iter().any(|entry| {
        matches!(&entry.record_id, PersistedRecordId::Node(node_id) if node_id == target.id())
    }));
    fs::remove_dir_all(root.path()).unwrap();
}

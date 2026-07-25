// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
#![allow(clippy::unwrap_used)]

use std::fs;

use graph_core::{Graph, NodeInput};
use graph_storage::{
    CanonicalEngineStore, CanonicalProjectionRequest, CanonicalStoreOptions, DurableTransactionId,
    GraphId, MutationCrashStage, RecordFormat, StorageManifest, StorageTimestamp, StorageVersion,
    create_storage_root, read_atomic_persistent_audit_events,
};

fn root(name: &str) -> graph_storage::StorageRoot {
    let path = std::env::temp_dir().join(format!(
        "corrobore-opencti-write-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let root = create_storage_root(
        path,
        StorageManifest {
            storage_version: StorageVersion::V1,
            graph_id: GraphId {
                value: format!("graph--{name}"),
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

fn transition() -> (Graph, Graph) {
    let previous = Graph::new();
    let mut current = Graph::new();
    current
        .create_node(NodeInput::new(vec!["OpenCtiObject".to_owned()]))
        .unwrap();
    (previous, current)
}

#[test]
fn acknowledged_receipt_is_wal_bound_replay_safe_and_restored_after_restart() {
    let root = root("receipt");
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    let (previous, current) = transition();
    let transaction_id = DurableTransactionId::new("tx--idempotency-hash").unwrap();
    let receipt = r#"{"schema_version":1,"idempotency_key_hash":"sha256:abc","correlation_id":"correlation--1","source_offset":"offset--7","before_version":null,"after_version":1,"outcome":"applied"}"#;

    let first = store
        .commit_transition_with_audit(
            &previous,
            &current,
            transaction_id.clone(),
            vec![receipt.to_owned()],
            None,
        )
        .unwrap();
    let replay = store
        .commit_transition_with_audit(
            &previous,
            &current,
            transaction_id.clone(),
            vec![receipt.to_owned()],
            None,
        )
        .unwrap();

    assert!(first.applied);
    assert!(!replay.applied);
    assert_eq!(
        read_atomic_persistent_audit_events(&root, &transaction_id).unwrap(),
        vec![receipt]
    );

    drop(store);
    let mut reopened =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    assert_eq!(
        reopened
            .load_projection(CanonicalProjectionRequest::all_nodes())
            .unwrap()
            .list_nodes()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        read_atomic_persistent_audit_events(&root, &transaction_id).unwrap(),
        vec![receipt]
    );
    fs::remove_dir_all(root.path()).unwrap();
}

#[test]
fn recovery_exposes_only_transactions_that_crossed_the_applied_marker() {
    for (stage, visible) in [
        (MutationCrashStage::AfterWalIntent, false),
        (MutationCrashStage::AfterPayloadRecords, false),
        (MutationCrashStage::BeforeAppliedMarker, false),
        (MutationCrashStage::BeforeCheckpointWrite, true),
    ] {
        let root = root(&format!("crash-{stage:?}"));
        let mut store =
            CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
        let (previous, current) = transition();
        let transaction_id = DurableTransactionId::new(format!("tx--crash-{stage:?}")).unwrap();
        store
            .commit_transition_with_audit(
                &previous,
                &current,
                transaction_id.clone(),
                vec!["durable-write-receipt".to_owned()],
                Some(stage),
            )
            .unwrap_err();
        drop(store);

        let mut reopened =
            CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
        assert_eq!(
            !reopened
                .load_projection(CanonicalProjectionRequest::all_nodes())
                .unwrap()
                .list_nodes()
                .unwrap()
                .is_empty(),
            visible,
            "unexpected recovery visibility at {stage:?}"
        );
        assert_eq!(
            !read_atomic_persistent_audit_events(&root, &transaction_id)
                .unwrap()
                .is_empty(),
            visible,
            "audit visibility must match commit visibility at {stage:?}"
        );
        fs::remove_dir_all(root.path()).unwrap();
    }
}

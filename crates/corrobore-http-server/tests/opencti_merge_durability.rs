// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
#![allow(clippy::unwrap_used)]

use std::{collections::BTreeMap, fs};

use graph_core::Graph;
use graph_storage::{
    CanonicalEngineStore, CanonicalProjectionRequest, CanonicalStoreOptions, DurableTransactionId,
    GraphId, MutationCrashStage, RecordFormat, StorageManifest, StorageTimestamp, StorageVersion,
    create_storage_root, read_atomic_persistent_audit_events,
};
use opencti_adapter::{
    MergeLimits, OpenCtiMergeExecutor, OpenCtiMergeRequest, OpenCtiWriteBatch,
    OpenCtiWriteExecutor, OpenCtiWriteOperation, WriteLimits,
};
use serde_json::json;

fn root(stage: MutationCrashStage) -> graph_storage::StorageRoot {
    let path = std::env::temp_dir().join(format!(
        "corrobore-opencti-merge-{stage:?}-{}-{}",
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
                value: "graph--merge-durability".to_owned(),
            },
            created_at: StorageTimestamp {
                value: "2026-07-26T00:00:00Z".to_owned(),
            },
            updated_at: StorageTimestamp {
                value: "2026-07-26T00:00:00Z".to_owned(),
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

fn seeded_graph() -> Graph {
    let records = vec![
        json!({"id":"indicator--target","type":"indicator","name":"target"}),
        json!({"id":"indicator--source","type":"indicator","name":"source"}),
        json!({"id":"malware--one","type":"malware","name":"malware"}),
        json!({
            "id":"relationship--target","type":"relationship","relationship_type":"indicates",
            "source_ref":"indicator--target","target_ref":"malware--one"
        }),
        json!({
            "id":"relationship--source","type":"relationship","relationship_type":"indicates",
            "source_ref":"indicator--source","target_ref":"malware--one"
        }),
    ];
    OpenCtiWriteExecutor::new(WriteLimits::default())
        .apply(
            &Graph::new(),
            &OpenCtiWriteBatch::new(
                "seed--merge-durability",
                true,
                records
                    .into_iter()
                    .enumerate()
                    .map(|(index, record)| {
                        OpenCtiWriteOperation::create(format!("seed-{index}"), record)
                    })
                    .collect(),
            )
            .unwrap(),
        )
        .unwrap()
        .graph
}

#[test]
fn merge_recovers_atomically_and_replays_at_every_wal_crash_boundary() {
    for (stage, merge_visible_after_crash) in [
        (MutationCrashStage::AfterWalIntent, false),
        (MutationCrashStage::AfterPayloadRecords, false),
        (MutationCrashStage::BeforeAppliedMarker, false),
        (MutationCrashStage::BeforeCheckpointWrite, true),
    ] {
        let root = root(stage);
        let mut store =
            CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
        let seeded = seeded_graph();
        store
            .commit_transition(
                &Graph::new(),
                &seeded,
                DurableTransactionId::new("tx--seed").unwrap(),
                None,
            )
            .unwrap();
        let previous = store
            .load_projection(CanonicalProjectionRequest::all())
            .unwrap();
        let merged = OpenCtiMergeExecutor::new(MergeLimits::default())
            .apply(
                &previous,
                &OpenCtiMergeRequest::new(
                    "merge--durability",
                    "indicator--target",
                    vec!["indicator--source".to_owned()],
                    BTreeMap::new(),
                )
                .unwrap(),
            )
            .unwrap()
            .graph;
        let transaction_id = DurableTransactionId::new("tx--merge-durability").unwrap();
        let receipt = "merge--durability-receipt".to_owned();
        store
            .commit_transition_with_audit(
                &previous,
                &merged,
                transaction_id.clone(),
                vec![receipt.clone()],
                Some(stage),
            )
            .unwrap_err();
        drop(store);

        let mut reopened =
            CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
        let after_crash = reopened
            .load_projection(CanonicalProjectionRequest::all())
            .unwrap();
        assert_eq!(
            after_crash.list_nodes().unwrap().len() == 2
                && after_crash.list_relationships().unwrap().len() == 1,
            merge_visible_after_crash,
            "merge visibility is not atomic at {stage:?}"
        );
        assert_eq!(
            !read_atomic_persistent_audit_events(&root, &transaction_id)
                .unwrap()
                .is_empty(),
            merge_visible_after_crash,
            "receipt visibility must match merge visibility at {stage:?}"
        );

        reopened
            .commit_transition_with_audit(
                &previous,
                &merged,
                transaction_id.clone(),
                vec![receipt.clone()],
                None,
            )
            .unwrap();
        drop(reopened);
        let mut recovered =
            CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
        let projection = recovered
            .load_projection(CanonicalProjectionRequest::all())
            .unwrap();
        assert_eq!(projection.list_nodes().unwrap().len(), 2);
        assert_eq!(projection.list_relationships().unwrap().len(), 1);
        assert_eq!(
            read_atomic_persistent_audit_events(&root, &transaction_id).unwrap(),
            vec![receipt]
        );
        fs::remove_dir_all(root.path()).unwrap();
    }
}

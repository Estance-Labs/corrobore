// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
#![allow(clippy::unwrap_used)]

use std::{fs, path::PathBuf};

use corrobore_http_server::opencti_sync::OpenCtiSyncRuntime;
use graph_storage::{
    CanonicalEngineStore, CanonicalProjectionRequest, CanonicalStoreOptions, GraphId, RecordFormat,
    StorageManifest, StorageTimestamp, StorageVersion, create_storage_root,
};
use opencti_adapter::{
    BulkLimits, MutationClass, OpenCtiMutation, OpenCtiSyncBatch, OpenCtiSynchronizer,
    OperationStatus, SyncPhase,
};
use serde_json::json;

fn root() -> graph_storage::StorageRoot {
    let path = std::env::temp_dir().join(format!(
        "corrobore-opencti-sync-{}-{}",
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
                value: "graph--opencti-sync-contract".to_owned(),
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

fn mutation(
    id: &str,
    sequence: u64,
    class: MutationClass,
    record: serde_json::Value,
) -> OpenCtiMutation {
    OpenCtiMutation::new(id, sequence, class, record).unwrap()
}

fn snapshot_batch() -> OpenCtiSyncBatch {
    OpenCtiSyncBatch::new(
        "opencti--primary",
        "snapshot--initial",
        SyncPhase::Snapshot,
        3,
        true,
        vec![
            mutation(
                "operation--1",
                1,
                MutationClass::Upsert,
                json!({"id": "indicator--1", "type": "indicator", "name": "one"}),
            ),
            mutation(
                "operation--2",
                2,
                MutationClass::Upsert,
                json!({"id": "malware--1", "type": "malware", "name": "malware", "is_family": false}),
            ),
            mutation(
                "operation--3",
                3,
                MutationClass::RelationshipUpsert,
                json!({
                    "id": "relationship--1",
                    "type": "relationship",
                    "relationship_type": "indicates",
                    "source_ref": "indicator--1",
                    "target_ref": "malware--1"
                }),
            ),
        ],
    )
    .unwrap()
}

fn catch_up_batch() -> OpenCtiSyncBatch {
    OpenCtiSyncBatch::new(
        "opencti--primary",
        "snapshot--initial",
        SyncPhase::CatchUp,
        4,
        false,
        vec![mutation(
            "operation--4",
            4,
            MutationClass::AccessPolicyUpdate,
            json!({"id": "indicator--1", "type": "indicator", "name": "one"}),
        )],
    )
    .unwrap()
}

#[test]
fn canonical_batch_commit_checkpoint_validation_and_restart_are_consistent() {
    let root = root();
    let state_path: PathBuf = root.path().join("runtime/opencti-sync-state.json");
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    let mut runtime =
        OpenCtiSyncRuntime::open(Some(state_path.clone()), BulkLimits::default()).unwrap();

    let snapshot = runtime.apply(&mut store, snapshot_batch(), None).unwrap();
    assert_eq!(snapshot.batch.acknowledged_sequence, 3);
    assert!(state_path.is_file());
    assert_eq!(
        fs::read_to_string(root.path().join("transactions/applied_mutations.log"))
            .unwrap()
            .lines()
            .count(),
        1,
        "the whole source batch must use one canonical WAL transaction"
    );

    let graph = store
        .load_projection(CanonicalProjectionRequest::all())
        .unwrap();
    let expected = OpenCtiSynchronizer::new(BulkLimits::default())
        .digest(&graph)
        .unwrap();
    let caught_up = runtime
        .apply(&mut store, catch_up_batch(), Some(&expected))
        .unwrap();
    assert_eq!(
        caught_up.batch.operations[0].status,
        OperationStatus::Duplicate
    );
    assert!(caught_up.validation.unwrap().shadow_reads_enabled);
    assert_eq!(runtime.status().phase, Some(SyncPhase::SteadyState));

    drop(runtime);
    drop(store);
    let mut reopened_store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    let mut reopened = OpenCtiSyncRuntime::open(Some(state_path), BulkLimits::default()).unwrap();
    assert_eq!(reopened.status().last_acknowledged_sequence, 4);
    assert!(reopened.status().shadow_reads_enabled);

    let replay = reopened
        .apply(&mut reopened_store, catch_up_batch(), Some(&expected))
        .unwrap();
    assert_eq!(
        replay.batch.operations[0].status,
        OperationStatus::Duplicate
    );
    assert_eq!(replay.checkpoint.last_acknowledged_sequence, 4);

    let unvalidated = OpenCtiSyncBatch::new(
        "opencti--primary",
        "snapshot--initial",
        SyncPhase::SteadyState,
        5,
        false,
        vec![mutation(
            "operation--5",
            5,
            MutationClass::Upsert,
            json!({"id": "indicator--1", "type": "indicator", "name": "changed"}),
        )],
    )
    .unwrap();
    reopened
        .apply(&mut reopened_store, unvalidated, None)
        .unwrap();
    assert!(
        !reopened.status().shadow_reads_enabled,
        "a canonical change must close the shadow-read gate until parity is revalidated"
    );

    let _ = fs::remove_dir_all(root.path());
}

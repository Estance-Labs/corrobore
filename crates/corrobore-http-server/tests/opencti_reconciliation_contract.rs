// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
#![allow(clippy::unwrap_used)]

use std::{
    fs,
    sync::atomic::{AtomicU64, Ordering},
};

use corrobore_http_server::opencti_reconciliation::{
    OpenCtiReconciliationRuntime, ReconciliationCrashStage,
};
use graph_storage::{
    CanonicalEngineStore, CanonicalProjectionRequest, CanonicalStoreOptions, GraphId, RecordFormat,
    StorageManifest, StorageTimestamp, StorageVersion, create_storage_root,
};
use opencti_adapter::{
    OpenCtiReconciliationCommand, ReconciliationLimits, ReconciliationMode, ReconciliationScope,
};
use serde_json::json;

static NEXT_ROOT: AtomicU64 = AtomicU64::new(1);

fn root() -> graph_storage::StorageRoot {
    let path = std::env::temp_dir().join(format!(
        "corrobore-opencti-reconciliation-{}-{}",
        std::process::id(),
        NEXT_ROOT.fetch_add(1, Ordering::Relaxed)
    ));
    let root = create_storage_root(
        path,
        StorageManifest {
            storage_version: StorageVersion::V1,
            graph_id: GraphId {
                value: "graph--reconciliation".to_owned(),
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

fn repair(command_id: &str, name: &str) -> OpenCtiReconciliationCommand {
    OpenCtiReconciliationCommand::new(
        command_id,
        ReconciliationMode::Repair,
        ReconciliationScope::Records {
            record_ids: vec!["indicator--one".to_owned()],
        },
        vec![json!({
            "id": "indicator--one",
            "type": "indicator",
            "name": name,
            "object_marking_refs": ["marking--one"]
        })],
        false,
    )
    .unwrap()
}

#[test]
fn dry_run_is_persistently_auditable_and_does_not_mutate_canonical_data() {
    let root = root();
    let state = root
        .path()
        .join("runtime/opencti-reconciliation-state.json");
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    let mut runtime = OpenCtiReconciliationRuntime::open(
        Some(state.clone()),
        ReconciliationLimits::default(),
        32,
    )
    .unwrap();
    let mut command = repair("dry-run", "expected");
    command.mode = ReconciliationMode::DryRun;
    let report = runtime.execute(&mut store, command).unwrap();
    assert!(!report.mutated);
    assert!(!report.parity_verified);
    assert!(
        store
            .load_projection(CanonicalProjectionRequest::all())
            .unwrap()
            .list_nodes()
            .unwrap()
            .is_empty()
    );
    drop(runtime);

    let reopened =
        OpenCtiReconciliationRuntime::open(Some(state), ReconciliationLimits::default(), 32)
            .unwrap();
    assert_eq!(reopened.reports(), &[report]);
    fs::remove_dir_all(root.path()).unwrap();
}

#[test]
fn repair_resumes_after_canonical_commit_and_replay_adds_no_duplicate_version() {
    let root = root();
    let state = root
        .path()
        .join("runtime/opencti-reconciliation-state.json");
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    let mut runtime = OpenCtiReconciliationRuntime::open(
        Some(state.clone()),
        ReconciliationLimits::default(),
        32,
    )
    .unwrap();
    let command = repair("repair-after-crash", "expected");
    let error = runtime
        .execute_with_crash(
            &mut store,
            command.clone(),
            Some(ReconciliationCrashStage::AfterCanonicalCommit),
        )
        .unwrap_err();
    assert!(error.contains("injected reconciliation crash"));
    drop(runtime);
    drop(store);

    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    let mut runtime =
        OpenCtiReconciliationRuntime::open(Some(state), ReconciliationLimits::default(), 32)
            .unwrap();
    let resumed = runtime.execute(&mut store, command.clone()).unwrap();
    assert!(resumed.parity_verified);
    let replay = runtime.execute(&mut store, command).unwrap();
    assert_eq!(replay, resumed);
    assert_eq!(
        store.catalog().historical_records.len(),
        0,
        "create is not replayed as another version"
    );
    assert_eq!(
        store
            .load_projection(CanonicalProjectionRequest::all_nodes())
            .unwrap()
            .list_nodes()
            .unwrap()
            .len(),
        1
    );
    fs::remove_dir_all(root.path()).unwrap();
}

#[test]
fn unsafe_extra_record_is_quarantined_and_survives_restart() {
    let root = root();
    let state = root
        .path()
        .join("runtime/opencti-reconciliation-state.json");
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    let mut runtime = OpenCtiReconciliationRuntime::open(
        Some(state.clone()),
        ReconciliationLimits::default(),
        32,
    )
    .unwrap();
    runtime
        .execute(&mut store, repair("seed-extra", "extra"))
        .unwrap();
    let command = OpenCtiReconciliationCommand::new(
        "quarantine-extra",
        ReconciliationMode::Repair,
        ReconciliationScope::Records {
            record_ids: vec!["indicator--one".to_owned()],
        },
        vec![],
        false,
    )
    .unwrap();
    let report = runtime.execute(&mut store, command).unwrap();
    assert_eq!(report.quarantined_record_ids, ["indicator--one"]);
    assert!(!report.parity_verified);
    drop(runtime);

    let reopened =
        OpenCtiReconciliationRuntime::open(Some(state), ReconciliationLimits::default(), 32)
            .unwrap();
    assert_eq!(reopened.status().quarantined_commands, 1);
    fs::remove_dir_all(root.path()).unwrap();
}

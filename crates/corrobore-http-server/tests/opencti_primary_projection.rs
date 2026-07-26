// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
#![allow(clippy::unwrap_used)]

use std::{collections::BTreeMap, fs, time::Instant};

use corrobore_engine::{
    AccessContext, ConsistencyLevel, ContractVersion, CreateRequest, KnowledgeDataOperation,
    KnowledgeDataRequest, RequestContext,
};
use corrobore_http_server::opencti_write::{
    AuthorityTransitionReadiness, OpenCtiWriteRuntime, ProjectionStatus, RollbackTrigger,
    WriteAuthority,
};
use graph_storage::{
    CanonicalEngineStore, CanonicalProjectionRequest, CanonicalStoreOptions, GraphId, RecordFormat,
    StorageManifest, StorageTimestamp, StorageVersion, create_storage_root,
};
use opencti_adapter::WriteLimits;
use serde_json::json;

fn root(test_name: &str) -> graph_storage::StorageRoot {
    let path = std::env::temp_dir().join(format!(
        "corrobore-{test_name}-{}-{}",
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
                value: format!("graph--{test_name}"),
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

fn request(key: &str, id: &str) -> KnowledgeDataRequest {
    KnowledgeDataRequest {
        contract_version: ContractVersion::CURRENT,
        context: RequestContext {
            request_id: format!("request--{key}"),
            correlation_id: format!("correlation--{key}"),
            idempotency_key: Some(key.to_owned()),
            deadline_unix_ms: Some(4_102_444_800_000),
            cancellation_id: None,
            access: AccessContext {
                subject_id: "identity--writer".to_owned(),
                roles: vec!["system".to_owned()],
                attributes: BTreeMap::new(),
                ..AccessContext::default()
            },
            consistency: ConsistencyLevel::ReadYourWrites,
        },
        operation: KnowledgeDataOperation::Create(CreateRequest {
            record: json!({"id": id, "type": "indicator", "name": key}),
        }),
    }
}

#[test]
fn committed_primary_write_is_recovered_into_the_outbox_after_a_crash_gap() {
    let root = root("primary-outbox-recovery");
    let state_path = root.path().join("runtime/opencti-write-state.json");
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    let write = request("secret-primary-key", "indicator--recovered");
    let mut runtime =
        OpenCtiWriteRuntime::open(Some(state_path.clone()), WriteLimits::default(), 8).unwrap();

    let sequence = runtime.prepare_projection(&write).unwrap();
    runtime
        .apply(&mut store, &write.operation, &write.context)
        .unwrap();
    drop(runtime);

    let mut reopened =
        OpenCtiWriteRuntime::open(Some(state_path), WriteLimits::default(), 8).unwrap();
    reopened.recover_projection_outbox(&mut store).unwrap();
    let entry = reopened.pending_projection().unwrap();
    assert_eq!(entry.sequence, sequence);
    assert_eq!(entry.status, ProjectionStatus::Pending);
    assert_eq!(entry.ordering_key, "indicator--recovered");
    assert_ne!(
        entry.request.context.idempotency_key.as_deref(),
        Some("secret-primary-key")
    );
    assert_eq!(reopened.status().projection_outbox_depth, 1);
    assert_eq!(
        reopened.status().write_authority,
        WriteAuthority::CorroborePrimary
    );
    fs::remove_dir_all(root.path()).unwrap();
}

#[test]
fn abandoned_prepare_is_removed_but_a_failed_projection_remains_ordered_and_retryable() {
    let root = root("primary-outbox-ordering");
    let state_path = root.path().join("runtime/opencti-write-state.json");
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    let first = request("first", "indicator--first");
    let second = request("second", "indicator--second");
    let abandoned = request("abandoned", "indicator--abandoned");
    let mut runtime =
        OpenCtiWriteRuntime::open(Some(state_path.clone()), WriteLimits::default(), 8).unwrap();

    let abandoned_sequence = runtime.prepare_projection(&abandoned).unwrap();
    let first_sequence = runtime.prepare_projection(&first).unwrap();
    let first_response = runtime
        .apply(&mut store, &first.operation, &first.context)
        .unwrap();
    runtime
        .activate_projection(first_sequence, first_response.clone())
        .unwrap();
    let second_sequence = runtime.prepare_projection(&second).unwrap();
    let second_response = runtime
        .apply(&mut store, &second.operation, &second.context)
        .unwrap();
    runtime
        .activate_projection(second_sequence, second_response)
        .unwrap();
    runtime
        .record_projection_failure(first_sequence, "reference unavailable")
        .unwrap();
    assert_eq!(runtime.prepare_projection(&first).unwrap(), first_sequence);
    let replay = runtime
        .apply(&mut store, &first.operation, &first.context)
        .unwrap();
    runtime.activate_projection(first_sequence, replay).unwrap();
    drop(runtime);

    let mut reopened =
        OpenCtiWriteRuntime::open(Some(state_path), WriteLimits::default(), 8).unwrap();
    reopened.recover_projection_outbox(&mut store).unwrap();
    assert!(
        reopened
            .projection_records()
            .iter()
            .all(|entry| entry.sequence != abandoned_sequence)
    );
    let pending = reopened.pending_projection().unwrap();
    assert_eq!(pending.sequence, first_sequence);
    assert_eq!(pending.attempts, 1);
    assert_eq!(reopened.status().projection_retries, 1);
    reopened
        .verify_projection(first_sequence, &first_response)
        .unwrap();
    assert_eq!(
        reopened.pending_projection().unwrap().sequence,
        second_sequence
    );
    fs::remove_dir_all(root.path()).unwrap();
}

#[test]
fn projection_backpressure_and_divergence_never_claim_synchronization() {
    let root = root("primary-outbox-backpressure");
    let mut runtime = OpenCtiWriteRuntime::open(
        Some(root.path().join("runtime/opencti-write-state.json")),
        WriteLimits::default(),
        1,
    )
    .unwrap();
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    let first = request("first", "indicator--first");
    let sequence = runtime.prepare_projection(&first).unwrap();
    let expected = runtime
        .apply(&mut store, &first.operation, &first.context)
        .unwrap();
    runtime.activate_projection(sequence, expected).unwrap();

    let error = runtime
        .prepare_projection(&request("second", "indicator--second"))
        .unwrap_err();
    assert!(error.contains("backpressure"));
    let divergent = corrobore_engine::KnowledgeDataResponse::Write(corrobore_engine::WriteResult {
        id: "indicator--different".to_owned(),
        revision: 99,
    });
    assert!(runtime.verify_projection(sequence, &divergent).is_err());
    assert_eq!(runtime.status().projection_quarantined, 1);
    assert!(!runtime.status().fully_synchronized);
    runtime
        .transition_authority(
            WriteAuthority::ReferencePrimary,
            AuthorityTransitionReadiness {
                reference_healthy: true,
                replay_complete: true,
                parity_verified: true,
            },
        )
        .unwrap();
    assert_eq!(runtime.status().projection_quarantined, 0);
    assert!(runtime.status().fully_synchronized);
    fs::remove_dir_all(root.path()).unwrap();
}

#[test]
fn reconstruction_is_lossless_and_authority_rollback_requires_replay_and_parity() {
    let root = root("primary-reconstruction");
    let mut runtime = OpenCtiWriteRuntime::open(
        Some(root.path().join("runtime/opencti-write-state.json")),
        WriteLimits::default(),
        8,
    )
    .unwrap();
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    let write = request("rebuild", "indicator--rebuild");
    let sequence = runtime.prepare_projection(&write).unwrap();
    let response = runtime
        .apply(&mut store, &write.operation, &write.context)
        .unwrap();
    runtime
        .activate_projection(sequence, response.clone())
        .unwrap();
    runtime.verify_projection(sequence, &response).unwrap();

    let plan = runtime.reconstruction_plan(&mut store).unwrap();
    assert_eq!(plan.high_water_sequence, sequence);
    assert_eq!(
        plan.records,
        vec![write.operation.create_record().unwrap().clone()]
    );

    runtime
        .suspend_writes(RollbackTrigger::ReferenceAvailability)
        .unwrap();
    let error = runtime
        .transition_authority(
            WriteAuthority::ReferencePrimary,
            AuthorityTransitionReadiness {
                reference_healthy: true,
                replay_complete: false,
                parity_verified: true,
            },
        )
        .unwrap_err();
    assert!(error.contains("replay"));
    assert_eq!(
        runtime.status().write_authority,
        WriteAuthority::WritesSuspended
    );
    runtime
        .transition_authority(
            WriteAuthority::ReferencePrimary,
            AuthorityTransitionReadiness {
                reference_healthy: true,
                replay_complete: true,
                parity_verified: true,
            },
        )
        .unwrap();
    assert_eq!(
        runtime.status().write_authority,
        WriteAuthority::ReferencePrimary
    );
    fs::remove_dir_all(root.path()).unwrap();
}

#[test]
fn primary_soak_survives_outages_restart_and_replays_without_secret_leakage() {
    let started = Instant::now();
    let root = root("primary-soak");
    let state_path = root.path().join("runtime/opencti-write-state.json");
    let mut runtime =
        OpenCtiWriteRuntime::open(Some(state_path.clone()), WriteLimits::default(), 128).unwrap();
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();

    for index in 0..32_u64 {
        let mut write = request(
            &format!("soak-secret-{index}"),
            &format!("indicator--soak-{index:03}"),
        );
        write.context.request_id = format!("request--soak-{index}");
        write.context.correlation_id = format!("correlation--soak-{index}");
        if let KnowledgeDataOperation::Create(create) = &mut write.operation {
            create.record["name"] = json!(format!("soak-{index}"));
        }
        let sequence = runtime.prepare_projection(&write).unwrap();
        let response = runtime
            .apply(&mut store, &write.operation, &write.context)
            .unwrap();
        runtime.activate_projection(sequence, response).unwrap();
        if index % 7 == 0 {
            runtime
                .record_projection_failure(sequence, "simulated reference outage")
                .unwrap();
        }
    }
    drop(runtime);
    drop(store);

    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    let mut runtime =
        OpenCtiWriteRuntime::open(Some(state_path.clone()), WriteLimits::default(), 128).unwrap();
    runtime.recover_projection_outbox(&mut store).unwrap();
    while let Some(record) = runtime.pending_projection().cloned() {
        runtime
            .verify_projection(record.sequence, record.expected_response.as_ref().unwrap())
            .unwrap();
    }

    assert_eq!(runtime.status().projection_retries, 5);
    assert!(runtime.status().fully_synchronized);
    assert_eq!(
        store
            .load_projection(CanonicalProjectionRequest::all_nodes())
            .unwrap()
            .list_nodes()
            .unwrap()
            .len(),
        32
    );
    let state = fs::read_to_string(&state_path).unwrap();
    assert!(!state.contains("soak-secret"));
    assert!(started.elapsed().as_secs() < 30);
    fs::remove_dir_all(root.path()).unwrap();
}

#[test]
fn unresolved_legacy_dual_write_state_suspends_primary_authority_on_upgrade() {
    let root = root("legacy-dual-write-upgrade");
    let state_path = root.path().join("runtime/opencti-write-state.json");
    fs::create_dir_all(state_path.parent().unwrap()).unwrap();
    fs::write(
        &state_path,
        serde_json::to_vec_pretty(&json!({
            "schema_version": 1,
            "reconciliations": [{
                "idempotency_key_hash": "sha256:legacy",
                "correlation_id": "correlation--legacy",
                "status": "pending",
                "attempts": 1,
                "diagnostic": "legacy partial write"
            }]
        }))
        .unwrap(),
    )
    .unwrap();

    let runtime = OpenCtiWriteRuntime::open(Some(state_path), WriteLimits::default(), 8).unwrap();
    assert_eq!(
        runtime.status().write_authority,
        WriteAuthority::WritesSuspended
    );
    assert!(!runtime.status().fully_synchronized);
    fs::remove_dir_all(root.path()).unwrap();
}

trait CreateRecordExt {
    fn create_record(&self) -> Option<&serde_json::Value>;
}

impl CreateRecordExt for KnowledgeDataOperation {
    fn create_record(&self) -> Option<&serde_json::Value> {
        match self {
            KnowledgeDataOperation::Create(request) => Some(&request.record),
            _ => None,
        }
    }
}

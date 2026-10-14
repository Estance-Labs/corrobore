// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
#![allow(clippy::unwrap_used)]

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
};
use corrobore_http_server::{AppState, ServerConfig, build_router};
use serde_json::{Value, json};
use tower::ServiceExt;

fn persistent_state(storage_dir: &Path) -> AppState {
    persistent_state_with_permissions(storage_dir, "read,write,trace,forget,consolidate")
}

fn persistent_state_with_permissions(storage_dir: &Path, permissions: &str) -> AppState {
    let runtime = storage_dir.parent().unwrap().join("runtime");
    let config = ServerConfig::from_map(&HashMap::from([
        (
            "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
            "memory-token".to_owned(),
        ),
        ("CORROBORE_STORAGE_MODE".to_owned(), "persistent".to_owned()),
        (
            "CORROBORE_STORAGE_DIR".to_owned(),
            storage_dir.display().to_string(),
        ),
        (
            "CORROBORE_HTTP_SESSION_STORE_DIR".to_owned(),
            runtime.display().to_string(),
        ),
        (
            "CORROBORE_MEMORY_PERMISSIONS".to_owned(),
            permissions.to_owned(),
        ),
    ]))
    .expect("memory HTTP config should parse");
    AppState::new(config).expect("memory HTTP state should initialize")
}

#[tokio::test]
async fn standalone_permissions_are_enforced_independently_from_bearer_authentication() {
    let storage = temp_storage();
    let state = persistent_state_with_permissions(&storage, "read");
    let response = build_router(state)
        .oneshot(request(remember_payload(), "memory-http-denied"))
        .await
        .expect("permission denial should return a typed response");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["error"]["code"], "PERMISSION_DENIED");
}

fn temp_storage() -> PathBuf {
    static NEXT_STORAGE_ID: AtomicU64 = AtomicU64::new(0);
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = NEXT_STORAGE_ID.fetch_add(1, Ordering::Relaxed);
    let process = std::process::id();
    let root = std::env::temp_dir().join(format!(
        "corrobore-memory-http-{process}-{suffix}-{sequence}"
    ));
    fs::create_dir_all(&root).unwrap();
    root.join("graph")
}

fn request(payload: Value, correlation: &str) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri("/v1/memory/operations")
        .header(header::AUTHORIZATION, "Bearer memory-token")
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-request-id", correlation)
        .body(Body::from(payload.to_string()))
        .unwrap()
}

fn remember_payload() -> Value {
    json!({
        "contract_version": "v1",
        "idempotency_key": "http:remember:1",
        "operation": "remember",
        "input": {
            "identity_key": "http-memory",
            "kind": "observation",
            "schema_version": "1",
            "content": {"format": "text", "value": "portable durable memory"},
            "provenance": [{
                "source_id": "source--http",
                "locator": "urn:http:1",
                "observed_at": "2026-07-26T00:00:00Z"
            }],
            "confidence": 0.9,
            "valid_from": null,
            "valid_until": null,
            "expires_at": null,
            "tags": ["http"]
        }
    })
}

async fn execute(app: &Router, payload: Value, correlation: &str) -> Value {
    let response = app
        .clone()
        .oneshot(request(payload, correlation))
        .await
        .expect("conformance request should run");
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
async fn standalone_adapter_passes_the_shared_operation_conformance_journey() {
    let corpus: Value = serde_json::from_str(include_str!(
        "../../../compatibility/memory/v1/conformance.json"
    ))
    .expect("shared memory conformance corpus should parse");
    let expected = corpus["operations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["operation"].as_str().unwrap().to_owned())
        .collect::<std::collections::BTreeSet<_>>();
    let storage = temp_storage();
    let app = build_router(persistent_state(&storage));
    let mut seen = std::collections::BTreeSet::new();

    let alpha = execute(&app, remember_payload(), "corpus-remember-alpha").await;
    seen.insert("remember".to_owned());
    let alpha_id = alpha["result"]["result"]["record"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut beta_payload = remember_payload();
    beta_payload["idempotency_key"] = json!("http:remember:2");
    beta_payload["input"]["identity_key"] = json!("http-memory-beta");
    beta_payload["input"]["content"]["value"] = json!("related durable evidence");
    let beta = execute(&app, beta_payload, "corpus-remember-beta").await;
    let beta_id = beta["result"]["result"]["record"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    execute(
        &app,
        json!({
            "contract_version": "v1",
            "idempotency_key": "http:relate:1",
            "operation": "relate",
            "input": {
                "identity_key": "http-relation",
                "source_id": alpha_id,
                "target_id": beta_id,
                "kind": "supports",
                "properties": {"strength": "direct"},
                "provenance": [{"source_id": "source--relation", "locator": null, "observed_at": null}],
                "confidence": 0.8,
                "valid_from": null,
                "valid_until": null,
                "expires_at": null,
                "lifecycle": "active"
            }
        }),
        "corpus-relate",
    )
    .await;
    seen.insert("relate".to_owned());

    let lookup = execute(
        &app,
        json!({
            "contract_version": "v1",
            "operation": "get",
            "input": {
                "identity_keys": ["http-memory", "http-memory-unknown"],
                "relationship_identity_keys": ["http-relation"]
            }
        }),
        "corpus-get",
    )
    .await;
    seen.insert("get".to_owned());

    let reinforced = execute(
        &app,
        json!({
            "contract_version": "v1",
            "idempotency_key": "http:reinforce:1",
            "operation": "reinforce",
            "input": {"target": {"kind": "memory", "id": alpha_id}, "amount": 2.0, "reason": "used in an answer"}
        }),
        "corpus-reinforce",
    )
    .await;
    seen.insert("reinforce".to_owned());
    assert_eq!(reinforced["result"]["operation"], "reinforce");
    let before = lookup["result"]["result"]["records"][0]["heat"]
        .as_f64()
        .unwrap();
    let after = reinforced["result"]["result"]["record"]["heat"]
        .as_f64()
        .unwrap();
    assert!((after - before - 2.0).abs() < 1e-3, "{before} -> {after}");
    assert!(reinforced["result"]["result"]["record"]["heat_updated_at"].is_string());
    assert_eq!(lookup["result"]["operation"], "get");
    let found = &lookup["result"]["result"];
    assert_eq!(found["records"].as_array().unwrap().len(), 1);
    assert_eq!(found["records"][0]["id"], alpha_id.as_str());
    assert_eq!(found["records"][0]["lifecycle"], "active");
    assert_eq!(found["records"][0]["recallable"], true);
    assert_eq!(found["records"][0]["superseded_by"], Value::Null);
    assert_eq!(found["records"][0]["evidence_count"], 1);
    assert_eq!(found["records"][0]["distinct_observation_days"], 1);
    assert_eq!(
        found["records"][0]["first_observed_at"],
        "2026-07-26T00:00:00Z"
    );
    assert_eq!(found["relationships"][0]["evidence_count"], 1);
    assert_eq!(found["relationships"][0]["identity_key"], "http-relation");
    assert_eq!(
        found["missing"],
        json!([{"field": "identity_keys", "value": "http-memory-unknown"}])
    );

    let recall = execute(
        &app,
        json!({
            "contract_version": "v1",
            "idempotency_key": null,
            "operation": "recall",
            "input": {
                "objective": "durable evidence",
                "seed_ids": [alpha_id],
                "limits": {"max_items": 10, "max_depth": 2, "max_payload_bytes": 65536, "max_cost": 100, "timeout_ms": 1000, "supernode_threshold": 20},
                "page_token": null
            }
        }),
        "corpus-recall",
    )
    .await;
    seen.insert("recall".to_owned());
    let recall_id = recall["result"]["result"]["recall_id"]
        .as_str()
        .unwrap()
        .to_owned();

    execute(
        &app,
        json!({
            "contract_version": "v1",
            "idempotency_key": "http:update:1",
            "operation": "update",
            "input": {
                "target": {"kind": "memory", "id": alpha_id},
                // `reinforce` above committed version 2.
                "expected_version": 2,
                "patch": {
                    "content": null,
                    "confidence": 0.95,
                    "add_provenance": [{"source_id": "source--update", "locator": null, "observed_at": null}],
                    "lifecycle": "active",
                    "expires_at": null,
                    "add_tags": ["updated"]
                }
            }
        }),
        "corpus-update",
    )
    .await;
    seen.insert("update".to_owned());

    execute(
        &app,
        json!({
            "contract_version": "v1",
            "idempotency_key": null,
            "operation": "trace",
            "input": {"target": {"kind": "recall", "id": recall_id}}
        }),
        "corpus-trace",
    )
    .await;
    seen.insert("trace".to_owned());

    let proposal = execute(
        &app,
        json!({
            "contract_version": "v1",
            "idempotency_key": null,
            "operation": "consolidate",
            "input": {
                "mode": {"mode": "propose"},
                "memory_ids": [alpha_id, beta_id],
                "canonical_id": alpha_id,
                "reason": "shared corpus duplicate proposal",
                "preserve_disagreements": true
            }
        }),
        "corpus-consolidate-propose",
    )
    .await;
    seen.insert("consolidate".to_owned());
    let proposal_id = proposal["result"]["result"]["proposal_id"]
        .as_str()
        .unwrap()
        .to_owned();
    execute(
        &app,
        json!({
            "contract_version": "v1",
            "idempotency_key": "http:consolidate:1",
            "operation": "consolidate",
            "input": {
                "mode": {"mode": "apply_approved", "proposal_id": proposal_id, "approval_policy": "policy--conformance"},
                "memory_ids": [alpha_id, beta_id],
                "canonical_id": alpha_id,
                "reason": "shared corpus approved consolidation",
                "preserve_disagreements": true
            }
        }),
        "corpus-consolidate-apply",
    )
    .await;

    execute(
        &app,
        json!({
            "contract_version": "v1",
            "idempotency_key": "http:forget:1",
            "operation": "forget",
            "input": {"memory_id": beta_id, "mode": "application_delete", "expires_at": null, "reason": "shared corpus cleanup"}
        }),
        "corpus-forget",
    )
    .await;
    seen.insert("forget".to_owned());
    assert_eq!(seen, expected);
}

#[tokio::test]
async fn standalone_adapter_reuses_contract_and_persists_memory_after_restart() {
    let storage = temp_storage();
    let first = persistent_state(&storage);
    let response = build_router(first)
        .oneshot(request(remember_payload(), "memory-http-write"))
        .await
        .expect("remember request should run");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["contract_version"], "v1");
    let memory_id = payload["result"]["result"]["record"]["id"]
        .as_str()
        .expect("remember should return stable ID")
        .to_owned();
    assert_eq!(
        payload["result"]["result"]["receipt"]["audit_correlation_id"],
        "memory-http-write"
    );

    let restarted = persistent_state(&storage);
    let trace = json!({
        "contract_version": "v1",
        "idempotency_key": null,
        "operation": "trace",
        "input": {"target": {"kind": "memory", "id": memory_id}}
    });
    let response = build_router(restarted)
        .oneshot(request(trace, "memory-http-trace"))
        .await
        .expect("trace request should run after restart");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["result"]["operation"], "trace");
    assert_eq!(
        payload["result"]["result"]["actor_id"],
        "actor--standalone-client"
    );
    assert_eq!(
        payload["result"]["result"]["session_id"],
        "session--standalone-api"
    );

    // The application identity key resolves to the same durable record after
    // the restart, without the client mirroring engine ids.
    let restarted = persistent_state(&storage);
    let lookup = execute(
        &build_router(restarted),
        json!({
            "contract_version": "v1",
            "operation": "get",
            "input": {"identity_keys": ["http-memory"]}
        }),
        "memory-http-get-after-restart",
    )
    .await;
    assert_eq!(lookup["result"]["result"]["records"][0]["id"], memory_id);
    assert_eq!(
        lookup["result"]["result"]["records"][0]["identity_key"],
        "http-memory"
    );
    assert_eq!(
        lookup["result"]["result"]["records"][0]["evidence_count"],
        1
    );
    assert_eq!(
        lookup["result"]["result"]["records"][0]["last_observed_at"],
        "2026-07-26T00:00:00Z"
    );
}

#[tokio::test]
async fn untrusted_payload_cannot_override_workspace_or_permissions() {
    let storage = temp_storage();
    let state = persistent_state(&storage);
    let mut payload = remember_payload();
    payload["workspace_id"] = json!("workspace--attacker");
    payload["permissions"] = json!({"write": true});
    let response = build_router(state)
        .oneshot(request(payload, "memory-http-untrusted"))
        .await
        .expect("invalid request should return a typed response");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["error"]["code"], "INVALID_REQUEST");
    assert_eq!(payload["correlation_id"], "memory-http-untrusted");
}

fn admin_state(storage_dir: &Path, extra: &[(&str, &str)]) -> AppState {
    let runtime = storage_dir.parent().unwrap().join("runtime");
    let mut vars = HashMap::from([
        (
            "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
            "memory-token".to_owned(),
        ),
        (
            "CORROBORE_HTTP_ADMIN_AUTH_TOKEN".to_owned(),
            "memory-admin".to_owned(),
        ),
        ("CORROBORE_STORAGE_MODE".to_owned(), "persistent".to_owned()),
        (
            "CORROBORE_STORAGE_DIR".to_owned(),
            storage_dir.display().to_string(),
        ),
        (
            "CORROBORE_HTTP_SESSION_STORE_DIR".to_owned(),
            runtime.display().to_string(),
        ),
    ]);
    for (name, value) in extra {
        vars.insert((*name).to_owned(), (*value).to_owned());
    }
    AppState::new(ServerConfig::from_map(&vars).unwrap()).unwrap()
}

#[test]
fn heat_policy_configuration_has_defaults_and_rejects_invalid_values() {
    let base = HashMap::from([(
        "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
        "memory-token".to_owned(),
    )]);
    let config = ServerConfig::from_map(&base).unwrap();
    assert!((config.memory_heat_policy.half_life_days() - 21.0).abs() < f64::EPSILON);
    assert!((config.memory_heat_policy.evidence_reward() - 1.0).abs() < f64::EPSILON);
    assert!(config.memory_heat_policy.cold_threshold().abs() < f64::EPSILON);

    let mut tuned = base.clone();
    tuned.insert(
        "CORROBORE_MEMORY_HEAT_HALF_LIFE_DAYS".to_owned(),
        "7.5".to_owned(),
    );
    tuned.insert(
        "CORROBORE_MEMORY_HEAT_EVIDENCE_REWARD".to_owned(),
        "2".to_owned(),
    );
    tuned.insert(
        "CORROBORE_MEMORY_HEAT_COLD_THRESHOLD".to_owned(),
        "0.25".to_owned(),
    );
    let config = ServerConfig::from_map(&tuned).unwrap();
    assert!((config.memory_heat_policy.half_life_days() - 7.5).abs() < f64::EPSILON);
    assert!((config.memory_heat_policy.cold_threshold() - 0.25).abs() < f64::EPSILON);

    for (name, value) in [
        ("CORROBORE_MEMORY_HEAT_HALF_LIFE_DAYS", "0"),
        ("CORROBORE_MEMORY_HEAT_HALF_LIFE_DAYS", "-3"),
        ("CORROBORE_MEMORY_HEAT_HALF_LIFE_DAYS", "NaN"),
        ("CORROBORE_MEMORY_HEAT_HALF_LIFE_DAYS", "soon"),
        ("CORROBORE_MEMORY_HEAT_EVIDENCE_REWARD", "-1"),
        ("CORROBORE_MEMORY_HEAT_COLD_THRESHOLD", "inf"),
        ("CORROBORE_MEMORY_HEAT_COLD_THRESHOLD", "-0.5"),
    ] {
        let mut invalid = base.clone();
        invalid.insert(name.to_owned(), value.to_owned());
        assert!(
            ServerConfig::from_map(&invalid).is_err(),
            "{name}={value} must be rejected"
        );
    }
}

#[tokio::test]
async fn heat_and_durability_survive_restart_and_snapshot_restore() {
    let storage = temp_storage();
    let app = build_router(admin_state(&storage, &[]));
    let mut payload = remember_payload();
    payload["input"]["durable"] = json!(true);
    let remembered = execute(&app, payload, "heat-remember").await;
    let memory_id = remembered["result"]["result"]["record"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(remembered["result"]["result"]["record"]["durable"], true);
    let reinforced = execute(
        &app,
        json!({
            "contract_version": "v1",
            "idempotency_key": "heat:reinforce",
            "operation": "reinforce",
            "input": {"target": {"kind": "memory", "id": memory_id}, "amount": 3.0}
        }),
        "heat-reinforce",
    )
    .await;
    let record = &reinforced["result"]["result"]["record"];
    let heat = record["heat"].as_f64().unwrap();
    let updated_at = record["heat_updated_at"].as_str().unwrap().to_owned();
    assert!(heat > 3.0);

    let snapshot = storage.parent().unwrap().join("snapshot");
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/admin/storage/snapshots")
                .header(header::AUTHORIZATION, "Bearer memory-admin")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"destination": snapshot, "retention_hook": "test"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    drop(app);

    let lookup = json!({
        "contract_version": "v1",
        "operation": "get",
        "input": {"identity_keys": ["http-memory"]}
    });
    let assert_retained = |payload: &Value| {
        let record = &payload["result"]["result"]["records"][0];
        assert_eq!(record["id"], memory_id.as_str());
        assert_eq!(record["durable"], true);
        assert_eq!(record["heat_updated_at"], updated_at.as_str());
        // Lazy decay between the reinforce and this read is negligible.
        assert!((record["heat"].as_f64().unwrap() - heat).abs() < 1e-3);
    };
    let restarted = build_router(admin_state(&storage, &[]));
    assert_retained(&execute(&restarted, lookup.clone(), "heat-restart").await);
    drop(restarted);

    let restored = storage.parent().unwrap().join("restored");
    graph_storage::restore_consistent_snapshot(&snapshot, &restored, None).unwrap();
    let restored_app = build_router(admin_state(&restored, &[]));
    assert_retained(&execute(&restored_app, lookup, "heat-restored").await);
}

#[tokio::test]
async fn merging_consolidation_moves_evidence_and_survives_restart() {
    let storage = temp_storage();
    let app = build_router(persistent_state(&storage));
    let alpha = execute(&app, remember_payload(), "merge-alpha").await;
    let alpha_id = alpha["result"]["result"]["record"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut beta_payload = remember_payload();
    beta_payload["idempotency_key"] = json!("merge:beta");
    beta_payload["input"]["identity_key"] = json!("http-memory-duplicate");
    beta_payload["input"]["provenance"] = json!([{
        "source_id": "source--duplicate",
        "locator": null,
        "observed_at": "2026-07-27T00:00:00Z"
    }]);
    let beta = execute(&app, beta_payload, "merge-beta").await;
    let beta_id = beta["result"]["result"]["record"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let input = |mode: Value| {
        json!({
            "mode": mode,
            "memory_ids": [alpha_id, beta_id],
            "canonical_id": alpha_id,
            "reason": "duplicate identity",
            "preserve_disagreements": true,
            "merge_provenance": true
        })
    };
    let proposal = execute(
        &app,
        json!({"contract_version": "v1", "operation": "consolidate", "input": input(json!({"mode": "propose"}))}),
        "merge-propose",
    )
    .await;
    assert_eq!(proposal["result"]["result"]["merged_provenance_count"], 1);
    let proposal_id = proposal["result"]["result"]["proposal_id"].clone();
    let applied = execute(
        &app,
        json!({
            "contract_version": "v1",
            "idempotency_key": "merge:apply",
            "operation": "consolidate",
            "input": input(json!({"mode": "apply_approved", "proposal_id": proposal_id, "approval_policy": "policy--merge"}))
        }),
        "merge-apply",
    )
    .await;
    assert_eq!(applied["result"]["result"]["applied"], true);
    drop(app);

    let restarted = build_router(persistent_state(&storage));
    let lookup = execute(
        &restarted,
        json!({
            "contract_version": "v1",
            "operation": "get",
            "input": {"identity_keys": ["http-memory", "http-memory-duplicate"]}
        }),
        "merge-get",
    )
    .await;
    let records = &lookup["result"]["result"]["records"];
    assert_eq!(records[0]["evidence_count"], 2);
    assert_eq!(records[0]["distinct_observation_days"], 2);
    assert_eq!(records[1]["lifecycle"], "superseded");
    assert_eq!(records[1]["superseded_by"], alpha_id.as_str());
}

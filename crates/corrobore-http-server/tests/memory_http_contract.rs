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
async fn standalone_adapter_passes_the_shared_seven_operation_conformance_journey() {
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
                "expected_version": 1,
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

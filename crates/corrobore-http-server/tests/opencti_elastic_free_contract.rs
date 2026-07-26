// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
#![allow(clippy::unwrap_used)]

use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
};
use corrobore_http_server::{AppState, ServerConfig, build_router};
use serde_json::{Value, json};
use tower::ServiceExt;

fn unique_root() -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("corrobore-opencti-elastic-free-{suffix}"))
}

fn write_request(idempotency_key: &str) -> Value {
    json!({
        "contract_version": {"major": 1, "minor": 0},
        "context": {
            "request_id": "request--elastic-free-write",
            "correlation_id": "correlation--elastic-free-write",
            "idempotency_key": idempotency_key,
            "deadline_unix_ms": 4102444800000_u64,
            "cancellation_id": null,
            "access": {
                "subject_id": "identity--system",
                "organization_ids": [],
                "marking_ids": [],
                "tenant_id": null,
                "roles": ["system"],
                "attributes": {}
            },
            "consistency": "read_your_writes"
        },
        "operation": {
            "operation": "create",
            "request": {"record": {"id": "indicator--elastic-free", "type": "indicator", "name": "Elastic free"}}
        }
    })
}

fn read_request() -> Value {
    json!({
        "request": {
            "contract_version": {"major": 1, "minor": 0},
            "context": {
                "request_id": "request--elastic-free-read",
                "correlation_id": "correlation--elastic-free-read",
                "access": {
                    "subject_id": "identity--system",
                    "organization_ids": [],
                    "marking_ids": [],
                    "tenant_id": null,
                    "roles": ["system"],
                    "attributes": {}
                },
                "consistency": "read_your_writes"
            },
            "operation": {"operation": "get_by_id", "request": {"id": "indicator--elastic-free"}}
        },
        "metadata": {"environment": "production", "feature_flags": []}
    })
}

fn initialize_request() -> Value {
    json!({
        "request": {
            "contract_version": {"major": 1, "minor": 0},
            "context": {
                "request_id": "opencti-provider-initialize",
                "correlation_id": "opencti-provider-initialize",
                "access": {
                    "subject_id": "system",
                    "organization_ids": [],
                    "marking_ids": [],
                    "tenant_id": null,
                    "roles": ["system"],
                    "attributes": {}
                },
                "consistency": "read_your_writes"
            },
            "operation": {
                "operation": "initialize",
                "request": {
                    "client_contract_version": {"major": 1, "minor": 0},
                    "required_capabilities": [
                        "initialize", "get_by_id", "list", "paginate", "search",
                        "count", "aggregate", "create", "update", "delete", "bulk", "merge"
                    ]
                }
            }
        },
        "metadata": {"environment": "production", "feature_flags": []}
    })
}

#[tokio::test]
async fn elastic_free_mode_reads_and_writes_without_reference_projection() {
    let root = unique_root();
    let storage = root.join("graph");
    let runtime = root.join("runtime");
    fs::create_dir_all(&runtime).unwrap();
    let policy = root.join("primary-reads.json");
    fs::write(
        &policy,
        json!({
            "policy_version": "elastic-free-v1",
            "mode": "primary_reads",
            "default_percentage_basis_points": 10000,
            "rules": [],
            "thresholds": {"max_error_rate_basis_points": 100, "max_latency_p95_ms": 120, "minimum_soak_requests": 1}
        })
        .to_string(),
    )
    .unwrap();
    let config = ServerConfig::from_map(&HashMap::from([
        (
            "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
            "token-54".to_owned(),
        ),
        ("CORROBORE_STORAGE_MODE".to_owned(), "persistent".to_owned()),
        (
            "CORROBORE_STORAGE_DIR".to_owned(),
            storage.display().to_string(),
        ),
        (
            "CORROBORE_HTTP_SESSION_STORE_DIR".to_owned(),
            runtime.display().to_string(),
        ),
        (
            "CORROBORE_OPENCTI_ELASTIC_FREE".to_owned(),
            "true".to_owned(),
        ),
        (
            "CORROBORE_OPENCTI_READ_ROUTING_POLICY_FILE".to_owned(),
            policy.display().to_string(),
        ),
        (
            "CORROBORE_HTTP_RATE_LIMIT_PER_SECOND".to_owned(),
            "1".to_owned(),
        ),
        ("CORROBORE_HTTP_RATE_LIMIT_BURST".to_owned(), "1".to_owned()),
        (
            "CORROBORE_OPENCTI_RATE_LIMIT_PER_SECOND".to_owned(),
            "250".to_owned(),
        ),
        (
            "CORROBORE_OPENCTI_RATE_LIMIT_BURST".to_owned(),
            "2000".to_owned(),
        ),
    ]))
    .unwrap();
    assert!(config.opencti_elastic_free);
    let app = build_router(AppState::new(config).unwrap());

    let initialize = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/opencti/reads")
                .header(header::AUTHORIZATION, "Bearer token-54")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(initialize_request().to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(initialize.status(), StatusCode::OK);
    let payload: Value =
        serde_json::from_slice(&to_bytes(initialize.into_body(), usize::MAX).await.unwrap())
            .unwrap();
    assert_eq!(payload["outcome"]["response"]["response"], "initialized");

    let write = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/opencti/writes")
                .header(header::AUTHORIZATION, "Bearer token-54")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(write_request("elastic-free-write").to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(write.status(), StatusCode::OK);

    let read = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/opencti/reads")
                .header(header::AUTHORIZATION, "Bearer token-54")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(read_request().to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(read.status(), StatusCode::OK);
    let payload: Value =
        serde_json::from_slice(&to_bytes(read.into_body(), usize::MAX).await.unwrap()).unwrap();
    assert_eq!(
        payload["outcome"]["response"]["data"]["id"],
        "indicator--elastic-free"
    );

    let status = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/opencti/writes/status")
                .header(header::AUTHORIZATION, "Bearer token-54")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let payload: Value =
        serde_json::from_slice(&to_bytes(status.into_body(), usize::MAX).await.unwrap()).unwrap();
    assert_eq!(payload["result"]["projection_outbox_depth"], 0);
    assert_eq!(payload["result"]["projection_lag"], 0);
    assert_eq!(payload["result"]["fully_synchronized"], true);

    let file = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/opencti/files")
                .header(header::AUTHORIZATION, "Bearer token-54")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "operation": "enqueue",
                        "descriptor": {
                            "file_id": "import/test.txt",
                            "source_object_id": "indicator--elastic-free",
                            "blob_key": "import/test.txt",
                            "name": "test.txt",
                            "mime_type": "text/plain",
                            "content_hash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                            "version": 1,
                            "access": {"marking_ids": [], "organization_ids": []}
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(file.status(), StatusCode::ACCEPTED);
    let file_payload: Value =
        serde_json::from_slice(&to_bytes(file.into_body(), usize::MAX).await.unwrap()).unwrap();
    assert_eq!(file_payload["ok"], true);
    assert_eq!(file_payload["result"], "enqueued");
    assert!(
        storage
            .join("file-content/metadata/file-jobs.json")
            .exists()
    );

    let version = app
        .oneshot(
            Request::builder()
                .uri("/version")
                .header(header::AUTHORIZATION, "Bearer token-54")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let payload: Value =
        serde_json::from_slice(&to_bytes(version.into_body(), usize::MAX).await.unwrap()).unwrap();
    assert_eq!(payload["opencti_mode"], "elastic_free");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn elastic_free_configuration_requires_persistence_and_forbids_a_reference() {
    let base = HashMap::from([
        (
            "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
            "token-54".to_owned(),
        ),
        (
            "CORROBORE_OPENCTI_ELASTIC_FREE".to_owned(),
            "true".to_owned(),
        ),
    ]);
    assert!(ServerConfig::from_map(&base).is_err());

    let root = unique_root();
    let mut persistent_without_policy = base.clone();
    persistent_without_policy.insert("CORROBORE_STORAGE_MODE".to_owned(), "persistent".to_owned());
    persistent_without_policy.insert(
        "CORROBORE_STORAGE_DIR".to_owned(),
        root.display().to_string(),
    );
    assert!(ServerConfig::from_map(&persistent_without_policy).is_err());

    let mut with_reference = base;
    with_reference.insert("CORROBORE_STORAGE_MODE".to_owned(), "persistent".to_owned());
    with_reference.insert(
        "CORROBORE_STORAGE_DIR".to_owned(),
        root.display().to_string(),
    );
    with_reference.insert(
        "CORROBORE_OPENCTI_SHADOW_REFERENCE_ENDPOINT".to_owned(),
        "https://reference.invalid/v1/knowledge-data".to_owned(),
    );
    with_reference.insert(
        "CORROBORE_OPENCTI_SHADOW_REFERENCE_VERSION".to_owned(),
        "opensearch-3.7.0".to_owned(),
    );
    assert!(ServerConfig::from_map(&with_reference).is_err());
}

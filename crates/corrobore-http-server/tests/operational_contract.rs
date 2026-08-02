// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

use std::{collections::HashMap, sync::Arc};

use axum::{
    body::{Body, to_bytes},
    http::{HeaderMap, Method, Request, StatusCode, header},
};
use corrobore_http_server::{AppState, ServerConfig, ServerLifecycle, build_router};
use serde_json::Value;
use tower::ServiceExt;

const REQUEST_ID: &str = "operation-client-01";

fn state_with_lifecycle(lifecycle: Arc<ServerLifecycle>) -> AppState {
    let config = ServerConfig::from_map(&HashMap::from([(
        "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
        "operational-secret".to_owned(),
    )]))
    .expect("configuration should parse");
    let mut state = AppState::new(config).expect("state should initialize");
    state.lifecycle = lifecycle;
    state
}

fn ready_state() -> AppState {
    AppState::new(
        ServerConfig::from_map(&HashMap::from([(
            "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
            "operational-secret".to_owned(),
        )]))
        .expect("configuration should parse"),
    )
    .expect("state should initialize")
}

async fn json_response(app: axum::Router, path: &str) -> (StatusCode, HeaderMap, Value) {
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(path)
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should respond");
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body should be readable");
    let payload = serde_json::from_slice(&body).expect("response should be JSON");
    (status, headers, payload)
}

#[tokio::test]
async fn liveness_only_reports_the_running_event_loop() {
    let lifecycle = Arc::new(ServerLifecycle::initializing());
    let app = build_router(state_with_lifecycle(Arc::clone(&lifecycle)));

    let (status, _, payload) = json_response(app, "/health/live").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(payload["status"], "live");
    assert_eq!(payload["live"], true);
    assert_eq!(payload["lifecycle_state"], "initializing");
    assert!(
        payload.get("ready").is_none(),
        "liveness must not claim readiness"
    );
}

#[tokio::test]
async fn readiness_is_false_before_initialization_and_during_draining() {
    for (lifecycle, expected_state) in [
        (Arc::new(ServerLifecycle::initializing()), "initializing"),
        (
            {
                let lifecycle = Arc::new(ServerLifecycle::initializing());
                lifecycle.mark_ready();
                lifecycle.begin_draining();
                lifecycle
            },
            "draining",
        ),
    ] {
        let app = build_router(state_with_lifecycle(lifecycle));
        let (status, _, payload) = json_response(app, "/health/ready").await;

        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(payload["status"], "not_ready");
        assert_eq!(payload["ready"], false);
        assert_eq!(payload["lifecycle_state"], expected_state);
    }
}

#[tokio::test]
async fn readiness_is_true_after_runtime_and_storage_initialization() {
    let app = build_router(ready_state());

    let (status, _, payload) = json_response(app, "/health/ready").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(payload["status"], "ready");
    assert_eq!(payload["ready"], true);
    assert_eq!(payload["lifecycle_state"], "ready");
    assert_eq!(payload["checks"]["engine_initialized"], true);
    assert_eq!(payload["checks"]["storage_recovered"], true);
    assert_eq!(payload["checks"]["accepting_requests"], true);
}

#[tokio::test]
async fn legacy_health_is_preserved_and_explicitly_deprecated() {
    let app = build_router(ready_state());

    let (status, headers, payload) = json_response(app, "/health").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(payload["status"], "ok");
    assert_eq!(
        headers
            .get("deprecation")
            .expect("legacy health should be deprecated"),
        "true"
    );
    assert_eq!(
        headers
            .get(header::LINK)
            .expect("legacy health should link to its successor"),
        "</health/ready>; rel=\"successor-version\""
    );
}

#[tokio::test]
async fn version_exposes_reproducible_build_and_storage_compatibility_metadata() {
    let app = build_router(ready_state());

    let (status, _, payload) = json_response(app, "/version").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(payload["service"], "corrobore-http-server");
    assert_eq!(payload["version"], env!("CARGO_PKG_VERSION"));
    assert!(payload["commit"].is_string());
    assert!(payload["build_target"].is_string());
    assert_eq!(
        payload["storage_compatibility"]["supported_versions"],
        serde_json::json!(["V1"])
    );
    assert_eq!(
        payload["storage_compatibility"]["supported_record_formats"],
        serde_json::json!(["JsonLinesV1"])
    );
    assert_eq!(
        payload["storage_compatibility"]["active_storage_version"],
        Value::Null
    );
    assert_eq!(
        payload["storage_compatibility"]["active_record_format"],
        Value::Null
    );
    let serialized = serde_json::to_string(&payload).expect("version payload should serialize");
    assert!(!serialized.contains("operational-secret"));
}

#[tokio::test]
async fn every_response_echoes_or_generates_a_correlation_id() {
    let app = build_router(ready_state());
    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/cypher/read")
        .header("x-request-id", REQUEST_ID)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"query":"MATCH (n) RETURN n"}"#))
        .expect("request should build");

    let response = app
        .clone()
        .oneshot(request)
        .await
        .expect("error request should respond");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response
            .headers()
            .get("x-request-id")
            .expect("response should echo the request ID"),
        REQUEST_ID
    );
    let payload: Value = serde_json::from_slice(
        &to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("error body should be readable"),
    )
    .expect("error should be JSON");
    assert_eq!(payload["correlation_id"], REQUEST_ID);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/health/live")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("liveness should respond");
    let generated = response
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .expect("response should contain a generated request ID");
    assert!(
        uuid::Uuid::parse_str(generated).is_ok(),
        "generated request ID should be a UUID, got {generated}"
    );
}

#[tokio::test]
async fn invalid_client_correlation_id_is_replaced() {
    let app = build_router(ready_state());
    let oversized_client_id = "a".repeat(200);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/health/live")
                .header("x-request-id", &oversized_client_id)
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("liveness should respond");
    let generated = response
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .expect("response should contain a generated request ID");

    assert_ne!(generated, oversized_client_id);
    assert!(uuid::Uuid::parse_str(generated).is_ok());
}

#[tokio::test]
async fn metrics_expose_lifecycle_readiness_activity_and_shutdown_state() {
    let state = ready_state();
    let lifecycle = Arc::clone(&state.lifecycle);
    lifecycle.begin_draining();
    let app = build_router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("metrics should respond");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("metrics body should be readable");
    let exposition = String::from_utf8(body.to_vec()).expect("metrics should be UTF-8");

    for expected in [
        "corrobore_lifecycle_state{state=\"draining\"} 1",
        "corrobore_ready 0",
        "corrobore_active_requests 0",
        "# TYPE corrobore_shutdown_started_total counter",
        "corrobore_shutdown_started_total 1",
        "# TYPE corrobore_shutdown_failures_total counter",
    ] {
        assert!(
            exposition.contains(expected),
            "metrics should contain {expected}, got:\n{exposition}"
        );
    }
}

#[test]
fn openapi_documents_all_operational_surfaces_and_correlation_header() {
    let openapi = include_str!("../../../docs/api/openapi.yaml");

    for path in ["/health/live:", "/health/ready:", "/version:", "/metrics:"] {
        assert!(openapi.contains(path), "OpenAPI is missing {path}");
    }
    assert!(
        openapi.contains("X-Request-Id"),
        "OpenAPI should document correlation propagation"
    );
    assert!(
        openapi.contains("deprecated: true"),
        "legacy /health should be explicitly deprecated"
    );
}

#[test]
fn openapi_versions_the_evidence_aware_stix_import_contract() {
    let openapi = include_str!("../../../docs/api/openapi.yaml");

    for contract in [
        "StixImportRequest:",
        "StixEvidenceEnvelopeV1:",
        "StixEvidenceRecordV1:",
        "StixEvidenceLocatorV1:",
        "StixRecordAnnotationV1:",
        "const: '1.0'",
        "content_sha256:",
        "const: candidate",
        "STIX 0-100 confidence normalized deterministically to native 0-1",
    ] {
        assert!(
            openapi.contains(contract),
            "OpenAPI is missing evidence import contract fragment: {contract}"
        );
    }
}

#[test]
fn openapi_documents_accountable_dependency_safe_stix_receipts() {
    let openapi = include_str!("../../../docs/api/openapi.yaml");

    for contract in [
        "StixImportObjectOutcome:",
        "StixImportMutationMetrics:",
        "enum: [created, updated, duplicate, rejected, unresolved_reference, failed]",
        "applied mutations.",
    ] {
        assert!(
            openapi.contains(contract)
                || include_str!("../../../docs/user-guide/http-server.md").contains(contract),
            "documentation is missing accountable STIX import contract: {contract}"
        );
    }
}

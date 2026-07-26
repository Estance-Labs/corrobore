// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
#![allow(clippy::unwrap_used)]

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
};
use corrobore_http_server::{AppState, ServerConfig, build_router};
use serde_json::{Value, json};
use tower::ServiceExt;

fn unique_path(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "corrobore-issue-52-http-{name}-{}-{unique}",
        std::process::id()
    ))
}

fn persistent_app(storage: &Path) -> axum::Router {
    let config = ServerConfig::from_map(&HashMap::from([
        (
            "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
            "token-123".to_owned(),
        ),
        (
            "CORROBORE_HTTP_ADMIN_AUTH_TOKEN".to_owned(),
            "admin-123".to_owned(),
        ),
        (
            "CORROBORE_HTTP_SESSION_STORE_DIR".to_owned(),
            unique_path("sessions").display().to_string(),
        ),
        ("CORROBORE_STORAGE_MODE".to_owned(), "persistent".to_owned()),
        (
            "CORROBORE_STORAGE_DIR".to_owned(),
            storage.display().to_string(),
        ),
    ]))
    .unwrap();
    build_router(AppState::new(config).unwrap())
}

async fn seed_checkpoint(app: &axum::Router) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/cypher/write")
                .header(header::AUTHORIZATION, "Bearer token-123")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"query": "CREATE (n:Indicator {name: 'snapshot.example'})"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn authenticated_online_snapshot_holds_the_canonical_barrier_and_updates_status() {
    let storage = unique_path("storage");
    let destination = unique_path("snapshot");
    let app = persistent_app(&storage);
    seed_checkpoint(&app).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/admin/storage/snapshots")
                .header(header::AUTHORIZATION, "Bearer admin-123")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "destination": destination,
                        "encryption_key_id": "kms://operations/http",
                        "retention_hook": "retain-30-days"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(destination.join("snapshot_manifest.json").is_file());

    let status = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/admin/storage/operations")
                .header(header::AUTHORIZATION, "Bearer admin-123")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(status.status(), StatusCode::OK);
    let payload: Value =
        serde_json::from_slice(&to_bytes(status.into_body(), usize::MAX).await.unwrap()).unwrap();
    assert_eq!(payload["result"]["snapshots_completed"], 1);
    assert!(payload["result"]["snapshot_bytes"].as_u64().unwrap() > 0);

    let metrics = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(metrics.status(), StatusCode::OK);
    let exposition = String::from_utf8(
        to_bytes(metrics.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(exposition.contains("corrobore_database_snapshots_total 1"));
    assert!(exposition.contains("corrobore_database_snapshot_failures_total 0"));
    assert!(exposition.contains("corrobore_database_snapshot_bytes "));
    assert!(exposition.contains("corrobore_database_snapshot_duration_ms "));

    drop(app);
    let _ = fs::remove_dir_all(storage);
    let _ = fs::remove_dir_all(destination);
}

#[tokio::test]
async fn database_operations_require_admin_authentication() {
    let storage = unique_path("unauthorized-storage");
    let app = persistent_app(&storage);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/admin/storage/indexes/rebuild")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    drop(app);
    let _ = fs::remove_dir_all(storage);
}

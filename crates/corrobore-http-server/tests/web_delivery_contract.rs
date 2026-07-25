// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.
#![allow(clippy::unwrap_used)]

use std::{fs, path::PathBuf};

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use corrobore_http_server::{AppState, ServerConfig, StorageMode, build_router};
use http_body_util::BodyExt;
use tower::ServiceExt;
use uuid::Uuid;

fn web_root() -> PathBuf {
    std::env::temp_dir().join(format!("corrobore-web-delivery-{}", Uuid::new_v4()))
}

fn config(web_dir: Option<String>) -> ServerConfig {
    ServerConfig {
        host: "127.0.0.1".to_owned(),
        port: 0,
        auth_mode: corrobore_http_server::security::AuthenticationMode::Required,
        auth_token: Some("test-token".to_owned()),
        auth_token_source: Some(corrobore_http_server::security::SecretSource::Inline),
        admin_auth_token: None,
        admin_auth_token_source: None,
        operational_endpoint_policy:
            corrobore_http_server::security::OperationalEndpointPolicy::Public,
        session_store_dir: web_root().to_string_lossy().into_owned(),
        log_dir: web_root().join("logs").to_string_lossy().into_owned(),
        request_timeout_ms: 1_000,
        shutdown_timeout_ms: 1_000,
        session_idle_ttl_ms: 0,
        max_body_bytes: 1_024,
        import_max_body_bytes: 2_048,
        opencti_sync_max_operations: 512,
        opencti_sync_max_replay_identities: 4_096,
        opencti_shadow: Default::default(),
        rate_limit_per_second: 1_000,
        rate_limit_burst: 1_000,
        web_dir,
        licensed_modules: vec!["cti".to_owned()],
        license_client_uuid: None,
        license_client_email: None,
        license_valid_until: None,
        license_is_nfr: None,
        storage_mode: StorageMode::Ephemeral,
        storage_dir: None,
        storage_require_fsync: false,
        storage_strict_recovery: false,
        storage_max_hot_nodes: 16_384,
        storage_max_hot_relationships: 32_768,
        storage_max_warm_adjacency_entries: 65_536,
        domain_provider_dir: None,
        domain_provider_manifest_file: None,
    }
}

#[tokio::test]
async fn production_web_assets_and_spa_routes_are_served_when_configured() {
    let root = web_root();
    fs::create_dir_all(root.join("assets")).unwrap();
    fs::write(
        root.join("index.html"),
        "<main data-testid=\"corrobore-web\">Corrobore explorer</main>",
    )
    .unwrap();
    fs::write(root.join("assets/app.js"), "console.log('corrobore')").unwrap();
    let app = build_router(
        AppState::new(config(Some(root.to_string_lossy().into_owned())))
            .expect("app state should initialize"),
    );

    let index = app
        .clone()
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(index.status(), StatusCode::OK);
    assert!(
        String::from_utf8(
            index
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec()
        )
        .unwrap()
        .contains("Corrobore explorer")
    );

    let asset = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/assets/app.js")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(asset.status(), StatusCode::OK);

    let client_route = app
        .oneshot(
            Request::builder()
                .uri("/sessions/session-alpha")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(client_route.status(), StatusCode::OK);
    assert!(
        String::from_utf8(
            client_route
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec(),
        )
        .unwrap()
        .contains("Corrobore explorer")
    );

    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn unknown_api_routes_never_fall_back_to_the_web_application() {
    let root = web_root();
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("index.html"), "Corrobore explorer").unwrap();
    let app = build_router(
        AppState::new(config(Some(root.to_string_lossy().into_owned())))
            .expect("app state should initialize"),
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/not-a-real-endpoint")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = String::from_utf8(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(!body.contains("Corrobore explorer"));

    fs::remove_dir_all(root).unwrap();
}

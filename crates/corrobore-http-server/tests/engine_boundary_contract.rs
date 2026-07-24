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
use corrobore_engine::{CypherResponseData, CypherResponseStatus};
use corrobore_http_server::{AppState, ServerConfig, build_router};
use serde_json::{Value, json};
use tower::ServiceExt;

fn test_state() -> AppState {
    let config = ServerConfig::from_map(&HashMap::from([(
        "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
        "boundary-token".to_owned(),
    )]))
    .expect("boundary test config should parse");

    AppState::new(config).expect("boundary test state should initialize")
}

fn persistent_test_state(storage_dir: &Path) -> AppState {
    let runtime_dir = storage_dir
        .parent()
        .expect("storage fixture should have a parent")
        .join("runtime");
    let config = ServerConfig::from_map(&HashMap::from([
        (
            "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
            "boundary-token".to_owned(),
        ),
        ("CORROBORE_STORAGE_MODE".to_owned(), "persistent".to_owned()),
        (
            "CORROBORE_STORAGE_DIR".to_owned(),
            storage_dir.display().to_string(),
        ),
        (
            "CORROBORE_HTTP_SESSION_STORE_DIR".to_owned(),
            runtime_dir.display().to_string(),
        ),
    ]))
    .expect("persistent boundary config should parse");

    AppState::new(config).expect("persistent boundary state should initialize")
}

fn persistent_temp_dir() -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should follow Unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("corrobore-engine-persistence-{suffix}"));
    fs::create_dir_all(&path).expect("persistent boundary directory should be created");
    path
}

#[tokio::test]
async fn embedded_mutation_is_visible_through_the_http_engine_boundary() {
    let state = test_state();
    {
        let mut engine = state
            .engine
            .lock()
            .expect("public engine lock should be available");
        let response = engine
            .write("CREATE (n:Indicator {name: 'embedded-boundary'})")
            .expect("embedded mutation should execute");
        assert_eq!(response.status, CypherResponseStatus::Success);
    }

    let app = build_router(state);
    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/cypher/read")
        .header(header::AUTHORIZATION, "Bearer boundary-token")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({"query": "MATCH (n:Indicator) RETURN n"}).to_string(),
        ))
        .expect("HTTP request should build");

    let response = app.oneshot(request).await.expect("HTTP request should run");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("HTTP body should be readable");
    let payload: Value = serde_json::from_slice(&body).expect("HTTP body should be JSON");
    assert_eq!(payload["result"]["status"], "Success");
    assert!(
        payload["result"]["data"]["Records"]
            .as_array()
            .is_some_and(|records| !records.is_empty()),
        "HTTP read should observe the mutation performed through the embedded API"
    );
}

#[tokio::test]
async fn http_auto_mutation_is_visible_through_the_embedded_engine_boundary() {
    let state = test_state();
    let app = build_router(state.clone());
    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/cypher/execute")
        .header(header::AUTHORIZATION, "Bearer boundary-token")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({"query": "CREATE (n:Indicator {name: 'http-boundary'})"}).to_string(),
        ))
        .expect("HTTP request should build");

    let response = app.oneshot(request).await.expect("HTTP request should run");
    assert_eq!(response.status(), StatusCode::OK);

    let mut engine = state
        .engine
        .lock()
        .expect("public engine lock should be available");
    let response = engine
        .read("MATCH (n:Indicator) RETURN n")
        .expect("embedded read should execute");
    assert_eq!(response.status, CypherResponseStatus::Success);
    assert!(
        matches!(
            response.data,
            CypherResponseData::Records(ref records) if !records.is_empty()
        ),
        "embedded read should observe the auto-routed mutation performed through HTTP"
    );
}

#[tokio::test]
async fn persistent_http_mutation_survives_app_state_restart() {
    let directory = persistent_temp_dir();
    let storage_dir = directory.join("graph");
    let first = persistent_test_state(&storage_dir);
    let write = Request::builder()
        .method(Method::POST)
        .uri("/v1/cypher/write")
        .header(header::AUTHORIZATION, "Bearer boundary-token")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({"query": "CREATE (n:Indicator {name: 'durable-boundary'})"}).to_string(),
        ))
        .expect("persistent write request should build");
    let response = build_router(first)
        .oneshot(write)
        .await
        .expect("persistent write should run");
    let write_status = response.status();
    let write_body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("persistent write body should be readable");
    assert_eq!(
        write_status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&write_body)
    );

    let restarted = persistent_test_state(&storage_dir);
    let read = Request::builder()
        .method(Method::POST)
        .uri("/v1/cypher/read")
        .header(header::AUTHORIZATION, "Bearer boundary-token")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({"query": "MATCH (n:Indicator) RETURN n"}).to_string(),
        ))
        .expect("persistent read request should build");
    let response = build_router(restarted)
        .oneshot(read)
        .await
        .expect("persistent read should run");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("persistent read body should be readable");
    let payload: Value = serde_json::from_slice(&body).expect("persistent read should be JSON");
    assert!(
        payload["result"]["data"]["Records"]
            .as_array()
            .is_some_and(|records| !records.is_empty()),
        "HTTP mutation must remain visible after rebuilding AppState from persistent storage"
    );
}

#[tokio::test]
async fn http_parameters_create_and_read_a_relationship_through_the_engine_boundary() {
    let state = test_state();
    let app = build_router(state);
    for (query, params) in [
        (
            "MERGE (n:Entity {id: $entityId})",
            json!({"entityId": "source"}),
        ),
        (
            "MERGE (n:Entity {id: $entityId})",
            json!({"entityId": "target"}),
        ),
        (
            "MATCH (s:Entity {id: $sourceId}) MERGE (s)-[r:TARGETS]->(o:Entity {id: $targetId}) SET r.fact_id = $factId",
            json!({"sourceId": "source", "targetId": "target", "factId": "fact-1"}),
        ),
    ] {
        let request = Request::builder()
            .method(Method::POST)
            .uri("/v1/cypher/write")
            .header(header::AUTHORIZATION, "Bearer boundary-token")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({"query": query, "params": params}).to_string(),
            ))
            .expect("HTTP mutation request should build");
        let response = app
            .clone()
            .oneshot(request)
            .await
            .expect("HTTP mutation should run");
        assert_eq!(response.status(), StatusCode::OK);
    }

    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/cypher/read")
        .header(header::AUTHORIZATION, "Bearer boundary-token")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({
                "query": "MATCH (s:Entity)-[r:TARGETS]->(o:Entity) WHERE s.id = $sourceId RETURN s.id, r.fact_id, o.id",
                "params": {"sourceId": "source"}
            })
            .to_string(),
        ))
        .expect("HTTP read request should build");
    let response = app.oneshot(request).await.expect("HTTP read should run");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("HTTP body should be readable");
    let payload: Value = serde_json::from_slice(&body).expect("HTTP body should be JSON");
    let fields = &payload["result"]["data"]["Records"][0]["fields"];
    assert_eq!(fields["s.id"], "source");
    assert_eq!(fields["r.fact_id"], "fact-1");
    assert_eq!(fields["o.id"], "target");
}

#[test]
fn dependency_boundary_keeps_server_dependencies_out_of_the_embedded_engine() {
    let engine_manifest = include_str!("../../corrobore-engine/Cargo.toml");
    let direct_dependencies = manifest_section(engine_manifest, "[dependencies]");
    for forbidden in [
        "axum",
        "clap",
        "dotenvy",
        "tokio",
        "tower",
        "tower-http",
        "tower_governor",
        "tracing-appender",
    ] {
        assert!(
            !direct_dependencies
                .lines()
                .any(|line| line.trim_start().starts_with(&format!("{forbidden} "))),
            "embedded engine must not depend directly on server-only crate '{forbidden}'"
        );
    }

    let server_manifest = include_str!("../Cargo.toml");
    let server_dependencies = manifest_section(server_manifest, "[dependencies]");
    assert!(
        server_dependencies
            .lines()
            .any(|line| line.trim_start().starts_with("corrobore-engine ")),
        "HTTP server must consume the public corrobore-engine boundary"
    );
}

#[test]
fn server_state_does_not_own_a_shared_runtime_gateway() {
    let app_source = include_str!("../src/app.rs");
    assert!(
        !app_source.contains("CypherGateway"),
        "AppState must own CorroboreEngine instead of a lower-level CypherGateway"
    );
    assert!(
        !app_source.contains("pub gateway:"),
        "the lower-level gateway must not remain part of server state"
    );
}

fn manifest_section<'a>(manifest: &'a str, heading: &str) -> &'a str {
    let start = manifest
        .find(heading)
        .unwrap_or_else(|| panic!("manifest should contain {heading}"));
    let rest = &manifest[start + heading.len()..];
    let end = rest.find("\n[").unwrap_or(rest.len());
    &rest[..end]
}

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
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode},
};
use corrobore_http_server::{AppState, ServerConfig, build_router};
use serde_json::{Value, json};
use std::collections::HashMap;
use tower::ServiceExt;
fn state(path: &std::path::Path) -> AppState {
    let config = ServerConfig::from_map(&HashMap::from([
        ("CORROBORE_HTTP_AUTH_TOKEN".into(), "token-123".into()),
        (
            "CORROBORE_HTTP_SESSION_STORE_DIR".into(),
            path.join("sessions").display().to_string(),
        ),
        ("CORROBORE_STORAGE_MODE".into(), "persistent".into()),
        (
            "CORROBORE_STORAGE_DIR".into(),
            path.join("storage").display().to_string(),
        ),
    ]))
    .expect("config");
    AppState::new(config).expect("state")
}
fn unique_path() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("candidate-http-{}", uuid::Uuid::new_v4()))
}
async fn send(app: &Router, method: Method, path: &str, value: Value) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", "Bearer token-123")
                .header("content-type", "application/json")
                .body(Body::from(value.to_string()))
                .expect("request"),
        )
        .await
        .expect("response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (
        status,
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned())),
    )
}
#[tokio::test]
async fn raw_candidate_survives_restart_and_promotion_is_explicit() {
    let path = unique_path();
    let raw = " { \"name\" : \"proposition\" } \n";
    {
        let state = state(&path);
        let app = build_router(state.clone());
        let (status, body) = send(&app, Method::POST, "/v1/import/candidates", json!({"id":"candidate--1","extraction_run_id":"run--1","actor":"actor--extractor","raw_payload":raw})).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["tier"], "Shadow");
        assert_eq!(body["candidate"]["raw_payload"], raw);
        assert!(
            state
                .engine
                .lock()
                .expect("lock")
                .graph()
                .list_nodes()
                .expect("nodes")
                .is_empty()
        );
    }
    {
        let state = state(&path);
        let app = build_router(state.clone());
        let (status, body) = send(
            &app,
            Method::GET,
            "/v1/import/candidates/candidate--1",
            Value::Null,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["candidate"]["raw_payload"], raw);
        let promotion = json!({"actor":"actor--reviewer","reason":"reviewed","record":{"kind":"node","labels":["Entity"],"properties":{}}});
        let (status, body) = send(
            &app,
            Method::POST,
            "/v1/import/candidates/candidate--1/promote",
            promotion.clone(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["tier"], "Canonical");
        let (_, retry) = send(
            &app,
            Method::POST,
            "/v1/import/candidates/candidate--1/promote",
            promotion,
        )
        .await;
        assert_eq!(body, retry);
        assert_eq!(
            state
                .engine
                .lock()
                .expect("lock")
                .graph()
                .list_nodes()
                .expect("nodes")
                .len(),
            1
        );
    }
    let state = state(&path);
    let app = build_router(state.clone());
    let (status, body) = send(
        &app,
        Method::GET,
        "/v1/import/candidates/candidate--1",
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["tier"], "Canonical");
    assert_eq!(body["candidate"]["raw_payload"], raw);
    assert_eq!(body["promotions"].as_array().expect("promotions").len(), 1);
}
#[tokio::test]
async fn missing_run_or_canonical_submission_never_writes_graph_state() {
    let state = state(&unique_path());
    let app = build_router(state.clone());
    for value in [
        json!({"id":"candidate--1","actor":"actor--extractor","raw_payload":"raw"}),
        json!({"id":"candidate--1","actor":"actor--extractor","extraction_run_id":"","raw_payload":"raw"}),
        json!({"id":"candidate--1","actor":"actor--extractor","extraction_run_id":"run--1","raw_payload":"raw","tier":"Canonical"}),
    ] {
        let (status, _) = send(&app, Method::POST, "/v1/import/candidates", value).await;
        assert!(status.is_client_error());
    }
    let engine = state.engine.lock().expect("lock");
    assert!(engine.graph().list_nodes().expect("nodes").is_empty());
    assert!(engine.graph().epistemic_stores().candidates.is_empty());
}
#[test]
fn legacy_stix_contract_cannot_silently_ignore_candidate_mode() {
    assert!(serde_json::from_value::<corrobore_http_server::handlers::import::ImportStixRequest>(json!({"bundle":{"type":"bundle","objects":[]},"candidate":true,"extraction_run_id":"run--1"})).is_err());
}

#[tokio::test]
async fn multipart_candidate_metadata_is_rejected_before_any_graph_write() {
    let state = state(&unique_path());
    let app = build_router(state.clone());
    for name in ["candidate", "extraction_run_id", "tier", "raw_payload"] {
        let body = format!(
            "--candidate-boundary\r\nContent-Disposition: form-data; name=\"file\"; filename=\"bundle.stix\"\r\nContent-Type: application/json\r\n\r\n{{\"type\":\"bundle\",\"objects\":[{{\"type\":\"identity\",\"id\":\"identity--1\",\"name\":\"proposal\"}}]}}\r\n--candidate-boundary\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\nShadow\r\n--candidate-boundary--\r\n"
        );
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/import/stix/file")
                    .header("authorization", "Bearer token-123")
                    .header(
                        "content-type",
                        "multipart/form-data; boundary=candidate-boundary",
                    )
                    .body(Body::from(body))
                    .expect("multipart request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        let body: Value = serde_json::from_slice(&bytes).expect("JSON error response");
        assert_eq!(body["error"]["code"], "UNKNOWN_IMPORT_FIELD");
    }
    assert!(
        state
            .engine
            .lock()
            .expect("engine lock")
            .graph()
            .list_nodes()
            .expect("nodes")
            .is_empty()
    );
}

#[tokio::test]
async fn every_candidate_route_requires_authentication() {
    let state = state(&unique_path());
    let app = build_router(state.clone());
    for (method, path) in [
        (Method::POST, "/v1/import/candidates"),
        (Method::GET, "/v1/import/candidates/candidate--1"),
        (Method::POST, "/v1/import/candidates/candidate--1/promote"),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .expect("unauthenticated request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
    assert!(
        state
            .engine
            .lock()
            .expect("engine lock")
            .graph()
            .epistemic_stores()
            .candidates
            .is_empty()
    );
}

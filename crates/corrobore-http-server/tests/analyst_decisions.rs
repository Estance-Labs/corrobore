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
async fn analyst_override_and_reversal_are_persisted_and_audited_separately() {
    use graph_core::*;
    let path = unique_path();
    let app_state = state(&path);
    app_state
        .engine
        .lock()
        .expect("lock")
        .mutate_graph_atomically(
            corrobore_engine::EngineMutationContext::new("test", "human", "human"),
            |graph| {
                graph
                    .epistemic_stores_mut()
                    .claims
                    .create_asserted_claim(ClaimInput::new(
                        ClaimId::new("claim")?,
                        ClaimStatement::new("Stored claim")?,
                        ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new(
                            "subject", None,
                        )),
                    ))?;
                Ok(())
            },
        )
        .expect("seed");
    let app = build_router(app_state.clone());
    let unauthorized = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/claims/claim/decisions")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    let payload = json!({"id":"override", "actor":"analyst", "recorded_at":"2026-09-06T18:00:00Z", "action":{"kind":"override", "judgment":"Human conclusion", "rationale":"Reviewed original source"}});
    let (status, body) = send(
        &app,
        Method::POST,
        "/v1/claims/claim/decisions",
        payload.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        send(
            &app,
            Method::POST,
            "/v1/claims/claim/decisions",
            payload.clone()
        )
        .await
        .1
    );
    let mut invalid = payload.clone();
    invalid["actor"] = json!("");
    assert_eq!(
        send(&app, Method::POST, "/v1/claims/claim/decisions", invalid)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        send(&app, Method::POST, "/v1/claims/unknown/decisions", payload)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let reversal = json!({"id":"reverse", "actor":"analyst", "recorded_at":"2026-09-06T19:00:00Z", "action":{"kind":"reversal", "decision_id":"override", "rationale":"Withdrawn after review"}});
    assert_eq!(
        send(&app, Method::POST, "/v1/claims/claim/decisions", reversal)
            .await
            .0,
        StatusCode::OK
    );
    let (status, audit) = send(&app, Method::GET, "/v1/claims/claim/audit", json!(null)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        audit["analyst_decisions"]
            .as_array()
            .expect("human decisions")
            .len(),
        2
    );
    assert!(audit["current_verdict"].is_null());
    drop(app);
    drop(app_state);
    let app = build_router(state(&path));
    assert_eq!(
        audit,
        send(&app, Method::GET, "/v1/claims/claim/audit", json!(null))
            .await
            .1
    );
    drop(app);
    std::fs::remove_dir_all(path).expect("cleanup");
}

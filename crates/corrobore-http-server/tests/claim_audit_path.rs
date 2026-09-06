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
async fn audit_and_projection_are_authenticated_read_only_and_survive_restart() {
    use graph_core::*;
    let path = unique_path();
    let expected;
    {
        let state = state(&path);
        state
            .engine
            .lock()
            .expect("lock")
            .mutate_graph_atomically(
                corrobore_engine::EngineMutationContext::new("test", "audit", "audit"),
                |graph| {
                    graph
                        .epistemic_stores_mut()
                        .claims
                        .create_asserted_claim(ClaimInput::new(
                            ClaimId::new("audit-claim")?,
                            ClaimStatement::new("Stored assertion")?,
                            ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new(
                                "audit", None,
                            )),
                        ))?;
                    Ok(())
                },
            )
            .expect("seed");
        let before = serde_json::to_value(
            state
                .engine
                .lock()
                .expect("lock")
                .graph()
                .persistence_snapshot(),
        )
        .expect("snapshot");
        let app = build_router(state.clone());
        let (status, body) = send(
            &app,
            Method::GET,
            "/v1/claims/audit-claim/audit",
            json!(null),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(body["current_verdict"].is_null());
        assert!(
            !body["unverified_steps"]
                .as_array()
                .expect("gaps")
                .is_empty()
        );
        expected = body;
        for endpoint in ["/v1/claims/audit-claim/audit", "/v1/epistemic/projection"] {
            let (status, first) = send(&app, Method::GET, endpoint, json!(null)).await;
            assert_eq!(status, StatusCode::OK, "{first}");
            assert_eq!(
                first,
                send(&app, Method::GET, endpoint, json!(null)).await.1
            );
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(endpoint)
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response");
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
        assert_eq!(
            send(&app, Method::GET, "/v1/claims/unknown/audit", json!(null))
                .await
                .0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            before,
            serde_json::to_value(
                state
                    .engine
                    .lock()
                    .expect("lock")
                    .graph()
                    .persistence_snapshot()
            )
            .expect("snapshot")
        );
    }
    {
        let app = build_router(state(&path));
        assert_eq!(
            expected,
            send(
                &app,
                Method::GET,
                "/v1/claims/audit-claim/audit",
                json!(null)
            )
            .await
            .1
        );
    }
    std::fs::remove_dir_all(path).expect("cleanup");
}

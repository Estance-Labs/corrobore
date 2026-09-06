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
async fn all_stage_metrics_are_authenticated_engine_reports_and_do_not_mutate_evidence() {
    use graph_core::{PipelineStage, StageMeasurement};
    let path = unique_path();
    let app_state = state(&path);
    let before = serde_json::to_value(
        app_state
            .engine
            .lock()
            .expect("engine")
            .graph()
            .persistence_snapshot(),
    )
    .expect("snapshot");
    let app = build_router(app_state.clone());
    let endpoint = "/v1/metrics/stages/regression";
    for method in [Method::GET, Method::POST] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(endpoint)
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
    assert_eq!(
        send(&app, Method::GET, endpoint, json!(null)).await.0,
        StatusCode::NOT_FOUND
    );
    for stage in PipelineStage::ALL {
        let failures = if stage == PipelineStage::Retrieval {
            3
        } else {
            0
        };
        let payload = serde_json::to_value(
            StageMeasurement::new("sample", stage, "fixture-v1", 10, 10 - failures, failures)
                .expect("measurement"),
        )
        .expect("json");
        let (status, report) = send(&app, Method::POST, endpoint, payload.clone()).await;
        assert_eq!(status, StatusCode::OK, "{report}");
        assert_eq!(send(&app, Method::POST, endpoint, payload).await.1, report);
    }
    let (status, report) = send(&app, Method::GET, endpoint, json!(null)).await;
    assert_eq!(status, StatusCode::OK);
    let fixture: Value = serde_json::from_str(include_str!(
        "../../graph-core/tests/fixtures/pipeline-stage-metrics-v1.json"
    ))
    .expect("fixture");
    assert_eq!(report, fixture);
    assert_eq!(
        serde_json::to_value(
            app_state
                .engine
                .lock()
                .expect("engine")
                .pipeline_stage_report("regression")
                .expect("report")
        )
        .expect("json"),
        fixture
    );
    assert_eq!(
        before,
        serde_json::to_value(
            app_state
                .engine
                .lock()
                .expect("engine")
                .graph()
                .persistence_snapshot()
        )
        .expect("snapshot")
    );
    let invalid = json!({"schema_version":"v999","measurement_id":"bad","stage":"verdict","producer":"p","inputs":1,"outputs":1,"failures":0});
    assert_eq!(
        send(&app, Method::POST, endpoint, invalid).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        send(&app, Method::GET, endpoint, json!(null)).await.1,
        fixture
    );
    drop(app);
    drop(app_state);
    std::fs::remove_dir_all(path).expect("cleanup");
}

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
    http::{Request, StatusCode},
};
use corrobore_http_server::{AppState, ServerConfig, build_router};

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
#[path = "../../graph-core/tests/support/ingestion_quality.rs"]
mod fixtures;
async fn scrape(app: Router) -> String {
    let response = app
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    String::from_utf8(
        to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body")
            .to_vec(),
    )
    .expect("text")
}
#[tokio::test]
async fn metrics_exports_independent_quality_series_and_restores_them_after_restart() {
    let path = unique_path();
    {
        let state = state(&path);
        state
            .engine
            .lock()
            .expect("engine")
            .mutate_graph_atomically(
                corrobore_engine::EngineMutationContext::new("http-default", "test", "test"),
                |g| {
                    *g = fixtures::seeded();
                    g.create_node(graph_core::NodeInput::new(["UnrelatedCanonicalPayload"]))?;
                    Ok(())
                },
            )
            .expect("seed");
    }
    let restored = state(&path);
    assert!(
        restored
            .engine
            .lock()
            .expect("engine")
            .graph()
            .list_nodes()
            .expect("nodes")
            .is_empty()
    );
    let text = scrape(build_router(restored.clone())).await;
    assert!(
        restored
            .engine
            .lock()
            .expect("engine")
            .graph()
            .list_nodes()
            .expect("nodes")
            .is_empty(),
        "scraping must not hydrate canonical payloads"
    );
    for line in [
        "corrobore_ingestion_repair_success_rate 0.25",
        "corrobore_ingestion_false_repair_rate 0.5",
        "corrobore_ingestion_extraction_accuracy 0.5",
        "corrobore_ingestion_reconciliation_accuracy{outcome=\"merge\"} 0.5",
        "corrobore_ingestion_reconciliation_accuracy{outcome=\"distinct\"} 1",
        "corrobore_ingestion_reconciliation_accuracy{outcome=\"abstain\"} 0.5",
        "corrobore_ingestion_abstain_rate 0.4",
        "corrobore_ingestion_reviewed_repair_count 4",
    ] {
        assert!(
            text.lines().any(|actual| actual == line),
            "missing {line} in {text}"
        );
    }
    assert!(!text.contains("fixture ground truth"));
    std::fs::remove_dir_all(path).expect("cleanup");
}
#[tokio::test]
async fn unevaluated_metrics_are_nan_not_perfect_or_zero_accuracy() {
    let path = unique_path();
    let text = scrape(build_router(state(&path))).await;
    assert!(
        text.lines()
            .any(|line| line == "corrobore_ingestion_repair_success_rate NaN")
    );
    assert!(
        text.lines()
            .any(|line| line == "corrobore_ingestion_false_repair_rate NaN")
    );
    assert!(
        text.lines()
            .any(|line| line == "corrobore_ingestion_abstain_rate NaN")
    );
    std::fs::remove_dir_all(path).expect("cleanup");
}

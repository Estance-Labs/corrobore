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

fn actor() -> ReconciliationDecider {
    ReconciliationDecider::Actor(ActorId::new("reviewer").expect("actor"))
}
fn mention(graph: &mut Graph, id: &str, surface: &str, context: &str) -> EntityMentionId {
    let source = SourceId::new(format!("source--{id}")).expect("source");
    let observation = ObservationId::new(format!("observation--{id}")).expect("observation");
    let stores = graph.epistemic_stores_mut();
    stores
        .sources
        .register_source(SourceInput::new(
            source.clone(),
            format!("https://example.org/{id}"),
            EvidenceSourceType::Document,
        ))
        .expect("source");
    stores
        .observations
        .create_observation(
            ObservationInput::new(
                observation.clone(),
                source,
                surface,
                ObservationModality::Text,
            ),
            &stores.sources,
        )
        .expect("observe");
    graph
        .create_entity_mention(
            EntityMentionInput::new(
                EntityMentionId::new(id).expect("mention"),
                observation,
                MentionOffsets {
                    start: 0,
                    end: surface.len() as u64,
                },
                surface,
            )
            .with_features(MentionFeatures {
                source_context: Some(context.into()),
                ..Default::default()
            }),
        )
        .expect("mention")
}
fn input(
    id: &str,
    left: &EntityMentionId,
    right: &EntityMentionId,
    outcome: ReconciliationOutcome,
    feature: ReconciliationFeature,
) -> ReconciliationInput {
    ReconciliationInput::new(
        ReconciliationRecordId::new(id).expect("id"),
        left.clone(),
        right.clone(),
        outcome,
        actor(),
        TemporalTimestamp::new("2026-09-06T12:00:00Z").expect("time"),
        "Reviewed source-grounded identity evidence",
    )
    .with_evidence(vec![
        ReconciliationEvidence::Mention {
            mention_id: left.clone(),
            feature,
        },
        ReconciliationEvidence::Mention {
            mention_id: right.clone(),
            feature,
        },
    ])
}

use graph_core::*;
fn seeded(path: &std::path::Path) -> AppState {
    let state = state(path);
    state
        .engine
        .lock()
        .expect("engine")
        .mutate_graph_atomically(
            corrobore_engine::EngineMutationContext::new("http-default", "test", "test"),
            |g| {
                let a = mention(g, "a", "IBM", "registry C001");
                let b = mention(g, "b", "International Business Machines", "registry C001");
                g.record_reconciliation(input(
                    "merge",
                    &a,
                    &b,
                    ReconciliationOutcome::Merge,
                    ReconciliationFeature::SourceContext,
                ))?;
                Ok(())
            },
        )
        .expect("seed");
    state
}
#[tokio::test]
async fn analyst_inspects_applies_and_undoes_a_merge_durably() {
    let path = unique_path();
    {
        let state = seeded(&path);
        let app = build_router(state);
        let (status, body) =
            send(&app, Method::GET, "/v1/reconciliations/merge", json!(null)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["active"], false);
        let (status, body) = send(
            &app,
            Method::POST,
            "/v1/reconciliations/merge/merge",
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["active"], true);
    }
    {
        let app = build_router(state(&path));
        let (status, body) =
            send(&app, Method::GET, "/v1/reconciliations/merge", json!(null)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["active"], true);
        let payload = json!({"id":"undo1","actor":"analyst","undone_at":"2026-09-06T13:00:00Z","rationale":"Registry correction"});
        for _ in 0..2 {
            let (status, body) = send(
                &app,
                Method::POST,
                "/v1/reconciliations/merge/undo",
                payload.clone(),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_eq!(body["active"], false);
            assert_eq!(body["undos"].as_array().expect("undos").len(), 1);
        }
    }
    let app = build_router(state(&path));
    let (status, body) = send(&app, Method::GET, "/v1/reconciliations/merge", json!(null)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["record"]["outcome"], "Merge");
    assert_eq!(body["undos"][0]["actor"]["value"], "analyst");
    std::fs::remove_dir_all(path).expect("cleanup");
}
#[tokio::test]
async fn dependent_reconciliation_returns_structured_conflict_and_keeps_merge_active() {
    let path = unique_path();
    let state = seeded(&path);
    let app = build_router(state.clone());
    let (status, body) = send(
        &app,
        Method::POST,
        "/v1/reconciliations/merge/merge",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let dependent = input(
        "later",
        &EntityMentionId::new("a").expect("id"),
        &EntityMentionId::new("b").expect("id"),
        ReconciliationOutcome::Abstain,
        ReconciliationFeature::SourceContext,
    );
    let (status, body) = send(
        &app,
        Method::POST,
        "/v1/reconciliations",
        json!({"record":dependent}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status,body)=send(&app,Method::POST,"/v1/reconciliations/merge/undo",json!({"id":"undo1","actor":"analyst","undone_at":"2026-09-06T13:00:00Z","rationale":"Review"})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "DEPENDENT_RECONCILIATION");
    assert_eq!(body["error"]["dependent_record"], "later");
    let (_, body) = send(&app, Method::GET, "/v1/reconciliations/merge", json!(null)).await;
    assert_eq!(body["active"], true);
    assert_eq!(body["undos"], json!([]));
    drop(app);
    drop(state);
    std::fs::remove_dir_all(path).expect("cleanup");
}

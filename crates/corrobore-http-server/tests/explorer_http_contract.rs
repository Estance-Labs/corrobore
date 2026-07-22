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
use std::{collections::HashMap, path::PathBuf};

use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
};
use corrobore_http_server::{
    AppState, ServerConfig, build_router,
    explorer_timeline::{ExplorerTimelineStore, ExplorerTimeshotInput},
    session_runtime::{SessionRuntime, StartSessionInput},
};
use graph_core::{ActorId, SnapshotCreateRequest, SnapshotId, SnapshotManager, TransactionId};
use serde_json::Value;
use shared_runtime::ActorKind;
use tower::ServiceExt;

fn unique_store_dir(suffix: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "corrobore-explorer-http-{suffix}-{}",
        uuid::Uuid::new_v4()
    ))
}

fn test_state(suffix: &str) -> AppState {
    let store_dir = unique_store_dir(suffix);
    let config = ServerConfig::from_map(&HashMap::from([
        (
            "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
            "token-123".to_owned(),
        ),
        (
            "CORROBORE_HTTP_SESSION_STORE_DIR".to_owned(),
            store_dir.display().to_string(),
        ),
    ]))
    .expect("configuration should parse");
    AppState::new(config).expect("app state should initialize")
}

fn authorized_get(uri: impl AsRef<str>) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(uri.as_ref())
        .header(header::AUTHORIZATION, "Bearer token-123")
        .body(Body::empty())
        .expect("request should build")
}

async fn json_body(response: axum::response::Response) -> Value {
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body should be readable");
    serde_json::from_slice(&bytes).expect("response body should be JSON")
}

fn start_session(runtime: &mut SessionRuntime, workspace_id: &str, actor_id: &str) -> String {
    runtime
        .start_session(StartSessionInput {
            workspace_id: workspace_id.to_owned(),
            actor_id: actor_id.to_owned(),
            actor_kind: ActorKind::Agent,
            metadata: HashMap::new(),
        })
        .expect("session should start")
        .session_id
}

fn seed_timeline(state: &AppState) -> (String, String) {
    let mut sessions = state.sessions.lock().expect("sessions lock should work");
    let owner_id = start_session(&mut sessions, "workspace--owner", "actor--owner");
    let other_id = start_session(&mut sessions, "workspace--other", "actor--other");
    let owner = sessions
        .session_health(&owner_id)
        .expect("owner session should exist");
    drop(sessions);

    let mut manager = SnapshotManager::new();
    let snapshot = manager
        .create_snapshot(
            SnapshotCreateRequest::new(
                SnapshotId::new("snapshot--http-baseline").expect("snapshot id should be valid"),
                TransactionId::new("transaction--http-40").expect("transaction id should be valid"),
                ActorId::new("actor--owner").expect("actor id should be valid"),
                "HTTP explorer checkpoint",
                "baseline",
            )
            .expect("snapshot request should be valid"),
            "2026-07-17T04:00:00Z",
        )
        .expect("snapshot should be created");

    let mut timeline = state.timeline.lock().expect("timeline lock should work");
    timeline
        .record_snapshot(&owner, &snapshot, None)
        .expect("snapshot should be recorded");
    timeline
        .record_timeshot(
            &owner,
            ExplorerTimeshotInput::new(
                "timeshot--http-analysis",
                "snapshot--http-baseline",
                Some("transaction--http-41"),
                "2026-07-17T04:01:00Z",
                "analysis",
            )
            .expect("timeshot should be valid"),
        )
        .expect("timeshot should be recorded");

    (owner_id, other_id)
}

#[tokio::test]
async fn explorer_routes_require_bearer_authentication() {
    let app = build_router(test_state("auth"));
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/explorer/sessions")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("route should respond");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn explorer_session_listing_excludes_stopped_by_default_and_can_include_them() {
    let state = test_state("sessions");
    let (owner_id, other_id) = {
        let mut sessions = state.sessions.lock().expect("sessions lock should work");
        let owner = start_session(&mut sessions, "workspace--owner", "actor--owner");
        let other = start_session(&mut sessions, "workspace--other", "actor--other");
        sessions.stop_session(&other).expect("other should stop");
        (owner, other)
    };
    let app = build_router(state);

    let current = app
        .clone()
        .oneshot(authorized_get("/v1/explorer/sessions"))
        .await
        .expect("current session route should respond");
    assert_eq!(current.status(), StatusCode::OK);
    let current = json_body(current).await;
    assert_eq!(
        current["result"]["sessions"]
            .as_array()
            .expect("sessions should be an array")
            .len(),
        1
    );
    assert_eq!(current["result"]["sessions"][0]["session_id"], owner_id);

    let all = app
        .oneshot(authorized_get("/v1/explorer/sessions?include_stopped=true"))
        .await
        .expect("all session route should respond");
    let all = json_body(all).await;
    let ids = all["result"]["sessions"]
        .as_array()
        .expect("sessions should be an array")
        .iter()
        .map(|session| {
            session["session_id"]
                .as_str()
                .expect("session id should be a string")
        })
        .collect::<Vec<_>>();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&owner_id.as_str()));
    assert!(ids.contains(&other_id.as_str()));
    assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));
}

#[tokio::test]
async fn timeline_and_projection_routes_preserve_the_selected_boundary() {
    let state = test_state("timeline-projection");
    let (owner_id, _) = seed_timeline(&state);
    let app = build_router(state);

    let timeline = app
        .clone()
        .oneshot(authorized_get(format!(
            "/v1/explorer/sessions/{owner_id}/timeline"
        )))
        .await
        .expect("timeline route should respond");
    assert_eq!(timeline.status(), StatusCode::OK);
    let timeline = json_body(timeline).await;
    assert_eq!(timeline["result"]["session_id"], owner_id);
    assert_eq!(
        timeline["result"]["roots"][0]["boundary"]["boundary_id"],
        "snapshot--http-baseline"
    );
    assert_eq!(
        timeline["result"]["roots"][0]["children"][0]["boundary"]["kind"],
        "timeshot"
    );

    let graph = app
        .clone()
        .oneshot(authorized_get(format!(
            "/v1/explorer/sessions/{owner_id}/graph?boundary_kind=snapshot&boundary_id=snapshot--http-baseline&max_nodes=10&max_relationships=10&max_properties_per_record=10&max_payload_bytes=16384&max_computation_units=100"
        )))
        .await
        .expect("graph route should respond");
    assert_eq!(graph.status(), StatusCode::OK);
    let graph = json_body(graph).await;
    assert_eq!(graph["result"]["boundary"]["kind"], "snapshot");
    assert_eq!(
        graph["result"]["boundary"]["boundary_id"],
        "snapshot--http-baseline"
    );
    assert_eq!(
        graph["result"]["boundary"]["transaction_id"],
        "transaction--http-40"
    );

    let timeshot = app
        .oneshot(authorized_get(format!(
            "/v1/explorer/sessions/{owner_id}/graph?boundary_kind=timeshot&boundary_id=timeshot--http-analysis"
        )))
        .await
        .expect("timeshot graph route should respond");
    assert_eq!(timeshot.status(), StatusCode::OK);
    let timeshot = json_body(timeshot).await;
    assert_eq!(timeshot["result"]["boundary"]["kind"], "timeshot");
    assert_eq!(
        timeshot["result"]["boundary"]["boundary_id"],
        "timeshot--http-analysis"
    );
    assert_eq!(timeshot["result"]["boundary"]["at"], "2026-07-17T04:01:00Z");
}

#[tokio::test]
async fn unknown_and_cross_session_boundaries_return_leak_safe_typed_errors() {
    let state = test_state("boundary-errors");
    let (owner_id, other_id) = seed_timeline(&state);
    let app = build_router(state);

    let unknown_session = app
        .clone()
        .oneshot(authorized_get(
            "/v1/explorer/sessions/00000000-0000-0000-0000-000000000000/timeline",
        ))
        .await
        .expect("unknown session route should respond");
    assert_eq!(unknown_session.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        json_body(unknown_session).await["error"]["code"],
        "SESSION_NOT_FOUND"
    );

    let cross_session = app
        .clone()
        .oneshot(authorized_get(format!(
            "/v1/explorer/sessions/{other_id}/graph?boundary_kind=snapshot&boundary_id=snapshot--http-baseline"
        )))
        .await
        .expect("cross-session route should respond");
    assert_eq!(cross_session.status(), StatusCode::NOT_FOUND);
    let cross_session = json_body(cross_session).await;
    assert_eq!(
        cross_session["error"]["code"],
        "TEMPORAL_BOUNDARY_NOT_FOUND"
    );
    assert!(!cross_session.to_string().contains(&owner_id));

    let unknown_boundary = app
        .oneshot(authorized_get(format!(
            "/v1/explorer/sessions/{owner_id}/graph?boundary_kind=timeshot&boundary_id=timeshot--missing"
        )))
        .await
        .expect("unknown boundary route should respond");
    assert_eq!(unknown_boundary.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        json_body(unknown_boundary).await["error"]["code"],
        "TEMPORAL_BOUNDARY_NOT_FOUND"
    );
}

#[tokio::test]
async fn graph_projection_rejects_invalid_selection_and_budget_with_typed_errors() {
    let state = test_state("invalid-projection");
    let session_id = {
        let mut sessions = state.sessions.lock().expect("sessions lock should work");
        start_session(&mut sessions, "workspace--projection", "actor--projection")
    };
    let app = build_router(state);

    let invalid_selection = app
        .clone()
        .oneshot(authorized_get(format!(
            "/v1/explorer/sessions/{session_id}/graph?boundary_kind=current&boundary_id=snapshot--unexpected"
        )))
        .await
        .expect("invalid selection should respond");
    assert_eq!(invalid_selection.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        json_body(invalid_selection).await["error"]["code"],
        "INVALID_TEMPORAL_BOUNDARY"
    );

    let invalid_budget = app
        .oneshot(authorized_get(format!(
            "/v1/explorer/sessions/{session_id}/graph?max_nodes=0"
        )))
        .await
        .expect("invalid budget should respond");
    assert_eq!(invalid_budget.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        json_body(invalid_budget).await["error"]["code"],
        "INVALID_VISUALIZATION_PROJECTION"
    );
}

#[test]
fn app_state_uses_the_configured_store_for_timeline_persistence() {
    let state = test_state("store-wiring");
    let expected = PathBuf::from(&state.config.session_store_dir).join("explorer-timeline.json");
    let actual = state
        .timeline
        .lock()
        .expect("timeline lock should work")
        .store_file()
        .to_path_buf();
    assert_eq!(actual, expected);
    assert!(matches!(
        ExplorerTimelineStore::new(&state.config.session_store_dir).store_file(),
        path if path == expected
    ));
}

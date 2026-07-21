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
use std::collections::HashMap;

use graph_core::{ActorId, SessionId, WorkspaceId};
use shared_runtime::{
    ActorKind, ActorRef, RuntimeError, RuntimeSessionMetadata, RuntimeTimestamp, SessionRegistry,
    StartSessionRequest,
};

fn actor_ref(value: &str) -> ActorRef {
    ActorRef::new(
        ActorId::new(value).expect("actor id should be valid"),
        ActorKind::Agent,
    )
}

#[test]
fn start_session_registers_runtime_context() {
    let mut registry = SessionRegistry::default();
    let session_id = SessionId::new("session--runtime-alpha").expect("session id should be valid");
    let workspace_id =
        WorkspaceId::new("workspace--runtime-alpha").expect("workspace id should be valid");
    let actor = actor_ref("actor--runtime-alpha");
    let mut metadata = HashMap::new();
    metadata.insert("request_id".to_owned(), "request--001".to_owned());

    let created_session_id = registry
        .start_session(StartSessionRequest {
            id: session_id.clone(),
            actor: Some(actor.clone()),
            workspace_id: workspace_id.clone(),
            started_at: RuntimeTimestamp::from_millis(1_727_500_000_000),
            metadata: metadata.clone(),
        })
        .expect("session start should succeed");

    let session = registry
        .session(&created_session_id)
        .expect("session should be retrievable after start");

    assert_eq!(session.id, session_id);
    assert_eq!(session.actor, actor);
    assert_eq!(session.workspace_id, workspace_id);
    assert_eq!(session.metadata, metadata);
}

#[test]
fn start_session_rejects_missing_actor_metadata() {
    let mut registry = SessionRegistry::default();

    let error = registry
        .start_session(StartSessionRequest {
            id: SessionId::new("session--missing-actor").expect("session id should be valid"),
            actor: None,
            workspace_id: WorkspaceId::new("workspace--missing-actor")
                .expect("workspace id should be valid"),
            started_at: RuntimeTimestamp::from_millis(10),
            metadata: HashMap::new(),
        })
        .expect_err("missing actor metadata should be rejected");

    assert!(matches!(error, RuntimeError::MissingActor));
}

#[test]
fn read_session_metadata_returns_registered_context() {
    let mut registry = SessionRegistry::default();
    let session_id = SessionId::new("session--metadata").expect("session id should be valid");
    let workspace_id =
        WorkspaceId::new("workspace--metadata").expect("workspace id should be valid");
    let actor = actor_ref("actor--metadata");

    registry
        .start_session(StartSessionRequest {
            id: session_id.clone(),
            actor: Some(actor.clone()),
            workspace_id: workspace_id.clone(),
            started_at: RuntimeTimestamp::from_millis(42),
            metadata: HashMap::new(),
        })
        .expect("session start should succeed");

    let session_metadata = registry
        .read_session_metadata(&session_id)
        .expect("session metadata should be retrievable");

    assert_eq!(
        session_metadata,
        RuntimeSessionMetadata {
            id: session_id,
            actor,
            workspace_id,
            started_at: RuntimeTimestamp::from_millis(42),
            metadata: HashMap::new(),
        }
    );
}

#[test]
fn read_session_metadata_rejects_missing_session() {
    let registry = SessionRegistry::default();

    let error = registry
        .read_session_metadata(
            &SessionId::new("session--unknown").expect("session id should be valid"),
        )
        .expect_err("missing session should fail metadata read");

    assert!(matches!(error, RuntimeError::SessionNotFound(_)));
}

#[test]
fn validate_workspace_session_consistency_detects_mismatch() {
    let mut registry = SessionRegistry::default();
    let session_id = SessionId::new("session--scope").expect("session id should be valid");

    registry
        .start_session(StartSessionRequest {
            id: session_id.clone(),
            actor: Some(actor_ref("actor--scope")),
            workspace_id: WorkspaceId::new("workspace--registered")
                .expect("workspace id should be valid"),
            started_at: RuntimeTimestamp::from_millis(200),
            metadata: HashMap::new(),
        })
        .expect("session start should succeed");

    let error = registry
        .validate_workspace_session_consistency(
            &WorkspaceId::new("workspace--request").expect("workspace id should be valid"),
            &session_id,
        )
        .expect_err("mismatch should be explicit");

    assert!(matches!(
        error,
        RuntimeError::WorkspaceSessionMismatch { .. }
    ));
}

#[test]
fn validate_workspace_session_consistency_accepts_matching_workspace() {
    let mut registry = SessionRegistry::default();
    let session_id = SessionId::new("session--matching").expect("session id should be valid");
    let workspace_id =
        WorkspaceId::new("workspace--matching").expect("workspace id should be valid");

    registry
        .start_session(StartSessionRequest {
            id: session_id.clone(),
            actor: Some(actor_ref("actor--matching")),
            workspace_id: workspace_id.clone(),
            started_at: RuntimeTimestamp::from_millis(250),
            metadata: HashMap::new(),
        })
        .expect("session start should succeed");

    let validation = registry.validate_workspace_session_consistency(&workspace_id, &session_id);

    assert!(validation.is_ok());
}

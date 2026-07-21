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
    ActorKind, ActorRef, CreateWorkspaceRequest, CypherBudgetRef, CypherParameters, CypherRequest,
    RequestValidator, RuntimeError, RuntimePolicy, RuntimeTimestamp, SessionRegistry,
    StartSessionRequest, WorkspaceName, WorkspaceRegistry,
};

fn actor_ref(value: &str, kind: ActorKind) -> ActorRef {
    ActorRef::new(ActorId::new(value).expect("actor id should be valid"), kind)
}

struct TestRuntime {
    workspace_registry: WorkspaceRegistry,
    session_registry: SessionRegistry,
    validator: RequestValidator,
    // Test double storage boundary: durable data is workspace-scoped.
    durable_workspace_state: HashMap<String, Vec<String>>,
}

impl TestRuntime {
    fn new() -> Self {
        Self {
            workspace_registry: WorkspaceRegistry::default(),
            session_registry: SessionRegistry::default(),
            validator: RequestValidator::new(RuntimePolicy::strict_default()),
            durable_workspace_state: HashMap::new(),
        }
    }

    fn create_workspace(&mut self, workspace_id: &WorkspaceId, actor: &ActorRef) {
        self.workspace_registry
            .create_workspace(CreateWorkspaceRequest {
                id: workspace_id.clone(),
                name: WorkspaceName::new(format!("Workspace {}", workspace_id.as_str()))
                    .expect("workspace name should be valid"),
                created_by: actor.clone(),
                created_at: RuntimeTimestamp::from_millis(1),
            })
            .expect("workspace creation should succeed");
    }

    fn start_session(
        &mut self,
        session_id: &SessionId,
        actor: &ActorRef,
        workspace_id: &WorkspaceId,
    ) {
        self.session_registry
            .start_session(StartSessionRequest {
                id: session_id.clone(),
                actor: Some(actor.clone()),
                workspace_id: workspace_id.clone(),
                started_at: RuntimeTimestamp::from_millis(2),
                metadata: HashMap::new(),
            })
            .expect("session start should succeed");
    }

    fn apply_stub_validated_mutation(
        &mut self,
        workspace_id: &WorkspaceId,
        session_id: &SessionId,
        payload: &str,
    ) -> Result<(), RuntimeError> {
        self.session_registry
            .validate_workspace_session_consistency(workspace_id, session_id)?;

        let mutation_request = CypherRequest::build_mutation_request(
            "MATCH (n) CREATE (m:SharedMutation)",
            CypherParameters::default(),
            workspace_id.clone(),
            session_id.clone(),
            CypherBudgetRef::new("budget--shared-runtime")?,
        )?;

        self.validator
            .validate_mutation_request(&mutation_request)?;

        self.durable_workspace_state
            .entry(workspace_id.as_str().to_owned())
            .or_default()
            .push(payload.to_owned());

        Ok(())
    }

    fn read_workspace_state(
        &self,
        workspace_id: &WorkspaceId,
        session_id: &SessionId,
    ) -> Result<Vec<String>, RuntimeError> {
        self.session_registry
            .validate_workspace_session_consistency(workspace_id, session_id)?;

        Ok(self
            .durable_workspace_state
            .get(workspace_id.as_str())
            .cloned()
            .unwrap_or_default())
    }
}

#[test]
fn orchestrator_mutation_is_visible_to_worker_in_same_workspace() {
    let mut runtime = TestRuntime::new();
    let workspace_id = WorkspaceId::new("workspace--shared-alpha").expect("workspace id valid");
    let orchestrator = actor_ref("actor--orchestrator", ActorKind::OrchestratorAgent);
    let worker = actor_ref("actor--worker", ActorKind::WorkerAgent);
    let orchestrator_session =
        SessionId::new("session--orchestrator").expect("session id should be valid");
    let worker_session = SessionId::new("session--worker").expect("session id should be valid");

    runtime.create_workspace(&workspace_id, &orchestrator);
    runtime.start_session(&orchestrator_session, &orchestrator, &workspace_id);
    runtime.start_session(&worker_session, &worker, &workspace_id);

    runtime
        .apply_stub_validated_mutation(&workspace_id, &orchestrator_session, "ioc:alpha")
        .expect("validated mutation should succeed");

    let observed_by_worker = runtime
        .read_workspace_state(&workspace_id, &worker_session)
        .expect("worker should read shared workspace state");

    assert_eq!(observed_by_worker, vec!["ioc:alpha".to_owned()]);
}

#[test]
fn workspace_state_is_isolated_between_different_workspaces() {
    let mut runtime = TestRuntime::new();
    let orchestrator = actor_ref("actor--orchestrator-iso", ActorKind::OrchestratorAgent);
    let worker = actor_ref("actor--worker-iso", ActorKind::WorkerAgent);

    let workspace_alpha = WorkspaceId::new("workspace--alpha").expect("workspace id valid");
    let workspace_beta = WorkspaceId::new("workspace--beta").expect("workspace id valid");
    let alpha_session = SessionId::new("session--alpha").expect("session id should be valid");
    let beta_session = SessionId::new("session--beta").expect("session id should be valid");

    runtime.create_workspace(&workspace_alpha, &orchestrator);
    runtime.create_workspace(&workspace_beta, &worker);
    runtime.start_session(&alpha_session, &orchestrator, &workspace_alpha);
    runtime.start_session(&beta_session, &worker, &workspace_beta);

    runtime
        .apply_stub_validated_mutation(&workspace_alpha, &alpha_session, "ioc:isolated")
        .expect("mutation in workspace alpha should succeed");

    let observed_in_beta = runtime
        .read_workspace_state(&workspace_beta, &beta_session)
        .expect("workspace beta read should succeed");

    assert!(observed_in_beta.is_empty());
}

#[test]
fn workspace_session_mismatch_is_explicit_in_runtime_contract() {
    let mut runtime = TestRuntime::new();
    let actor = actor_ref("actor--mismatch", ActorKind::OrchestratorAgent);
    let workspace_registered =
        WorkspaceId::new("workspace--registered").expect("workspace id should be valid");
    let workspace_other =
        WorkspaceId::new("workspace--other").expect("workspace id should be valid");
    let session_id = SessionId::new("session--mismatch").expect("session id should be valid");

    runtime.create_workspace(&workspace_registered, &actor);
    runtime.create_workspace(&workspace_other, &actor);
    runtime.start_session(&session_id, &actor, &workspace_registered);

    let error = runtime
        .apply_stub_validated_mutation(&workspace_other, &session_id, "ioc:bad-scope")
        .expect_err("workspace/session mismatch should be explicit");

    assert!(matches!(
        error,
        RuntimeError::WorkspaceSessionMismatch { .. }
    ));
}

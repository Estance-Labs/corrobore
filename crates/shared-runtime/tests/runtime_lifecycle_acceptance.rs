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

use graph_core::{ActorId, SessionId, TransactionId, WorkspaceId};
use shared_runtime::{
    ActorKind, ActorRef, CreateTransactionMetadataRequest, CreateWorkspaceRequest,
    CypherAuditReference, CypherBudgetRef, CypherFixHint, CypherMutationSummary, CypherParameters,
    CypherRequest, CypherResponse, CypherResponseData, CypherResponseStatus, CypherValidationError,
    RequestValidator, RuntimeBudget, RuntimeBudgetUsage, RuntimeError, RuntimePolicy,
    RuntimeTimestamp, SessionRegistry, StartSessionRequest, WorkspaceName, WorkspaceRegistry,
};

struct RuntimeHarness {
    opened: bool,
    workspace_registry: WorkspaceRegistry,
    session_registry: SessionRegistry,
    validator: RequestValidator,
    budget: RuntimeBudget,
    durable_workspace_state: HashMap<String, Vec<String>>,
}

impl RuntimeHarness {
    fn open() -> Self {
        Self {
            opened: true,
            workspace_registry: WorkspaceRegistry::default(),
            session_registry: SessionRegistry::default(),
            validator: RequestValidator::new(RuntimePolicy::strict_default()),
            budget: RuntimeBudget::strict_default(),
            durable_workspace_state: HashMap::new(),
        }
    }

    fn create_workspace(&mut self, workspace_id: &WorkspaceId, created_by: &ActorRef) {
        self.workspace_registry
            .create_workspace(CreateWorkspaceRequest {
                id: workspace_id.clone(),
                name: WorkspaceName::new(format!("Case {}", workspace_id.as_str()))
                    .expect("workspace name should be valid"),
                created_by: created_by.clone(),
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

    fn apply_validated_mutation(
        &mut self,
        workspace_id: &WorkspaceId,
        session_id: &SessionId,
        payload: &str,
        request_id: &str,
    ) -> Result<CypherResponse, RuntimeError> {
        self.session_registry
            .validate_workspace_session_consistency(workspace_id, session_id)?;

        let request = CypherRequest::build_mutation_request(
            "MATCH (n) CREATE (m:SharedMutation)",
            CypherParameters::default(),
            workspace_id.clone(),
            session_id.clone(),
            CypherBudgetRef::new("budget--epic-0004")?,
        )?;

        self.validator.validate_mutation_request(&request)?;
        self.validator
            .validate_request_budget_limits(&request, &self.budget)?;

        let transaction_id =
            TransactionId::new("transaction--epic-0004").expect("transaction id should be valid");
        let transaction_metadata = self
            .session_registry
            .create_transaction_metadata_from_session(CreateTransactionMetadataRequest {
                transaction_id: transaction_id.clone(),
                session_id: session_id.clone(),
                started_at: RuntimeTimestamp::from_millis(3),
                policy_name: Some("runtime-policy--default".to_owned()),
            })?;

        self.session_registry
            .validate_transaction_metadata_for_mutation(Some(&transaction_metadata))?;

        let budget_usage = self.validator.record_budget_usage(
            request.budget_ref(),
            &self.budget,
            RuntimeBudgetUsage {
                query_length: request.query_text.chars().count(),
                parameter_count: request.parameters.values().len(),
                loaded_record_count: 1,
                returned_record_count: 1,
                mutation_count: 1,
                payload_bytes: payload.len(),
                execution_time_ms: 5,
            },
        )?;

        self.durable_workspace_state
            .entry(workspace_id.as_str().to_owned())
            .or_default()
            .push(payload.to_owned());

        Ok(CypherResponse {
            status: CypherResponseStatus::Success,
            data: CypherResponseData::MutationSummary(CypherMutationSummary {
                matched_rows: 0,
                created_nodes: 1,
                updated_nodes: 0,
                deleted_nodes: 0,
                created_relationships: 0,
                updated_relationships: 0,
                deleted_relationships: 0,
                properties_set: 1,
                native_fields_changed: 0,
                property_fields_changed: 1,
            }),
            warnings: vec![],
            validation_errors: vec![],
            budget_usage: Some(budget_usage),
            audit_references: vec![CypherAuditReference {
                transaction_id: Some(transaction_id),
                request_id: Some(request_id.to_owned()),
            }],
            fix_hints: vec![],
        })
    }

    fn reject_unsafe_write_attempt(
        &self,
        workspace_id: &WorkspaceId,
        session_id: &SessionId,
        request_id: &str,
    ) -> CypherResponse {
        let request = CypherRequest::build_read_only_request(
            "MATCH (n) CREATE (m:Unsafe)",
            CypherParameters::default(),
            workspace_id.clone(),
            session_id.clone(),
            CypherBudgetRef::new("budget--epic-0004").expect("budget ref should be valid"),
        )
        .expect("request should be valid");

        let error = self
            .validator
            .validate_read_only_request(&request)
            .expect_err("unsafe write in read-only mode should be rejected");

        match error {
            RuntimeError::UnsafeMutationAttempt { reason, fix_hint } => CypherResponse {
                status: CypherResponseStatus::Rejected,
                data: CypherResponseData::Empty,
                warnings: vec![reason],
                validation_errors: vec![CypherValidationError {
                    code: "UNSAFE_MUTATION".to_owned(),
                    message: "mutation clauses are not allowed in read-only mode".to_owned(),
                    field: Some("query_text".to_owned()),
                }],
                budget_usage: None,
                audit_references: vec![CypherAuditReference {
                    transaction_id: None,
                    request_id: Some(request_id.to_owned()),
                }],
                fix_hints: vec![CypherFixHint {
                    code: "SWITCH_MODE".to_owned(),
                    message: fix_hint,
                }],
            },
            _ => panic!("expected UnsafeMutationAttempt"),
        }
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

trait RequestBudgetRef {
    fn budget_ref(&self) -> &CypherBudgetRef;
}

impl RequestBudgetRef for CypherRequest {
    fn budget_ref(&self) -> &CypherBudgetRef {
        &self.budget_ref
    }
}

fn actor_ref(value: &str, kind: ActorKind) -> ActorRef {
    ActorRef::new(ActorId::new(value).expect("actor id should be valid"), kind)
}

#[test]
fn epic_0004_acceptance_suite_covers_runtime_contracts() {
    let mut runtime = RuntimeHarness::open();
    assert!(
        runtime.opened,
        "runtime lifecycle should expose opened state"
    );

    let orchestrator = actor_ref(
        "actor--orchestrator-acceptance",
        ActorKind::OrchestratorAgent,
    );
    let worker = actor_ref("actor--worker-acceptance", ActorKind::WorkerAgent);

    let workspace_shared =
        WorkspaceId::new("workspace--acceptance-shared").expect("workspace id should be valid");
    let workspace_isolated =
        WorkspaceId::new("workspace--acceptance-isolated").expect("workspace id should be valid");

    runtime.create_workspace(&workspace_shared, &orchestrator);
    runtime.create_workspace(&workspace_isolated, &worker);

    let resolved_workspace = runtime
        .workspace_registry
        .workspace(&workspace_shared)
        .expect("workspace lookup should succeed");
    assert_eq!(resolved_workspace.id, workspace_shared);

    let orchestrator_session =
        SessionId::new("session--acceptance-orchestrator").expect("session id should be valid");
    let worker_session =
        SessionId::new("session--acceptance-worker").expect("session id should be valid");
    let isolated_session =
        SessionId::new("session--acceptance-isolated").expect("session id should be valid");

    runtime.start_session(&orchestrator_session, &orchestrator, &workspace_shared);
    runtime.start_session(&worker_session, &worker, &workspace_shared);
    runtime.start_session(&isolated_session, &worker, &workspace_isolated);

    let session_metadata = runtime
        .session_registry
        .read_session_metadata(&orchestrator_session)
        .expect("session metadata should be retrievable");
    assert_eq!(session_metadata.actor, orchestrator);
    assert_eq!(session_metadata.workspace_id, workspace_shared);

    let success_response = runtime
        .apply_validated_mutation(
            &workspace_shared,
            &orchestrator_session,
            "ioc:acceptance-shared",
            "request--acceptance-001",
        )
        .expect("validated mutation should succeed");
    assert_eq!(success_response.status, CypherResponseStatus::Success);
    assert_eq!(success_response.audit_references.len(), 1);
    assert!(
        success_response.audit_references[0]
            .transaction_id
            .is_some()
    );

    let worker_view = runtime
        .read_workspace_state(&workspace_shared, &worker_session)
        .expect("worker should observe shared workspace state");
    assert_eq!(worker_view, vec!["ioc:acceptance-shared".to_owned()]);

    let isolated_view = runtime
        .read_workspace_state(&workspace_isolated, &isolated_session)
        .expect("isolated workspace read should succeed");
    assert!(isolated_view.is_empty());

    let mismatch_error = runtime
        .read_workspace_state(&workspace_isolated, &orchestrator_session)
        .expect_err("workspace/session mismatch should be explicit");
    assert!(matches!(
        mismatch_error,
        RuntimeError::WorkspaceSessionMismatch { .. }
    ));

    let rejected_response = runtime.reject_unsafe_write_attempt(
        &workspace_shared,
        &worker_session,
        "request--acceptance-002",
    );
    assert_eq!(rejected_response.status, CypherResponseStatus::Rejected);
    assert_eq!(rejected_response.audit_references.len(), 1);
    assert!(
        rejected_response.audit_references[0]
            .transaction_id
            .is_none()
    );
    assert_eq!(rejected_response.fix_hints.len(), 1);

    let policy_error = runtime
        .validator
        .validate_read_only_request(
            &CypherRequest::build_read_only_request(
                "MATCH (n) CREATE (m:Unsafe)",
                CypherParameters::default(),
                workspace_shared,
                worker_session,
                CypherBudgetRef::new("budget--acceptance").expect("budget ref should be valid"),
            )
            .expect("request should be valid"),
        )
        .expect_err("unsafe read-only mutation should fail");

    assert!(matches!(
    policy_error,
    RuntimeError::UnsafeMutationAttempt { ref fix_hint, .. } if !fix_hint.is_empty()
    ));

    let budget_error = runtime
        .validator
        .record_budget_usage(
            &CypherBudgetRef::new("budget--acceptance").expect("budget ref should be valid"),
            &RuntimeBudget {
                max_mutation_count: 0,
                ..RuntimeBudget::strict_default()
            },
            RuntimeBudgetUsage {
                query_length: 12,
                parameter_count: 0,
                loaded_record_count: 1,
                returned_record_count: 1,
                mutation_count: 1,
                payload_bytes: 16,
                execution_time_ms: 3,
            },
        )
        .expect_err("mutation budget overflow should be explicit");

    assert!(matches!(
    budget_error,
    RuntimeError::MutationBudgetExceeded { ref details } if !details.fix_hint.is_empty()
    ));

    let transaction_error = runtime
        .session_registry
        .create_transaction_metadata_from_session(CreateTransactionMetadataRequest {
            transaction_id: TransactionId::new("transaction--acceptance-missing-policy")
                .expect("transaction id should be valid"),
            session_id: orchestrator_session,
            started_at: RuntimeTimestamp::from_millis(11),
            policy_name: None,
        })
        .expect_err("transaction metadata should require policy name");

    assert!(matches!(
        transaction_error,
        RuntimeError::MissingTransactionMetadata("policy_name")
    ));
}

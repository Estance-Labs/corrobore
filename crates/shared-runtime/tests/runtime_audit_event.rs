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
    AcceptedMutationAuditInput, ActorKind, ActorRef, CypherBudgetRef, CypherParameters,
    CypherRequest, CypherRequestMode, CypherResponse, CypherResponseData, CypherResponseStatus,
    RejectedWriteAuditInput, RuntimeAuditEvent, RuntimeAuditEventContext, RuntimeAuditEventId,
    RuntimeAuditEventKind, RuntimeAuditOutcome, RuntimeAuditReasonMetadata, RuntimeError,
    RuntimeTimestamp, UnsafeAttemptAuditInput,
};

fn actor() -> ActorRef {
    ActorRef::new(
        ActorId::new("actor--audit").expect("actor id should be valid"),
        ActorKind::OrchestratorAgent,
    )
}

fn workspace_id() -> WorkspaceId {
    WorkspaceId::new("workspace--audit").expect("workspace id should be valid")
}

fn session_id() -> SessionId {
    SessionId::new("session--audit").expect("session id should be valid")
}

fn request() -> CypherRequest {
    CypherRequest::new(
        "MATCH (n) RETURN n",
        CypherParameters::new(HashMap::new()),
        CypherRequestMode::ReadOnly,
        workspace_id(),
        session_id(),
        CypherBudgetRef::new("budget--audit").expect("budget ref should be valid"),
    )
    .expect("request should be valid")
}

fn context(event_id: &str, timestamp: u64, request_id: Option<&str>) -> RuntimeAuditEventContext {
    RuntimeAuditEventContext {
        event_id: RuntimeAuditEventId::new(event_id).expect("event id should be valid"),
        actor: actor(),
        session_id: session_id(),
        workspace_id: workspace_id(),
        timestamp: RuntimeTimestamp::from_millis(timestamp),
        request: request(),
        request_id: request_id.map(str::to_owned),
    }
}

#[test]
fn build_accepted_mutation_audit_metadata_captures_core_context() {
    let transaction_id =
        TransactionId::new("transaction--audit-ok").expect("transaction id should be valid");

    let event =
        RuntimeAuditEvent::build_accepted_mutation_audit_metadata(AcceptedMutationAuditInput {
            context: context(
                "audit-event--accepted",
                1_729_000_000_000,
                Some("request--audit-001"),
            ),
            transaction_id: Some(transaction_id.clone()),
            outcome: RuntimeAuditOutcome {
                status: CypherResponseStatus::Success,
                message: "mutation accepted".to_owned(),
            },
            affected_ids: vec!["node--1".to_owned(), "relationship--1".to_owned()],
            before_version_ids: vec!["node-version--9".to_owned()],
            after_version_ids: vec!["node-version--10".to_owned()],
        })
        .expect("accepted mutation audit metadata should be valid");

    assert_eq!(event.kind, RuntimeAuditEventKind::AcceptedMutation);
    assert_eq!(event.transaction_id, Some(transaction_id));
    assert_eq!(
        event.affected_ids,
        vec!["node--1".to_owned(), "relationship--1".to_owned()]
    );
    assert_eq!(event.before_version_ids, vec!["node-version--9".to_owned()]);
    assert_eq!(event.after_version_ids, vec!["node-version--10".to_owned()]);
    assert!(!event.query_text_hash.is_empty());
}

#[test]
fn build_rejected_write_audit_metadata_preserves_reason_details() {
    let reason = RuntimeAuditReasonMetadata {
        code: "POLICY_REJECTED".to_owned(),
        message: "write mode disabled".to_owned(),
        fix_hint: Some("Use read-only mode or update runtime policy".to_owned()),
    };

    let event = RuntimeAuditEvent::build_rejected_write_audit_metadata(RejectedWriteAuditInput {
        context: context(
            "audit-event--rejected",
            1_729_000_000_010,
            Some("request--audit-002"),
        ),
        outcome: RuntimeAuditOutcome {
            status: CypherResponseStatus::Rejected,
            message: "write rejected".to_owned(),
        },
        reason: reason.clone(),
    })
    .expect("rejected write audit metadata should be valid");

    assert_eq!(event.kind, RuntimeAuditEventKind::RejectedMutation);
    assert_eq!(event.reason, Some(reason));
}

#[test]
fn build_unsafe_attempt_audit_metadata_sets_expected_kind() {
    let event = RuntimeAuditEvent::build_unsafe_attempt_audit_metadata(UnsafeAttemptAuditInput {
        context: context(
            "audit-event--unsafe",
            1_729_000_000_020,
            Some("request--audit-003"),
        ),
        reason: RuntimeAuditReasonMetadata {
            code: "UNSAFE_MUTATION".to_owned(),
            message: "mutation clause in read-only mode".to_owned(),
            fix_hint: Some("Switch to mutation mode".to_owned()),
        },
    })
    .expect("unsafe attempt audit metadata should be valid");

    assert_eq!(event.kind, RuntimeAuditEventKind::RejectedUnsafeRequest);
}

#[test]
fn accepted_mutation_builder_rejects_missing_transaction_context() {
    let error =
        RuntimeAuditEvent::build_accepted_mutation_audit_metadata(AcceptedMutationAuditInput {
            context: context(
                "audit-event--missing-tx",
                1_729_000_000_030,
                Some("request--audit-004"),
            ),
            transaction_id: None,
            outcome: RuntimeAuditOutcome {
                status: CypherResponseStatus::Success,
                message: "mutation accepted".to_owned(),
            },
            affected_ids: vec![],
            before_version_ids: vec![],
            after_version_ids: vec![],
        })
        .expect_err("accepted mutation should require transaction context");

    assert!(matches!(
    error,
    RuntimeError::AuditMetadataCreationFailed(field) if field == "transaction_id"
    ));
}

#[test]
fn attach_audit_references_to_cypher_response_includes_audit_context() {
    let event = RuntimeAuditEvent::build_rejected_write_audit_metadata(RejectedWriteAuditInput {
        context: context(
            "audit-event--attach",
            1_729_000_000_040,
            Some("request--audit-005"),
        ),
        outcome: RuntimeAuditOutcome {
            status: CypherResponseStatus::Rejected,
            message: "rejected".to_owned(),
        },
        reason: RuntimeAuditReasonMetadata {
            code: "POLICY".to_owned(),
            message: "rejected".to_owned(),
            fix_hint: None,
        },
    })
    .expect("audit event should be created");

    let mut response = CypherResponse {
        status: CypherResponseStatus::Rejected,
        data: CypherResponseData::Empty,
        warnings: vec![],
        validation_errors: vec![],
        budget_usage: None,
        audit_references: vec![],
        fix_hints: vec![],
    };

    RuntimeAuditEvent::attach_audit_references_to_cypher_response(&mut response, &[event])
        .expect("audit reference attachment should succeed");

    assert_eq!(response.audit_references.len(), 1);
    assert_eq!(
        response.audit_references[0].request_id,
        Some("request--audit-005".to_owned())
    );
}

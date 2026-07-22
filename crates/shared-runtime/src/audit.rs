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
use crate::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Runtime audit event id.
pub struct RuntimeAuditEventId {
    value: String,
}

impl RuntimeAuditEventId {
    /// Creates a new instance.
    pub fn new(value: impl Into<String>) -> Result<Self, RuntimeError> {
        let value = value.into();

        if value.trim().is_empty() {
            return Err(RuntimeError::AuditMetadataCreationFailed("event_id"));
        }

        Ok(Self { value })
    }

    /// Returns the value as str.
    pub fn as_str(&self) -> &str {
        self.value.as_str()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Runtime audit event kind.
pub enum RuntimeAuditEventKind {
    /// Accepted mutation.
    AcceptedMutation,
    /// Rejected mutation.
    RejectedMutation,
    /// Rejected unsafe request.
    RejectedUnsafeRequest,
    /// Validation failure.
    ValidationFailure,
    /// Over budget request.
    OverBudgetRequest,
    /// Session opened.
    SessionOpened,
    /// Workspace created.
    WorkspaceCreated,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Runtime request summary.
pub struct RuntimeRequestSummary {
    /// Request mode.
    pub mode: CypherRequestMode,
    /// Query preview.
    pub query_preview: String,
    /// Parameter count.
    pub parameter_count: usize,
    /// Budget ref.
    pub budget_ref: CypherBudgetRef,
}

impl RuntimeRequestSummary {
    /// Creates an instance from request.
    pub fn from_request(request: &CypherRequest) -> Self {
        let query_preview: String = request.query_text.chars().take(120).collect();

        Self {
            mode: request.mode.clone(),
            query_preview,
            // Parameter count.
            parameter_count: request.parameters.values().len(),
            // Budget ref.
            budget_ref: request.budget_ref.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Runtime audit outcome.
pub struct RuntimeAuditOutcome {
    /// Status.
    pub status: CypherResponseStatus,
    /// Message.
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Runtime audit reason metadata.
pub struct RuntimeAuditReasonMetadata {
    /// Code.
    pub code: String,
    /// Message.
    pub message: String,
    /// Fix hint.
    pub fix_hint: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Runtime audit event.
pub struct RuntimeAuditEvent {
    /// Event id.
    pub event_id: RuntimeAuditEventId,
    /// Kind.
    pub kind: RuntimeAuditEventKind,
    /// Actor.
    pub actor: ActorRef,
    /// Session id.
    pub session_id: SessionId,
    /// Workspace id.
    pub workspace_id: WorkspaceId,
    /// Transaction id.
    pub transaction_id: Option<TransactionId>,
    /// Timestamp.
    pub timestamp: RuntimeTimestamp,
    /// Request summary.
    pub request_summary: RuntimeRequestSummary,
    /// Query text hash.
    pub query_text_hash: String,
    /// Affected ids.
    pub affected_ids: Vec<String>,
    /// Before version ids.
    pub before_version_ids: Vec<String>,
    /// After version ids.
    pub after_version_ids: Vec<String>,
    /// Outcome.
    pub outcome: RuntimeAuditOutcome,
    /// Reason.
    pub reason: Option<RuntimeAuditReasonMetadata>,
    /// Request id.
    pub request_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Runtime audit event context.
pub struct RuntimeAuditEventContext {
    /// Event id.
    pub event_id: RuntimeAuditEventId,
    /// Actor.
    pub actor: ActorRef,
    /// Session id.
    pub session_id: SessionId,
    /// Workspace id.
    pub workspace_id: WorkspaceId,
    /// Timestamp.
    pub timestamp: RuntimeTimestamp,
    /// Request.
    pub request: CypherRequest,
    /// Request id.
    pub request_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Accepted mutation audit input.
pub struct AcceptedMutationAuditInput {
    /// Context.
    pub context: RuntimeAuditEventContext,
    /// Transaction id.
    pub transaction_id: Option<TransactionId>,
    /// Outcome.
    pub outcome: RuntimeAuditOutcome,
    /// Affected ids.
    pub affected_ids: Vec<String>,
    /// Before version ids.
    pub before_version_ids: Vec<String>,
    /// After version ids.
    pub after_version_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Rejected write audit input.
pub struct RejectedWriteAuditInput {
    /// Context.
    pub context: RuntimeAuditEventContext,
    /// Outcome.
    pub outcome: RuntimeAuditOutcome,
    /// Reason.
    pub reason: RuntimeAuditReasonMetadata,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Unsafe attempt audit input.
pub struct UnsafeAttemptAuditInput {
    /// Context.
    pub context: RuntimeAuditEventContext,
    /// Reason.
    pub reason: RuntimeAuditReasonMetadata,
}

impl RuntimeAuditEvent {
    // Audit event metadata models the runtime attribution contract only.
    // Durable append-only audit log persistence is intentionally out of scope.
    /// Creates the accepted mutation audit metadata.
    pub fn build_accepted_mutation_audit_metadata(
        input: AcceptedMutationAuditInput,
    ) -> Result<Self, RuntimeError> {
        let transaction_id = input
            .transaction_id
            .ok_or(RuntimeError::AuditMetadataCreationFailed("transaction_id"))?;

        Ok(Self {
            event_id: input.context.event_id,
            kind: RuntimeAuditEventKind::AcceptedMutation,
            actor: input.context.actor,
            session_id: input.context.session_id,
            workspace_id: input.context.workspace_id,
            transaction_id: Some(transaction_id),
            timestamp: input.context.timestamp,
            request_summary: RuntimeRequestSummary::from_request(&input.context.request),
            query_text_hash: stable_query_text_hash(input.context.request.query_text.as_str()),
            affected_ids: input.affected_ids,
            before_version_ids: input.before_version_ids,
            after_version_ids: input.after_version_ids,
            outcome: input.outcome,
            reason: None,
            request_id: input.context.request_id,
        })
    }

    /// Creates the rejected write audit metadata.
    pub fn build_rejected_write_audit_metadata(
        input: RejectedWriteAuditInput,
    ) -> Result<Self, RuntimeError> {
        validate_audit_reason(&input.reason)?;

        Ok(Self {
            event_id: input.context.event_id,
            kind: RuntimeAuditEventKind::RejectedMutation,
            actor: input.context.actor,
            session_id: input.context.session_id,
            workspace_id: input.context.workspace_id,
            transaction_id: None,
            timestamp: input.context.timestamp,
            request_summary: RuntimeRequestSummary::from_request(&input.context.request),
            query_text_hash: stable_query_text_hash(input.context.request.query_text.as_str()),
            affected_ids: Vec::new(),
            before_version_ids: Vec::new(),
            after_version_ids: Vec::new(),
            outcome: input.outcome,
            reason: Some(input.reason),
            request_id: input.context.request_id,
        })
    }

    /// Creates the unsafe attempt audit metadata.
    pub fn build_unsafe_attempt_audit_metadata(
        input: UnsafeAttemptAuditInput,
    ) -> Result<Self, RuntimeError> {
        validate_audit_reason(&input.reason)?;

        Ok(Self {
            event_id: input.context.event_id,
            kind: RuntimeAuditEventKind::RejectedUnsafeRequest,
            actor: input.context.actor,
            session_id: input.context.session_id,
            workspace_id: input.context.workspace_id,
            transaction_id: None,
            timestamp: input.context.timestamp,
            request_summary: RuntimeRequestSummary::from_request(&input.context.request),
            query_text_hash: stable_query_text_hash(input.context.request.query_text.as_str()),
            affected_ids: Vec::new(),
            before_version_ids: Vec::new(),
            after_version_ids: Vec::new(),
            outcome: RuntimeAuditOutcome {
                status: CypherResponseStatus::Rejected,
                message: "unsafe request rejected".to_owned(),
            },
            reason: Some(input.reason),
            request_id: input.context.request_id,
        })
    }

    /// Attach audit references to cypher response.
    pub fn attach_audit_references_to_cypher_response(
        response: &mut CypherResponse,
        events: &[RuntimeAuditEvent],
    ) -> Result<(), RuntimeError> {
        if events.is_empty() {
            return Err(RuntimeError::AuditMetadataCreationFailed("audit_events"));
        }

        response
            .audit_references
            .extend(events.iter().map(|event| CypherAuditReference {
                transaction_id: event.transaction_id.clone(),
                request_id: event.request_id.clone(),
            }));

        Ok(())
    }
}

pub(crate) fn validate_audit_reason(
    reason: &RuntimeAuditReasonMetadata,
) -> Result<(), RuntimeError> {
    if reason.code.trim().is_empty() {
        return Err(RuntimeError::AuditMetadataCreationFailed("reason.code"));
    }

    if reason.message.trim().is_empty() {
        return Err(RuntimeError::AuditMetadataCreationFailed("reason.message"));
    }

    Ok(())
}

pub(crate) fn stable_query_text_hash(query_text: &str) -> String {
    // Use FNV-1a 64-bit for a stable, dependency-free query fingerprint.
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in query_text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{:016x}", hash)
}

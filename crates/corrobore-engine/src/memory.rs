// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//! Versioned, domain-neutral high-level memory operation contract.
//!
//! The operation payload deliberately excludes workspace, actor, agent,
//! session, permissions, and correlation identity. Hosts supply those values
//! through [`MemoryServiceContext`] after authentication and policy resolution.

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    time::Instant,
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, FixedOffset, Utc};
use graph_core::{
    Confidence, Graph, Node, NodeId, NodeInput, NodePatch, PropertyValue, RecordStatus,
    Relationship, RelationshipId, RelationshipInput, RelationshipPatch,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::CorroboreEngine;

/// Stable schema version for the high-level memory boundary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryContractVersion {
    /// Initial compatibility contract.
    #[default]
    V1,
}

/// Independently enforceable permissions resolved by a trusted host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryPermissions {
    /// Recall and ordinary record reads.
    pub read: bool,
    /// Remember, relate, and update mutations.
    pub write: bool,
    /// Provenance and decision tracing.
    pub trace: bool,
    /// Application forgetting.
    pub forget: bool,
    /// Consolidation proposal and approved apply.
    pub consolidate: bool,
}

impl MemoryPermissions {
    /// Grants all high-level operation capabilities.
    pub const fn all() -> Self {
        Self {
            read: true,
            write: true,
            trace: true,
            forget: true,
            consolidate: true,
        }
    }

    /// Grants ordinary reads only.
    pub const fn read_only() -> Self {
        Self {
            read: true,
            write: false,
            trace: false,
            forget: false,
            consolidate: false,
        }
    }
}

/// Trusted runtime context kept outside untrusted operation payloads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryServiceContext {
    /// Trusted workspace boundary.
    pub workspace_id: String,
    /// Authenticated actor.
    pub actor_id: String,
    /// Optional authenticated agent acting for the actor.
    pub agent_id: Option<String>,
    /// Trusted session identifier.
    pub session_id: String,
    /// Policy-resolved operation permissions.
    pub permissions: MemoryPermissions,
    /// Runtime request identifier.
    pub request_id: String,
    /// Non-sensitive audit correlation identifier.
    pub correlation_id: String,
}

impl MemoryServiceContext {
    /// Builds a validated trusted context.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        workspace_id: impl Into<String>,
        actor_id: impl Into<String>,
        agent_id: Option<String>,
        session_id: impl Into<String>,
        permissions: MemoryPermissions,
        request_id: impl Into<String>,
        correlation_id: impl Into<String>,
    ) -> Result<Self, MemoryError> {
        let context = Self {
            workspace_id: workspace_id.into(),
            actor_id: actor_id.into(),
            agent_id,
            session_id: session_id.into(),
            permissions,
            request_id: request_id.into(),
            correlation_id: correlation_id.into(),
        };
        for (field, value) in [
            ("workspace_id", context.workspace_id.as_str()),
            ("actor_id", context.actor_id.as_str()),
            ("session_id", context.session_id.as_str()),
            ("request_id", context.request_id.as_str()),
            ("correlation_id", context.correlation_id.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(MemoryError::new(
                    MemoryErrorCode::InvalidRequest,
                    format!("trusted context field {field} must not be empty"),
                    false,
                ));
            }
        }
        if context
            .agent_id
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
        {
            return Err(MemoryError::new(
                MemoryErrorCode::InvalidRequest,
                "trusted context field agent_id must not be empty",
                false,
            ));
        }
        Ok(context)
    }
}

/// Versioned operation envelope accepted by embedded and transport adapters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryRequest {
    /// Contract version selected by the caller.
    pub contract_version: MemoryContractVersion,
    /// Stable caller key required for mutations.
    pub idempotency_key: Option<String>,
    /// One domain-neutral operation.
    #[serde(flatten)]
    pub operation: MemoryOperation,
}

impl MemoryRequest {
    /// Creates a version-one request.
    pub fn new(operation: MemoryOperation) -> Self {
        Self {
            contract_version: MemoryContractVersion::V1,
            idempotency_key: None,
            operation,
        }
    }

    /// Attaches a mutation idempotency key.
    pub fn with_idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }
}

/// Seven high-level memory operations.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "operation", content = "input", rename_all = "snake_case")]
pub enum MemoryOperation {
    /// Create or idempotently upsert a memory.
    Remember(RememberRequest),
    /// Create or update an evidence-bearing relationship.
    Relate(RelateRequest),
    /// Retrieve a bounded explained working set.
    Recall(RecallRequest),
    /// Apply an auditable patch.
    Update(MemoryUpdateRequest),
    /// Remove a memory from ordinary retrieval.
    Forget(ForgetRequest),
    /// Propose or apply policy-gated consolidation.
    Consolidate(ConsolidateRequest),
    /// Explain a record, relationship, recall, or mutation.
    Trace(TraceRequest),
}

/// Domain-neutral memory content.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "format", content = "value", rename_all = "snake_case")]
pub enum MemoryContent {
    /// Free-form text.
    Text(String),
    /// Structured application properties.
    Properties(serde_json::Value),
    /// Text plus structured application properties.
    TextAndProperties {
        /// Free-form text.
        text: String,
        /// Structured application properties.
        properties: serde_json::Value,
    },
}

/// Source reference carried by a memory or relationship version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProvenanceReference {
    /// Stable source identity.
    pub source_id: String,
    /// Optional source-local locator.
    pub locator: Option<String>,
    /// Optional RFC3339 observation time.
    pub observed_at: Option<String>,
}

/// High-level lifecycle independent of domain workflows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryLifecycle {
    /// Available for ordinary retrieval.
    #[default]
    Active,
    /// Expired under application policy.
    Expired,
    /// Retained as an original superseded by consolidation.
    Superseded,
    /// Tombstoned from ordinary retrieval.
    Tombstoned,
}

/// Input for `remember`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RememberRequest {
    /// Optional application-owned stable identity.
    pub identity_key: Option<String>,
    /// Application-defined kind.
    pub kind: String,
    /// Application schema version.
    pub schema_version: String,
    /// Text or structured payload.
    pub content: MemoryContent,
    /// Source provenance.
    pub provenance: Vec<ProvenanceReference>,
    /// Confidence in the inclusive range 0..=1.
    pub confidence: Option<f64>,
    /// Inclusive validity start.
    pub valid_from: Option<String>,
    /// Inclusive validity end.
    pub valid_until: Option<String>,
    /// Application expiry time.
    pub expires_at: Option<String>,
    /// Application tags.
    pub tags: Vec<String>,
}

/// Input for `relate`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelateRequest {
    /// Optional application-owned relationship identity.
    pub identity_key: Option<String>,
    /// Source memory ID.
    pub source_id: String,
    /// Target memory ID.
    pub target_id: String,
    /// Application-defined relationship kind.
    pub kind: String,
    /// Structured application properties.
    pub properties: serde_json::Value,
    /// Relationship-owned provenance.
    pub provenance: Vec<ProvenanceReference>,
    /// Relationship-owned confidence.
    pub confidence: Option<f64>,
    /// Inclusive validity start.
    pub valid_from: Option<String>,
    /// Inclusive validity end.
    pub valid_until: Option<String>,
    /// Application expiry time.
    pub expires_at: Option<String>,
    /// Relationship lifecycle.
    pub lifecycle: MemoryLifecycle,
}

/// Explicit recall safety limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryLimits {
    /// Maximum returned memories.
    pub max_items: usize,
    /// Maximum relationship traversal depth.
    pub max_depth: u32,
    /// Maximum serialized response payload.
    pub max_payload_bytes: usize,
    /// Maximum deterministic expansion cost.
    pub max_cost: u64,
    /// Maximum execution time in milliseconds.
    pub timeout_ms: u64,
    /// Degree at which expansion is treated as a supernode.
    pub supernode_threshold: usize,
}

impl MemoryLimits {
    /// Conservative default suitable for local agents.
    pub const fn strict_default() -> Self {
        Self {
            max_items: 50,
            max_depth: 2,
            max_payload_bytes: 256 * 1024,
            max_cost: 1_000,
            timeout_ms: 2_000,
            supernode_threshold: 1_000,
        }
    }
}

/// Input for `recall`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecallRequest {
    /// Required objective used for seed selection and explanations.
    pub objective: String,
    /// Optional explicit seed memories.
    pub seed_ids: Vec<String>,
    /// Required traversal and response limits.
    pub limits: MemoryLimits,
    /// Opaque workspace-bound continuation token.
    pub page_token: Option<String>,
}

/// Target accepted by update and trace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum MemoryTarget {
    /// Memory record.
    Memory(String),
    /// Relationship record.
    Relationship(String),
    /// Recall execution.
    Recall(String),
    /// Mutation receipt correlation.
    Mutation(String),
}

/// Auditable update patch; omitted fields preserve existing values.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdatePatch {
    /// Replacement content.
    pub content: Option<MemoryContent>,
    /// Replacement confidence.
    pub confidence: Option<f64>,
    /// Provenance references appended to existing evidence.
    pub add_provenance: Vec<ProvenanceReference>,
    /// Lifecycle transition.
    pub lifecycle: Option<MemoryLifecycle>,
    /// Replacement expiry.
    pub expires_at: Option<String>,
    /// Tags appended to existing tags.
    pub add_tags: Vec<String>,
}

/// Input for `update`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryUpdateRequest {
    /// Memory or relationship target.
    pub target: MemoryTarget,
    /// Optional optimistic version precondition.
    pub expected_version: Option<u64>,
    /// Patch applied as a new version.
    pub patch: UpdatePatch,
}

/// Application forgetting mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForgetMode {
    /// Mark expired while preserving all versions.
    Expire,
    /// Create a logical tombstone.
    Tombstone,
    /// Application deletion with only policy-allowed audit retained.
    ApplicationDelete,
}

/// Input for `forget`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForgetRequest {
    /// Memory to forget.
    pub memory_id: String,
    /// Application forgetting semantics.
    pub mode: ForgetMode,
    /// Required for explicit expiry and ignored otherwise.
    pub expires_at: Option<String>,
    /// Non-empty audit reason.
    pub reason: String,
}

/// Consolidation execution mode.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ConsolidateMode {
    /// Return a non-destructive proposal.
    Propose,
    /// Apply an already approved proposal under named policy.
    ApplyApproved {
        /// Proposal returned earlier.
        proposal_id: String,
        /// Trusted policy or approval reference.
        approval_policy: String,
    },
}

/// Input for `consolidate`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsolidateRequest {
    /// Proposal or approved application.
    pub mode: ConsolidateMode,
    /// Bounded original memories.
    pub memory_ids: Vec<String>,
    /// Optional canonical memory.
    pub canonical_id: Option<String>,
    /// Decision rationale.
    pub reason: String,
    /// Whether disagreements must remain explicit.
    pub preserve_disagreements: bool,
}

/// Input for `trace`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TraceRequest {
    /// Record, recall, or mutation to explain.
    pub target: MemoryTarget,
}

/// One public memory record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemoryRecord {
    /// Engine-owned stable identity.
    pub id: String,
    /// Optional application identity.
    pub identity_key: Option<String>,
    /// Application-defined kind.
    pub kind: String,
    /// Application schema version.
    pub schema_version: String,
    /// Stored content.
    pub content: MemoryContent,
    /// Accumulated provenance.
    pub provenance: Vec<ProvenanceReference>,
    /// Confidence.
    pub confidence: Option<f64>,
    /// Validity start.
    pub valid_from: Option<String>,
    /// Validity end.
    pub valid_until: Option<String>,
    /// Recording time.
    pub recorded_at: String,
    /// Application expiry.
    pub expires_at: Option<String>,
    /// Lifecycle.
    pub lifecycle: MemoryLifecycle,
    /// Monotonic record version.
    pub version: u64,
    /// Application tags.
    pub tags: Vec<String>,
}

/// One public relationship record with first-class metadata.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemoryRelationship {
    /// Engine-owned stable identity.
    pub id: String,
    /// Optional application identity.
    pub identity_key: Option<String>,
    /// Source memory.
    pub source_id: String,
    /// Target memory.
    pub target_id: String,
    /// Application relationship kind.
    pub kind: String,
    /// Structured properties.
    pub properties: serde_json::Value,
    /// Relationship-owned provenance.
    pub provenance: Vec<ProvenanceReference>,
    /// Confidence.
    pub confidence: Option<f64>,
    /// Validity start.
    pub valid_from: Option<String>,
    /// Validity end.
    pub valid_until: Option<String>,
    /// Recording time.
    pub recorded_at: String,
    /// Application expiry.
    pub expires_at: Option<String>,
    /// Lifecycle.
    pub lifecycle: MemoryLifecycle,
    /// Monotonic record version.
    pub version: u64,
}

/// Stable receipt returned only after the configured durability gate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MutationReceipt {
    /// Committed record identity.
    pub committed_id: String,
    /// Committed record version.
    pub committed_version: u64,
    /// Non-sensitive audit correlation.
    pub audit_correlation_id: String,
    /// Whether an identical durable request was replayed.
    pub replayed: bool,
}

/// Reason one recalled item was selected.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecallItem {
    /// Selected record.
    pub record: MemoryRecord,
    /// Stable human-readable selection reasons.
    pub selection_reasons: Vec<String>,
    /// Deterministic rank score.
    pub score: f64,
}

/// Budgets actually consumed by recall.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryBudgetUsage {
    /// Returned items.
    pub items: usize,
    /// Deepest expansion level.
    pub depth: u32,
    /// Serialized payload bytes.
    pub payload_bytes: usize,
    /// Deterministic expansion cost.
    pub cost: u64,
    /// Wall time observed by the engine.
    pub elapsed_ms: u64,
}

/// Recall completeness and bounded-outcome signals.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecallOutcome {
    /// Expansion stopped at a relationship-degree safety threshold.
    SupernodeBlocked,
    /// Deterministic traversal cost reached the caller budget.
    CostBudgetExhausted,
    /// Response shaping removed items to respect the byte budget.
    PayloadBudgetExhausted,
    /// Execution reached the caller wall-time budget.
    Timeout,
    /// Optional semantic seed resolution was unavailable; bounded lexical and explicit seeds remain.
    SemanticProviderUnavailable,
    /// A storage adapter supplied a deliberately incomplete authorized projection.
    PartialPageIn,
    /// A trusted runtime cancellation interrupted work.
    Cancelled,
    /// A trusted runtime capacity gate rejected additional work.
    Overloaded,
}

/// Recall completeness and bounded-outcome signals.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecallCompleteness {
    /// Whether every eligible result fit the budgets.
    pub complete: bool,
    /// Whether one or more budgets truncated the result.
    pub truncated: bool,
    /// Stable typed outcome codes such as `supernode_blocked`.
    pub outcomes: Vec<RecallOutcome>,
}

/// Bounded recall result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecallResult {
    /// Traceable recall execution identity.
    pub recall_id: String,
    /// Ranked selected memories.
    pub items: Vec<RecallItem>,
    /// Evidence-bearing relationships in the returned working set.
    pub relationships: Vec<MemoryRelationship>,
    /// Completeness signals.
    pub completeness: RecallCompleteness,
    /// Budgets consumed.
    pub usage: MemoryBudgetUsage,
    /// Opaque continuation token when more results remain.
    pub next_page_token: Option<String>,
}

/// Consolidation result preserving proposal and original identities.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsolidationResult {
    /// Stable proposal identity.
    pub proposal_id: String,
    /// Whether policy-approved changes were applied.
    pub applied: bool,
    /// Canonical memory when one was selected.
    pub canonical_id: Option<String>,
    /// Original records retained for audit and disagreement.
    pub originals_retained: Vec<String>,
    /// Whether disagreement preservation was enforced.
    pub disagreements_retained: bool,
    /// Optional mutation receipt for approved application.
    pub receipt: Option<MutationReceipt>,
}

/// One version entry in a trace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceVersion {
    /// Record identity.
    pub record_id: String,
    /// Monotonic version.
    pub version: u64,
    /// Lifecycle at that version.
    pub lifecycle: MemoryLifecycle,
    /// Recorded time.
    pub recorded_at: String,
}

/// One relationship path supporting a trace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TracePath {
    /// Ordered memory identities.
    pub memory_ids: Vec<String>,
    /// Ordered relationship identities.
    pub relationship_ids: Vec<String>,
    /// Evidence/source references supporting the path.
    pub evidence_source_ids: Vec<String>,
}

/// Explainability result for records, recalls, and mutations.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceResult {
    /// Requested target.
    pub target: MemoryTarget,
    /// Version history relevant to the explanation.
    pub versions: Vec<TraceVersion>,
    /// Supporting paths.
    pub paths: Vec<TracePath>,
    /// Authenticated actor attribution.
    pub actor_id: String,
    /// Optional agent attribution.
    pub agent_id: Option<String>,
    /// Trusted session attribution.
    pub session_id: String,
    /// Non-sensitive policy decisions.
    pub policy_decisions: Vec<String>,
    /// Additional stable explanation fields.
    pub details: BTreeMap<String, String>,
}

/// Responses for the seven high-level operations.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "operation", content = "result", rename_all = "snake_case")]
pub enum MemoryResponse {
    /// Remember result.
    Remember {
        /// Current record.
        record: MemoryRecord,
        /// Durable mutation receipt.
        receipt: MutationReceipt,
    },
    /// Relate result.
    Relate {
        /// Current relationship.
        relationship: MemoryRelationship,
        /// Durable mutation receipt.
        receipt: MutationReceipt,
    },
    /// Recall result.
    Recall(RecallResult),
    /// Update result for a memory record.
    Update {
        /// Updated memory.
        record: MemoryRecord,
        /// Durable mutation receipt.
        receipt: MutationReceipt,
    },
    /// Update result for a relationship record.
    UpdateRelationship {
        /// Updated relationship.
        relationship: MemoryRelationship,
        /// Durable mutation receipt.
        receipt: MutationReceipt,
    },
    /// Forget result.
    Forget {
        /// Durable mutation receipt.
        receipt: MutationReceipt,
        /// Applied application forgetting mode.
        mode: ForgetMode,
    },
    /// Consolidation result.
    Consolidate(ConsolidationResult),
    /// Trace result.
    Trace(TraceResult),
}

/// Stable error taxonomy shared by embedded and standalone adapters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MemoryErrorCode {
    /// Payload or trusted context is malformed.
    InvalidRequest,
    /// Explicit recall budgets are invalid.
    InvalidBudget,
    /// Permission is not granted.
    PermissionDenied,
    /// Target is absent or hidden by workspace policy.
    NotFound,
    /// Expected version did not match.
    VersionConflict,
    /// Idempotency key was reused with different input.
    IdempotencyConflict,
    /// Required mutation idempotency key was absent.
    IdempotencyKeyRequired,
    /// Traversal or response budget was exhausted.
    BudgetExceeded,
    /// Execution was cancelled.
    Cancelled,
    /// Runtime is overloaded.
    Overloaded,
    /// Optional semantic provider is unavailable and no bounded fallback exists.
    SemanticProviderUnavailable,
    /// Durability acknowledgement failed.
    DurabilityFailed,
    /// Consolidation was not policy-approved.
    PolicyApprovalRequired,
    /// Stable internal failure with no sensitive storage details.
    Internal,
}

/// Typed memory operation error.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryError {
    /// Stable machine code.
    pub code: MemoryErrorCode,
    /// Non-sensitive explanation.
    pub message: String,
    /// Whether retry may succeed without changing the request.
    pub retryable: bool,
    /// Non-sensitive audit correlation when available.
    pub correlation_id: Option<String>,
}

impl MemoryError {
    pub(crate) fn new(code: MemoryErrorCode, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code,
            message: message.into(),
            retryable,
            correlation_id: None,
        }
    }

    pub(crate) fn correlated(mut self, context: &MemoryServiceContext) -> Self {
        self.correlation_id = Some(context.correlation_id.clone());
        self
    }
}

impl std::fmt::Display for MemoryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.message)
    }
}

impl std::error::Error for MemoryError {}

impl CorroboreEngine {
    /// Executes one high-level memory operation against trusted runtime context.
    ///
    pub fn execute_memory(
        &mut self,
        context: &MemoryServiceContext,
        request: &MemoryRequest,
    ) -> Result<MemoryResponse, MemoryError> {
        validate_request_permission(context, &request.operation)
            .map_err(|error| error.correlated(context))?;
        validate_request(request).map_err(|error| error.correlated(context))?;
        let prepared = self
            .persistence
            .as_mut()
            .map(|adapter| adapter.prepare_memory_operation(&request.operation, context))
            .transpose()
            .map_err(|reason| {
                MemoryError::new(
                    MemoryErrorCode::DurabilityFailed,
                    format!("memory projection is unavailable: {reason}"),
                    true,
                )
                .correlated(context)
            })?
            .flatten();
        if let Some(graph) = prepared {
            self.gateway.replace_graph(graph);
        }

        if is_mutation(&request.operation) {
            let idempotency_key = request.idempotency_key.as_deref().ok_or_else(|| {
                MemoryError::new(
                    MemoryErrorCode::IdempotencyKeyRequired,
                    "mutations require a non-empty idempotency_key",
                    false,
                )
                .correlated(context)
            })?;
            let request_hash = request_hash(request)?;
            if let Some(response) = replay_response(
                self.gateway.graph(),
                &context.workspace_id,
                idempotency_key,
                &request_hash,
            )
            .map_err(|error| error.correlated(context))?
            {
                return Ok(mark_replayed(response));
            }

            let previous = self.gateway.graph().clone();
            let mut next = previous.clone();
            let response =
                execute_operation(&mut next, &mut self.memory_recall_traces, context, request)
                    .map_err(|error| error.correlated(context))?;
            store_receipt(
                &mut next,
                context,
                idempotency_key,
                &request_hash,
                &response,
            )
            .map_err(|error| error.correlated(context))?;
            if let Some(adapter) = self.persistence.as_mut()
                && let Err(reason) = adapter.persist_graph_transition(&previous, &next)
            {
                return Err(MemoryError::new(
                    MemoryErrorCode::DurabilityFailed,
                    format!("configured durability gate rejected the mutation: {reason}"),
                    true,
                )
                .correlated(context));
            }
            self.gateway.replace_graph(next);
            return Ok(response);
        }

        let mut graph = self.gateway.graph().clone();
        execute_operation(&mut graph, &mut self.memory_recall_traces, context, request)
            .map_err(|error| error.correlated(context))
    }
}

const MEMORY_LABEL: &str = "CorroboreMemory";
const RECEIPT_LABEL: &str = "CorroboreMemoryReceipt";
const RELATIONSHIP_KIND: &str = "CORROBORE_MEMORY_RELATION";
const CONSOLIDATION_KIND: &str = "CORROBORE_MEMORY_CONSOLIDATION";
const P_WORKSPACE: &str = "corrobore.memory.workspace";
const P_IDENTITY: &str = "corrobore.memory.identity_key";
const P_KIND: &str = "corrobore.memory.kind";
const P_SCHEMA: &str = "corrobore.memory.schema_version";
const P_CONTENT: &str = "corrobore.memory.content";
const P_PROPERTIES: &str = "corrobore.memory.properties";
const P_PROVENANCE: &str = "corrobore.memory.provenance";
const P_CONFIDENCE: &str = "corrobore.memory.confidence";
const P_VALID_FROM: &str = "corrobore.memory.valid_from";
const P_VALID_UNTIL: &str = "corrobore.memory.valid_until";
const P_RECORDED_AT: &str = "corrobore.memory.recorded_at";
const P_EXPIRES_AT: &str = "corrobore.memory.expires_at";
const P_LIFECYCLE: &str = "corrobore.memory.lifecycle";
const P_TAGS: &str = "corrobore.memory.tags";
const P_ACTOR: &str = "corrobore.memory.actor";
const P_AGENT: &str = "corrobore.memory.agent";
const P_SESSION: &str = "corrobore.memory.session";
const P_REQUEST: &str = "corrobore.memory.request";
const P_CORRELATION: &str = "corrobore.memory.correlation";
const P_REL_IDENTITY: &str = "corrobore.memory.relationship.identity_key";
const P_REL_KIND: &str = "corrobore.memory.relationship.kind";
const P_RECEIPT_KEY: &str = "corrobore.memory.receipt.key";
const P_RECEIPT_HASH: &str = "corrobore.memory.receipt.hash";
const P_RECEIPT_RESPONSE: &str = "corrobore.memory.receipt.response";

fn is_mutation(operation: &MemoryOperation) -> bool {
    matches!(
        operation,
        MemoryOperation::Remember(_)
            | MemoryOperation::Relate(_)
            | MemoryOperation::Update(_)
            | MemoryOperation::Forget(_)
            | MemoryOperation::Consolidate(ConsolidateRequest {
                mode: ConsolidateMode::ApplyApproved { .. },
                ..
            })
    )
}

fn validate_request_permission(
    context: &MemoryServiceContext,
    operation: &MemoryOperation,
) -> Result<(), MemoryError> {
    let allowed = match operation {
        MemoryOperation::Remember(_) | MemoryOperation::Relate(_) | MemoryOperation::Update(_) => {
            context.permissions.write
        }
        MemoryOperation::Recall(_) => context.permissions.read,
        MemoryOperation::Forget(_) => context.permissions.forget,
        MemoryOperation::Consolidate(_) => context.permissions.consolidate,
        MemoryOperation::Trace(_) => context.permissions.trace,
    };
    if allowed {
        Ok(())
    } else {
        Err(MemoryError::new(
            MemoryErrorCode::PermissionDenied,
            "trusted runtime policy denied this memory capability",
            false,
        ))
    }
}

fn validate_request(request: &MemoryRequest) -> Result<(), MemoryError> {
    if request.contract_version != MemoryContractVersion::V1 {
        return Err(invalid("unsupported memory contract version"));
    }
    if is_mutation(&request.operation)
        && request
            .idempotency_key
            .as_deref()
            .is_some_and(|key| key.trim().is_empty() || key.len() > 256)
    {
        return Err(invalid("idempotency_key must contain 1 to 256 characters"));
    }
    match &request.operation {
        MemoryOperation::Remember(input) => {
            require_text("kind", &input.kind)?;
            require_text("schema_version", &input.schema_version)?;
            validate_confidence(input.confidence)?;
            validate_time_range(input.valid_from.as_deref(), input.valid_until.as_deref())?;
            validate_optional_time(input.expires_at.as_deref())?;
            validate_content(&input.content)?;
            validate_provenance(&input.provenance)?;
        }
        MemoryOperation::Relate(input) => {
            require_text("source_id", &input.source_id)?;
            require_text("target_id", &input.target_id)?;
            require_text("kind", &input.kind)?;
            validate_confidence(input.confidence)?;
            validate_time_range(input.valid_from.as_deref(), input.valid_until.as_deref())?;
            validate_optional_time(input.expires_at.as_deref())?;
            validate_provenance(&input.provenance)?;
        }
        MemoryOperation::Recall(input) => validate_recall(input)?,
        MemoryOperation::Update(input) => {
            if !matches!(
                input.target,
                MemoryTarget::Memory(_) | MemoryTarget::Relationship(_)
            ) {
                return Err(invalid("update target must be a memory or relationship"));
            }
            validate_confidence(input.patch.confidence)?;
            validate_optional_time(input.patch.expires_at.as_deref())?;
            validate_provenance(&input.patch.add_provenance)?;
            if let Some(content) = &input.patch.content {
                validate_content(content)?;
            }
        }
        MemoryOperation::Forget(input) => {
            require_text("memory_id", &input.memory_id)?;
            require_text("reason", &input.reason)?;
            if input.mode == ForgetMode::Expire && input.expires_at.is_none() {
                return Err(invalid("expires_at is required for expire mode"));
            }
            validate_optional_time(input.expires_at.as_deref())?;
        }
        MemoryOperation::Consolidate(input) => {
            if input.memory_ids.len() < 2 || input.memory_ids.len() > 100 {
                return Err(invalid("consolidation requires 2 to 100 memory_ids"));
            }
            require_text("reason", &input.reason)?;
            if let ConsolidateMode::ApplyApproved {
                proposal_id,
                approval_policy,
            } = &input.mode
            {
                require_text("proposal_id", proposal_id)?;
                require_text("approval_policy", approval_policy)?;
                if !input.preserve_disagreements {
                    return Err(MemoryError::new(
                        MemoryErrorCode::PolicyApprovalRequired,
                        "destructive consolidation is unavailable; disagreements must be preserved",
                        false,
                    ));
                }
            }
        }
        MemoryOperation::Trace(input) => match &input.target {
            MemoryTarget::Memory(id)
            | MemoryTarget::Relationship(id)
            | MemoryTarget::Recall(id)
            | MemoryTarget::Mutation(id) => require_text("target.id", id)?,
        },
    }
    Ok(())
}

fn validate_recall(input: &RecallRequest) -> Result<(), MemoryError> {
    require_text("objective", &input.objective)?;
    let limits = input.limits;
    if limits.max_items == 0
        || limits.max_depth == 0
        || limits.max_payload_bytes == 0
        || limits.max_cost == 0
        || limits.timeout_ms == 0
        || limits.supernode_threshold == 0
    {
        return Err(MemoryError::new(
            MemoryErrorCode::InvalidBudget,
            "recall limits must all be greater than zero",
            false,
        ));
    }
    if limits.max_items > 10_000
        || limits.max_depth > 16
        || limits.max_payload_bytes > 16 * 1024 * 1024
        || limits.max_cost > 1_000_000
        || limits.timeout_ms > 60_000
    {
        return Err(MemoryError::new(
            MemoryErrorCode::InvalidBudget,
            "recall limits exceed the stable contract maxima",
            false,
        ));
    }
    Ok(())
}

fn require_text(field: &str, value: &str) -> Result<(), MemoryError> {
    if value.trim().is_empty() {
        Err(invalid(format!("{field} must not be empty")))
    } else {
        Ok(())
    }
}

fn validate_content(content: &MemoryContent) -> Result<(), MemoryError> {
    match content {
        MemoryContent::Text(text) | MemoryContent::TextAndProperties { text, .. } => {
            require_text("content.text", text)
        }
        MemoryContent::Properties(value) => {
            if value.is_object() {
                Ok(())
            } else {
                Err(invalid("structured memory content must be a JSON object"))
            }
        }
    }
}

fn validate_provenance(values: &[ProvenanceReference]) -> Result<(), MemoryError> {
    for value in values {
        require_text("provenance.source_id", &value.source_id)?;
        validate_optional_time(value.observed_at.as_deref())?;
    }
    Ok(())
}

fn validate_confidence(value: Option<f64>) -> Result<(), MemoryError> {
    if value.is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(&value)) {
        Err(invalid(
            "confidence must be finite and in the inclusive range 0..=1",
        ))
    } else {
        Ok(())
    }
}

fn validate_optional_time(value: Option<&str>) -> Result<(), MemoryError> {
    if let Some(value) = value {
        chrono::DateTime::parse_from_rfc3339(value)
            .map_err(|_| invalid("temporal values must use RFC3339"))?;
    }
    Ok(())
}

fn validate_time_range(from: Option<&str>, until: Option<&str>) -> Result<(), MemoryError> {
    validate_optional_time(from)?;
    validate_optional_time(until)?;
    if let (Some(from), Some(until)) = (from, until) {
        let from = DateTime::parse_from_rfc3339(from)
            .map_err(|_| invalid("temporal values must use RFC3339"))?;
        let until = DateTime::parse_from_rfc3339(until)
            .map_err(|_| invalid("temporal values must use RFC3339"))?;
        if from > until {
            return Err(invalid(
                "valid_from must be less than or equal to valid_until",
            ));
        }
    }
    Ok(())
}

fn invalid(message: impl Into<String>) -> MemoryError {
    MemoryError::new(MemoryErrorCode::InvalidRequest, message, false)
}

fn request_hash(request: &MemoryRequest) -> Result<String, MemoryError> {
    let bytes = serde_json::to_vec(request)
        .map_err(|_| invalid("memory request could not be canonicalized"))?;
    Ok(hex_hash(&bytes))
}

fn hex_hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn execute_operation(
    graph: &mut Graph,
    recall_traces: &mut BTreeMap<String, RecallResult>,
    context: &MemoryServiceContext,
    request: &MemoryRequest,
) -> Result<MemoryResponse, MemoryError> {
    match &request.operation {
        MemoryOperation::Remember(input) => remember_operation(graph, context, input),
        MemoryOperation::Relate(input) => relate_operation(graph, context, input),
        MemoryOperation::Recall(input) => recall_operation(graph, recall_traces, context, input),
        MemoryOperation::Update(input) => update_operation(graph, context, input),
        MemoryOperation::Forget(input) => forget_operation(graph, context, input),
        MemoryOperation::Consolidate(input) => consolidate_operation(graph, context, input),
        MemoryOperation::Trace(input) => trace_operation(graph, recall_traces, context, input),
    }
}

fn remember_operation(
    graph: &mut Graph,
    context: &MemoryServiceContext,
    input: &RememberRequest,
) -> Result<MemoryResponse, MemoryError> {
    let now = Utc::now().to_rfc3339();
    let existing = input.identity_key.as_deref().and_then(|identity| {
        graph.list_nodes().ok()?.into_iter().find(|node| {
            node.has_label(MEMORY_LABEL)
                && belongs_to_node(node, &context.workspace_id)
                && string_property(node.properties(), P_IDENTITY) == Some(identity)
        })
    });

    let node = if let Some(existing) = existing {
        let mut provenance =
            json_property::<Vec<ProvenanceReference>>(existing.properties(), P_PROVENANCE)
                .unwrap_or_default();
        append_unique(&mut provenance, &input.provenance);
        let mut patch = NodePatch::default()
            .set_property(P_KIND, PropertyValue::String(input.kind.clone()))
            .set_property(
                P_SCHEMA,
                PropertyValue::String(input.schema_version.clone()),
            )
            .set_property(P_CONTENT, json_value(&input.content)?)
            .set_property(P_PROVENANCE, json_value(&provenance)?)
            .set_property(P_CONFIDENCE, optional_json_value(input.confidence))
            .set_property(
                P_VALID_FROM,
                optional_string_value(input.valid_from.as_deref()),
            )
            .set_property(
                P_VALID_UNTIL,
                optional_string_value(input.valid_until.as_deref()),
            )
            .set_property(
                P_EXPIRES_AT,
                optional_string_value(input.expires_at.as_deref()),
            )
            .set_property(P_LIFECYCLE, lifecycle_value(MemoryLifecycle::Active))
            .set_property(P_TAGS, json_value(&input.tags)?)
            .set_property(P_RECORDED_AT, PropertyValue::String(now.clone()));
        if let Some(confidence) = input.confidence {
            patch = patch.set_confidence(map_confidence(confidence)?);
        }
        graph
            .update_node(existing.id(), patch)
            .map_err(map_graph_error)?;
        graph
            .get_node(existing.id())
            .map_err(map_graph_error)?
            .ok_or_else(|| internal("updated memory disappeared"))?
    } else {
        let mut node_input = NodeInput::new([MEMORY_LABEL])
            .with_property(
                P_WORKSPACE,
                PropertyValue::String(context.workspace_id.clone()),
            )
            .with_property(P_KIND, PropertyValue::String(input.kind.clone()))
            .with_property(
                P_SCHEMA,
                PropertyValue::String(input.schema_version.clone()),
            )
            .with_property(P_CONTENT, json_value(&input.content)?)
            .with_property(P_PROVENANCE, json_value(&input.provenance)?)
            .with_property(P_CONFIDENCE, optional_json_value(input.confidence))
            .with_property(
                P_VALID_FROM,
                optional_string_value(input.valid_from.as_deref()),
            )
            .with_property(
                P_VALID_UNTIL,
                optional_string_value(input.valid_until.as_deref()),
            )
            .with_property(P_RECORDED_AT, PropertyValue::String(now))
            .with_property(
                P_EXPIRES_AT,
                optional_string_value(input.expires_at.as_deref()),
            )
            .with_property(P_LIFECYCLE, lifecycle_value(MemoryLifecycle::Active))
            .with_property(P_TAGS, json_value(&input.tags)?)
            .with_property(P_ACTOR, PropertyValue::String(context.actor_id.clone()))
            .with_property(P_AGENT, optional_string_value(context.agent_id.as_deref()))
            .with_property(P_SESSION, PropertyValue::String(context.session_id.clone()))
            .with_property(P_REQUEST, PropertyValue::String(context.request_id.clone()))
            .with_property(
                P_CORRELATION,
                PropertyValue::String(context.correlation_id.clone()),
            );
        if let Some(identity_key) = &input.identity_key {
            node_input =
                node_input.with_property(P_IDENTITY, PropertyValue::String(identity_key.clone()));
        }
        if let Some(confidence) = input.confidence {
            node_input = node_input.with_confidence(map_confidence(confidence)?);
        }
        let id = graph.create_node(node_input).map_err(map_graph_error)?;
        graph
            .get_node(&id)
            .map_err(map_graph_error)?
            .ok_or_else(|| internal("created memory disappeared"))?
    };
    let record = memory_record(&node)?;
    Ok(MemoryResponse::Remember {
        receipt: receipt(context, &record.id, record.version),
        record,
    })
}

fn relate_operation(
    graph: &mut Graph,
    context: &MemoryServiceContext,
    input: &RelateRequest,
) -> Result<MemoryResponse, MemoryError> {
    let source = visible_memory_node(graph, &context.workspace_id, &input.source_id)?;
    let target = visible_memory_node(graph, &context.workspace_id, &input.target_id)?;
    let now = Utc::now().to_rfc3339();
    let existing = graph
        .list_relationships()
        .map_err(map_graph_error)?
        .into_iter()
        .find(|relationship| {
            belongs_to_relationship(relationship, &context.workspace_id)
                && input.identity_key.as_deref().is_some_and(|identity| {
                    string_property(relationship.properties(), P_REL_IDENTITY) == Some(identity)
                })
        });
    let relationship = if let Some(existing) = existing {
        let mut provenance =
            json_property::<Vec<ProvenanceReference>>(existing.properties(), P_PROVENANCE)
                .unwrap_or_default();
        append_unique(&mut provenance, &input.provenance);
        let mut patch = RelationshipPatch::default()
            .set_property(P_REL_KIND, PropertyValue::String(input.kind.clone()))
            .set_property(P_PROPERTIES, PropertyValue::Json(input.properties.clone()))
            .set_property(P_PROVENANCE, json_value(&provenance)?)
            .set_property(P_CONFIDENCE, optional_json_value(input.confidence))
            .set_property(
                P_VALID_FROM,
                optional_string_value(input.valid_from.as_deref()),
            )
            .set_property(
                P_VALID_UNTIL,
                optional_string_value(input.valid_until.as_deref()),
            )
            .set_property(
                P_EXPIRES_AT,
                optional_string_value(input.expires_at.as_deref()),
            )
            .set_property(P_LIFECYCLE, lifecycle_value(input.lifecycle))
            .set_property(P_RECORDED_AT, PropertyValue::String(now));
        if let Some(confidence) = input.confidence {
            patch = patch.set_confidence(map_confidence(confidence)?);
        }
        graph
            .update_relationship(existing.id(), patch)
            .map_err(map_graph_error)?;
        graph
            .get_relationship(existing.id())
            .map_err(map_graph_error)?
            .ok_or_else(|| internal("updated relationship disappeared"))?
    } else {
        let mut relationship_input =
            RelationshipInput::new(source.id().clone(), RELATIONSHIP_KIND, target.id().clone())
                .map_err(map_graph_error)?
                .with_property(
                    P_WORKSPACE,
                    PropertyValue::String(context.workspace_id.clone()),
                )
                .with_property(P_REL_KIND, PropertyValue::String(input.kind.clone()))
                .with_property(P_PROPERTIES, PropertyValue::Json(input.properties.clone()))
                .with_property(P_PROVENANCE, json_value(&input.provenance)?)
                .with_property(P_CONFIDENCE, optional_json_value(input.confidence))
                .with_property(
                    P_VALID_FROM,
                    optional_string_value(input.valid_from.as_deref()),
                )
                .with_property(
                    P_VALID_UNTIL,
                    optional_string_value(input.valid_until.as_deref()),
                )
                .with_property(
                    P_EXPIRES_AT,
                    optional_string_value(input.expires_at.as_deref()),
                )
                .with_property(P_LIFECYCLE, lifecycle_value(input.lifecycle))
                .with_property(P_RECORDED_AT, PropertyValue::String(now))
                .with_property(P_ACTOR, PropertyValue::String(context.actor_id.clone()))
                .with_property(P_AGENT, optional_string_value(context.agent_id.as_deref()))
                .with_property(P_SESSION, PropertyValue::String(context.session_id.clone()))
                .with_property(P_REQUEST, PropertyValue::String(context.request_id.clone()))
                .with_property(
                    P_CORRELATION,
                    PropertyValue::String(context.correlation_id.clone()),
                );
        if let Some(identity_key) = &input.identity_key {
            relationship_input = relationship_input
                .with_property(P_REL_IDENTITY, PropertyValue::String(identity_key.clone()));
        }
        if let Some(confidence) = input.confidence {
            relationship_input = relationship_input.with_confidence(map_confidence(confidence)?);
        }
        let id = graph
            .create_relationship(relationship_input)
            .map_err(map_graph_error)?;
        graph
            .get_relationship(&id)
            .map_err(map_graph_error)?
            .ok_or_else(|| internal("created relationship disappeared"))?
    };
    let relationship = memory_relationship(&relationship)?;
    Ok(MemoryResponse::Relate {
        receipt: receipt(context, &relationship.id, relationship.version),
        relationship,
    })
}

fn recall_operation(
    graph: &Graph,
    recall_traces: &mut BTreeMap<String, RecallResult>,
    context: &MemoryServiceContext,
    input: &RecallRequest,
) -> Result<MemoryResponse, MemoryError> {
    let started = Instant::now();
    let offset = decode_page_token(
        input.page_token.as_deref(),
        &context.workspace_id,
        &input.objective,
    )?;
    let objective = input.objective.to_ascii_lowercase();
    let now = Utc::now().to_rfc3339();
    let all_nodes = graph
        .list_nodes()
        .map_err(map_graph_error)?
        .into_iter()
        .filter(|node| {
            node.has_label(MEMORY_LABEL)
                && belongs_to_node(node, &context.workspace_id)
                && visible_at(node.properties(), &now)
        })
        .collect::<Vec<_>>();
    let by_id = all_nodes
        .iter()
        .map(|node| (node.id().as_str().to_owned(), node.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut queue = VecDeque::new();
    let mut depths = BTreeMap::<String, u32>::new();
    let mut outcomes = Vec::new();

    for seed in &input.seed_ids {
        if by_id.contains_key(seed) {
            queue.push_back(seed.clone());
            depths.insert(seed.clone(), 0);
        }
    }
    let mut lexical = all_nodes
        .iter()
        .filter_map(|node| {
            let record = memory_record(node).ok()?;
            let haystack = format!(
                "{} {} {} {}",
                record.identity_key.as_deref().unwrap_or_default(),
                record.kind,
                content_text(&record.content),
                record.tags.join(" ")
            )
            .to_ascii_lowercase();
            let matches = objective
                .split_whitespace()
                .filter(|term| haystack.contains(term))
                .count();
            (matches > 0).then_some((node.id().as_str().to_owned(), matches))
        })
        .collect::<Vec<_>>();
    lexical.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(&right.0)));
    for (id, _) in &lexical {
        if !depths.contains_key(id) {
            queue.push_back(id.clone());
            depths.insert(id.clone(), 0);
        }
    }
    if queue.is_empty() {
        outcomes.push(RecallOutcome::SemanticProviderUnavailable);
    }

    let mut selected_ids = Vec::new();
    let mut selected = BTreeSet::new();
    let mut relationship_ids = BTreeSet::new();
    let mut cost = 0_u64;
    let mut deepest = 0_u32;
    let target_count = offset.saturating_add(input.limits.max_items);
    while let Some(id) = queue.pop_front() {
        if started.elapsed().as_millis() as u64 >= input.limits.timeout_ms {
            outcomes.push(RecallOutcome::Timeout);
            break;
        }
        if cost >= input.limits.max_cost {
            outcomes.push(RecallOutcome::CostBudgetExhausted);
            break;
        }
        if !selected.insert(id.clone()) {
            continue;
        }
        selected_ids.push(id.clone());
        let depth = depths.get(&id).copied().unwrap_or(0);
        deepest = deepest.max(depth);
        if selected_ids.len() >= target_count.saturating_add(1) {
            break;
        }
        if depth >= input.limits.max_depth {
            continue;
        }
        let node_id = NodeId::new(id.clone()).map_err(map_graph_error)?;
        let mut edges = graph.outgoing(&node_id).map_err(map_graph_error)?;
        edges.extend(graph.incoming(&node_id).map_err(map_graph_error)?);
        edges.retain(|relationship| {
            belongs_to_relationship(relationship, &context.workspace_id)
                && visible_at(relationship.properties(), &now)
        });
        edges.sort_by(|left, right| left.id().as_str().cmp(right.id().as_str()));
        edges.dedup_by(|left, right| left.id() == right.id());
        if edges.len() > input.limits.supernode_threshold {
            outcomes.push(RecallOutcome::SupernodeBlocked);
            continue;
        }
        for relationship in edges {
            cost = cost.saturating_add(1);
            relationship_ids.insert(relationship.id().as_str().to_owned());
            let neighbor = if relationship.source().as_str() == id {
                relationship.target().as_str()
            } else {
                relationship.source().as_str()
            };
            if by_id.contains_key(neighbor) && !depths.contains_key(neighbor) {
                depths.insert(neighbor.to_owned(), depth + 1);
                queue.push_back(neighbor.to_owned());
            }
            if cost >= input.limits.max_cost {
                break;
            }
        }
    }

    let more = selected_ids.len() > target_count;
    selected_ids.truncate(target_count);
    let page_ids = selected_ids
        .iter()
        .skip(offset)
        .take(input.limits.max_items)
        .cloned()
        .collect::<Vec<_>>();
    let lexical_scores = lexical.into_iter().collect::<BTreeMap<_, _>>();
    let items = page_ids
        .iter()
        .filter_map(|id| by_id.get(id))
        .map(|node| {
            let record = memory_record(node)?;
            let depth = depths.get(record.id.as_str()).copied().unwrap_or(0);
            let mut reasons = Vec::new();
            if input.seed_ids.contains(&record.id) {
                reasons.push("explicit_seed".to_owned());
            }
            if lexical_scores.contains_key(&record.id) {
                reasons.push("objective_lexical_match".to_owned());
            }
            if depth > 0 {
                reasons.push(format!("authorized_relationship_depth_{depth}"));
            }
            if reasons.is_empty() {
                reasons.push("bounded_authorized_neighborhood".to_owned());
            }
            Ok(RecallItem {
                score: lexical_scores.get(&record.id).copied().unwrap_or(0) as f64
                    + 1.0 / f64::from(depth + 1),
                record,
                selection_reasons: reasons,
            })
        })
        .collect::<Result<Vec<_>, MemoryError>>()?;
    let page_set = page_ids.iter().cloned().collect::<BTreeSet<_>>();
    let relationships = relationship_ids
        .iter()
        .filter_map(|id| RelationshipId::new(id.clone()).ok())
        .filter_map(|id| graph.get_relationship(&id).ok().flatten())
        .filter(|relationship| {
            page_set.contains(relationship.source().as_str())
                && page_set.contains(relationship.target().as_str())
        })
        .map(|relationship| memory_relationship(&relationship))
        .collect::<Result<Vec<_>, _>>()?;
    let mut truncated = more || !outcomes.is_empty() || !queue.is_empty();
    let recall_id = format!(
        "recall--{}",
        &hex_hash(
            format!(
                "{}:{}:{}:{}",
                context.workspace_id, context.request_id, input.objective, offset
            )
            .as_bytes()
        )[..24]
    );
    let mut result = RecallResult {
        recall_id,
        items,
        relationships,
        completeness: RecallCompleteness {
            complete: !truncated,
            truncated,
            outcomes,
        },
        usage: MemoryBudgetUsage {
            items: page_ids.len(),
            depth: deepest,
            payload_bytes: 0,
            cost,
            elapsed_ms: started.elapsed().as_millis() as u64,
        },
        next_page_token: more.then(|| {
            encode_page_token(
                &context.workspace_id,
                &input.objective,
                offset.saturating_add(input.limits.max_items),
            )
        }),
    };
    while serde_json::to_vec(&result).map_or(usize::MAX, |bytes| bytes.len())
        > input.limits.max_payload_bytes
        && !result.items.is_empty()
    {
        result.items.pop();
        let kept = result
            .items
            .iter()
            .map(|item| item.record.id.as_str())
            .collect::<BTreeSet<_>>();
        result.relationships.retain(|relationship| {
            kept.contains(relationship.source_id.as_str())
                && kept.contains(relationship.target_id.as_str())
        });
        truncated = true;
        if !result
            .completeness
            .outcomes
            .contains(&RecallOutcome::PayloadBudgetExhausted)
        {
            result
                .completeness
                .outcomes
                .push(RecallOutcome::PayloadBudgetExhausted);
        }
    }
    result.usage.items = result.items.len();
    result.usage.payload_bytes = serde_json::to_vec(&result).map_or(0, |bytes| bytes.len());
    result.completeness.complete = !truncated;
    result.completeness.truncated = truncated;
    recall_traces.insert(result.recall_id.clone(), result.clone());
    while recall_traces.len() > 1_024 {
        if let Some(oldest) = recall_traces.keys().next().cloned() {
            recall_traces.remove(&oldest);
        }
    }
    Ok(MemoryResponse::Recall(result))
}

fn update_operation(
    graph: &mut Graph,
    context: &MemoryServiceContext,
    input: &MemoryUpdateRequest,
) -> Result<MemoryResponse, MemoryError> {
    match &input.target {
        MemoryTarget::Memory(id) => {
            let node = visible_memory_node(graph, &context.workspace_id, id)?;
            if input
                .expected_version
                .is_some_and(|expected| expected != node.version())
            {
                return Err(MemoryError::new(
                    MemoryErrorCode::VersionConflict,
                    "memory version precondition failed",
                    false,
                ));
            }
            let mut patch = common_node_patch(node.properties(), context, &input.patch)?;
            if let Some(confidence) = input.patch.confidence {
                patch = patch.set_confidence(map_confidence(confidence)?);
            }
            graph
                .update_node(node.id(), patch)
                .map_err(map_graph_error)?;
            let current = graph
                .get_node(node.id())
                .map_err(map_graph_error)?
                .ok_or_else(|| internal("updated memory disappeared"))?;
            let record = memory_record(&current)?;
            Ok(MemoryResponse::Update {
                receipt: receipt(context, &record.id, record.version),
                record,
            })
        }
        MemoryTarget::Relationship(id) => {
            let relationship = visible_memory_relationship(graph, &context.workspace_id, id)?;
            if input
                .expected_version
                .is_some_and(|expected| expected != relationship.version())
            {
                return Err(MemoryError::new(
                    MemoryErrorCode::VersionConflict,
                    "relationship version precondition failed",
                    false,
                ));
            }
            let mut patch =
                common_relationship_patch(relationship.properties(), context, &input.patch)?;
            if let Some(confidence) = input.patch.confidence {
                patch = patch.set_confidence(map_confidence(confidence)?);
            }
            graph
                .update_relationship(relationship.id(), patch)
                .map_err(map_graph_error)?;
            let current = graph
                .get_relationship(relationship.id())
                .map_err(map_graph_error)?
                .ok_or_else(|| internal("updated relationship disappeared"))?;
            let relationship = memory_relationship(&current)?;
            Ok(MemoryResponse::UpdateRelationship {
                receipt: receipt(context, &relationship.id, relationship.version),
                relationship,
            })
        }
        _ => Err(invalid("update target must be a memory or relationship")),
    }
}

fn common_node_patch(
    properties: &std::collections::HashMap<String, PropertyValue>,
    context: &MemoryServiceContext,
    input: &UpdatePatch,
) -> Result<NodePatch, MemoryError> {
    let mut provenance =
        json_property::<Vec<ProvenanceReference>>(properties, P_PROVENANCE).unwrap_or_default();
    append_unique(&mut provenance, &input.add_provenance);
    let mut tags = json_property::<Vec<String>>(properties, P_TAGS).unwrap_or_default();
    append_unique(&mut tags, &input.add_tags);
    let mut patch = NodePatch::default()
        .set_property(P_PROVENANCE, json_value(&provenance)?)
        .set_property(P_TAGS, json_value(&tags)?)
        .set_property(
            P_RECORDED_AT,
            PropertyValue::String(Utc::now().to_rfc3339()),
        )
        .set_property(P_ACTOR, PropertyValue::String(context.actor_id.clone()))
        .set_property(P_AGENT, optional_string_value(context.agent_id.as_deref()))
        .set_property(P_SESSION, PropertyValue::String(context.session_id.clone()))
        .set_property(P_REQUEST, PropertyValue::String(context.request_id.clone()))
        .set_property(
            P_CORRELATION,
            PropertyValue::String(context.correlation_id.clone()),
        );
    if let Some(content) = &input.content {
        patch = patch.set_property(P_CONTENT, json_value(content)?);
    }
    if let Some(confidence) = input.confidence {
        patch = patch.set_property(P_CONFIDENCE, optional_json_value(Some(confidence)));
    }
    if let Some(lifecycle) = input.lifecycle {
        patch = patch.set_property(P_LIFECYCLE, lifecycle_value(lifecycle));
    }
    if let Some(expires_at) = &input.expires_at {
        patch = patch.set_property(P_EXPIRES_AT, PropertyValue::String(expires_at.clone()));
    }
    Ok(patch)
}

fn common_relationship_patch(
    properties: &std::collections::HashMap<String, PropertyValue>,
    context: &MemoryServiceContext,
    input: &UpdatePatch,
) -> Result<RelationshipPatch, MemoryError> {
    let mut provenance =
        json_property::<Vec<ProvenanceReference>>(properties, P_PROVENANCE).unwrap_or_default();
    append_unique(&mut provenance, &input.add_provenance);
    let mut patch = RelationshipPatch::default()
        .set_property(P_PROVENANCE, json_value(&provenance)?)
        .set_property(
            P_RECORDED_AT,
            PropertyValue::String(Utc::now().to_rfc3339()),
        )
        .set_property(P_ACTOR, PropertyValue::String(context.actor_id.clone()))
        .set_property(P_AGENT, optional_string_value(context.agent_id.as_deref()))
        .set_property(P_SESSION, PropertyValue::String(context.session_id.clone()))
        .set_property(P_REQUEST, PropertyValue::String(context.request_id.clone()))
        .set_property(
            P_CORRELATION,
            PropertyValue::String(context.correlation_id.clone()),
        );
    if let Some(content) = &input.content {
        patch = patch.set_property(P_PROPERTIES, json_value(content)?);
    }
    if let Some(confidence) = input.confidence {
        patch = patch.set_property(P_CONFIDENCE, optional_json_value(Some(confidence)));
    }
    if let Some(lifecycle) = input.lifecycle {
        patch = patch.set_property(P_LIFECYCLE, lifecycle_value(lifecycle));
    }
    if let Some(expires_at) = &input.expires_at {
        patch = patch.set_property(P_EXPIRES_AT, PropertyValue::String(expires_at.clone()));
    }
    Ok(patch)
}

fn forget_operation(
    graph: &mut Graph,
    context: &MemoryServiceContext,
    input: &ForgetRequest,
) -> Result<MemoryResponse, MemoryError> {
    let node = visible_memory_node(graph, &context.workspace_id, &input.memory_id)?;
    let version = node.version().saturating_add(1);
    match input.mode {
        ForgetMode::Expire => {
            let patch = NodePatch::default()
                .set_property(P_LIFECYCLE, lifecycle_value(MemoryLifecycle::Expired))
                .set_property(
                    P_EXPIRES_AT,
                    PropertyValue::String(input.expires_at.clone().unwrap_or_default()),
                )
                .set_property(
                    P_RECORDED_AT,
                    PropertyValue::String(Utc::now().to_rfc3339()),
                )
                .set_property(
                    P_CORRELATION,
                    PropertyValue::String(context.correlation_id.clone()),
                );
            graph
                .update_node(node.id(), patch)
                .map_err(map_graph_error)?;
        }
        ForgetMode::Tombstone | ForgetMode::ApplicationDelete => {
            graph.tombstone_node(node.id()).map_err(map_graph_error)?;
        }
    }
    Ok(MemoryResponse::Forget {
        receipt: receipt(context, &input.memory_id, version),
        mode: input.mode,
    })
}

fn consolidate_operation(
    graph: &mut Graph,
    context: &MemoryServiceContext,
    input: &ConsolidateRequest,
) -> Result<MemoryResponse, MemoryError> {
    let mut ids = input.memory_ids.clone();
    ids.sort();
    ids.dedup();
    if ids.len() < 2 {
        return Err(invalid("consolidation requires two distinct memory_ids"));
    }
    for id in &ids {
        visible_memory_node(graph, &context.workspace_id, id)?;
    }
    let canonical_id = input.canonical_id.clone().or_else(|| ids.first().cloned());
    if canonical_id
        .as_ref()
        .is_some_and(|canonical| !ids.contains(canonical))
    {
        return Err(invalid("canonical_id must be one of memory_ids"));
    }
    let proposal_material = serde_json::to_vec(&(
        context.workspace_id.as_str(),
        &ids,
        &canonical_id,
        input.preserve_disagreements,
    ))
    .map_err(|_| internal("consolidation proposal could not be canonicalized"))?;
    let proposal_id = format!("consolidation--{}", &hex_hash(&proposal_material)[..24]);
    match &input.mode {
        ConsolidateMode::Propose => Ok(MemoryResponse::Consolidate(ConsolidationResult {
            proposal_id,
            applied: false,
            canonical_id,
            originals_retained: ids,
            disagreements_retained: input.preserve_disagreements,
            receipt: None,
        })),
        ConsolidateMode::ApplyApproved {
            proposal_id: supplied,
            approval_policy,
        } => {
            if supplied != &proposal_id {
                return Err(MemoryError::new(
                    MemoryErrorCode::PolicyApprovalRequired,
                    "approved proposal does not match the bounded consolidation inputs",
                    false,
                ));
            }
            let canonical = canonical_id
                .clone()
                .ok_or_else(|| invalid("approved consolidation requires canonical_id"))?;
            let canonical_node = visible_memory_node(graph, &context.workspace_id, &canonical)?;
            let mut max_version = canonical_node.version();
            for id in &ids {
                if id == &canonical {
                    continue;
                }
                let original = visible_memory_node(graph, &context.workspace_id, id)?;
                let patch = NodePatch::default()
                    .set_property(P_LIFECYCLE, lifecycle_value(MemoryLifecycle::Superseded))
                    .set_property(
                        P_RECORDED_AT,
                        PropertyValue::String(Utc::now().to_rfc3339()),
                    )
                    .set_property(
                        "corrobore.memory.superseded_by",
                        PropertyValue::String(canonical.clone()),
                    )
                    .set_property(
                        "corrobore.memory.consolidation_policy",
                        PropertyValue::String(approval_policy.clone()),
                    );
                graph
                    .update_node(original.id(), patch)
                    .map_err(map_graph_error)?;
                max_version = max_version.max(original.version().saturating_add(1));
                let relation_input = RelationshipInput::new(
                    original.id().clone(),
                    CONSOLIDATION_KIND,
                    canonical_node.id().clone(),
                )
                .map_err(map_graph_error)?
                .with_property(
                    P_WORKSPACE,
                    PropertyValue::String(context.workspace_id.clone()),
                )
                .with_property(
                    P_REL_KIND,
                    PropertyValue::String("superseded_by".to_owned()),
                )
                .with_property(P_PROVENANCE, PropertyValue::Json(serde_json::json!([])))
                .with_property(P_LIFECYCLE, lifecycle_value(MemoryLifecycle::Active))
                .with_property(
                    P_RECORDED_AT,
                    PropertyValue::String(Utc::now().to_rfc3339()),
                )
                .with_property(
                    P_CORRELATION,
                    PropertyValue::String(context.correlation_id.clone()),
                );
                graph
                    .create_relationship(relation_input)
                    .map_err(map_graph_error)?;
            }
            Ok(MemoryResponse::Consolidate(ConsolidationResult {
                proposal_id: proposal_id.clone(),
                applied: true,
                canonical_id: Some(canonical),
                originals_retained: ids,
                disagreements_retained: true,
                receipt: Some(receipt(context, &proposal_id, max_version)),
            }))
        }
    }
}

fn trace_operation(
    graph: &Graph,
    recall_traces: &BTreeMap<String, RecallResult>,
    context: &MemoryServiceContext,
    input: &TraceRequest,
) -> Result<MemoryResponse, MemoryError> {
    let mut versions = Vec::new();
    let mut paths = Vec::new();
    let mut details = BTreeMap::new();
    match &input.target {
        MemoryTarget::Memory(id) => {
            let node_versions = visible_memory_versions(graph, &context.workspace_id, id)?;
            versions.extend(node_versions.iter().map(trace_node_version));
            let node_id = NodeId::new(id.clone()).map_err(map_graph_error)?;
            let mut relationships = graph.outgoing(&node_id).map_err(map_graph_error)?;
            relationships.extend(graph.incoming(&node_id).map_err(map_graph_error)?);
            for relationship in relationships {
                if belongs_to_relationship(&relationship, &context.workspace_id) {
                    paths.push(trace_path(&relationship)?);
                }
            }
            details.insert("record_kind".to_owned(), "memory".to_owned());
        }
        MemoryTarget::Relationship(id) => {
            let relationship_versions =
                visible_relationship_versions(graph, &context.workspace_id, id)?;
            versions.extend(relationship_versions.iter().map(trace_relationship_version));
            if let Some(current) = relationship_versions.last() {
                paths.push(trace_path(current)?);
            }
            details.insert("record_kind".to_owned(), "relationship".to_owned());
        }
        MemoryTarget::Recall(id) => {
            let recall = recall_traces.get(id).ok_or_else(not_found)?;
            for item in &recall.items {
                versions.extend(
                    visible_memory_versions(graph, &context.workspace_id, &item.record.id)?
                        .iter()
                        .map(trace_node_version),
                );
            }
            for relationship in &recall.relationships {
                paths.push(TracePath {
                    memory_ids: vec![
                        relationship.source_id.clone(),
                        relationship.target_id.clone(),
                    ],
                    relationship_ids: vec![relationship.id.clone()],
                    evidence_source_ids: relationship
                        .provenance
                        .iter()
                        .map(|source| source.source_id.clone())
                        .collect(),
                });
            }
            details.insert("objective_trace".to_owned(), id.clone());
            details.insert(
                "completeness".to_owned(),
                if recall.completeness.complete {
                    "complete"
                } else {
                    "truncated"
                }
                .to_owned(),
            );
        }
        MemoryTarget::Mutation(id) => {
            let receipt_node = graph
                .list_nodes()
                .map_err(map_graph_error)?
                .into_iter()
                .find(|node| {
                    node.has_label(RECEIPT_LABEL)
                        && belongs_to_node(node, &context.workspace_id)
                        && (string_property(node.properties(), P_CORRELATION) == Some(id)
                            || string_property(node.properties(), P_RECEIPT_KEY) == Some(id))
                })
                .ok_or_else(not_found)?;
            details.insert(
                "idempotency_key".to_owned(),
                string_property(receipt_node.properties(), P_RECEIPT_KEY)
                    .unwrap_or_default()
                    .to_owned(),
            );
        }
    }
    versions.sort_by(|left, right| {
        left.record_id
            .cmp(&right.record_id)
            .then(left.version.cmp(&right.version))
    });
    paths.sort_by(|left, right| left.relationship_ids.cmp(&right.relationship_ids));
    paths.dedup();
    Ok(MemoryResponse::Trace(TraceResult {
        target: input.target.clone(),
        versions,
        paths,
        actor_id: context.actor_id.clone(),
        agent_id: context.agent_id.clone(),
        session_id: context.session_id.clone(),
        policy_decisions: vec![
            "trusted_workspace_filter_applied".to_owned(),
            "trace_permission_allowed".to_owned(),
            "non_sensitive_audit_projection".to_owned(),
        ],
        details,
    }))
}

fn replay_response(
    graph: &Graph,
    workspace: &str,
    key: &str,
    request_hash: &str,
) -> Result<Option<MemoryResponse>, MemoryError> {
    let node = graph
        .list_nodes()
        .map_err(map_graph_error)?
        .into_iter()
        .find(|node| {
            node.has_label(RECEIPT_LABEL)
                && belongs_to_node(node, workspace)
                && string_property(node.properties(), P_RECEIPT_KEY) == Some(key)
        });
    let Some(node) = node else {
        return Ok(None);
    };
    if string_property(node.properties(), P_RECEIPT_HASH) != Some(request_hash) {
        return Err(MemoryError::new(
            MemoryErrorCode::IdempotencyConflict,
            "idempotency key was already committed with a different request",
            false,
        ));
    }
    let response = match node.property(P_RECEIPT_RESPONSE) {
        Some(PropertyValue::Json(value)) => serde_json::from_value(value.clone())
            .map_err(|_| internal("durable idempotency receipt is unreadable"))?,
        _ => {
            return Err(internal(
                "durable idempotency receipt is missing its response",
            ));
        }
    };
    Ok(Some(response))
}

fn store_receipt(
    graph: &mut Graph,
    context: &MemoryServiceContext,
    key: &str,
    request_hash: &str,
    response: &MemoryResponse,
) -> Result<(), MemoryError> {
    let input = NodeInput::new([RECEIPT_LABEL])
        .with_property(
            P_WORKSPACE,
            PropertyValue::String(context.workspace_id.clone()),
        )
        .with_property(P_RECEIPT_KEY, PropertyValue::String(key.to_owned()))
        .with_property(
            P_RECEIPT_HASH,
            PropertyValue::String(request_hash.to_owned()),
        )
        .with_property(P_RECEIPT_RESPONSE, json_value(response)?)
        .with_property(
            P_CORRELATION,
            PropertyValue::String(context.correlation_id.clone()),
        )
        .with_property(
            P_RECORDED_AT,
            PropertyValue::String(Utc::now().to_rfc3339()),
        );
    graph.create_node(input).map_err(map_graph_error)?;
    Ok(())
}

fn mark_replayed(mut response: MemoryResponse) -> MemoryResponse {
    let receipt = match &mut response {
        MemoryResponse::Remember { receipt, .. }
        | MemoryResponse::Relate { receipt, .. }
        | MemoryResponse::Update { receipt, .. }
        | MemoryResponse::UpdateRelationship { receipt, .. }
        | MemoryResponse::Forget { receipt, .. } => Some(receipt),
        MemoryResponse::Consolidate(result) => result.receipt.as_mut(),
        MemoryResponse::Recall(_) | MemoryResponse::Trace(_) => None,
    };
    if let Some(receipt) = receipt {
        receipt.replayed = true;
    }
    response
}

fn receipt(context: &MemoryServiceContext, id: &str, version: u64) -> MutationReceipt {
    MutationReceipt {
        committed_id: id.to_owned(),
        committed_version: version,
        audit_correlation_id: context.correlation_id.clone(),
        replayed: false,
    }
}

fn visible_memory_node(graph: &Graph, workspace: &str, id: &str) -> Result<Node, MemoryError> {
    let id = NodeId::new(id.to_owned()).map_err(|_| not_found())?;
    graph
        .get_node(&id)
        .map_err(map_graph_error)?
        .filter(|node| node.has_label(MEMORY_LABEL) && belongs_to_node(node, workspace))
        .ok_or_else(not_found)
}

fn visible_memory_relationship(
    graph: &Graph,
    workspace: &str,
    id: &str,
) -> Result<Relationship, MemoryError> {
    let id = RelationshipId::new(id.to_owned()).map_err(|_| not_found())?;
    graph
        .get_relationship(&id)
        .map_err(map_graph_error)?
        .filter(|relationship| belongs_to_relationship(relationship, workspace))
        .ok_or_else(not_found)
}

fn visible_memory_versions(
    graph: &Graph,
    workspace: &str,
    id: &str,
) -> Result<Vec<Node>, MemoryError> {
    let id = NodeId::new(id.to_owned()).map_err(|_| not_found())?;
    let mut versions = graph
        .list_node_versions(&id)
        .map_err(map_graph_error)?
        .into_iter()
        .filter(|node| node.has_label(MEMORY_LABEL) && belongs_to_node(node, workspace))
        .collect::<Vec<_>>();
    if versions.is_empty() {
        return Err(not_found());
    }
    versions.sort_by_key(Node::version);
    Ok(versions)
}

fn visible_relationship_versions(
    graph: &Graph,
    workspace: &str,
    id: &str,
) -> Result<Vec<Relationship>, MemoryError> {
    let id = RelationshipId::new(id.to_owned()).map_err(|_| not_found())?;
    let mut versions = graph
        .list_relationship_versions(&id)
        .map_err(map_graph_error)?
        .into_iter()
        .filter(|relationship| belongs_to_relationship(relationship, workspace))
        .collect::<Vec<_>>();
    if versions.is_empty() {
        return Err(not_found());
    }
    versions.sort_by_key(Relationship::version);
    Ok(versions)
}

fn memory_record(node: &Node) -> Result<MemoryRecord, MemoryError> {
    Ok(MemoryRecord {
        id: node.id().as_str().to_owned(),
        identity_key: string_property(node.properties(), P_IDENTITY).map(str::to_owned),
        kind: required_string_property(node.properties(), P_KIND)?,
        schema_version: required_string_property(node.properties(), P_SCHEMA)?,
        content: required_json_property(node.properties(), P_CONTENT)?,
        provenance: json_property(node.properties(), P_PROVENANCE).unwrap_or_default(),
        confidence: json_property(node.properties(), P_CONFIDENCE)
            .or_else(|| node.confidence().map(Confidence::value)),
        valid_from: string_property(node.properties(), P_VALID_FROM).map(str::to_owned),
        valid_until: string_property(node.properties(), P_VALID_UNTIL).map(str::to_owned),
        recorded_at: required_string_property(node.properties(), P_RECORDED_AT)?,
        expires_at: string_property(node.properties(), P_EXPIRES_AT).map(str::to_owned),
        lifecycle: lifecycle_property(node.properties(), node.status()),
        version: node.version(),
        tags: json_property(node.properties(), P_TAGS).unwrap_or_default(),
    })
}

fn memory_relationship(relationship: &Relationship) -> Result<MemoryRelationship, MemoryError> {
    Ok(MemoryRelationship {
        id: relationship.id().as_str().to_owned(),
        identity_key: string_property(relationship.properties(), P_REL_IDENTITY).map(str::to_owned),
        source_id: relationship.source().as_str().to_owned(),
        target_id: relationship.target().as_str().to_owned(),
        kind: required_string_property(relationship.properties(), P_REL_KIND)?,
        properties: match relationship.property(P_PROPERTIES) {
            Some(PropertyValue::Json(value)) => value.clone(),
            _ => serde_json::json!({}),
        },
        provenance: json_property(relationship.properties(), P_PROVENANCE).unwrap_or_default(),
        confidence: json_property(relationship.properties(), P_CONFIDENCE)
            .or_else(|| relationship.confidence().map(Confidence::value)),
        valid_from: string_property(relationship.properties(), P_VALID_FROM).map(str::to_owned),
        valid_until: string_property(relationship.properties(), P_VALID_UNTIL).map(str::to_owned),
        recorded_at: required_string_property(relationship.properties(), P_RECORDED_AT)?,
        expires_at: string_property(relationship.properties(), P_EXPIRES_AT).map(str::to_owned),
        lifecycle: lifecycle_property(relationship.properties(), relationship.status()),
        version: relationship.version(),
    })
}

fn trace_node_version(node: &Node) -> TraceVersion {
    TraceVersion {
        record_id: node.id().as_str().to_owned(),
        version: node.version(),
        lifecycle: lifecycle_property(node.properties(), node.status()),
        recorded_at: string_property(node.properties(), P_RECORDED_AT)
            .unwrap_or("unknown")
            .to_owned(),
    }
}

fn trace_relationship_version(relationship: &Relationship) -> TraceVersion {
    TraceVersion {
        record_id: relationship.id().as_str().to_owned(),
        version: relationship.version(),
        lifecycle: lifecycle_property(relationship.properties(), relationship.status()),
        recorded_at: string_property(relationship.properties(), P_RECORDED_AT)
            .unwrap_or("unknown")
            .to_owned(),
    }
}

fn trace_path(relationship: &Relationship) -> Result<TracePath, MemoryError> {
    Ok(TracePath {
        memory_ids: vec![
            relationship.source().as_str().to_owned(),
            relationship.target().as_str().to_owned(),
        ],
        relationship_ids: vec![relationship.id().as_str().to_owned()],
        evidence_source_ids: json_property::<Vec<ProvenanceReference>>(
            relationship.properties(),
            P_PROVENANCE,
        )
        .unwrap_or_default()
        .into_iter()
        .map(|source| source.source_id)
        .collect(),
    })
}

fn belongs_to_node(node: &Node, workspace: &str) -> bool {
    string_property(node.properties(), P_WORKSPACE) == Some(workspace)
}

fn belongs_to_relationship(relationship: &Relationship, workspace: &str) -> bool {
    string_property(relationship.properties(), P_WORKSPACE) == Some(workspace)
}

fn visible_at(properties: &std::collections::HashMap<String, PropertyValue>, now: &str) -> bool {
    let Ok(now) = DateTime::parse_from_rfc3339(now) else {
        return false;
    };
    lifecycle_property(properties, RecordStatus::Candidate) == MemoryLifecycle::Active
        && optional_timestamp_matches(properties, P_EXPIRES_AT, |value| value > now)
        && optional_timestamp_matches(properties, P_VALID_FROM, |value| value <= now)
        && optional_timestamp_matches(properties, P_VALID_UNTIL, |value| value >= now)
}

fn optional_timestamp_matches(
    properties: &std::collections::HashMap<String, PropertyValue>,
    key: &str,
    predicate: impl FnOnce(DateTime<FixedOffset>) -> bool,
) -> bool {
    string_property(properties, key)
        .is_none_or(|value| DateTime::parse_from_rfc3339(value).is_ok_and(predicate))
}

fn lifecycle_property(
    properties: &std::collections::HashMap<String, PropertyValue>,
    status: RecordStatus,
) -> MemoryLifecycle {
    if status == RecordStatus::Tombstoned {
        return MemoryLifecycle::Tombstoned;
    }
    match string_property(properties, P_LIFECYCLE) {
        Some("expired") => MemoryLifecycle::Expired,
        Some("superseded") => MemoryLifecycle::Superseded,
        Some("tombstoned") => MemoryLifecycle::Tombstoned,
        _ => MemoryLifecycle::Active,
    }
}

fn lifecycle_value(value: MemoryLifecycle) -> PropertyValue {
    PropertyValue::String(
        match value {
            MemoryLifecycle::Active => "active",
            MemoryLifecycle::Expired => "expired",
            MemoryLifecycle::Superseded => "superseded",
            MemoryLifecycle::Tombstoned => "tombstoned",
        }
        .to_owned(),
    )
}

fn string_property<'a>(
    properties: &'a std::collections::HashMap<String, PropertyValue>,
    key: &str,
) -> Option<&'a str> {
    match properties.get(key) {
        Some(PropertyValue::String(value)) => Some(value),
        _ => None,
    }
}

fn required_string_property(
    properties: &std::collections::HashMap<String, PropertyValue>,
    key: &str,
) -> Result<String, MemoryError> {
    string_property(properties, key)
        .map(str::to_owned)
        .ok_or_else(|| internal("memory record metadata is incomplete"))
}

fn json_property<T: for<'de> Deserialize<'de>>(
    properties: &std::collections::HashMap<String, PropertyValue>,
    key: &str,
) -> Option<T> {
    match properties.get(key) {
        Some(PropertyValue::Json(value)) => serde_json::from_value(value.clone()).ok(),
        _ => None,
    }
}

fn required_json_property<T: for<'de> Deserialize<'de>>(
    properties: &std::collections::HashMap<String, PropertyValue>,
    key: &str,
) -> Result<T, MemoryError> {
    json_property(properties, key)
        .ok_or_else(|| internal("memory record structured metadata is incomplete"))
}

fn json_value<T: Serialize>(value: &T) -> Result<PropertyValue, MemoryError> {
    serde_json::to_value(value)
        .map(PropertyValue::Json)
        .map_err(|_| internal("memory metadata could not be serialized"))
}

fn optional_json_value<T: Serialize>(value: Option<T>) -> PropertyValue {
    PropertyValue::Json(serde_json::to_value(value).unwrap_or(serde_json::Value::Null))
}

fn optional_string_value(value: Option<&str>) -> PropertyValue {
    value.map_or(PropertyValue::Null, |value| {
        PropertyValue::String(value.to_owned())
    })
}

fn map_confidence(value: f64) -> Result<Confidence, MemoryError> {
    Confidence::new(value).map_err(map_graph_error)
}

fn map_graph_error(error: graph_core::GraphError) -> MemoryError {
    MemoryError::new(
        MemoryErrorCode::Internal,
        format!("graph operation failed: {error}"),
        false,
    )
}

fn internal(message: impl Into<String>) -> MemoryError {
    MemoryError::new(MemoryErrorCode::Internal, message, false)
}

fn not_found() -> MemoryError {
    MemoryError::new(
        MemoryErrorCode::NotFound,
        "memory target was not found",
        false,
    )
}

fn append_unique<T: Clone + PartialEq>(target: &mut Vec<T>, values: &[T]) {
    for value in values {
        if !target.contains(value) {
            target.push(value.clone());
        }
    }
}

fn content_text(content: &MemoryContent) -> String {
    match content {
        MemoryContent::Text(text) | MemoryContent::TextAndProperties { text, .. } => text.clone(),
        MemoryContent::Properties(properties) => properties.to_string(),
    }
}

#[derive(Serialize, Deserialize)]
struct PageToken {
    workspace_hash: String,
    objective_hash: String,
    offset: usize,
}

fn encode_page_token(workspace: &str, objective: &str, offset: usize) -> String {
    let token = PageToken {
        workspace_hash: hex_hash(workspace.as_bytes()),
        objective_hash: hex_hash(objective.as_bytes()),
        offset,
    };
    URL_SAFE_NO_PAD.encode(serde_json::to_vec(&token).unwrap_or_default())
}

fn decode_page_token(
    token: Option<&str>,
    workspace: &str,
    objective: &str,
) -> Result<usize, MemoryError> {
    let Some(token) = token else {
        return Ok(0);
    };
    let bytes = URL_SAFE_NO_PAD
        .decode(token)
        .map_err(|_| invalid("page_token is invalid"))?;
    let token: PageToken =
        serde_json::from_slice(&bytes).map_err(|_| invalid("page_token is invalid"))?;
    if token.workspace_hash != hex_hash(workspace.as_bytes())
        || token.objective_hash != hex_hash(objective.as_bytes())
    {
        return Err(not_found());
    }
    Ok(token.offset)
}

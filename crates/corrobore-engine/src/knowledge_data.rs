// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! Backend-neutral Knowledge Data Engine contract.
//!
//! This module is the shared semantic boundary for embedded providers and
//! serialized remote calls. Product adapters translate their domain model into
//! these types; transports only serialize the same envelopes. Provider
//! implementations remain responsible for capability negotiation and must
//! reject unsupported behavior explicitly.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    time::{SystemTime, UNIX_EPOCH},
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use graph_core::{Node, NodeId};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::CorroboreEngine;

/// Current stable contract version.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ContractVersion {
    /// Breaking-change generation.
    pub major: u16,
    /// Backward-compatible feature generation.
    pub minor: u16,
}

impl ContractVersion {
    /// Contract implemented by this crate.
    pub const CURRENT: Self = Self::new(1, 0);

    /// Builds a contract version.
    pub const fn new(major: u16, minor: u16) -> Self {
        Self { major, minor }
    }

    /// Returns whether this provider version can serve the requested client.
    pub const fn accepts(self, requested: Self) -> bool {
        self.major == requested.major && requested.minor <= self.minor
    }
}

/// Consistency requested by one logical operation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsistencyLevel {
    /// Results may lag a committed write.
    Eventual,
    /// The caller observes its prior successful writes.
    #[default]
    ReadYourWrites,
    /// Pagination and multi-read operations share a stable snapshot.
    Snapshot,
}

/// Authorization facts propagated to a provider without transport metadata.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessContext {
    /// Stable caller identity.
    pub subject_id: String,
    /// Organizations visible to the caller.
    pub organization_ids: Vec<String>,
    /// Markings visible to the caller.
    pub marking_ids: Vec<String>,
    /// Optional tenant boundary.
    pub tenant_id: Option<String>,
    /// Provider-neutral role names.
    pub roles: Vec<String>,
    /// Extension attributes negotiated by an adapter.
    pub attributes: BTreeMap<String, String>,
}

/// Cross-cutting request context shared by embedded and remote execution.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestContext {
    /// Unique request identity.
    pub request_id: String,
    /// Correlation identity preserved in the response.
    pub correlation_id: String,
    /// Optional replay-safe mutation key.
    pub idempotency_key: Option<String>,
    /// Absolute Unix epoch deadline in milliseconds.
    pub deadline_unix_ms: Option<u64>,
    /// Provider-neutral cancellation registration identity.
    pub cancellation_id: Option<String>,
    /// Authorization context.
    pub access: AccessContext,
    /// Required consistency.
    pub consistency: ConsistencyLevel,
}

/// Every semantic operation in version 1 of the contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    /// Initialize and negotiate a provider.
    Initialize,
    /// Inspect provider health.
    Health,
    /// Apply an ordered schema migration.
    Migrate,
    /// Fetch one record by stable identifier.
    GetById,
    /// List records using bounded typed filters.
    List,
    /// Continue a snapshot-bound record list.
    Paginate,
    /// Execute structured or full-text search.
    Search,
    /// Count matching records.
    Count,
    /// Compute supported aggregations.
    Aggregate,
    /// Read direct graph neighbors.
    Neighbors,
    /// Traverse a bounded graph path.
    Traverse,
    /// Return a bounded subgraph.
    Subgraph,
    /// Create one record.
    Create,
    /// Update one record.
    Update,
    /// Delete one record.
    Delete,
    /// Execute an ordered bulk mutation.
    Bulk,
    /// Reconcile records into a survivor.
    Merge,
    /// Create a portable snapshot.
    Snapshot,
    /// Restore a portable snapshot.
    Restore,
    /// Rebuild provider-owned indexes.
    RebuildIndexes,
}

impl OperationKind {
    /// Ordered list used by capability negotiation and conformance checks.
    pub const ALL: [Self; 20] = [
        Self::Initialize,
        Self::Health,
        Self::Migrate,
        Self::GetById,
        Self::List,
        Self::Paginate,
        Self::Search,
        Self::Count,
        Self::Aggregate,
        Self::Neighbors,
        Self::Traverse,
        Self::Subgraph,
        Self::Create,
        Self::Update,
        Self::Delete,
        Self::Bulk,
        Self::Merge,
        Self::Snapshot,
        Self::Restore,
        Self::RebuildIndexes,
    ];
}

impl fmt::Display for OperationKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = serde_json::to_value(self)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_else(|| "unknown".to_owned());
        formatter.write_str(&value)
    }
}

/// Provider support declaration for one operation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ProviderCapabilityStatus {
    /// Operation is implemented with the declared contract semantics.
    Supported,
    /// Operation is recognized but deliberately unavailable.
    Unsupported {
        /// Durable explanation, normally pointing to its delivery issue.
        reason: String,
    },
}

impl ProviderCapabilityStatus {
    /// Returns whether dispatch is allowed.
    pub const fn is_supported(&self) -> bool {
        matches!(self, Self::Supported)
    }
}

/// Versioned provider capability declaration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCapability {
    /// Typed operation.
    pub operation: OperationKind,
    /// Support status.
    pub status: ProviderCapabilityStatus,
    /// Contract version that introduced the declaration.
    pub since: ContractVersion,
    /// Optional version after which callers must migrate.
    pub deprecated_after: Option<ContractVersion>,
}

impl ProviderCapability {
    /// Returns whether a caller version may rely on this capability.
    pub fn is_available_to(&self, version: ContractVersion) -> bool {
        self.status.is_supported()
            && version.major == self.since.major
            && version >= self.since
            && self
                .deprecated_after
                .is_none_or(|deprecated_after| version < deprecated_after)
    }
}

/// Configuration-only provider route used by application adapters.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "provider", rename_all = "snake_case")]
pub enum ProviderRouteConfig {
    /// Execute against the in-process Corrobore provider.
    EmbeddedCorrobore,
    /// Execute the same contract against a configured reference provider.
    RemoteReference {
        /// Remote contract endpoint selected by deployment configuration.
        endpoint: String,
    },
}

/// Initialization and capability negotiation input.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitializeRequest {
    /// Contract version requested by the caller.
    pub client_contract_version: ContractVersion,
    /// Capabilities without which the caller cannot start.
    pub required_capabilities: Vec<OperationKind>,
}

impl Default for ContractVersion {
    fn default() -> Self {
        Self::CURRENT
    }
}

/// Health inspection input.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthRequest {
    /// Include provider detail safe for system operators.
    pub verbose: bool,
}

/// Migration input.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MigrateRequest {
    /// Ordered migration identifier.
    pub migration_id: String,
    /// Target schema version.
    pub target_schema_version: String,
    /// Backend-neutral migration document.
    pub plan: Value,
}

/// Point-read input.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetByIdRequest {
    /// Stable record identifier.
    pub id: String,
}

/// Bounded list input.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListRequest {
    /// Record kinds to include; empty means all kinds.
    pub kinds: Vec<String>,
    /// Maximum records returned before pagination.
    pub limit: u32,
}

impl Default for ListRequest {
    fn default() -> Self {
        Self {
            kinds: Vec::new(),
            limit: 100,
        }
    }
}

/// Snapshot-consistent pagination input.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaginateRequest {
    /// Original normalized list query.
    pub query: ListRequest,
    /// Bounded page size.
    pub page_size: u32,
    /// Opaque continuation token.
    pub token: Option<String>,
}

impl Default for PaginateRequest {
    fn default() -> Self {
        Self {
            query: ListRequest::default(),
            page_size: 100,
            token: None,
        }
    }
}

/// Structured search input.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SearchRequest {
    /// Search expression interpreted by a negotiated search capability.
    pub expression: Value,
    /// Maximum number of records.
    pub limit: u32,
}

/// Count input.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CountRequest {
    /// Backend-neutral filter expression.
    pub filter: Value,
}

/// Aggregation input.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AggregateRequest {
    /// Backend-neutral aggregation plan.
    pub plan: Value,
}

/// Neighbor input.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NeighborsRequest {
    /// Stable record identifier.
    pub id: String,
    /// Whether incoming relationships are included.
    pub incoming: bool,
    /// Whether outgoing relationships are included.
    pub outgoing: bool,
}

/// Bounded traversal input.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TraverseRequest {
    /// Starting record identifiers.
    pub start_ids: Vec<String>,
    /// Maximum traversal depth.
    pub max_depth: u32,
    /// Backend-neutral traversal constraints.
    pub constraints: Value,
}

/// Subgraph input.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SubgraphRequest {
    /// Stable record identifiers to project.
    pub ids: Vec<String>,
    /// Backend-neutral projection constraints.
    pub projection: Value,
}

/// Single-record create input.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CreateRequest {
    /// Provider-neutral record.
    pub record: Value,
}

/// Single-record update input.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UpdateRequest {
    /// Stable record identifier.
    pub id: String,
    /// Expected revision for optimistic concurrency.
    pub expected_revision: Option<u64>,
    /// Provider-neutral patch.
    pub patch: Value,
}

/// Single-record delete input.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteRequest {
    /// Stable record identifier.
    pub id: String,
    /// Expected revision for optimistic concurrency.
    pub expected_revision: Option<u64>,
}

/// Ordered bulk mutation input.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BulkRequest {
    /// Ordered mutation documents.
    pub operations: Vec<Value>,
    /// Whether all operations must commit atomically.
    pub atomic: bool,
}

/// Record merge input.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeRequest {
    /// Survivor record.
    pub target_id: String,
    /// Records reconciled into the survivor.
    pub source_ids: Vec<String>,
}

/// Portable snapshot input.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotRequest {
    /// Caller-defined snapshot label.
    pub label: String,
}

/// Portable restore input.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestoreRequest {
    /// Portable artifact location or identifier.
    pub artifact_id: String,
}

/// Index rebuild input.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebuildIndexesRequest {
    /// Logical indexes to rebuild; empty means all provider-owned indexes.
    pub indexes: Vec<String>,
}

/// Typed operation payload.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "operation", content = "request", rename_all = "snake_case")]
pub enum KnowledgeDataOperation {
    /// Initialize.
    Initialize(InitializeRequest),
    /// Health.
    Health(HealthRequest),
    /// Migrate.
    Migrate(MigrateRequest),
    /// Get by ID.
    GetById(GetByIdRequest),
    /// List.
    List(ListRequest),
    /// Paginate.
    Paginate(PaginateRequest),
    /// Search.
    Search(SearchRequest),
    /// Count.
    Count(CountRequest),
    /// Aggregate.
    Aggregate(AggregateRequest),
    /// Neighbors.
    Neighbors(NeighborsRequest),
    /// Traverse.
    Traverse(TraverseRequest),
    /// Subgraph.
    Subgraph(SubgraphRequest),
    /// Create.
    Create(CreateRequest),
    /// Update.
    Update(UpdateRequest),
    /// Delete.
    Delete(DeleteRequest),
    /// Bulk.
    Bulk(BulkRequest),
    /// Merge.
    Merge(MergeRequest),
    /// Snapshot.
    Snapshot(SnapshotRequest),
    /// Restore.
    Restore(RestoreRequest),
    /// Rebuild indexes.
    RebuildIndexes(RebuildIndexesRequest),
}

impl KnowledgeDataOperation {
    /// Returns the capability required to dispatch the payload.
    pub const fn kind(&self) -> OperationKind {
        match self {
            Self::Initialize(_) => OperationKind::Initialize,
            Self::Health(_) => OperationKind::Health,
            Self::Migrate(_) => OperationKind::Migrate,
            Self::GetById(_) => OperationKind::GetById,
            Self::List(_) => OperationKind::List,
            Self::Paginate(_) => OperationKind::Paginate,
            Self::Search(_) => OperationKind::Search,
            Self::Count(_) => OperationKind::Count,
            Self::Aggregate(_) => OperationKind::Aggregate,
            Self::Neighbors(_) => OperationKind::Neighbors,
            Self::Traverse(_) => OperationKind::Traverse,
            Self::Subgraph(_) => OperationKind::Subgraph,
            Self::Create(_) => OperationKind::Create,
            Self::Update(_) => OperationKind::Update,
            Self::Delete(_) => OperationKind::Delete,
            Self::Bulk(_) => OperationKind::Bulk,
            Self::Merge(_) => OperationKind::Merge,
            Self::Snapshot(_) => OperationKind::Snapshot,
            Self::Restore(_) => OperationKind::Restore,
            Self::RebuildIndexes(_) => OperationKind::RebuildIndexes,
        }
    }
}

/// Versioned provider request envelope.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KnowledgeDataRequest {
    /// Wire and semantic contract version.
    pub contract_version: ContractVersion,
    /// Cross-cutting execution context.
    pub context: RequestContext,
    /// Typed operation.
    pub operation: KnowledgeDataOperation,
}

/// Stable provider error categories.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum KnowledgeDataErrorCode {
    /// Malformed or incomplete request.
    #[serde(rename = "INVALID_REQUEST")]
    InvalidRequest,
    /// Contract major/minor cannot be served.
    #[serde(rename = "INCOMPATIBLE_CONTRACT_VERSION")]
    IncompatibleContractVersion,
    /// Provider recognizes but does not implement the operation.
    #[serde(rename = "UNSUPPORTED_CAPABILITY")]
    UnsupportedCapability,
    /// Requested record does not exist.
    #[serde(rename = "NOT_FOUND")]
    NotFound,
    /// Optimistic concurrency or idempotency conflict.
    #[serde(rename = "CONFLICT")]
    Conflict,
    /// Access context does not authorize the operation.
    #[serde(rename = "UNAUTHORIZED")]
    Unauthorized,
    /// Deadline elapsed before completion.
    #[serde(rename = "DEADLINE_EXCEEDED")]
    DeadlineExceeded,
    /// Cancellation was requested.
    #[serde(rename = "CANCELLED")]
    Cancelled,
    /// Pagination token has invalid syntax or integrity.
    #[serde(rename = "INVALID_PAGINATION_TOKEN")]
    InvalidPaginationToken,
    /// Pagination token belongs to another query or schema.
    #[serde(rename = "INCOMPATIBLE_PAGINATION_TOKEN")]
    IncompatiblePaginationToken,
    /// Provider is unavailable.
    #[serde(rename = "BACKEND_UNAVAILABLE")]
    BackendUnavailable,
    /// Provider schema cannot serve the requested contract.
    #[serde(rename = "SCHEMA_INCOMPATIBLE")]
    SchemaIncompatible,
    /// Provider invariant failed.
    #[serde(rename = "INTERNAL")]
    Internal,
}

impl KnowledgeDataErrorCode {
    /// Stable string carried by embedded and remote errors.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidRequest => "INVALID_REQUEST",
            Self::IncompatibleContractVersion => "INCOMPATIBLE_CONTRACT_VERSION",
            Self::UnsupportedCapability => "UNSUPPORTED_CAPABILITY",
            Self::NotFound => "NOT_FOUND",
            Self::Conflict => "CONFLICT",
            Self::Unauthorized => "UNAUTHORIZED",
            Self::DeadlineExceeded => "DEADLINE_EXCEEDED",
            Self::Cancelled => "CANCELLED",
            Self::InvalidPaginationToken => "INVALID_PAGINATION_TOKEN",
            Self::IncompatiblePaginationToken => "INCOMPATIBLE_PAGINATION_TOKEN",
            Self::BackendUnavailable => "BACKEND_UNAVAILABLE",
            Self::SchemaIncompatible => "SCHEMA_INCOMPATIBLE",
            Self::Internal => "INTERNAL",
        }
    }
}

/// Stable, serializable provider failure.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeDataError {
    /// Machine-readable error category.
    pub code: KnowledgeDataErrorCode,
    /// Safe diagnostic message.
    pub message: String,
    /// Whether retrying unchanged input may succeed.
    pub retryable: bool,
}

impl KnowledgeDataError {
    fn new(code: KnowledgeDataErrorCode, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code,
            message: message.into(),
            retryable,
        }
    }

    fn unsupported(operation: OperationKind) -> Self {
        Self::new(
            KnowledgeDataErrorCode::UnsupportedCapability,
            format!("{operation} is not implemented by this provider"),
            false,
        )
    }
}

/// Readiness details returned by initialization and health.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderReadiness {
    /// Whether new operations are accepted.
    pub accepting_requests: bool,
    /// Optional blockers.
    pub blockers: Vec<String>,
}

/// Recovery details returned by initialization and health.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRecovery {
    /// Stable recovery state.
    pub state: String,
    /// Optional recovery detail.
    pub detail: Option<String>,
}

/// Successful initialization response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderInitialization {
    /// Negotiated contract.
    pub contract_version: ContractVersion,
    /// Provider implementation version.
    pub engine_version: String,
    /// Provider schema version.
    pub schema_version: String,
    /// Full supported and unsupported surface.
    pub capabilities: Vec<ProviderCapability>,
    /// Readiness.
    pub readiness: ProviderReadiness,
    /// Recovery.
    pub recovery: ProviderRecovery,
}

/// Provider health response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderHealth {
    /// Readiness.
    pub readiness: ProviderReadiness,
    /// Recovery.
    pub recovery: ProviderRecovery,
    /// Count of logical records when requested.
    pub record_count: Option<u64>,
}

/// Provider-neutral record projection.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KnowledgeRecord {
    /// Stable identifier.
    pub id: String,
    /// Primary logical kind.
    pub kind: String,
    /// Current revision.
    pub revision: u64,
    /// Provider-neutral body.
    pub body: Value,
}

/// Ordered record page.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecordPage {
    /// Stable record ordering.
    pub records: Vec<KnowledgeRecord>,
    /// Opaque continuation token.
    pub next_token: Option<String>,
}

/// Count response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CountResult {
    /// Matching record count.
    pub count: u64,
}

/// Aggregation response.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AggregationResult {
    /// Ordered aggregation buckets.
    pub buckets: Vec<Value>,
}

/// Graph response.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GraphResult {
    /// Projected nodes.
    pub records: Vec<KnowledgeRecord>,
    /// Projected relationships.
    pub relationships: Vec<Value>,
}

/// Write response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteResult {
    /// Stable record identifier.
    pub id: String,
    /// New revision.
    pub revision: u64,
}

/// Bulk response.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BulkResult {
    /// Ordered per-operation results.
    pub results: Vec<Value>,
}

/// Snapshot response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotResult {
    /// Portable snapshot artifact identity.
    pub artifact_id: String,
    /// Provider schema recorded in the artifact.
    pub schema_version: String,
}

/// Generic acknowledgement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationAcknowledgement {
    /// Whether the provider accepted the operation.
    pub acknowledged: bool,
}

/// Typed successful response.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "response", content = "data", rename_all = "snake_case")]
pub enum KnowledgeDataResponse {
    /// Initialization.
    Initialized(ProviderInitialization),
    /// Health.
    Health(ProviderHealth),
    /// Point read.
    Record(Option<KnowledgeRecord>),
    /// Record list.
    Records(RecordPage),
    /// Count.
    Count(CountResult),
    /// Aggregation.
    Aggregation(AggregationResult),
    /// Graph projection.
    Graph(GraphResult),
    /// Single write.
    Write(WriteResult),
    /// Bulk write.
    Bulk(BulkResult),
    /// Snapshot.
    Snapshot(SnapshotResult),
    /// Generic acknowledgement.
    Acknowledged(OperationAcknowledgement),
}

/// Success or stable failure in a response envelope.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum KnowledgeDataOutcome {
    /// Successful provider response.
    Success {
        /// Typed response.
        response: KnowledgeDataResponse,
    },
    /// Stable provider failure.
    Failure {
        /// Typed error.
        error: KnowledgeDataError,
    },
}

impl KnowledgeDataOutcome {
    /// Returns the error when the outcome failed.
    pub const fn error(&self) -> Option<&KnowledgeDataError> {
        match self {
            Self::Failure { error } => Some(error),
            Self::Success { .. } => None,
        }
    }
}

/// Versioned remote/embedded response envelope.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KnowledgeDataResponseEnvelope {
    /// Contract used for the response.
    pub contract_version: ContractVersion,
    /// Correlation identity copied from the request.
    pub correlation_id: String,
    /// Provider outcome.
    pub outcome: KnowledgeDataOutcome,
}

/// Expected outcome for a reusable conformance case.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExpectedConformanceOutcome {
    /// The provider must return a successful typed response.
    Success,
    /// The provider must return this stable error category.
    Error(KnowledgeDataErrorCode),
}

/// One reusable provider conformance case.
#[derive(Clone, Debug)]
pub struct ConformanceCase {
    /// Stable case name.
    pub name: String,
    /// Typed request executed through an embedded or remote endpoint.
    pub request: KnowledgeDataRequest,
    /// Expected stable outcome.
    pub expected: ExpectedConformanceOutcome,
}

/// One failed conformance assertion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConformanceFailure {
    /// Stable case name.
    pub name: String,
    /// Human-readable mismatch.
    pub message: String,
}

/// Result of a reusable provider conformance run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConformanceReport {
    /// Number of cases executed.
    pub total: usize,
    /// Mismatches.
    pub failures: Vec<ConformanceFailure>,
}

impl ConformanceReport {
    /// Returns whether every case matched its stable outcome.
    pub fn is_conformant(&self) -> bool {
        self.failures.is_empty()
    }
}

/// Runs the same typed cases against any embedded or remote execution closure.
pub fn run_conformance_cases(
    cases: &[ConformanceCase],
    mut execute: impl FnMut(KnowledgeDataRequest) -> KnowledgeDataResponseEnvelope,
) -> ConformanceReport {
    let failures = cases
        .iter()
        .filter_map(|case| {
            let response = execute(case.request.clone());
            let matches = match (&case.expected, &response.outcome) {
                (ExpectedConformanceOutcome::Success, KnowledgeDataOutcome::Success { .. }) => true,
                (
                    ExpectedConformanceOutcome::Error(expected),
                    KnowledgeDataOutcome::Failure { error },
                ) => expected == &error.code,
                _ => false,
            };
            (!matches).then(|| ConformanceFailure {
                name: case.name.clone(),
                message: format!(
                    "expected {:?}, received {:?}",
                    case.expected, response.outcome
                ),
            })
        })
        .collect();
    ConformanceReport {
        total: cases.len(),
        failures,
    }
}

/// Claims protected by one opaque pagination token.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaginationTokenClaims {
    /// Token format version.
    pub version: u8,
    /// Canonical query fingerprint.
    pub query_fingerprint: String,
    /// Schema version at issuance time.
    pub schema_version: String,
    /// Last stable record cursor.
    pub cursor: String,
}

/// Issues and verifies opaque pagination tokens.
#[derive(Clone, Debug)]
pub struct PaginationTokenCodec {
    key: Vec<u8>,
}

impl PaginationTokenCodec {
    /// Builds a codec with a minimum 256-bit integrity key.
    pub fn new(key: &[u8]) -> Result<Self, KnowledgeDataError> {
        if key.len() < 32 {
            return Err(KnowledgeDataError::new(
                KnowledgeDataErrorCode::InvalidRequest,
                "pagination token key must contain at least 32 bytes",
                false,
            ));
        }
        Ok(Self { key: key.to_vec() })
    }

    /// Issues a canonical token protected by HMAC-SHA-256.
    pub fn issue(&self, claims: &PaginationTokenClaims) -> Result<String, KnowledgeDataError> {
        let payload = serde_json::to_vec(claims).map_err(|error| {
            KnowledgeDataError::new(
                KnowledgeDataErrorCode::Internal,
                format!("failed to serialize pagination claims: {error}"),
                false,
            )
        })?;
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.key).map_err(|_| {
            KnowledgeDataError::new(
                KnowledgeDataErrorCode::Internal,
                "failed to initialize pagination integrity",
                false,
            )
        })?;
        mac.update(&payload);
        let tag = mac.finalize().into_bytes();
        Ok(format!(
            "kde1.{}.{}",
            URL_SAFE_NO_PAD.encode(payload),
            URL_SAFE_NO_PAD.encode(tag)
        ))
    }

    /// Verifies integrity and query/schema compatibility.
    pub fn verify(
        &self,
        token: &str,
        query_fingerprint: &str,
        schema_version: &str,
    ) -> Result<PaginationTokenClaims, KnowledgeDataError> {
        let mut parts = token.split('.');
        if parts.next() != Some("kde1") {
            return Err(invalid_pagination_token());
        }
        let Some(payload_part) = parts.next() else {
            return Err(invalid_pagination_token());
        };
        let Some(tag_part) = parts.next() else {
            return Err(invalid_pagination_token());
        };
        if parts.next().is_some() {
            return Err(invalid_pagination_token());
        }
        let payload = URL_SAFE_NO_PAD
            .decode(payload_part)
            .map_err(|_| invalid_pagination_token())?;
        let tag = URL_SAFE_NO_PAD
            .decode(tag_part)
            .map_err(|_| invalid_pagination_token())?;
        let mut mac =
            Hmac::<Sha256>::new_from_slice(&self.key).map_err(|_| invalid_pagination_token())?;
        mac.update(&payload);
        mac.verify_slice(&tag)
            .map_err(|_| invalid_pagination_token())?;
        let claims: PaginationTokenClaims =
            serde_json::from_slice(&payload).map_err(|_| invalid_pagination_token())?;
        if claims.version != 1
            || claims.query_fingerprint != query_fingerprint
            || claims.schema_version != schema_version
        {
            return Err(KnowledgeDataError::new(
                KnowledgeDataErrorCode::IncompatiblePaginationToken,
                "pagination token is incompatible with the query, schema, or token version",
                false,
            ));
        }
        Ok(claims)
    }
}

fn invalid_pagination_token() -> KnowledgeDataError {
    KnowledgeDataError::new(
        KnowledgeDataErrorCode::InvalidPaginationToken,
        "pagination token syntax or integrity is invalid",
        false,
    )
}

/// Provider semantic interface.
pub trait KnowledgeDataEngine {
    /// Declares supported and unsupported operations.
    fn capabilities(&self) -> Vec<ProviderCapability>;

    /// Executes one typed operation.
    fn execute_operation(
        &mut self,
        operation: KnowledgeDataOperation,
        context: &RequestContext,
    ) -> Result<KnowledgeDataResponse, KnowledgeDataError>;

    /// Returns whether the identified operation was cancelled.
    fn is_cancelled(&self, _cancellation_id: &str) -> bool {
        false
    }
}

/// Corrobore provider that borrows the existing embedded engine facade.
pub struct CorroboreKnowledgeDataProvider<'a> {
    engine: &'a mut CorroboreEngine,
    pagination: PaginationTokenCodec,
    cancelled: BTreeSet<String>,
}

impl<'a> CorroboreKnowledgeDataProvider<'a> {
    /// Wraps an embedded engine without duplicating its graph state.
    pub fn new(
        engine: &'a mut CorroboreEngine,
        pagination_key: &[u8],
    ) -> Result<Self, KnowledgeDataError> {
        Ok(Self {
            engine,
            pagination: PaginationTokenCodec::new(pagination_key)?,
            cancelled: BTreeSet::new(),
        })
    }

    /// Registers cancellation before dispatch.
    pub fn cancel(&mut self, cancellation_id: impl Into<String>) {
        self.cancelled.insert(cancellation_id.into());
    }

    /// Validates the envelope and dispatches through the semantic provider.
    pub fn execute(&mut self, request: KnowledgeDataRequest) -> KnowledgeDataResponseEnvelope {
        execute_contract(self, request)
    }
}

impl KnowledgeDataEngine for CorroboreKnowledgeDataProvider<'_> {
    fn capabilities(&self) -> Vec<ProviderCapability> {
        OperationKind::ALL
            .into_iter()
            .map(|operation| ProviderCapability {
                operation,
                status: capability_status(operation),
                since: ContractVersion::CURRENT,
                deprecated_after: None,
            })
            .collect()
    }

    fn execute_operation(
        &mut self,
        operation: KnowledgeDataOperation,
        context: &RequestContext,
    ) -> Result<KnowledgeDataResponse, KnowledgeDataError> {
        match operation {
            KnowledgeDataOperation::Initialize(request) => self.initialize(request),
            KnowledgeDataOperation::Health(request) => self.health(request),
            KnowledgeDataOperation::GetById(request) => self.get_by_id(request),
            KnowledgeDataOperation::List(request) => self.list(request),
            KnowledgeDataOperation::Paginate(request) => self.paginate(request),
            KnowledgeDataOperation::Count(request) => self.count(request),
            KnowledgeDataOperation::Neighbors(request) => self.neighbors(request),
            unsupported => {
                let operation = unsupported.kind();
                let _ = context;
                Err(KnowledgeDataError::unsupported(operation))
            }
        }
    }

    fn is_cancelled(&self, cancellation_id: &str) -> bool {
        self.cancelled.contains(cancellation_id)
    }
}

/// Executes the exact same contract through canonical JSON bytes.
pub fn execute_remote_contract(
    provider: &mut impl KnowledgeDataEngine,
    request_json: &[u8],
) -> Vec<u8> {
    let request = serde_json::from_slice::<KnowledgeDataRequest>(request_json);
    let response = match request {
        Ok(request) => execute_contract(provider, request),
        Err(error) => KnowledgeDataResponseEnvelope {
            contract_version: ContractVersion::CURRENT,
            correlation_id: String::new(),
            outcome: KnowledgeDataOutcome::Failure {
                error: KnowledgeDataError::new(
                    KnowledgeDataErrorCode::InvalidRequest,
                    error.to_string(),
                    false,
                ),
            },
        },
    };
    serde_json::to_vec(&response).unwrap_or_default()
}

const CORROBORE_SCHEMA_VERSION: &str = "corrobore-graph-v1";

fn capability_status(operation: OperationKind) -> ProviderCapabilityStatus {
    match operation {
        OperationKind::Initialize
        | OperationKind::Health
        | OperationKind::GetById
        | OperationKind::List
        | OperationKind::Paginate
        | OperationKind::Count
        | OperationKind::Neighbors => ProviderCapabilityStatus::Supported,
        OperationKind::Migrate | OperationKind::Snapshot | OperationKind::Restore => {
            unsupported_status("portable lifecycle support is delivered by issue #52")
        }
        OperationKind::Search => unsupported_status(
            "structured and full-text search are delivered by issues #46 and #47",
        ),
        OperationKind::Aggregate | OperationKind::Traverse | OperationKind::Subgraph => {
            unsupported_status("advanced read planning is delivered by issue #47")
        }
        OperationKind::Create
        | OperationKind::Update
        | OperationKind::Delete
        | OperationKind::Bulk => {
            unsupported_status("typed transactional writes are delivered by issue #50")
        }
        OperationKind::Merge => {
            unsupported_status("merge and reconciliation are delivered by issue #51")
        }
        OperationKind::RebuildIndexes => {
            unsupported_status("index maintenance is delivered by issue #52")
        }
    }
}

fn unsupported_status(reason: &str) -> ProviderCapabilityStatus {
    ProviderCapabilityStatus::Unsupported {
        reason: reason.to_owned(),
    }
}

fn execute_contract(
    provider: &mut impl KnowledgeDataEngine,
    request: KnowledgeDataRequest,
) -> KnowledgeDataResponseEnvelope {
    let correlation_id = request.context.correlation_id.clone();
    let outcome = validate_request(provider, &request)
        .and_then(|()| provider.execute_operation(request.operation, &request.context));
    KnowledgeDataResponseEnvelope {
        contract_version: ContractVersion::CURRENT,
        correlation_id,
        outcome: match outcome {
            Ok(response) => KnowledgeDataOutcome::Success { response },
            Err(error) => KnowledgeDataOutcome::Failure { error },
        },
    }
}

fn validate_request(
    provider: &impl KnowledgeDataEngine,
    request: &KnowledgeDataRequest,
) -> Result<(), KnowledgeDataError> {
    if !ContractVersion::CURRENT.accepts(request.contract_version) {
        return Err(KnowledgeDataError::new(
            KnowledgeDataErrorCode::IncompatibleContractVersion,
            format!(
                "provider contract {}.{} cannot serve client contract {}.{}",
                ContractVersion::CURRENT.major,
                ContractVersion::CURRENT.minor,
                request.contract_version.major,
                request.contract_version.minor
            ),
            false,
        ));
    }
    for (field, value) in [
        ("request_id", request.context.request_id.as_str()),
        ("correlation_id", request.context.correlation_id.as_str()),
        (
            "access.subject_id",
            request.context.access.subject_id.as_str(),
        ),
    ] {
        if value.trim().is_empty() {
            return Err(KnowledgeDataError::new(
                KnowledgeDataErrorCode::InvalidRequest,
                format!("{field} must not be empty"),
                false,
            ));
        }
    }
    if request
        .context
        .deadline_unix_ms
        .is_some_and(|deadline| deadline <= now_unix_ms())
    {
        return Err(KnowledgeDataError::new(
            KnowledgeDataErrorCode::DeadlineExceeded,
            "request deadline elapsed before provider dispatch",
            true,
        ));
    }
    if request
        .context
        .cancellation_id
        .as_deref()
        .is_some_and(|id| provider.is_cancelled(id))
    {
        return Err(KnowledgeDataError::new(
            KnowledgeDataErrorCode::Cancelled,
            "request was cancelled before provider dispatch",
            false,
        ));
    }
    Ok(())
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

impl CorroboreKnowledgeDataProvider<'_> {
    fn initialize(
        &self,
        request: InitializeRequest,
    ) -> Result<KnowledgeDataResponse, KnowledgeDataError> {
        if !ContractVersion::CURRENT.accepts(request.client_contract_version) {
            return Err(KnowledgeDataError::new(
                KnowledgeDataErrorCode::IncompatibleContractVersion,
                "requested client contract cannot be negotiated",
                false,
            ));
        }
        let capabilities = self.capabilities();
        for required in request.required_capabilities {
            let supported = capabilities.iter().any(|capability| {
                capability.operation == required && capability.status.is_supported()
            });
            if !supported {
                return Err(KnowledgeDataError::new(
                    KnowledgeDataErrorCode::UnsupportedCapability,
                    format!("required capability {required} is unsupported"),
                    false,
                ));
            }
        }
        Ok(KnowledgeDataResponse::Initialized(ProviderInitialization {
            contract_version: ContractVersion::CURRENT,
            engine_version: env!("CARGO_PKG_VERSION").to_owned(),
            schema_version: CORROBORE_SCHEMA_VERSION.to_owned(),
            capabilities,
            readiness: ready(),
            recovery: recovered(),
        }))
    }

    fn health(&self, request: HealthRequest) -> Result<KnowledgeDataResponse, KnowledgeDataError> {
        let record_count = request
            .verbose
            .then(|| self.engine.graph().list_nodes())
            .transpose()
            .map_err(graph_error)?
            .map(|records| records.len() as u64);
        Ok(KnowledgeDataResponse::Health(ProviderHealth {
            readiness: ready(),
            recovery: recovered(),
            record_count,
        }))
    }

    fn get_by_id(
        &self,
        request: GetByIdRequest,
    ) -> Result<KnowledgeDataResponse, KnowledgeDataError> {
        let id = NodeId::new(request.id).map_err(graph_error)?;
        let record = self
            .engine
            .graph()
            .get_node(&id)
            .map_err(graph_error)?
            .map(node_to_record)
            .transpose()?;
        Ok(KnowledgeDataResponse::Record(record))
    }

    fn list(&self, request: ListRequest) -> Result<KnowledgeDataResponse, KnowledgeDataError> {
        validate_list_request(&request)?;
        let mut records = self.filtered_records(&request)?;
        records.truncate(request.limit as usize);
        Ok(KnowledgeDataResponse::Records(RecordPage {
            records,
            next_token: None,
        }))
    }

    fn paginate(
        &self,
        request: PaginateRequest,
    ) -> Result<KnowledgeDataResponse, KnowledgeDataError> {
        validate_list_request(&request.query)?;
        if request.page_size == 0 || request.page_size > 1_000 {
            return Err(KnowledgeDataError::new(
                KnowledgeDataErrorCode::InvalidRequest,
                "page_size must be between 1 and 1000",
                false,
            ));
        }
        let query_fingerprint = query_fingerprint(&request.query)?;
        let cursor = request
            .token
            .as_deref()
            .map(|token| {
                self.pagination
                    .verify(token, &query_fingerprint, CORROBORE_SCHEMA_VERSION)
                    .map(|claims| claims.cursor)
            })
            .transpose()?;
        let mut records = self.filtered_records(&request.query)?;
        if let Some(cursor) = cursor {
            records.retain(|record| record.id > cursor);
        }
        records.truncate(request.query.limit as usize);
        let page_size = request.page_size as usize;
        let has_more = records.len() > page_size;
        records.truncate(page_size);
        let next_token = if has_more {
            let cursor = records
                .last()
                .map(|record| record.id.clone())
                .ok_or_else(|| {
                    KnowledgeDataError::new(
                        KnowledgeDataErrorCode::Internal,
                        "pagination page lost its cursor",
                        false,
                    )
                })?;
            Some(self.pagination.issue(&PaginationTokenClaims {
                version: 1,
                query_fingerprint,
                schema_version: CORROBORE_SCHEMA_VERSION.to_owned(),
                cursor,
            })?)
        } else {
            None
        };
        Ok(KnowledgeDataResponse::Records(RecordPage {
            records,
            next_token,
        }))
    }

    fn count(&self, request: CountRequest) -> Result<KnowledgeDataResponse, KnowledgeDataError> {
        if !request.filter.is_null()
            && request
                .filter
                .as_object()
                .is_none_or(|filter| !filter.is_empty())
        {
            return Err(KnowledgeDataError::unsupported(OperationKind::Count));
        }
        let count = self.engine.graph().list_nodes().map_err(graph_error)?.len() as u64;
        Ok(KnowledgeDataResponse::Count(CountResult { count }))
    }

    fn neighbors(
        &self,
        request: NeighborsRequest,
    ) -> Result<KnowledgeDataResponse, KnowledgeDataError> {
        if !request.incoming && !request.outgoing {
            return Err(KnowledgeDataError::new(
                KnowledgeDataErrorCode::InvalidRequest,
                "neighbors requires incoming, outgoing, or both",
                false,
            ));
        }
        let id = NodeId::new(request.id).map_err(graph_error)?;
        let mut relationships = Vec::new();
        if request.incoming {
            relationships.extend(self.engine.graph().incoming(&id).map_err(graph_error)?);
        }
        if request.outgoing {
            relationships.extend(self.engine.graph().outgoing(&id).map_err(graph_error)?);
        }
        relationships.sort_by(|left, right| left.id().as_str().cmp(right.id().as_str()));
        relationships.dedup_by(|left, right| left.id() == right.id());

        let mut node_ids = Vec::new();
        for relationship in &relationships {
            node_ids.push(relationship.source().clone());
            node_ids.push(relationship.target().clone());
        }
        node_ids.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        node_ids.dedup_by(|left, right| left == right);
        let records = node_ids
            .into_iter()
            .map(|node_id| self.engine.graph().get_node(&node_id))
            .collect::<Result<Vec<_>, _>>()
            .map_err(graph_error)?
            .into_iter()
            .flatten()
            .map(node_to_record)
            .collect::<Result<Vec<_>, _>>()?;
        let relationships = relationships
            .into_iter()
            .map(|relationship| serde_json::to_value(relationship).map_err(serialization_error))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(KnowledgeDataResponse::Graph(GraphResult {
            records,
            relationships,
        }))
    }

    fn filtered_records(
        &self,
        request: &ListRequest,
    ) -> Result<Vec<KnowledgeRecord>, KnowledgeDataError> {
        let mut records = self
            .engine
            .graph()
            .list_nodes()
            .map_err(graph_error)?
            .into_iter()
            .filter(|node| {
                request.kinds.is_empty()
                    || request
                        .kinds
                        .iter()
                        .any(|kind| node.has_label(kind.as_str()))
            })
            .map(node_to_record)
            .collect::<Result<Vec<_>, _>>()?;
        records.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(records)
    }
}

fn validate_list_request(request: &ListRequest) -> Result<(), KnowledgeDataError> {
    if request.limit == 0 || request.limit > 10_000 {
        return Err(KnowledgeDataError::new(
            KnowledgeDataErrorCode::InvalidRequest,
            "list limit must be between 1 and 10000",
            false,
        ));
    }
    if request.kinds.iter().any(|kind| kind.trim().is_empty()) {
        return Err(KnowledgeDataError::new(
            KnowledgeDataErrorCode::InvalidRequest,
            "record kinds must not contain blank values",
            false,
        ));
    }
    Ok(())
}

fn ready() -> ProviderReadiness {
    ProviderReadiness {
        accepting_requests: true,
        blockers: Vec::new(),
    }
}

fn recovered() -> ProviderRecovery {
    ProviderRecovery {
        state: "ready".to_owned(),
        detail: None,
    }
}

fn query_fingerprint(request: &ListRequest) -> Result<String, KnowledgeDataError> {
    let bytes = serde_json::to_vec(request).map_err(serialization_error)?;
    let digest = Sha256::digest(bytes);
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn node_to_record(node: Node) -> Result<KnowledgeRecord, KnowledgeDataError> {
    let id = node.id().as_str().to_owned();
    let kind = node.labels().first().cloned().unwrap_or_default();
    let revision = node.version();
    let body = serde_json::to_value(node).map_err(serialization_error)?;
    Ok(KnowledgeRecord {
        id,
        kind,
        revision,
        body,
    })
}

fn graph_error(error: impl fmt::Display) -> KnowledgeDataError {
    KnowledgeDataError::new(
        KnowledgeDataErrorCode::Internal,
        format!("graph provider error: {error}"),
        false,
    )
}

fn serialization_error(error: impl fmt::Display) -> KnowledgeDataError {
    KnowledgeDataError::new(
        KnowledgeDataErrorCode::Internal,
        format!("contract serialization error: {error}"),
        false,
    )
}

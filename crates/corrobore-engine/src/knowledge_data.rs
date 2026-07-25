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
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet, HashSet},
    fmt,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use graph_core::{Graph, Node, NodeId, PropertyValue, Relationship};
use hmac::{Hmac, Mac};
pub use opencti_access::{
    AccessContext, AccessDecision, AccessDecisionReason, AccessMetadata, AccessPolicyError,
    OpenCtiAccessPolicy,
};
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

/// Supported scalar predicate operators for fundamental reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadFilterOperator {
    /// Exact typed equality.
    Equal,
    /// Typed inequality.
    NotEqual,
    /// Property presence, independent of its value.
    Exists,
    /// Strict lower-bound comparison.
    GreaterThan,
    /// Inclusive lower-bound comparison.
    GreaterThanOrEqual,
    /// Strict upper-bound comparison.
    LessThan,
    /// Inclusive upper-bound comparison.
    LessThanOrEqual,
}

/// One provider-neutral scalar property predicate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadFilter {
    /// OpenCTI field or provider-neutral record field.
    pub field: String,
    /// Comparison operator.
    pub operator: ReadFilterOperator,
    /// Comparison value. `exists` requires this to be absent.
    pub value: Option<Value>,
}

/// Stable ordering direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortDirection {
    /// Lowest value first.
    Ascending,
    /// Highest value first.
    Descending,
}

/// One stable sort key. Canonical record identity is always the final tie-breaker.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadOrder {
    /// OpenCTI field or provider-neutral record field.
    pub field: String,
    /// Sort direction.
    pub direction: SortDirection,
}

/// Bounded list input.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListRequest {
    /// Record kinds to include; empty means all kinds.
    pub kinds: Vec<String>,
    /// Conjunctive scalar predicates.
    #[serde(default)]
    pub filters: Vec<ReadFilter>,
    /// Stable sort keys before the canonical-ID tie-breaker.
    #[serde(default)]
    pub order_by: Vec<ReadOrder>,
    /// Maximum records returned before pagination.
    pub limit: u32,
}

impl Default for ListRequest {
    fn default() -> Self {
        Self {
            kinds: Vec::new(),
            filters: Vec::new(),
            order_by: Vec::new(),
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
    /// Record kinds to include; empty means all kinds.
    #[serde(default)]
    pub kinds: Vec<String>,
    /// Conjunctive scalar predicates.
    #[serde(default)]
    pub filters: Vec<ReadFilter>,
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
    /// Bounded graph-read filters and budgets.
    #[serde(default)]
    pub policy: GraphReadPolicy,
}

/// Direction applied to a bounded graph expansion.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphDirection {
    /// Follow relationships from source to target.
    Outgoing,
    /// Follow relationships from target to source.
    Incoming,
    /// Follow either direction.
    #[default]
    Both,
}

/// Filters and deterministic safety limits shared by graph reads.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphReadPolicy {
    /// Relationship types accepted during expansion.
    pub relationship_types: Vec<String>,
    /// Neighbor node kinds accepted during expansion.
    pub node_kinds: Vec<String>,
    /// Conjunctive neighbor predicates.
    pub filters: Vec<ReadFilter>,
    /// Maximum returned records.
    pub max_results: u32,
    /// Maximum relationship expansions.
    pub max_expansions: u32,
    /// Degree at which an unguarded expansion is treated as a supernode.
    pub supernode_threshold: u32,
}

impl Default for GraphReadPolicy {
    fn default() -> Self {
        Self {
            relationship_types: Vec::new(),
            node_kinds: Vec::new(),
            filters: Vec::new(),
            max_results: 1_000,
            max_expansions: 10_000,
            supernode_threshold: 10_000,
        }
    }
}

/// Bounded traversal input.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TraverseRequest {
    /// Starting record identifiers.
    pub start_ids: Vec<String>,
    /// Maximum traversal depth.
    pub max_depth: u32,
    /// Expansion direction.
    pub direction: GraphDirection,
    /// Backend-neutral traversal constraints.
    pub constraints: Value,
    /// Typed filters and deterministic budgets.
    pub policy: GraphReadPolicy,
}

/// Subgraph input.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SubgraphRequest {
    /// Stable record identifiers to project.
    pub ids: Vec<String>,
    /// Backend-neutral projection constraints.
    pub projection: Value,
    /// Maximum expansion depth.
    pub max_depth: u32,
    /// Expansion direction.
    pub direction: GraphDirection,
    /// Typed filters and deterministic budgets.
    pub policy: GraphReadPolicy,
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
    /// Pagination token belongs to an older consistent-read boundary.
    #[serde(rename = "STALE_PAGINATION_TOKEN")]
    StalePaginationToken,
    /// A graph operation omitted a mandatory deterministic bound.
    #[serde(rename = "UNBOUNDED_OPERATION")]
    UnboundedOperation,
    /// A bounded read exhausted its declared work or result budget.
    #[serde(rename = "QUERY_BUDGET_EXCEEDED")]
    QueryBudgetExceeded,
    /// A high-degree node expansion lacked narrowing guards.
    #[serde(rename = "SUPERNODE_EXPANSION_BLOCKED")]
    SupernodeExpansionBlocked,
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
            Self::StalePaginationToken => "STALE_PAGINATION_TOKEN",
            Self::UnboundedOperation => "UNBOUNDED_OPERATION",
            Self::QueryBudgetExceeded => "QUERY_BUDGET_EXCEEDED",
            Self::SupernodeExpansionBlocked => "SUPERNODE_EXPANSION_BLOCKED",
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
    /// Ordered provenance for each returned traversal path.
    pub paths: Vec<GraphPath>,
    /// Whether a declared result bound stopped expansion.
    pub truncated: bool,
}

/// One ordered node step in returned path provenance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphPathStep {
    /// Canonical node identity.
    pub node_id: String,
    /// Canonical node revision.
    pub node_revision: u64,
    /// Relationship used to enter this node, absent for the seed.
    pub relationship_id: Option<String>,
    /// Canonical relationship revision.
    pub relationship_revision: Option<u64>,
}

/// Ordered canonical path from a requested seed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphPath {
    /// Seed-first path steps.
    pub steps: Vec<GraphPathStep>,
}

/// Bounded fundamental-read metric dimensions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreReadQueryClass {
    /// Identifier lookup.
    PointRead,
    /// Bounded record list.
    List,
    /// Snapshot-bound cursor page.
    Pagination,
    /// Filtered count.
    Count,
    /// One-hop neighborhood.
    Neighbors,
    /// Bounded traversal.
    Traverse,
    /// Bounded subgraph.
    Subgraph,
}

impl CoreReadQueryClass {
    /// Stable low-cardinality metrics label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PointRead => "point_read",
            Self::List => "list",
            Self::Pagination => "pagination",
            Self::Count => "count",
            Self::Neighbors => "neighbors",
            Self::Traverse => "traverse",
            Self::Subgraph => "subgraph",
        }
    }
}

/// Cumulative metrics for one bounded query class.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreReadMetricSeries {
    /// Completed requests.
    pub requests: u64,
    /// Approximate median latency from fixed buckets.
    pub p50_latency_ms: u64,
    /// Approximate 95th-percentile latency from fixed buckets.
    pub p95_latency_ms: u64,
    /// Approximate 99th-percentile latency from fixed buckets.
    pub p99_latency_ms: u64,
    /// Records evaluated by exact provider-neutral predicates.
    pub records_examined: u64,
    /// Persistent payload page-ins attributed to this class.
    pub page_ins: u64,
    /// Persistent payload cache hits attributed to this class.
    pub cache_hits: u64,
    #[serde(skip)]
    latency_buckets: [u64; 16],
}

/// Low-cardinality cumulative metrics keyed only by query class.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreReadMetrics {
    series: BTreeMap<CoreReadQueryClass, CoreReadMetricSeries>,
}

impl CoreReadMetrics {
    /// Return a cumulative series.
    pub fn series(&self, query_class: CoreReadQueryClass) -> Option<&CoreReadMetricSeries> {
        self.series.get(&query_class)
    }

    pub(crate) fn record(
        &mut self,
        query_class: CoreReadQueryClass,
        latency_ms: u64,
        records_examined: u64,
        page_ins: u64,
        cache_hits: u64,
    ) {
        const BOUNDS: [u64; 16] = [
            0,
            1,
            2,
            4,
            8,
            16,
            32,
            64,
            128,
            250,
            500,
            1_000,
            2_000,
            5_000,
            10_000,
            u64::MAX,
        ];
        let series = self.series.entry(query_class).or_default();
        series.requests = series.requests.saturating_add(1);
        series.records_examined = series.records_examined.saturating_add(records_examined);
        series.page_ins = series.page_ins.saturating_add(page_ins);
        series.cache_hits = series.cache_hits.saturating_add(cache_hits);
        let bucket = BOUNDS
            .iter()
            .position(|bound| latency_ms <= *bound)
            .unwrap_or(BOUNDS.len() - 1);
        series.latency_buckets[bucket] = series.latency_buckets[bucket].saturating_add(1);
        series.p50_latency_ms = histogram_percentile(&series.latency_buckets, &BOUNDS, 50);
        series.p95_latency_ms = histogram_percentile(&series.latency_buckets, &BOUNDS, 95);
        series.p99_latency_ms = histogram_percentile(&series.latency_buckets, &BOUNDS, 99);
    }
}

fn histogram_percentile(counts: &[u64; 16], bounds: &[u64; 16], percentile: u64) -> u64 {
    let total = counts.iter().sum::<u64>();
    let target = total.saturating_mul(percentile).saturating_add(99) / 100;
    let mut cumulative = 0_u64;
    for (index, count) in counts.iter().enumerate() {
        cumulative = cumulative.saturating_add(*count);
        if cumulative >= target {
            return bounds[index].min(10_000);
        }
    }
    0
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

/// Payload-free security audit event for one policy decision summary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityAuditEvent {
    /// Request correlation identity.
    pub correlation_id: String,
    /// Operation evaluated.
    pub operation: OperationKind,
    /// Stable policy generation.
    pub policy_version: String,
    /// Whether the request produced an authorized view.
    pub allowed: bool,
    /// Number of candidates rejected before they could enter the result.
    pub authorization_denials: u64,
    /// Stable decision reason without record identifiers or payload values.
    pub reason: AccessDecisionReason,
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
    /// Fingerprint of the stable result snapshot at issuance.
    #[serde(default)]
    pub snapshot_fingerprint: String,
    /// Number of records already returned under the original query limit.
    #[serde(default)]
    pub returned: u32,
    /// Policy generation active when the cursor was issued.
    #[serde(default)]
    pub policy_version: String,
    /// Pseudonymous normalized access-context fingerprint.
    #[serde(default)]
    pub access_fingerprint: String,
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

    /// Verify query claims and reject reuse after an access-policy change.
    pub fn verify_for_access(
        &self,
        token: &str,
        query_fingerprint: &str,
        schema_version: &str,
        policy: &OpenCtiAccessPolicy,
    ) -> Result<PaginationTokenClaims, KnowledgeDataError> {
        let claims = self.verify(token, query_fingerprint, schema_version)?;
        if claims.policy_version != policy.policy_version()
            || claims.access_fingerprint != self.access_binding(policy)?
        {
            return Err(KnowledgeDataError::new(
                KnowledgeDataErrorCode::StalePaginationToken,
                "pagination token belongs to another access-policy generation",
                false,
            ));
        }
        Ok(claims)
    }

    fn access_binding(&self, policy: &OpenCtiAccessPolicy) -> Result<String, KnowledgeDataError> {
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.key).map_err(|_| {
            KnowledgeDataError::new(
                KnowledgeDataErrorCode::Internal,
                "failed to initialize pagination access binding",
                false,
            )
        })?;
        mac.update(b"opencti-access-binding\0");
        mac.update(policy.fingerprint().as_bytes());
        Ok(URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()))
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
        validate_operation_before_projection(&operation)?;
        let operation_kind = operation.kind();
        let access_policy = OpenCtiAccessPolicy::compile(&context.access).map_err(|error| {
            KnowledgeDataError::new(
                KnowledgeDataErrorCode::InvalidRequest,
                error.to_string(),
                false,
            )
        })?;
        let query_class = core_read_query_class(&operation);
        let audit_events_before = self.engine.security_audit_events.len();
        let started = Instant::now();
        let projection_stats = self
            .engine
            .prepare_knowledge_data_operation(&operation, &context.access)
            .map_err(|error| map_projection_error(&error.to_string()))?
            .unwrap_or_default();
        let authorization_denials = projection_stats.2;
        let result = match operation {
            KnowledgeDataOperation::Initialize(request) => self.initialize(request),
            KnowledgeDataOperation::Health(request) => self.health(request),
            KnowledgeDataOperation::GetById(request) => {
                self.get_by_id(request, context, &access_policy)
            }
            KnowledgeDataOperation::List(request) => self.list(request, &access_policy),
            KnowledgeDataOperation::Paginate(request) => self.paginate(request, &access_policy),
            KnowledgeDataOperation::Count(request) => self.count(request, &access_policy),
            KnowledgeDataOperation::Neighbors(request) => self.neighbors(request, &access_policy),
            KnowledgeDataOperation::Traverse(request) => self.traverse(request, &access_policy),
            KnowledgeDataOperation::Subgraph(request) => self.subgraph(request, &access_policy),
            unsupported => {
                let operation = unsupported.kind();
                let _ = context;
                Err(KnowledgeDataError::unsupported(operation))
            }
        };
        if let Some(query_class) = query_class {
            let records_examined = self
                .engine
                .graph()
                .list_nodes()
                .map_or(0, |records| records.len() as u64);
            self.engine.core_read_metrics.record(
                query_class,
                u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                records_examined,
                projection_stats.0,
                projection_stats.1,
            );
        }
        if self.engine.security_audit_events.len() == audit_events_before {
            self.engine.record_security_audit_event(SecurityAuditEvent {
                correlation_id: context.correlation_id.clone(),
                operation: operation_kind,
                policy_version: access_policy.policy_version().to_owned(),
                allowed: !matches!(operation_kind, OperationKind::GetById)
                    || authorization_denials == 0,
                authorization_denials,
                reason: if access_policy.is_system() {
                    AccessDecisionReason::System
                } else {
                    AccessDecisionReason::PolicyApplied
                },
            });
        }
        result
    }

    fn is_cancelled(&self, cancellation_id: &str) -> bool {
        self.cancelled.contains(cancellation_id)
    }
}

fn validate_operation_before_projection(
    operation: &KnowledgeDataOperation,
) -> Result<(), KnowledgeDataError> {
    match operation {
        KnowledgeDataOperation::GetById(request) if request.id.trim().is_empty() => {
            Err(KnowledgeDataError::new(
                KnowledgeDataErrorCode::InvalidRequest,
                "record identifier must not be blank",
                false,
            ))
        }
        KnowledgeDataOperation::List(request) => validate_list_request(request),
        KnowledgeDataOperation::Paginate(request) => {
            validate_list_request(&request.query)?;
            if request.page_size == 0 || request.page_size > 1_000 {
                return Err(KnowledgeDataError::new(
                    KnowledgeDataErrorCode::InvalidRequest,
                    "page_size must be between 1 and 1000",
                    false,
                ));
            }
            Ok(())
        }
        KnowledgeDataOperation::Neighbors(request) => {
            if !request.incoming && !request.outgoing {
                return Err(KnowledgeDataError::new(
                    KnowledgeDataErrorCode::InvalidRequest,
                    "neighbors requires incoming, outgoing, or both",
                    false,
                ));
            }
            validate_graph_read(std::slice::from_ref(&request.id), 1, &request.policy)
        }
        KnowledgeDataOperation::Traverse(request) => {
            validate_graph_read(&request.start_ids, request.max_depth, &request.policy)
        }
        KnowledgeDataOperation::Subgraph(request) => {
            validate_graph_read(&request.ids, request.max_depth, &request.policy)
        }
        _ => Ok(()),
    }
}

fn core_read_query_class(operation: &KnowledgeDataOperation) -> Option<CoreReadQueryClass> {
    match operation {
        KnowledgeDataOperation::GetById(_) => Some(CoreReadQueryClass::PointRead),
        KnowledgeDataOperation::List(_) => Some(CoreReadQueryClass::List),
        KnowledgeDataOperation::Paginate(_) => Some(CoreReadQueryClass::Pagination),
        KnowledgeDataOperation::Count(_) => Some(CoreReadQueryClass::Count),
        KnowledgeDataOperation::Neighbors(_) => Some(CoreReadQueryClass::Neighbors),
        KnowledgeDataOperation::Traverse(_) => Some(CoreReadQueryClass::Traverse),
        KnowledgeDataOperation::Subgraph(_) => Some(CoreReadQueryClass::Subgraph),
        _ => None,
    }
}

fn map_projection_error(message: &str) -> KnowledgeDataError {
    let lower = message.to_ascii_lowercase();
    if lower.contains("supernode") {
        KnowledgeDataError::new(
            KnowledgeDataErrorCode::SupernodeExpansionBlocked,
            message,
            false,
        )
    } else if lower.contains("budget exceeded") || lower.contains("requires") {
        KnowledgeDataError::new(KnowledgeDataErrorCode::QueryBudgetExceeded, message, false)
    } else {
        KnowledgeDataError::new(
            KnowledgeDataErrorCode::BackendUnavailable,
            "failed to prepare the persistent Knowledge Data projection",
            true,
        )
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
        | OperationKind::Neighbors
        | OperationKind::Traverse
        | OperationKind::Subgraph => ProviderCapabilityStatus::Supported,
        OperationKind::Migrate | OperationKind::Snapshot | OperationKind::Restore => {
            unsupported_status("portable lifecycle support is delivered by issue #52")
        }
        OperationKind::Search => unsupported_status(
            "structured and full-text search are delivered by issues #46 and #47",
        ),
        OperationKind::Aggregate => {
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
        &mut self,
        request: GetByIdRequest,
        context: &RequestContext,
        access_policy: &OpenCtiAccessPolicy,
    ) -> Result<KnowledgeDataResponse, KnowledgeDataError> {
        if request.id.trim().is_empty() {
            return Err(KnowledgeDataError::new(
                KnowledgeDataErrorCode::InvalidRequest,
                "record identifier must not be blank",
                false,
            ));
        }
        let Some(node) = resolve_node_by_identifier(self.engine.graph(), &request.id)? else {
            return Ok(KnowledgeDataResponse::Record(None));
        };
        let decision = node_access_decision(&node, access_policy);
        self.engine.record_security_audit_event(SecurityAuditEvent {
            correlation_id: context.correlation_id.clone(),
            operation: OperationKind::GetById,
            policy_version: access_policy.policy_version().to_owned(),
            allowed: decision.allowed(),
            authorization_denials: u64::from(!decision.allowed()),
            reason: decision.reason(),
        });
        if !decision.allowed() {
            return Ok(KnowledgeDataResponse::Record(None));
        }
        Ok(KnowledgeDataResponse::Record(Some(node_to_record(node)?)))
    }

    fn list(
        &self,
        request: ListRequest,
        access_policy: &OpenCtiAccessPolicy,
    ) -> Result<KnowledgeDataResponse, KnowledgeDataError> {
        validate_list_request(&request)?;
        let mut records = self.filtered_records(&request, access_policy)?;
        records.truncate(request.limit as usize);
        Ok(KnowledgeDataResponse::Records(RecordPage {
            records,
            next_token: None,
        }))
    }

    fn paginate(
        &self,
        request: PaginateRequest,
        access_policy: &OpenCtiAccessPolicy,
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
        let claims = request
            .token
            .as_deref()
            .map(|token| {
                self.pagination.verify_for_access(
                    token,
                    &query_fingerprint,
                    CORROBORE_SCHEMA_VERSION,
                    access_policy,
                )
            })
            .transpose()?;
        let records = self.filtered_records(&request.query, access_policy)?;
        let snapshot_fingerprint = snapshot_fingerprint(&records)?;
        if claims
            .as_ref()
            .is_some_and(|claims| claims.snapshot_fingerprint != snapshot_fingerprint)
        {
            return Err(KnowledgeDataError::new(
                KnowledgeDataErrorCode::StalePaginationToken,
                "pagination token belongs to an older result snapshot",
                false,
            ));
        }
        let start = match &claims {
            Some(claims) => records
                .iter()
                .position(|record| record_cursor(record, &request.query.order_by) == claims.cursor)
                .map(|index| index + 1)
                .ok_or_else(|| {
                    KnowledgeDataError::new(
                        KnowledgeDataErrorCode::StalePaginationToken,
                        "pagination cursor no longer exists in the declared snapshot",
                        false,
                    )
                })?,
            None => 0,
        };
        let returned = claims.as_ref().map_or(0, |claims| claims.returned);
        let remaining = request.query.limit.saturating_sub(returned) as usize;
        let take = remaining.min(request.page_size as usize);
        let page_records = records
            .iter()
            .skip(start)
            .take(take)
            .cloned()
            .collect::<Vec<_>>();
        let total_returned =
            returned.saturating_add(u32::try_from(page_records.len()).unwrap_or(u32::MAX));
        let has_more = start.saturating_add(page_records.len()) < records.len()
            && total_returned < request.query.limit;
        let next_token = if has_more {
            let cursor = page_records
                .last()
                .map(|record| record_cursor(record, &request.query.order_by))
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
                snapshot_fingerprint,
                returned: total_returned,
                policy_version: access_policy.policy_version().to_owned(),
                access_fingerprint: self.pagination.access_binding(access_policy)?,
            })?)
        } else {
            None
        };
        Ok(KnowledgeDataResponse::Records(RecordPage {
            records: page_records,
            next_token,
        }))
    }

    fn count(
        &self,
        request: CountRequest,
        access_policy: &OpenCtiAccessPolicy,
    ) -> Result<KnowledgeDataResponse, KnowledgeDataError> {
        if !request.filter.is_null()
            && request
                .filter
                .as_object()
                .is_none_or(|filter| !filter.is_empty())
        {
            return Err(KnowledgeDataError::unsupported(OperationKind::Count));
        }
        let query = ListRequest {
            kinds: request.kinds,
            filters: request.filters,
            order_by: Vec::new(),
            limit: 10_000,
        };
        validate_list_request(&query)?;
        let count = self.filtered_records(&query, access_policy)?.len() as u64;
        Ok(KnowledgeDataResponse::Count(CountResult { count }))
    }

    fn neighbors(
        &self,
        request: NeighborsRequest,
        access_policy: &OpenCtiAccessPolicy,
    ) -> Result<KnowledgeDataResponse, KnowledgeDataError> {
        if !request.incoming && !request.outgoing {
            return Err(KnowledgeDataError::new(
                KnowledgeDataErrorCode::InvalidRequest,
                "neighbors requires incoming, outgoing, or both",
                false,
            ));
        }
        let direction = match (request.incoming, request.outgoing) {
            (true, true) => GraphDirection::Both,
            (true, false) => GraphDirection::Incoming,
            (false, true) => GraphDirection::Outgoing,
            (false, false) => unreachable!("validated above"),
        };
        self.bounded_graph_read(
            vec![request.id],
            1,
            direction,
            request.policy,
            access_policy,
        )
        .map(KnowledgeDataResponse::Graph)
    }

    fn traverse(
        &self,
        request: TraverseRequest,
        access_policy: &OpenCtiAccessPolicy,
    ) -> Result<KnowledgeDataResponse, KnowledgeDataError> {
        if !request.constraints.is_null()
            && request
                .constraints
                .as_object()
                .is_none_or(|constraints| !constraints.is_empty())
        {
            return Err(KnowledgeDataError::new(
                KnowledgeDataErrorCode::InvalidRequest,
                "typed constraints must use graph read policy fields",
                false,
            ));
        }
        self.bounded_graph_read(
            request.start_ids,
            request.max_depth,
            request.direction,
            request.policy,
            access_policy,
        )
        .map(KnowledgeDataResponse::Graph)
    }

    fn subgraph(
        &self,
        request: SubgraphRequest,
        access_policy: &OpenCtiAccessPolicy,
    ) -> Result<KnowledgeDataResponse, KnowledgeDataError> {
        if !request.projection.is_null()
            && request
                .projection
                .as_object()
                .is_none_or(|projection| !projection.is_empty())
        {
            return Err(KnowledgeDataError::new(
                KnowledgeDataErrorCode::InvalidRequest,
                "typed projection constraints must use graph read policy fields",
                false,
            ));
        }
        self.bounded_graph_read(
            request.ids,
            request.max_depth,
            request.direction,
            request.policy,
            access_policy,
        )
        .map(KnowledgeDataResponse::Graph)
    }

    fn filtered_records(
        &self,
        request: &ListRequest,
        access_policy: &OpenCtiAccessPolicy,
    ) -> Result<Vec<KnowledgeRecord>, KnowledgeDataError> {
        let mut records: Vec<KnowledgeRecord> = self
            .engine
            .graph()
            .list_nodes()
            .map_err(graph_error)?
            .into_iter()
            .filter(|node| node_access_decision(node, access_policy).allowed())
            .map(node_to_record)
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|record| record_matches_request(record, request))
            .collect();
        records.sort_by(|left, right| compare_records(left, right, &request.order_by));
        Ok(records)
    }

    fn bounded_graph_read(
        &self,
        start_ids: Vec<String>,
        max_depth: u32,
        direction: GraphDirection,
        policy: GraphReadPolicy,
        access_policy: &OpenCtiAccessPolicy,
    ) -> Result<GraphResult, KnowledgeDataError> {
        validate_graph_read(&start_ids, max_depth, &policy)?;
        let graph = self.engine.graph();
        let mut selected_nodes = BTreeMap::<String, Node>::new();
        let mut selected_relationships = BTreeMap::<String, Relationship>::new();
        let mut paths = Vec::new();
        let mut queue = Vec::new();
        let mut visited = HashSet::new();

        for identifier in start_ids {
            let Some(node) = resolve_node_by_identifier(graph, &identifier)? else {
                continue;
            };
            if !node_access_decision(&node, access_policy).allowed() {
                continue;
            }
            let canonical_id = canonical_node_id(&node);
            if visited.insert(node.id().clone()) {
                let path = vec![GraphPathStep {
                    node_id: canonical_id.clone(),
                    node_revision: node.version(),
                    relationship_id: None,
                    relationship_revision: None,
                }];
                paths.push(GraphPath {
                    steps: path.clone(),
                });
                queue.push((node.id().clone(), 0_u32, path));
                selected_nodes.insert(canonical_id, node);
            }
        }

        let mut expansions = 0_u32;
        let mut truncated = false;
        let mut cursor = 0_usize;
        while cursor < queue.len() {
            let (owner, depth, path) = queue[cursor].clone();
            cursor += 1;
            if depth >= max_depth {
                continue;
            }
            let mut relationships = incident_relationships(graph, &owner, direction)?;
            relationships.retain(|relationship| {
                if !relationship_access_decision(relationship, access_policy).allowed() {
                    return false;
                }
                let neighbor_id = if relationship.source() == &owner {
                    relationship.target()
                } else {
                    relationship.source()
                };
                graph
                    .get_node(neighbor_id)
                    .ok()
                    .flatten()
                    .is_some_and(|neighbor| {
                        node_access_decision(&neighbor, access_policy).allowed()
                    })
            });
            relationships.sort_by(|left, right| {
                canonical_relationship_id(left).cmp(&canonical_relationship_id(right))
            });
            relationships.dedup_by(|left, right| left.id() == right.id());
            let guarded = !policy.relationship_types.is_empty()
                || !policy.node_kinds.is_empty()
                || !policy.filters.is_empty();
            if relationships.len() as u32 > policy.supernode_threshold && !guarded {
                return Err(KnowledgeDataError::new(
                    KnowledgeDataErrorCode::SupernodeExpansionBlocked,
                    format!(
                        "supernode expansion blocked at {} with degree {}",
                        canonical_node_id(
                            &graph
                                .get_node(&owner)
                                .map_err(graph_error)?
                                .ok_or_else(|| graph_error("missing traversal owner"))?
                        ),
                        relationships.len()
                    ),
                    false,
                ));
            }

            for relationship in relationships {
                if !policy.relationship_types.is_empty()
                    && !policy
                        .relationship_types
                        .iter()
                        .any(|kind| kind == relationship.rel_type().as_str())
                {
                    continue;
                }
                expansions = expansions.saturating_add(1);
                if expansions > policy.max_expansions {
                    return Err(KnowledgeDataError::new(
                        KnowledgeDataErrorCode::QueryBudgetExceeded,
                        "graph read exhausted its relationship expansion budget",
                        false,
                    ));
                }
                let neighbor_id = if relationship.source() == &owner {
                    relationship.target().clone()
                } else {
                    relationship.source().clone()
                };
                let Some(neighbor) = graph.get_node(&neighbor_id).map_err(graph_error)? else {
                    continue;
                };
                if !node_access_decision(&neighbor, access_policy).allowed() {
                    continue;
                }
                let neighbor_record = node_to_record(neighbor.clone())?;
                if !record_matches_graph_policy(&neighbor_record, &policy) {
                    continue;
                }
                if selected_nodes.len() as u32 >= policy.max_results
                    && !selected_nodes.contains_key(&neighbor_record.id)
                {
                    truncated = true;
                    break;
                }
                let relationship_id = canonical_relationship_id(&relationship);
                selected_relationships
                    .entry(relationship_id.clone())
                    .or_insert_with(|| relationship.clone());
                selected_nodes
                    .entry(neighbor_record.id.clone())
                    .or_insert_with(|| neighbor.clone());
                if visited.insert(neighbor_id.clone()) {
                    let mut neighbor_path = path.clone();
                    neighbor_path.push(GraphPathStep {
                        node_id: neighbor_record.id,
                        node_revision: neighbor.version(),
                        relationship_id: Some(relationship_id),
                        relationship_revision: Some(relationship.version()),
                    });
                    paths.push(GraphPath {
                        steps: neighbor_path.clone(),
                    });
                    queue.push((neighbor_id, depth + 1, neighbor_path));
                }
            }
            if truncated {
                break;
            }
        }

        let records = selected_nodes
            .into_values()
            .map(node_to_record)
            .collect::<Result<Vec<_>, _>>()?;
        let relationships = selected_relationships
            .into_values()
            .map(relationship_to_value)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(GraphResult {
            records,
            relationships,
            paths,
            truncated,
        })
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
    for filter in &request.filters {
        if filter.field.trim().is_empty()
            || (filter.operator == ReadFilterOperator::Exists && filter.value.is_some())
            || (filter.operator != ReadFilterOperator::Exists && filter.value.is_none())
        {
            return Err(KnowledgeDataError::new(
                KnowledgeDataErrorCode::InvalidRequest,
                "read filters require a field and operator-compatible value",
                false,
            ));
        }
    }
    if request
        .order_by
        .iter()
        .any(|order| order.field.trim().is_empty())
    {
        return Err(KnowledgeDataError::new(
            KnowledgeDataErrorCode::InvalidRequest,
            "read order fields must not be blank",
            false,
        ));
    }
    Ok(())
}

fn validate_graph_read(
    start_ids: &[String],
    max_depth: u32,
    policy: &GraphReadPolicy,
) -> Result<(), KnowledgeDataError> {
    if start_ids.is_empty()
        || start_ids.iter().any(|id| id.trim().is_empty())
        || max_depth == 0
        || max_depth > 8
        || policy.max_results == 0
        || policy.max_expansions == 0
        || policy.supernode_threshold == 0
    {
        return Err(KnowledgeDataError::new(
            KnowledgeDataErrorCode::UnboundedOperation,
            "graph reads require seeds, depth 1..=8, and non-zero result, expansion and supernode bounds",
            false,
        ));
    }
    validate_list_request(&ListRequest {
        kinds: policy.node_kinds.clone(),
        filters: policy.filters.clone(),
        order_by: Vec::new(),
        limit: policy.max_results.min(10_000),
    })
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
    let id = canonical_node_id(&node);
    let body = match node.property("opencti.raw") {
        Some(PropertyValue::Json(value)) => value.clone(),
        _ => serde_json::to_value(&node).map_err(serialization_error)?,
    };
    let kind = body
        .get("type")
        .and_then(Value::as_str)
        .or_else(|| match node.property("opencti.field.type") {
            Some(PropertyValue::String(value)) => Some(value.as_str()),
            _ => None,
        })
        .map(str::to_owned)
        .unwrap_or_else(|| node.labels().first().cloned().unwrap_or_default());
    let revision = node.version();
    Ok(KnowledgeRecord {
        id,
        kind,
        revision,
        body,
    })
}

fn canonical_node_id(node: &Node) -> String {
    match node.property("opencti.canonical_id") {
        Some(PropertyValue::String(value)) => value.clone(),
        _ => node.id().as_str().to_owned(),
    }
}

fn canonical_relationship_id(relationship: &Relationship) -> String {
    match relationship.property("opencti.canonical_id") {
        Some(PropertyValue::String(value)) => value.clone(),
        _ => relationship.id().as_str().to_owned(),
    }
}

fn relationship_to_value(relationship: Relationship) -> Result<Value, KnowledgeDataError> {
    match relationship.property("opencti.raw") {
        Some(PropertyValue::Json(value)) => Ok(value.clone()),
        _ => serde_json::to_value(relationship).map_err(serialization_error),
    }
}

fn resolve_node_by_identifier(
    graph: &Graph,
    identifier: &str,
) -> Result<Option<Node>, KnowledgeDataError> {
    let nodes = graph.list_nodes().map_err(graph_error)?;
    Ok(nodes
        .into_iter()
        .find(|node| node_matches_identifier(node, identifier)))
}

fn node_matches_identifier(node: &Node, identifier: &str) -> bool {
    if node.id().as_str() == identifier || canonical_node_id(node) == identifier {
        return true;
    }
    match node.property("opencti.identifiers") {
        Some(PropertyValue::Json(value)) => json_contains_identifier(value, identifier),
        Some(PropertyValue::String(value)) => value == identifier,
        Some(PropertyValue::StringList(values)) => values.iter().any(|value| value == identifier),
        _ => false,
    }
}

fn json_contains_identifier(value: &Value, identifier: &str) -> bool {
    match value {
        Value::String(value) => value == identifier,
        Value::Array(values) => values
            .iter()
            .any(|value| json_contains_identifier(value, identifier)),
        Value::Object(values) => ["value", "id", "external_id"]
            .iter()
            .filter_map(|key| values.get(*key))
            .any(|value| json_contains_identifier(value, identifier)),
        _ => false,
    }
}

fn node_access_decision(node: &Node, policy: &OpenCtiAccessPolicy) -> AccessDecision {
    let value = match node.property("opencti.access") {
        Some(PropertyValue::Json(value)) => Some(value),
        Some(_) => return policy.evaluate_value(Some(&Value::Bool(false))),
        None => None,
    };
    policy.evaluate_value(value)
}

fn relationship_access_decision(
    relationship: &Relationship,
    policy: &OpenCtiAccessPolicy,
) -> AccessDecision {
    let value = match relationship.property("opencti.access") {
        Some(PropertyValue::Json(value)) => Some(value),
        Some(_) => return policy.evaluate_value(Some(&Value::Bool(false))),
        None => None,
    };
    policy.evaluate_value(value)
}

fn record_matches_request(record: &KnowledgeRecord, request: &ListRequest) -> bool {
    (request.kinds.is_empty()
        || request
            .kinds
            .iter()
            .any(|kind| kind.eq_ignore_ascii_case(&record.kind)))
        && request
            .filters
            .iter()
            .all(|filter| record_matches_filter(record, filter))
}

fn record_matches_graph_policy(record: &KnowledgeRecord, policy: &GraphReadPolicy) -> bool {
    (policy.node_kinds.is_empty()
        || policy
            .node_kinds
            .iter()
            .any(|kind| kind.eq_ignore_ascii_case(&record.kind)))
        && policy
            .filters
            .iter()
            .all(|filter| record_matches_filter(record, filter))
}

fn record_matches_filter(record: &KnowledgeRecord, filter: &ReadFilter) -> bool {
    let actual = record_field(record, &filter.field);
    match filter.operator {
        ReadFilterOperator::Exists => actual.is_some(),
        ReadFilterOperator::Equal => actual == filter.value.as_ref(),
        ReadFilterOperator::NotEqual => actual.is_some() && actual != filter.value.as_ref(),
        operator => actual
            .zip(filter.value.as_ref())
            .and_then(|(actual, expected)| compare_values(actual, expected))
            .is_some_and(|ordering| match operator {
                ReadFilterOperator::GreaterThan => ordering.is_gt(),
                ReadFilterOperator::GreaterThanOrEqual => ordering.is_ge(),
                ReadFilterOperator::LessThan => ordering.is_lt(),
                ReadFilterOperator::LessThanOrEqual => ordering.is_le(),
                ReadFilterOperator::Equal
                | ReadFilterOperator::NotEqual
                | ReadFilterOperator::Exists => false,
            }),
    }
}

fn record_field<'a>(record: &'a KnowledgeRecord, field: &str) -> Option<&'a Value> {
    match field {
        "id" => record.body.get("id"),
        "type" | "kind" => record.body.get("type"),
        _ => {
            let mut value = &record.body;
            for component in field.split('.') {
                value = value.get(component)?;
            }
            Some(value)
        }
    }
}

fn compare_values(left: &Value, right: &Value) -> Option<Ordering> {
    match (left, right) {
        (Value::Number(left), Value::Number(right)) => left.as_f64()?.partial_cmp(&right.as_f64()?),
        (Value::String(left), Value::String(right)) => Some(left.cmp(right)),
        (Value::Bool(left), Value::Bool(right)) => Some(left.cmp(right)),
        (Value::Null, Value::Null) => Some(Ordering::Equal),
        _ => None,
    }
}

fn compare_records(
    left: &KnowledgeRecord,
    right: &KnowledgeRecord,
    ordering: &[ReadOrder],
) -> Ordering {
    for order in ordering {
        let value_order = match (
            record_field(left, &order.field),
            record_field(right, &order.field),
        ) {
            (Some(left), Some(right)) => compare_values(left, right).unwrap_or(Ordering::Equal),
            (None, Some(_)) => Ordering::Less,
            (Some(_), None) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        };
        let value_order = match order.direction {
            SortDirection::Ascending => value_order,
            SortDirection::Descending => value_order.reverse(),
        };
        if !value_order.is_eq() {
            return value_order;
        }
    }
    left.id.cmp(&right.id)
}

fn record_cursor(record: &KnowledgeRecord, ordering: &[ReadOrder]) -> String {
    let mut values = ordering
        .iter()
        .map(|order| {
            record_field(record, &order.field)
                .cloned()
                .unwrap_or(Value::Null)
        })
        .collect::<Vec<_>>();
    values.push(Value::String(record.id.clone()));
    serde_json::to_string(&values).unwrap_or_else(|_| record.id.clone())
}

fn snapshot_fingerprint(records: &[KnowledgeRecord]) -> Result<String, KnowledgeDataError> {
    let bytes = serde_json::to_vec(
        &records
            .iter()
            .map(|record| (&record.id, record.revision))
            .collect::<Vec<_>>(),
    )
    .map_err(serialization_error)?;
    let digest = Sha256::digest(bytes);
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn incident_relationships(
    graph: &Graph,
    node_id: &NodeId,
    direction: GraphDirection,
) -> Result<Vec<Relationship>, KnowledgeDataError> {
    let mut relationships = Vec::new();
    if matches!(direction, GraphDirection::Incoming | GraphDirection::Both) {
        relationships.extend(graph.incoming(node_id).map_err(graph_error)?);
    }
    if matches!(direction, GraphDirection::Outgoing | GraphDirection::Both) {
        relationships.extend(graph.outgoing(node_id).map_err(graph_error)?);
    }
    Ok(relationships)
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

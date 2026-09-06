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
#![warn(missing_docs)]

//! Embedded engine facade for the intelligence graph engine.
//!
//! This crate lets a host application drive Corrobore as an in-process library,
//! without running `corrobore-http-server`. It wraps the shared-runtime Cypher
//! gateway (policy validation, budgets, deterministic executor) behind an
//! ergonomic, synchronous API.
//!
//! Design boundary:
//!
//! - the facade owns a [`shared_runtime::CypherGateway`] and typed request
//!   identity defaults (workspace, session, budget reference);
//! - every query still passes through the full gateway pipeline, so embedded
//!   callers get the same policy, budget, and mutation-permission guarantees
//!   as HTTP callers;
//! - persistence wiring and Python bindings are explicit non-goals here.
//!
//! # Example
//!
//! ```
//! use corrobore_engine::{CypherResponseData, CorroboreEngine};
//!
//! let mut engine = CorroboreEngine::strict_default();
//! engine.write("CREATE (n:Indicator {name: 'observed-domain'})")?;
//!
//! let response = engine.read("MATCH (n:Indicator) RETURN n")?;
//! assert!(matches!(response.data, CypherResponseData::Records(_)));
//! # Ok::<(), corrobore_engine::EngineError>(())
//! ```
//!
//! Read-only hosts can disable mutations entirely:
//!
//! ```
//! use corrobore_engine::{CypherResponseStatus, CorroboreEngine};
//!
//! let mut engine = CorroboreEngine::builder().read_only(true).build()?;
//! let response = engine.write("CREATE (n:Indicator {name: 'blocked'})")?;
//! assert_eq!(response.status, CypherResponseStatus::Rejected);
//! # Ok::<(), corrobore_engine::EngineError>(())
//! ```

mod knowledge_data;
mod memory;
mod opencti_routing;
mod opencti_shadow;

use std::collections::{BTreeMap, HashMap};

use export_stix::{StixExportBundle, export_stix_subset_bundle};
use graph_core::{
    ExportMetadata, ExportPlanOptions, Graph, GraphSemanticSeedResolver, SessionId, TransactionId,
    ValidationErrorRecord, WorkspaceId, build_deterministic_export_plan_with_options,
};
pub use graph_core::{
    ExportMode, ExportProfile, GraphError, SemanticDomainProfile, SemanticSeedCandidate,
    SemanticSeedQueryRequest, SemanticSeedQueryResponse, SemanticSeedResolutionError,
    SemanticSeedResolutionErrorCode, SemanticSeedResolver, SemanticSeedRetrievalMode,
};
use shared_runtime::{
    CypherBudgetRef, CypherGateway, CypherParameters, CypherRequest, ExecutionPolicy,
    RuntimeBudget, RuntimeError, RuntimePolicy, contains_mutation_keywords,
};
pub use shared_runtime::{
    CypherMutationSummary, CypherRecord, CypherResponse, CypherResponseData, CypherResponseStatus,
    CypherValidationError, CypherValue,
};
use thiserror::Error;

pub use knowledge_data::*;
pub use memory::*;
pub use opencti_routing::*;
pub use opencti_shadow::*;

const DEFAULT_WORKSPACE_ID: &str = "workspace--embedded-default";
const DEFAULT_SESSION_ID: &str = "session--embedded-default";
const DEFAULT_BUDGET_REF: &str = "budget--embedded-default";

/// Execution mode selected at the public engine boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineRequestMode {
    /// Infer read or mutation mode from the Cypher query.
    Auto,
    /// Execute with read-only validation.
    ReadOnly,
    /// Execute with mutation permission validation.
    Mutation,
    /// Validate the query without executing it.
    ValidateOnly,
}

/// A contextual Cypher request accepted by the public engine boundary.
///
/// Protocol adapters should translate transport payloads into this type rather
/// than constructing shared-runtime requests themselves. Missing context fields
/// inherit the engine defaults configured by [`CorroboreEngineBuilder`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineRequest {
    query: String,
    parameters: HashMap<String, CypherValue>,
    mode: EngineRequestMode,
    workspace_id: Option<String>,
    session_id: Option<String>,
    budget_ref: Option<String>,
}

/// Trusted runtime context for one atomic typed graph mutation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineMutationContext {
    workspace_id: String,
    session_id: String,
    budget_ref: String,
}

impl EngineMutationContext {
    /// Creates a mutation context supplied by the authenticated host boundary.
    pub fn new(
        workspace_id: impl Into<String>,
        session_id: impl Into<String>,
        budget_ref: impl Into<String>,
    ) -> Self {
        Self {
            workspace_id: workspace_id.into(),
            session_id: session_id.into(),
            budget_ref: budget_ref.into(),
        }
    }
}

impl EngineRequest {
    /// Creates a public engine request with no context overrides.
    pub fn new(query: impl Into<String>, mode: EngineRequestMode) -> Self {
        Self {
            query: query.into(),
            parameters: HashMap::new(),
            mode,
            workspace_id: None,
            session_id: None,
            budget_ref: None,
        }
    }

    /// Sets string parameters for the request.
    ///
    /// Every value binds as text. Use [`EngineRequest::with_typed_parameters`]
    /// when a placeholder stands for a row count or a numeric comparison, since a
    /// string bound there is a type error rather than a silently empty result.
    pub fn with_parameters(mut self, parameters: HashMap<String, String>) -> Self {
        self.parameters = parameters
            .into_iter()
            .map(|(name, value)| (name, CypherValue::String(value)))
            .collect();
        self
    }

    /// Sets parameters whose scalar types are preserved end to end.
    pub fn with_typed_parameters(mut self, parameters: HashMap<String, CypherValue>) -> Self {
        self.parameters = parameters;
        self
    }

    /// Overrides the engine workspace identifier for this request.
    pub fn with_workspace_id(mut self, workspace_id: impl Into<String>) -> Self {
        self.workspace_id = Some(workspace_id.into());
        self
    }

    /// Overrides the engine session identifier for this request.
    pub fn with_session_id(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    /// Overrides the engine budget reference for this request.
    pub fn with_budget_ref(mut self, budget_ref: impl Into<String>) -> Self {
        self.budget_ref = Some(budget_ref.into());
        self
    }
}

/// Error surface for embedded engine callers.
#[derive(Debug, Error)]
pub enum EngineError {
    /// Engine construction rejected a configuration field.
    #[error("invalid engine configuration for {field}: {reason}")]
    InvalidConfiguration {
        /// Configuration field that failed validation.
        field: &'static str,
        /// Human-readable rejection reason.
        reason: String,
    },
    /// The runtime gateway rejected or failed the request.
    #[error("runtime error: {0}")]
    Runtime(#[from] RuntimeError),
    /// The graph layer rejected the operation (including typed semantic seed
    /// resolution failures).
    #[error("graph error: {0}")]
    Graph(#[from] GraphError),
    /// Deterministic export planning or rendering failed.
    #[error("export failed: {0}")]
    Export(String),
    /// A configured durable engine adapter failed to load or commit graph state.
    #[error("engine persistence failed: {0}")]
    Persistence(String),
}

/// One bounded Knowledge Data graph projection prepared by a durable adapter.
#[derive(Clone, Debug)]
pub struct PreparedKnowledgeDataProjection {
    /// Hydrated operational graph selected through durable metadata.
    pub graph: Graph,
    /// Payload page-ins needed to build this projection.
    pub page_ins: u64,
    /// Payloads reused from the immediately preceding projection.
    pub cache_hits: u64,
    /// Candidates rejected from payload-free authorization metadata.
    pub authorization_denials: u64,
    /// Pre-ranked durable full-text page, when the storage adapter executed
    /// access-aware search before hydrating any graph payload.
    pub full_text_page: Option<FullTextSearchPage>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct PreparedKnowledgeDataExecution {
    page_ins: u64,
    cache_hits: u64,
    authorization_denials: u64,
    full_text_page: Option<FullTextSearchPage>,
}

/// Optional durable graph adapter used by standalone hosts.
///
/// The embedded engine remains persistence-agnostic unless a host explicitly
/// supplies this boundary.
pub trait EnginePersistence: std::fmt::Debug + Send {
    /// Loads the graph before the engine accepts requests.
    fn load_graph(&self) -> Result<Graph, String>;

    /// Optional metadata-only quality snapshot for paged persistence adapters.
    /// Snapshot adapters use the engine's already loaded graph by default.
    fn ingestion_metrics(&self) -> Result<Option<graph_core::IngestionMetrics>, String> {
        Ok(None)
    }

    /// Atomically commits the graph after a successful mutation.
    fn persist_graph(&mut self, graph: &Graph) -> Result<(), String>;

    /// Prepare a request-scoped graph projection before execution.
    ///
    /// Snapshot adapters use the default and keep their already loaded graph.
    /// Paged adapters return a bounded projection selected through durable
    /// catalog metadata, allowing startup to remain payload-cold.
    fn prepare_graph_for_request(&mut self, _query: &str) -> Result<Option<Graph>, String> {
        Ok(None)
    }

    /// Prepare an index-selected projection for one typed Knowledge Data read.
    ///
    /// Snapshot adapters inherit the compatibility implementation. Persistent
    /// adapters override this boundary to map typed identifiers, filters and
    /// graph budgets to compact storage indexes.
    fn prepare_knowledge_data_operation(
        &mut self,
        operation: &KnowledgeDataOperation,
        _access: &AccessContext,
    ) -> Result<Option<PreparedKnowledgeDataProjection>, String> {
        let query = match operation {
            KnowledgeDataOperation::Health(request) if request.verbose => {
                Some("MATCH (n) RETURN n")
            }
            KnowledgeDataOperation::GetById(_)
            | KnowledgeDataOperation::List(_)
            | KnowledgeDataOperation::Paginate(_)
            | KnowledgeDataOperation::Count(_) => Some("MATCH (n) RETURN n"),
            KnowledgeDataOperation::Aggregate(_) => Some("MATCH (n)-[r]->(m) RETURN n, r, m"),
            KnowledgeDataOperation::Neighbors(_)
            | KnowledgeDataOperation::Traverse(_)
            | KnowledgeDataOperation::Subgraph(_) => Some("MATCH (n)-[r]->(m) RETURN n, r, m"),
            _ => None,
        };
        query
            .map(|query| self.prepare_graph_for_request(query))
            .transpose()
            .map(|projection| {
                projection
                    .flatten()
                    .map(|graph| PreparedKnowledgeDataProjection {
                        graph,
                        page_ins: 0,
                        cache_hits: 0,
                        authorization_denials: 0,
                        full_text_page: None,
                    })
            })
    }

    /// Prepare the trusted workspace projection required by one high-level
    /// memory operation. Snapshot adapters already carry the full graph and
    /// return `None`; paged standalone adapters select workspace records and
    /// their evidence-bearing relationships before any memory payload is read.
    fn prepare_memory_operation(
        &mut self,
        _operation: &MemoryOperation,
        _context: &MemoryServiceContext,
    ) -> Result<Option<Graph>, String> {
        Ok(None)
    }

    /// Commit the changed record versions between two operational projections.
    ///
    /// The default preserves compatibility with snapshot-style embedded
    /// adapters. Standalone paged adapters override this method and append only
    /// the record-level delta under one WAL transaction.
    fn persist_graph_transition(
        &mut self,
        _previous: &Graph,
        current: &Graph,
    ) -> Result<(), String> {
        self.persist_graph(current)
    }

    /// Execute a typed transactional mutation directly against a host-owned
    /// durable store. Paged hosts use this boundary to keep idempotency,
    /// optimistic concurrency, WAL acknowledgement, and bulk results outside
    /// the generic in-memory query executor.
    fn execute_knowledge_data_mutation(
        &mut self,
        _operation: &KnowledgeDataOperation,
        _context: &RequestContext,
    ) -> Result<Option<KnowledgeDataResponse>, KnowledgeDataError> {
        Ok(None)
    }
}

/// Options controlling an embedded STIX bundle export.
#[derive(Clone, Debug)]
pub struct StixExportOptions {
    /// Logical snapshot identifier recorded in export metadata.
    pub snapshot_id: String,
    /// Transaction identifier recorded in export metadata.
    pub transaction_id: String,
    /// Exporter version recorded in export metadata.
    pub exporter_version: String,
    /// Export profile.
    pub profile: ExportProfile,
    /// Export strictness mode.
    pub mode: ExportMode,
    /// Explicitly retain overridable CTI validation findings as diagnostics
    /// while exporting records that pass lifecycle and structural checks.
    pub force: bool,
}

impl Default for StixExportOptions {
    fn default() -> Self {
        Self {
            snapshot_id: "snapshot--current".to_owned(),
            transaction_id: "transaction--embedded-export".to_owned(),
            exporter_version: "corrobore-engine-v0".to_owned(),
            profile: ExportProfile::StixMvp,
            mode: ExportMode::Strict,
            force: false,
        }
    }
}

/// Builder for [`CorroboreEngine`] instances.
#[derive(Debug, Default)]
pub struct CorroboreEngineBuilder {
    workspace_id: Option<String>,
    session_id: Option<String>,
    budget_ref: Option<String>,
    read_only: bool,
    runtime_policy: Option<RuntimePolicy>,
    budget: Option<RuntimeBudget>,
    execution_policy: Option<ExecutionPolicy>,
    persistence: Option<Box<dyn EnginePersistence>>,
}

impl CorroboreEngineBuilder {
    /// Sets the workspace identifier used for every request.
    pub fn workspace_id(mut self, value: impl Into<String>) -> Self {
        self.workspace_id = Some(value.into());
        self
    }

    /// Sets the session identifier used for every request.
    pub fn session_id(mut self, value: impl Into<String>) -> Self {
        self.session_id = Some(value.into());
        self
    }

    /// Sets the budget reference used for every request.
    pub fn budget_ref(mut self, value: impl Into<String>) -> Self {
        self.budget_ref = Some(value.into());
        self
    }

    /// Disallows mutations through the gateway policy when set.
    pub fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    /// Overrides the runtime policy.
    pub fn runtime_policy(mut self, policy: RuntimePolicy) -> Self {
        self.runtime_policy = Some(policy);
        self
    }

    /// Overrides the runtime budget.
    pub fn budget(mut self, budget: RuntimeBudget) -> Self {
        self.budget = Some(budget);
        self
    }

    /// Overrides the execution policy.
    pub fn execution_policy(mut self, policy: ExecutionPolicy) -> Self {
        self.execution_policy = Some(policy);
        self
    }

    /// Configures an explicit durable graph adapter for a standalone host.
    pub fn persistence(mut self, persistence: Box<dyn EnginePersistence>) -> Self {
        self.persistence = Some(persistence);
        self
    }

    /// Validates the configuration and builds the engine.
    pub fn build(self) -> Result<CorroboreEngine, EngineError> {
        let workspace_id = WorkspaceId::new(
            self.workspace_id
                .unwrap_or_else(|| DEFAULT_WORKSPACE_ID.to_owned()),
        )
        .map_err(|error| EngineError::InvalidConfiguration {
            field: "workspace_id",
            reason: error.to_string(),
        })?;

        let session_id = SessionId::new(
            self.session_id
                .unwrap_or_else(|| DEFAULT_SESSION_ID.to_owned()),
        )
        .map_err(|error| EngineError::InvalidConfiguration {
            field: "session_id",
            reason: error.to_string(),
        })?;

        let budget_ref = CypherBudgetRef::new(
            self.budget_ref
                .unwrap_or_else(|| DEFAULT_BUDGET_REF.to_owned()),
        )
        .map_err(|error| EngineError::InvalidConfiguration {
            field: "budget_ref",
            reason: error.to_string(),
        })?;

        let mut runtime_policy = self
            .runtime_policy
            .unwrap_or_else(RuntimePolicy::strict_default);
        if self.read_only {
            runtime_policy.mutation_permissions = false;
        }

        let budget = self.budget.unwrap_or_else(RuntimeBudget::strict_default);

        // Mirror CypherGateway::strict_default: expose the full executor and
        // let the request mode plus runtime policy decide write permission.
        let execution_policy = self.execution_policy.unwrap_or(ExecutionPolicy {
            read_only_by_default: false,
        });

        let persistence = self.persistence;
        let graph = persistence
            .as_ref()
            .map_or_else(|| Ok(Graph::new()), |adapter| adapter.load_graph())
            .map_err(EngineError::Persistence)?;

        Ok(CorroboreEngine {
            gateway: CypherGateway::with_graph(
                runtime_policy.clone(),
                budget,
                execution_policy,
                graph,
            ),
            workspace_id,
            session_id,
            budget_ref,
            runtime_policy,
            persistence,
            core_read_metrics: CoreReadMetrics::default(),
            pipeline_metrics: graph_core::StageMetricsRegistry::default(),
            security_audit_events: Vec::new(),
            advanced_query_cache: BTreeMap::new(),
            memory_recall_traces: BTreeMap::new(),
        })
    }
}

/// Embedded, in-process Corrobore engine.
#[derive(Debug)]
pub struct CorroboreEngine {
    gateway: CypherGateway,
    workspace_id: WorkspaceId,
    session_id: SessionId,
    budget_ref: CypherBudgetRef,
    runtime_policy: RuntimePolicy,
    persistence: Option<Box<dyn EnginePersistence>>,
    core_read_metrics: CoreReadMetrics,
    pipeline_metrics: graph_core::StageMetricsRegistry,
    security_audit_events: Vec<SecurityAuditEvent>,
    advanced_query_cache: BTreeMap<String, AggregationResult>,
    memory_recall_traces: BTreeMap<String, RecallResult>,
}

impl CorroboreEngine {
    /// Builds an engine with strict default policies and budgets.
    pub fn strict_default() -> Self {
        Self::builder()
            .build()
            .expect("default embedded engine configuration is statically valid")
    }

    /// Returns a builder for custom engine configuration.
    pub fn builder() -> CorroboreEngineBuilder {
        CorroboreEngineBuilder::default()
    }

    /// Record a completed instrumented stage batch without mutating evidence.
    /// Identity retries are exact and per-run counts remain bounded.
    pub fn record_pipeline_stage(
        &mut self,
        run_id: &str,
        measurement: graph_core::StageMeasurement,
    ) -> Result<graph_core::PipelineStageReport, graph_core::StageMetricError> {
        self.pipeline_metrics.record(run_id, measurement)?;
        self.pipeline_metrics.report(run_id)
    }

    /// Emit a versioned report from this engine instance's retained stage measurements.
    pub fn pipeline_stage_report(
        &self,
        run_id: &str,
    ) -> Result<graph_core::PipelineStageReport, graph_core::StageMetricError> {
        self.pipeline_metrics.report(run_id)
    }

    /// Return cumulative low-cardinality metrics for fundamental read classes.
    pub fn core_read_metrics(&self) -> &CoreReadMetrics {
        &self.core_read_metrics
    }

    /// Return payload-free OpenCTI authorization audit summaries.
    pub fn security_audit_events(&self) -> &[SecurityAuditEvent] {
        &self.security_audit_events
    }

    fn record_security_audit_event(&mut self, event: SecurityAuditEvent) {
        const MAX_EVENTS: usize = 1_024;
        if self.security_audit_events.len() == MAX_EVENTS {
            self.security_audit_events.remove(0);
        }
        self.security_audit_events.push(event);
    }

    /// Executes one contextual request through the public engine boundary.
    pub fn execute_request(
        &mut self,
        request: EngineRequest,
    ) -> Result<CypherResponse, EngineError> {
        if contains_mutation_keywords(&request.query) {
            self.advanced_query_cache.clear();
        }
        self.prepare_graph_for_query(&request.query)?;
        let workspace_id = match request.workspace_id {
            Some(value) => {
                WorkspaceId::new(value).map_err(|error| EngineError::InvalidConfiguration {
                    field: "workspace_id",
                    reason: error.to_string(),
                })?
            }
            None => self.workspace_id.clone(),
        };
        let session_id = match request.session_id {
            Some(value) => {
                SessionId::new(value).map_err(|error| EngineError::InvalidConfiguration {
                    field: "session_id",
                    reason: error.to_string(),
                })?
            }
            None => self.session_id.clone(),
        };
        let budget_ref = match request.budget_ref {
            Some(value) => {
                CypherBudgetRef::new(value).map_err(|error| EngineError::InvalidConfiguration {
                    field: "budget_ref",
                    reason: error.to_string(),
                })?
            }
            None => self.budget_ref.clone(),
        };
        let mode = match request.mode {
            EngineRequestMode::Auto if contains_mutation_keywords(&request.query) => {
                shared_runtime::CypherRequestMode::Mutation
            }
            EngineRequestMode::Auto | EngineRequestMode::ReadOnly => {
                shared_runtime::CypherRequestMode::ReadOnly
            }
            EngineRequestMode::Mutation => shared_runtime::CypherRequestMode::Mutation,
            EngineRequestMode::ValidateOnly => shared_runtime::CypherRequestMode::ValidateOnly,
        };
        let durable_mutation =
            mode == shared_runtime::CypherRequestMode::Mutation && self.persistence.is_some();
        let runtime_request = CypherRequest::new(
            request.query,
            CypherParameters::typed(request.parameters),
            mode,
            workspace_id,
            session_id,
            budget_ref,
        )?;

        let previous_graph = durable_mutation.then(|| self.gateway.graph().clone());
        let response = self.gateway.execute(&runtime_request)?;
        if durable_mutation && response.status == CypherResponseStatus::Success {
            let committed_graph = self.gateway.graph().clone();
            if let Some(adapter) = self.persistence.as_mut()
                && let Err(error) = adapter.persist_graph_transition(
                    previous_graph
                        .as_ref()
                        .expect("durable mutation captured rollback"),
                    &committed_graph,
                )
            {
                self.gateway
                    .replace_graph(previous_graph.expect("durable mutation captured rollback"));
                return Err(EngineError::Persistence(error));
            }
        }
        Ok(response)
    }

    fn prepare_graph_for_query(&mut self, query: &str) -> Result<(), EngineError> {
        if let Some(adapter) = self.persistence.as_mut()
            && let Some(projection) = adapter
                .prepare_graph_for_request(query)
                .map_err(EngineError::Persistence)?
        {
            self.advanced_query_cache.clear();
            self.gateway.replace_graph(projection);
        }
        Ok(())
    }

    fn prepare_knowledge_data_operation(
        &mut self,
        operation: &KnowledgeDataOperation,
        access: &AccessContext,
    ) -> Result<Option<PreparedKnowledgeDataExecution>, EngineError> {
        if let Some(adapter) = self.persistence.as_mut()
            && let Some(projection) = adapter
                .prepare_knowledge_data_operation(operation, access)
                .map_err(EngineError::Persistence)?
        {
            self.advanced_query_cache.clear();
            let stats = PreparedKnowledgeDataExecution {
                page_ins: projection.page_ins,
                cache_hits: projection.cache_hits,
                authorization_denials: projection.authorization_denials,
                full_text_page: projection.full_text_page,
            };
            self.gateway.replace_graph(projection.graph);
            return Ok(Some(stats));
        }
        Ok(None)
    }

    /// Delegate one typed write to the host persistence boundary, mapping
    /// absence to a stable availability error and preserving typed storage
    /// conflicts without exposing payloads in diagnostics.
    fn execute_knowledge_data_mutation(
        &mut self,
        operation: &KnowledgeDataOperation,
        context: &RequestContext,
    ) -> Result<KnowledgeDataResponse, KnowledgeDataError> {
        let Some(persistence) = self.persistence.as_mut() else {
            return Err(KnowledgeDataError {
                code: KnowledgeDataErrorCode::BackendUnavailable,
                message: "transactional writes require durable host persistence".to_owned(),
                retryable: false,
            });
        };
        persistence
            .execute_knowledge_data_mutation(operation, context)?
            .ok_or_else(|| KnowledgeDataError {
                code: KnowledgeDataErrorCode::BackendUnavailable,
                message: "durable host does not expose transactional writes".to_owned(),
                retryable: false,
            })
    }

    /// Executes a query, auto-detecting read or mutation mode.
    pub fn execute(&mut self, query: &str) -> Result<CypherResponse, EngineError> {
        self.execute_with_params(query, HashMap::new())
    }

    /// Executes a parameterized query, auto-detecting read or mutation mode.
    pub fn execute_with_params(
        &mut self,
        query: &str,
        params: HashMap<String, String>,
    ) -> Result<CypherResponse, EngineError> {
        self.execute_request(
            EngineRequest::new(query, EngineRequestMode::Auto).with_parameters(params),
        )
    }

    /// Executes a query in read-only mode.
    pub fn read(&mut self, query: &str) -> Result<CypherResponse, EngineError> {
        self.read_with_params(query, HashMap::new())
    }

    /// Executes a parameterized query in read-only mode.
    pub fn read_with_params(
        &mut self,
        query: &str,
        params: HashMap<String, String>,
    ) -> Result<CypherResponse, EngineError> {
        self.execute_request(
            EngineRequest::new(query, EngineRequestMode::ReadOnly).with_parameters(params),
        )
    }

    /// Executes a query in mutation mode.
    pub fn write(&mut self, query: &str) -> Result<CypherResponse, EngineError> {
        self.write_with_params(query, HashMap::new())
    }

    /// Executes a parameterized query in mutation mode.
    pub fn write_with_params(
        &mut self,
        query: &str,
        params: HashMap<String, String>,
    ) -> Result<CypherResponse, EngineError> {
        self.execute_request(
            EngineRequest::new(query, EngineRequestMode::Mutation).with_parameters(params),
        )
    }

    /// Returns an immutable view of the runtime graph.
    pub fn graph(&self) -> &Graph {
        self.gateway.graph()
    }

    /// Read ingestion quality without paging in canonical graph payloads.
    pub fn ingestion_metrics(&self) -> Result<graph_core::IngestionMetrics, EngineError> {
        if let Some(adapter) = &self.persistence
            && let Some(metrics) = adapter
                .ingestion_metrics()
                .map_err(EngineError::Persistence)?
        {
            return Ok(metrics);
        }
        self.graph().ingestion_metrics().map_err(EngineError::Graph)
    }

    /// Hydrates the complete current graph from a configured paged persistence
    /// adapter before a host performs a graph-wide operation outside Cypher.
    ///
    /// Snapshot-backed and ephemeral engines keep their current graph. HTTP
    /// validation and export use this boundary after restart so they do not
    /// inspect the intentionally payload-cold startup graph.
    pub fn hydrate_full_graph(&mut self) -> Result<(), EngineError> {
        self.prepare_graph_for_query("MATCH (n)-[r]->(m) RETURN n, r, m")
    }

    /// Applies one typed graph mutation atomically and persists it as one
    /// transition when durable storage is configured.
    pub fn mutate_graph_atomically<T, F>(
        &mut self,
        context: EngineMutationContext,
        mutation: F,
    ) -> Result<T, EngineError>
    where
        F: FnOnce(&mut Graph) -> Result<T, GraphError>,
    {
        WorkspaceId::new(context.workspace_id).map_err(|error| {
            EngineError::InvalidConfiguration {
                field: "workspace_id",
                reason: error.to_string(),
            }
        })?;
        SessionId::new(context.session_id).map_err(|error| EngineError::InvalidConfiguration {
            field: "session_id",
            reason: error.to_string(),
        })?;
        CypherBudgetRef::new(context.budget_ref).map_err(|error| {
            EngineError::InvalidConfiguration {
                field: "budget_ref",
                reason: error.to_string(),
            }
        })?;
        if !self.runtime_policy.mutation_permissions
            || !self
                .runtime_policy
                .allowed_request_modes
                .contains(&shared_runtime::CypherRequestMode::Mutation)
        {
            return Err(EngineError::InvalidConfiguration {
                field: "mutation_policy",
                reason: "runtime policy disallows typed graph mutations".to_owned(),
            });
        }

        // Durable paged hosts start payload-cold. Hydrate the complete current
        // graph before deriving an idempotent typed transition.
        self.prepare_graph_for_query("MATCH (n)-[r]->(m) RETURN n, r, m")?;
        let previous = self.gateway.graph().clone();
        let mut next = previous.clone();
        let result = mutation(&mut next)?;
        if let Some(adapter) = self.persistence.as_mut()
            && let Err(error) = adapter.persist_graph_transition(&previous, &next)
        {
            return Err(EngineError::Persistence(error));
        }
        self.advanced_query_cache.clear();
        self.gateway.replace_graph(next);
        Ok(result)
    }

    /// Exports the current graph as a deterministic STIX subset bundle.
    pub fn export_stix_bundle(
        &self,
        options: &StixExportOptions,
    ) -> Result<StixExportBundle, EngineError> {
        self.export_stix_bundle_with_findings(options, &[])
    }

    /// Exports the current graph after applying caller-supplied, graph-addressed
    /// validation findings to the deterministic CTI selection.
    ///
    /// HTTP hosts use this boundary to pass the verdict of their licensed CTI
    /// provider. Embedded callers that operate their own provider can do the
    /// same without coupling the engine crate to one provider deployment.
    pub fn export_stix_bundle_with_findings(
        &self,
        options: &StixExportOptions,
        findings: &[ValidationErrorRecord],
    ) -> Result<StixExportBundle, EngineError> {
        let transaction_id =
            TransactionId::new(options.transaction_id.clone()).map_err(|error| {
                EngineError::InvalidConfiguration {
                    field: "transaction_id",
                    reason: error.to_string(),
                }
            })?;

        let metadata = ExportMetadata::new(
            options.snapshot_id.clone(),
            transaction_id,
            options.exporter_version.clone(),
            options.profile.clone(),
            options.mode,
            None,
        )
        .map_err(|error| EngineError::Export(error.to_string()))?;

        let plan = build_deterministic_export_plan_with_options(
            self.gateway.graph(),
            metadata,
            findings,
            export_plan_options(options),
        )
        .map_err(|error| EngineError::Export(error.to_string()))?;

        Ok(export_stix_subset_bundle(self.gateway.graph(), &plan))
    }

    /// Resolves a natural-language objective into ranked seed node candidates.
    ///
    /// Uses hybrid retrieval (lexical relevance plus graph signals) with the
    /// engine's workspace, a cross-domain profile, and no score threshold.
    /// Use [`CorroboreEngine::seed_search_with_request`] for full control.
    ///
    /// ```
    /// use corrobore_engine::CorroboreEngine;
    ///
    /// let mut engine = CorroboreEngine::strict_default();
    /// engine.write("CREATE (n:Campaign {name: 'acme phishing campaign'})")?;
    ///
    /// let seeds = engine.seed_search("phishing campaign", 5)?;
    /// assert_eq!(seeds.seed_candidates().len(), 1);
    /// # Ok::<(), corrobore_engine::EngineError>(())
    /// ```
    pub fn seed_search(
        &self,
        objective: &str,
        top_k: usize,
    ) -> Result<SemanticSeedQueryResponse, EngineError> {
        let request = SemanticSeedQueryRequest::new(
            objective,
            self.workspace_id.clone(),
            SemanticDomainProfile::CrossDomainInvestigation,
            SemanticSeedRetrievalMode::Hybrid,
            top_k,
            0.0,
        )?;

        self.seed_search_with_request(&request)
    }

    /// Resolves a fully specified semantic seed query against the embedded
    /// graph.
    pub fn seed_search_with_request(
        &self,
        request: &SemanticSeedQueryRequest,
    ) -> Result<SemanticSeedQueryResponse, EngineError> {
        let resolver = GraphSemanticSeedResolver::new(self.gateway.graph());
        Ok(resolver.resolve(request)?)
    }

    /// Returns the workspace identifier used for requests.
    pub fn workspace_id(&self) -> &str {
        self.workspace_id.as_str()
    }

    /// Returns the session identifier used for requests.
    pub fn session_id(&self) -> &str {
        self.session_id.as_str()
    }
}

fn export_plan_options(options: &StixExportOptions) -> ExportPlanOptions {
    // Map `force` only to semantic validation override behavior. Lifecycle,
    // identity, evidence-integrity, and endpoint findings remain enforced by
    // the planner regardless of this option.
    ExportPlanOptions::default().with_force_validation(options.force)
}

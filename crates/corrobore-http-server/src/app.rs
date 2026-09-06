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
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::{
    Router,
    body::Body,
    extract::DefaultBodyLimit,
    http::Request,
    middleware,
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use corrobore_engine::{
    CorroboreEngine, EnginePersistence, GraphDirection, KnowledgeDataOperation,
    OpenCtiReadRoutingRuntime, PreparedKnowledgeDataProjection, ReadFilter, ReadFilterOperator,
    ReadPredicate, ReadRoutingMode, ReadRoutingPolicy, ReadRoutingThresholds,
    file_content_query_from_search_request, full_text_query_from_search_request,
};
use corrobore_engine::{DivergenceBaseline, ShadowSamplingPolicy};
use graph_storage::{
    CanonicalAdjacencyProjection, CanonicalEngineStore, CanonicalProjectionRequest,
    CanonicalPropertyFilter, CanonicalPropertyOperator, CanonicalStoreOptions,
    DurableTransactionId, FileBackedGraphStore, GraphId, GraphStorageError,
    GraphStoreRecoveryReport, OperationReadiness, RecordFormat, StorageManifest, StorageTimestamp,
    StorageVersion, create_storage_root, derived_index_rebuild_status, open_storage_root,
    read_storage_manifest, rebuild_derived_indexes,
};
use opencti_adapter::{BulkLimits, ReconciliationLimits, WriteLimits};
use thiserror::Error;
use tower_governor::{
    GovernorLayer, governor::GovernorConfigBuilder, key_extractor::GlobalKeyExtractor,
};
use tower_http::{
    LatencyUnit,
    trace::{DefaultOnFailure, DefaultOnRequest, DefaultOnResponse, TraceLayer},
};
use tracing::Level;
use uuid::Uuid;

use crate::{
    ServerLifecycle,
    auth::require_bearer_auth,
    config::{ServerConfig, StorageMode},
    correlation::{RequestCorrelationId, correlate_request},
    error::ApiError,
    explorer_timeline::ExplorerTimelineStore,
    handlers::{
        cypher::{execute_cypher, execute_read_cypher, execute_write_cypher},
        database_operations::{create_snapshot, database_operation_status, rebuild_indexes},
        domain_provider_status::admin_domain_provider_status,
        domain_validate::validate_domain,
        explorer::{explorer_graph, explorer_sessions, explorer_timeline},
        export::export_stix,
        health::health,
        import::{import_stix_bundle, import_stix_bundle_file},
        license::{admin_license_status, license_status},
        memory::execute_memory_operation,
        metrics::metrics,
        opencti_files::execute_opencti_file_command,
        opencti_reconciliation::{execute_opencti_reconciliation, opencti_reconciliation_status},
        opencti_routing::{
            execute_opencti_routed_read, opencti_routing_decisions, opencti_routing_rollback,
        },
        opencti_shadow::{execute_opencti_shadow_read, opencti_shadow_reports},
        opencti_sync::{apply_opencti_sync_batch, opencti_sync_status},
        opencti_write::{
            drain_opencti_projection, execute_opencti_write, opencti_write_status,
            reconstruct_opencti_reference, suspend_opencti_writes,
            transition_opencti_write_authority,
        },
        operational::{liveness, readiness, version},
        seed::seed_search,
        session::{session_health, session_logs, start_session, stop_session},
        stix_validate::validate_stix,
    },
    opencti_reconciliation::OpenCtiReconciliationRuntime,
    opencti_shadow::OpenCtiShadowRuntime,
    opencti_sync::OpenCtiSyncRuntime,
    opencti_write::OpenCtiWriteRuntime,
    security::OperationalEndpointPolicy,
    session_runtime::SessionRuntime,
    storage_ownership::{DataDirectoryOwnership, DataDirectoryOwnershipError},
    web::attach_web_delivery,
};

#[derive(Clone, Debug)]
pub enum RuntimeStoreProvider {
    Ephemeral,
    Persistent(Box<PersistentRuntimeStore>),
}

#[derive(Clone, Debug)]
pub struct PersistentRuntimeStore {
    pub root_path: PathBuf,
    pub store: FileBackedGraphStore,
    pub recovery_report: GraphStoreRecoveryReport,
    pub manifest: StorageManifest,
    pub canonical_store: Arc<Mutex<CanonicalEngineStore>>,
    _ownership: Arc<DataDirectoryOwnership>,
}

#[derive(Debug, Error)]
pub enum AppStateInitError {
    #[error("persistent mode requires CORROBORE_STORAGE_DIR")]
    MissingPersistentStorageDir,
    #[error("failed to initialize persistent storage at {path}: {reason}")]
    PersistentStorageInitFailed { path: String, reason: String },
    #[error("persistent storage ownership conflict at {path}: {reason}")]
    PersistentStorageOwnershipConflict { path: String, reason: String },
    #[error("failed to initialize persistent storage ownership at {path}: {reason}")]
    PersistentStorageOwnershipFailed { path: String, reason: String },
    #[error("persistent storage is incompatible at {path}: {reason}")]
    PersistentStorageIncompatible { path: String, reason: String },
    #[error("persistent storage recovery failed at {path}: {reason}")]
    PersistentStorageRecoveryFailed { path: String, reason: String },
    #[error("failed to initialize enterprise domain providers: {reason}")]
    DomainProviderInitFailed { reason: String },
    #[error("failed to register domain provider verifiers: {reason}")]
    DomainVerifierRegistrationFailed { reason: String },
    #[error("failed to restore OpenCTI synchronization state: {reason}")]
    OpenCtiSyncStateFailed { reason: String },
    #[error("failed to restore OpenCTI shadow-read state: {reason}")]
    OpenCtiShadowStateFailed { reason: String },
    #[error("failed to restore OpenCTI read-routing state: {reason}")]
    OpenCtiRoutingStateFailed { reason: String },
    #[error("failed to restore OpenCTI transactional-write state: {reason}")]
    OpenCtiWriteStateFailed { reason: String },
    #[error("failed to restore OpenCTI reconciliation state: {reason}")]
    OpenCtiReconciliationStateFailed { reason: String },
}

#[derive(Clone)]
pub struct AppState {
    /// Public engine shared by embedded and protocol entry points.
    pub engine: Arc<Mutex<CorroboreEngine>>,
    pub runtime_store: RuntimeStoreProvider,
    pub sessions: Arc<Mutex<SessionRuntime>>,
    pub timeline: Arc<Mutex<ExplorerTimelineStore>>,
    pub config: Arc<ServerConfig>,
    pub lifecycle: Arc<ServerLifecycle>,
    pub opencti_sync: Arc<Mutex<OpenCtiSyncRuntime>>,
    pub opencti_shadow: Arc<Mutex<OpenCtiShadowRuntime>>,
    pub opencti_routing: Arc<Mutex<OpenCtiReadRoutingRuntime>>,
    pub opencti_write: Arc<Mutex<OpenCtiWriteRuntime>>,
    pub opencti_reconciliation: Arc<Mutex<OpenCtiReconciliationRuntime>>,
    pub database_operations: Arc<Mutex<crate::database_operations::DatabaseOperationMetrics>>,
    pub stix_import_metrics: Arc<Mutex<crate::handlers::import::ImportRuntimeMetrics>>,
    pub opencti_write_semaphore: Arc<tokio::sync::Semaphore>,
    pub(crate) domain_providers: Option<Arc<crate::enterprise::registry::DomainProviderRegistry>>,
    /// Verifiers available to governance workflows, including adapters
    /// registered by loaded domain providers.
    pub verifier_registry: Arc<graph_core::VerifierRegistry>,
    pub started_at: Instant,
}

impl AppState {
    pub fn new(config: ServerConfig) -> Result<Self, AppStateInitError> {
        let runtime_store = initialize_runtime_store(&config)?;
        let write_state_path = match &runtime_store {
            RuntimeStoreProvider::Persistent(runtime) => Some(
                runtime
                    .root_path
                    .join("runtime")
                    .join("opencti-write-state.json"),
            ),
            RuntimeStoreProvider::Ephemeral => None,
        };
        let opencti_write = Arc::new(Mutex::new(
            OpenCtiWriteRuntime::open(
                write_state_path,
                WriteLimits {
                    max_operations: config.opencti_sync_max_operations,
                    max_payload_bytes: config.import_max_body_bytes,
                },
                config.opencti_sync_max_replay_identities,
            )
            .map_err(|reason| AppStateInitError::OpenCtiWriteStateFailed { reason })?,
        ));
        if let RuntimeStoreProvider::Persistent(runtime) = &runtime_store {
            let mut write_runtime =
                opencti_write
                    .lock()
                    .map_err(|_| AppStateInitError::OpenCtiWriteStateFailed {
                        reason: "OpenCTI write runtime lock is poisoned".to_owned(),
                    })?;
            let mut canonical_store = runtime.canonical_store.lock().map_err(|_| {
                AppStateInitError::OpenCtiWriteStateFailed {
                    reason: "canonical graph store lock is poisoned".to_owned(),
                }
            })?;
            write_runtime
                .recover_projection_outbox(&mut canonical_store)
                .map_err(|reason| AppStateInitError::OpenCtiWriteStateFailed { reason })?;
        }
        let reconciliation_state_path = match &runtime_store {
            RuntimeStoreProvider::Persistent(runtime) => Some(
                runtime
                    .root_path
                    .join("runtime")
                    .join("opencti-reconciliation-state.json"),
            ),
            RuntimeStoreProvider::Ephemeral => None,
        };
        let opencti_reconciliation = Arc::new(Mutex::new(
            OpenCtiReconciliationRuntime::open(
                reconciliation_state_path,
                ReconciliationLimits {
                    max_records: config.opencti_sync_max_replay_identities,
                    max_payload_bytes: config.import_max_body_bytes,
                },
                config.opencti_sync_max_replay_identities,
            )
            .map_err(|reason| AppStateInitError::OpenCtiReconciliationStateFailed { reason })?,
        ));
        let engine = initialize_engine(
            &runtime_store,
            config.storage_require_fsync,
            Arc::clone(&opencti_write),
        )?;
        let sync_state_path = match &runtime_store {
            RuntimeStoreProvider::Persistent(runtime) => Some(
                runtime
                    .root_path
                    .join("runtime")
                    .join("opencti-sync-state.json"),
            ),
            RuntimeStoreProvider::Ephemeral => None,
        };
        let opencti_sync = OpenCtiSyncRuntime::open(
            sync_state_path,
            BulkLimits {
                max_operations: config.opencti_sync_max_operations,
                max_payload_bytes: config.import_max_body_bytes,
                max_replay_identities: config.opencti_sync_max_replay_identities,
            },
        )
        .map_err(|reason| AppStateInitError::OpenCtiSyncStateFailed { reason })?;
        let shadow_state_path = match &runtime_store {
            RuntimeStoreProvider::Persistent(runtime) => Some(
                runtime
                    .root_path
                    .join("runtime")
                    .join("opencti-shadow-reports.json"),
            ),
            RuntimeStoreProvider::Ephemeral => None,
        };
        let shadow_policy = config
            .opencti_shadow
            .sampling_policy_file
            .as_deref()
            .map(read_shadow_json::<ShadowSamplingPolicy>)
            .transpose()
            .map_err(|reason| AppStateInitError::OpenCtiShadowStateFailed { reason })?
            .unwrap_or(ShadowSamplingPolicy {
                default_percentage_basis_points: config.opencti_shadow.sample_basis_points,
                rules: Vec::new(),
            });
        let shadow_baselines = config
            .opencti_shadow
            .baseline_file
            .as_deref()
            .map(read_shadow_json::<Vec<DivergenceBaseline>>)
            .transpose()
            .map_err(|reason| AppStateInitError::OpenCtiShadowStateFailed { reason })?
            .unwrap_or_default();
        let opencti_shadow = OpenCtiShadowRuntime::open(
            shadow_state_path,
            shadow_policy,
            shadow_baselines,
            config.opencti_shadow.max_concurrency,
            config.opencti_shadow.max_reports,
        )
        .map_err(|reason| AppStateInitError::OpenCtiShadowStateFailed { reason })?;
        let routing_state_path = match &runtime_store {
            RuntimeStoreProvider::Persistent(runtime) => Some(
                runtime
                    .root_path
                    .join("runtime")
                    .join("opencti-read-routing.json"),
            ),
            RuntimeStoreProvider::Ephemeral => None,
        };
        let routing_policy = config
            .opencti_shadow
            .routing_policy_file
            .as_deref()
            .map(read_shadow_json::<ReadRoutingPolicy>)
            .transpose()
            .map_err(|reason| AppStateInitError::OpenCtiRoutingStateFailed { reason })?
            .unwrap_or_else(default_read_routing_policy);
        if config.opencti_elastic_free && routing_policy.mode != ReadRoutingMode::PrimaryReads {
            return Err(AppStateInitError::OpenCtiRoutingStateFailed {
                reason: "Elastic-free mode requires a primary_reads routing policy".to_owned(),
            });
        }
        let opencti_routing = OpenCtiReadRoutingRuntime::open(
            routing_state_path,
            routing_policy,
            config.opencti_shadow.routing_max_audits,
        )
        .map_err(|reason| AppStateInitError::OpenCtiRoutingStateFailed { reason })?;
        let domain_providers = initialize_domain_providers(&config)?;
        let verifier_registry = initialize_verifier_registry(domain_providers.as_ref())?;
        let timeline = ExplorerTimelineStore::new(&config.session_store_dir);
        let lifecycle = Arc::new(ServerLifecycle::initializing());
        let opencti_write_semaphore = Arc::new(tokio::sync::Semaphore::new(
            config.opencti_shadow.max_concurrency,
        ));
        let state = Self {
            engine: Arc::new(Mutex::new(engine)),
            runtime_store,
            sessions: Arc::new(Mutex::new(SessionRuntime::new(
                config.session_store_dir.clone(),
                config.session_idle_ttl_ms,
            ))),
            timeline: Arc::new(Mutex::new(timeline)),
            config: Arc::new(config),
            lifecycle: Arc::clone(&lifecycle),
            opencti_sync: Arc::new(Mutex::new(opencti_sync)),
            opencti_shadow: Arc::new(Mutex::new(opencti_shadow)),
            opencti_routing: Arc::new(Mutex::new(opencti_routing)),
            opencti_write,
            opencti_reconciliation,
            database_operations: Arc::new(Mutex::new(Default::default())),
            stix_import_metrics: Arc::new(Mutex::new(Default::default())),
            opencti_write_semaphore,
            domain_providers,
            verifier_registry,
            started_at: Instant::now(),
        };
        lifecycle.mark_ready();
        Ok(state)
    }
}

fn initialize_verifier_registry(
    domain_providers: Option<&Arc<crate::enterprise::registry::DomainProviderRegistry>>,
) -> Result<Arc<graph_core::VerifierRegistry>, AppStateInitError> {
    let mut registry = graph_core::VerifierRegistry::new();
    if let Some(providers) = domain_providers {
        providers
            .register_claim_verifiers(&mut registry)
            .map_err(
                |error| AppStateInitError::DomainVerifierRegistrationFailed {
                    reason: error.to_string(),
                },
            )?;
    }
    Ok(Arc::new(registry))
}

fn default_read_routing_policy() -> ReadRoutingPolicy {
    ReadRoutingPolicy {
        policy_version: "reference-only-v1".to_owned(),
        mode: ReadRoutingMode::ReferenceOnly,
        default_percentage_basis_points: 0,
        rules: Vec::new(),
        thresholds: ReadRoutingThresholds {
            max_error_rate_basis_points: 100,
            max_latency_p95_ms: 2_000,
            minimum_soak_requests: 10_000,
        },
    }
}

fn read_shadow_json<T: serde::de::DeserializeOwned>(path: &str) -> Result<T, String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("failed to read shadow configuration {path}: {error}"))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("failed to parse shadow configuration {path}: {error}"))
}

#[derive(Debug)]
struct PersistentEnginePersistence {
    store: Arc<Mutex<CanonicalEngineStore>>,
    opencti_write: Arc<Mutex<OpenCtiWriteRuntime>>,
}

impl EnginePersistence for PersistentEnginePersistence {
    fn ingestion_metrics(&self) -> Result<Option<graph_core::IngestionMetrics>, String> {
        self.store
            .lock()
            .map_err(|_| "canonical graph store lock is poisoned".to_owned())?
            .ingestion_metrics()
            .map(Some)
            .map_err(|error| error.to_string())
    }

    fn load_graph(&self) -> Result<graph_core::Graph, String> {
        // Persistent startup stays metadata-only. The first request asks the
        // adapter for a bounded pager-backed projection.
        Ok(graph_core::Graph::new())
    }

    fn persist_graph(&mut self, _graph: &graph_core::Graph) -> Result<(), String> {
        Err("paged persistence requires a graph transition".to_owned())
    }

    fn prepare_graph_for_request(
        &mut self,
        query: &str,
    ) -> Result<Option<graph_core::Graph>, String> {
        let request = canonical_projection_for_query(query);
        self.store
            .lock()
            .map_err(|_| "canonical graph store lock is poisoned".to_owned())?
            .load_projection(request)
            .map(Some)
            .map_err(|error| error.to_string())
    }

    fn prepare_knowledge_data_operation(
        &mut self,
        operation: &KnowledgeDataOperation,
        access: &corrobore_engine::AccessContext,
    ) -> Result<Option<PreparedKnowledgeDataProjection>, String> {
        if let KnowledgeDataOperation::Search(request) = operation {
            let mut store = self
                .store
                .lock()
                .map_err(|_| "canonical graph store lock is poisoned".to_owned())?;
            let page = if let Some(query) =
                file_content_query_from_search_request(request).map_err(|error| error.message)?
            {
                store
                    .search_file_content(&query, access)
                    .map_err(|error| error.to_string())?
            } else {
                let query =
                    full_text_query_from_search_request(request).map_err(|error| error.message)?;
                store
                    .search_full_text(&query, access)
                    .map_err(|error| error.to_string())?
            };
            return Ok(Some(PreparedKnowledgeDataProjection {
                graph: graph_core::Graph::new(),
                page_ins: 0,
                cache_hits: 0,
                authorization_denials: page.authorization_denials,
                full_text_page: Some(page),
            }));
        }
        let Some(request) = canonical_projection_for_knowledge_data(operation)
            .map(|request| request.with_access_context(access.clone()))
        else {
            return Ok(None);
        };
        let mut store = self
            .store
            .lock()
            .map_err(|_| "canonical graph store lock is poisoned".to_owned())?;
        let graph = store
            .load_projection(request)
            .map_err(|error| error.to_string())?;
        let stats = store.last_projection_stats().clone();
        Ok(Some(PreparedKnowledgeDataProjection {
            graph,
            page_ins: stats.page_ins,
            cache_hits: stats.cache_hits,
            authorization_denials: stats.authorization_denials,
            full_text_page: None,
        }))
    }

    fn prepare_memory_operation(
        &mut self,
        _operation: &corrobore_engine::MemoryOperation,
        context: &corrobore_engine::MemoryServiceContext,
    ) -> Result<Option<graph_core::Graph>, String> {
        let request = CanonicalProjectionRequest::all_nodes()
            .with_relationships(None)
            .with_property_filters([CanonicalPropertyFilter {
                field: "corrobore.memory.workspace".to_owned(),
                operator: CanonicalPropertyOperator::Equal,
                value: Some(serde_json::Value::String(context.workspace_id.clone())),
            }]);
        self.store
            .lock()
            .map_err(|_| "canonical graph store lock is poisoned".to_owned())?
            .load_projection(request)
            .map(Some)
            .map_err(|error| error.to_string())
    }

    fn persist_graph_transition(
        &mut self,
        previous: &graph_core::Graph,
        current: &graph_core::Graph,
    ) -> Result<(), String> {
        let transaction_id =
            DurableTransactionId::new(format!("tx--standalone-{}", Uuid::new_v4()))
                .map_err(|error| error.to_string())?;
        self.store
            .lock()
            .map_err(|_| "canonical graph store lock is poisoned".to_owned())?
            .commit_transition(previous, current, transaction_id, None)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn execute_knowledge_data_mutation(
        &mut self,
        operation: &KnowledgeDataOperation,
        context: &corrobore_engine::RequestContext,
    ) -> Result<Option<corrobore_engine::KnowledgeDataResponse>, corrobore_engine::KnowledgeDataError>
    {
        let mut runtime =
            self.opencti_write
                .lock()
                .map_err(|_| corrobore_engine::KnowledgeDataError {
                    code: corrobore_engine::KnowledgeDataErrorCode::BackendUnavailable,
                    message: "OpenCTI write runtime lock is poisoned".to_owned(),
                    retryable: true,
                })?;
        let mut store = self
            .store
            .lock()
            .map_err(|_| corrobore_engine::KnowledgeDataError {
                code: corrobore_engine::KnowledgeDataErrorCode::BackendUnavailable,
                message: "canonical graph store lock is poisoned".to_owned(),
                retryable: true,
            })?;
        runtime.apply(&mut store, operation, context).map(Some)
    }
}

fn initialize_engine(
    runtime_store: &RuntimeStoreProvider,
    _require_fsync: bool,
    opencti_write: Arc<Mutex<OpenCtiWriteRuntime>>,
) -> Result<CorroboreEngine, AppStateInitError> {
    let RuntimeStoreProvider::Persistent(runtime) = runtime_store else {
        return Ok(CorroboreEngine::strict_default());
    };
    CorroboreEngine::builder()
        .persistence(Box::new(PersistentEnginePersistence {
            store: Arc::clone(&runtime.canonical_store),
            opencti_write,
        }))
        .build()
        .map_err(|error| AppStateInitError::PersistentStorageRecoveryFailed {
            path: runtime.root_path.display().to_string(),
            reason: error.to_string(),
        })
}

fn initialize_domain_providers(
    config: &ServerConfig,
) -> Result<Option<Arc<crate::enterprise::registry::DomainProviderRegistry>>, AppStateInitError> {
    let (Some(provider_dir), Some(manifest_file)) = (
        config.domain_provider_dir.as_deref(),
        config.domain_provider_manifest_file.as_deref(),
    ) else {
        return Ok(None);
    };

    crate::enterprise::registry::DomainProviderRegistry::initialize(
        Path::new(provider_dir),
        Path::new(manifest_file),
    )
    .map(Arc::new)
    .map(Some)
    .map_err(|error| AppStateInitError::DomainProviderInitFailed {
        reason: error.to_string(),
    })
}

fn initialize_runtime_store(
    config: &ServerConfig,
) -> Result<RuntimeStoreProvider, AppStateInitError> {
    match config.storage_mode {
        StorageMode::Ephemeral => Ok(RuntimeStoreProvider::Ephemeral),
        StorageMode::Persistent => {
            let storage_dir = config
                .storage_dir
                .as_deref()
                .ok_or(AppStateInitError::MissingPersistentStorageDir)?;
            let root_path = PathBuf::from(storage_dir);
            initialize_persistent_runtime_store(
                root_path,
                config.storage_strict_recovery,
                CanonicalStoreOptions {
                    max_hot_nodes: config.storage_max_hot_nodes,
                    max_hot_relationships: config.storage_max_hot_relationships,
                    max_warm_adjacency_entries: config.storage_max_warm_adjacency_entries,
                },
            )
            .map(|store| RuntimeStoreProvider::Persistent(Box::new(store)))
        }
    }
}

fn initialize_persistent_runtime_store(
    root_path: PathBuf,
    strict_recovery: bool,
    store_options: CanonicalStoreOptions,
) -> Result<PersistentRuntimeStore, AppStateInitError> {
    // Ownership must be acquired before manifest creation, recovery, or any
    // writable handle is exposed. The Arc retains it across AppState clones.
    let ownership = Arc::new(
        DataDirectoryOwnership::acquire(&root_path).map_err(|error| match error {
            DataDirectoryOwnershipError::Conflict { .. } => {
                AppStateInitError::PersistentStorageOwnershipConflict {
                    path: root_path.display().to_string(),
                    reason: error.to_string(),
                }
            }
            DataDirectoryOwnershipError::Unavailable { .. } => {
                AppStateInitError::PersistentStorageOwnershipFailed {
                    path: root_path.display().to_string(),
                    reason: error.to_string(),
                }
            }
        })?,
    );
    if !root_path.exists() {
        let manifest = runtime_manifest();
        create_storage_root(root_path.clone(), manifest).map_err(|error| {
            AppStateInitError::PersistentStorageInitFailed {
                path: root_path.display().to_string(),
                reason: error.to_string(),
            }
        })?;
        initialize_required_record_logs(&root_path)?;
    }

    let root = open_storage_root(root_path.clone())
        .map_err(|error| classify_storage_open_error(&root_path, error))?;
    let mut canonical =
        CanonicalEngineStore::open_with_strict_recovery(root, store_options, strict_recovery)
            .map_err(|error| classify_storage_open_error(&root_path, error))?;
    if derived_index_rebuild_status(&root_path)
        .map_err(|error| classify_storage_open_error(&root_path, error))?
        .is_some_and(|status| status.readiness == OperationReadiness::Rebuilding)
    {
        rebuild_derived_indexes(&mut canonical, None)
            .map_err(|error| classify_storage_open_error(&root_path, error))?;
    }
    let store = canonical
        .file_backed_store()
        .map_err(|error| classify_storage_open_error(&root_path, error))?;
    let manifest = read_storage_manifest(store.root())
        .map_err(|error| classify_storage_open_error(&root_path, error))?;
    let startup = canonical.startup_report();
    let mut recovery_report = GraphStoreRecoveryReport {
        manifest_validated: true,
        required_components_validated: true,
        catalog_recovered: true,
        adjacency_storage_recovered: true,
        catalog_rebuild_report: None,
        warnings: startup.warnings.clone(),
    };

    if ownership.recovered_stale_owner() {
        recovery_report.warnings.push(format!(
            "recovered stale ownership metadata from {} for {}",
            ownership.lock_path().display(),
            ownership.root_path().display()
        ));
    }

    Ok(PersistentRuntimeStore {
        root_path,
        store,
        recovery_report,
        manifest,
        canonical_store: Arc::new(Mutex::new(canonical)),
        _ownership: ownership,
    })
}

fn canonical_projection_for_query(query: &str) -> CanonicalProjectionRequest {
    let Ok(ast) = cypher_parser::parse_query(query) else {
        return CanonicalProjectionRequest::all();
    };
    let Some(parsed) = ast.query else {
        return CanonicalProjectionRequest::all();
    };
    if let Some(match_clause) = parsed.match_clause {
        if let Some((relationship, _)) = parsed
            .merge_clause
            .as_ref()
            .and_then(|merge| merge.relationship.as_ref())
        {
            return CanonicalProjectionRequest::all()
                .with_relationships(relationship.rel_type.clone());
        }
        let relationship_type = match_clause
            .relationship
            .as_ref()
            .and_then(|(relationship, _)| relationship.rel_type.clone());
        let mut request = match match_clause.start.label {
            Some(label) => CanonicalProjectionRequest::for_label(label),
            None => CanonicalProjectionRequest::all_nodes(),
        };
        if match_clause.relationship.is_some() {
            request = request.with_relationships(relationship_type);
        }
        return request;
    }
    if let Some(merge_clause) = parsed.merge_clause {
        return merge_clause
            .pattern
            .label
            .map(CanonicalProjectionRequest::for_label)
            .unwrap_or_else(CanonicalProjectionRequest::all_nodes);
    }
    CanonicalProjectionRequest::default()
}

fn canonical_projection_for_knowledge_data(
    operation: &KnowledgeDataOperation,
) -> Option<CanonicalProjectionRequest> {
    match operation {
        KnowledgeDataOperation::Health(request) if request.verbose => {
            Some(CanonicalProjectionRequest::all_nodes())
        }
        KnowledgeDataOperation::GetById(request) => Some(
            CanonicalProjectionRequest::for_identifier(request.id.clone()),
        ),
        KnowledgeDataOperation::List(request) => Some(canonical_record_projection(
            &request.kinds,
            &request.filters,
            request.predicate.as_ref(),
            request.include_relationships,
        )),
        KnowledgeDataOperation::Paginate(request) => Some(canonical_record_projection(
            &request.query.kinds,
            &request.query.filters,
            request.query.predicate.as_ref(),
            request.query.include_relationships,
        )),
        KnowledgeDataOperation::Count(request) => Some(canonical_record_projection(
            &request.kinds,
            &request.filters,
            request.predicate.as_ref(),
            request.include_relationships,
        )),
        KnowledgeDataOperation::Aggregate(request) => {
            let mut projection = canonical_record_projection(
                &request.plan.kinds,
                &[],
                request.plan.predicate.as_ref(),
                request.plan.include_relationships,
            );
            if request.plan.kinds.is_empty()
                || request
                    .plan
                    .kinds
                    .iter()
                    .any(|kind| kind.eq_ignore_ascii_case("relationship"))
            {
                projection = projection.with_relationships(None);
            }
            Some(projection)
        }
        KnowledgeDataOperation::Neighbors(request) => Some(canonical_graph_projection(
            std::slice::from_ref(&request.id),
            1,
            if request.incoming && request.outgoing {
                GraphDirection::Both
            } else if request.incoming {
                GraphDirection::Incoming
            } else {
                GraphDirection::Outgoing
            },
            &request.policy,
        )),
        KnowledgeDataOperation::Traverse(request) => Some(canonical_graph_projection(
            &request.start_ids,
            request.max_depth,
            request.direction,
            &request.policy,
        )),
        KnowledgeDataOperation::Subgraph(request) => Some(canonical_graph_projection(
            &request.ids,
            request.max_depth,
            request.direction,
            &request.policy,
        )),
        _ => None,
    }
}

fn canonical_record_projection(
    kinds: &[String],
    filters: &[ReadFilter],
    predicate: Option<&ReadPredicate>,
    include_relationships: bool,
) -> CanonicalProjectionRequest {
    let mut request = if kinds.len() == 1 {
        CanonicalProjectionRequest::for_label(opencti_type_label(&kinds[0]))
    } else {
        CanonicalProjectionRequest::all_nodes()
    };
    if include_relationships {
        request = request.with_relationships(None);
    }
    let mut pushdown = filters
        .iter()
        .filter_map(canonical_property_filter)
        .collect::<Vec<_>>();
    if let Some(predicate) = predicate {
        collect_conjunctive_pushdown(predicate, &mut pushdown);
    }
    request.with_property_filters(pushdown)
}

fn collect_conjunctive_pushdown(
    predicate: &ReadPredicate,
    filters: &mut Vec<CanonicalPropertyFilter>,
) {
    match predicate {
        ReadPredicate::Condition(filter) => {
            if let Some(filter) = canonical_property_filter(filter) {
                filters.push(filter);
            }
        }
        ReadPredicate::And(children) => {
            for child in children {
                if !matches!(child, ReadPredicate::Or(_)) {
                    collect_conjunctive_pushdown(child, filters);
                }
            }
        }
        // An OR branch cannot be intersected with the other compact indexes
        // without changing semantics. It remains in the bounded exact evaluator.
        ReadPredicate::Or(_) | ReadPredicate::Nested { .. } => {}
    }
}

fn canonical_graph_projection(
    identifiers: &[String],
    max_depth: u32,
    direction: GraphDirection,
    policy: &corrobore_engine::GraphReadPolicy,
) -> CanonicalProjectionRequest {
    let mut request = identifiers
        .first()
        .map_or_else(CanonicalProjectionRequest::default, |identifier| {
            CanonicalProjectionRequest::for_identifier(identifier.clone())
        });
    request = request.with_identifiers(identifiers.iter().cloned());
    request.with_adjacency(CanonicalAdjacencyProjection {
        incoming: matches!(direction, GraphDirection::Incoming | GraphDirection::Both),
        outgoing: matches!(direction, GraphDirection::Outgoing | GraphDirection::Both),
        relationship_types: policy.relationship_types.clone(),
        max_depth,
        max_relationships: policy.max_expansions,
        supernode_threshold: policy.supernode_threshold,
    })
}

fn canonical_property_filter(filter: &ReadFilter) -> Option<CanonicalPropertyFilter> {
    Some(CanonicalPropertyFilter {
        field: match filter.field.as_str() {
            "id" => "opencti.canonical_id".to_owned(),
            field if field.starts_with("opencti.") => field.to_owned(),
            field => format!("opencti.field.{field}"),
        },
        operator: match filter.operator {
            ReadFilterOperator::Equal => CanonicalPropertyOperator::Equal,
            ReadFilterOperator::NotEqual => CanonicalPropertyOperator::NotEqual,
            ReadFilterOperator::Exists => CanonicalPropertyOperator::Exists,
            ReadFilterOperator::NotExists => CanonicalPropertyOperator::NotExists,
            ReadFilterOperator::In => CanonicalPropertyOperator::In,
            ReadFilterOperator::NotIn => CanonicalPropertyOperator::NotIn,
            ReadFilterOperator::GreaterThan => CanonicalPropertyOperator::GreaterThan,
            ReadFilterOperator::GreaterThanOrEqual => CanonicalPropertyOperator::GreaterThanOrEqual,
            ReadFilterOperator::LessThan => CanonicalPropertyOperator::LessThan,
            ReadFilterOperator::LessThanOrEqual => CanonicalPropertyOperator::LessThanOrEqual,
            ReadFilterOperator::Wildcard => return None,
        },
        value: filter.value.clone(),
    })
}

fn opencti_type_label(kind: &str) -> String {
    let suffix = kind
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    format!("OpenCtiType_{suffix}")
}

fn classify_storage_open_error(root_path: &Path, error: GraphStorageError) -> AppStateInitError {
    let path = root_path.display().to_string();
    let reason = error.to_string();
    match error {
        GraphStorageError::UnsupportedStorageVersion { .. }
        | GraphStorageError::UnsupportedRecordFormat { .. }
        | GraphStorageError::InvalidManifest { .. } => {
            AppStateInitError::PersistentStorageIncompatible { path, reason }
        }
        _ => AppStateInitError::PersistentStorageRecoveryFailed { path, reason },
    }
}

fn initialize_required_record_logs(root_path: &Path) -> Result<(), AppStateInitError> {
    let required_logs = [
        root_path.join("nodes").join("node_records.log"),
        root_path
            .join("relationships")
            .join("relationship_records.log"),
        root_path.join("adjacency").join("outgoing_adjacency.log"),
        root_path.join("adjacency").join("incoming_adjacency.log"),
    ];

    for path in required_logs {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                AppStateInitError::PersistentStorageInitFailed {
                    path: root_path.display().to_string(),
                    reason: format!(
                        "failed to create log directory {}: {error}",
                        parent.display()
                    ),
                }
            })?;
        }
        if !path.exists() {
            fs::write(&path, b"").map_err(|error| {
                AppStateInitError::PersistentStorageInitFailed {
                    path: root_path.display().to_string(),
                    reason: format!("failed to initialize log file {}: {error}", path.display()),
                }
            })?;
        }
    }

    Ok(())
}

fn runtime_manifest() -> StorageManifest {
    let now = chrono::Utc::now().to_rfc3339();
    StorageManifest {
        storage_version: StorageVersion::V1,
        graph_id: GraphId {
            value: format!("graph--runtime-{}", Uuid::new_v4()),
        },
        created_at: StorageTimestamp { value: now.clone() },
        updated_at: StorageTimestamp { value: now },
        record_format: RecordFormat::JsonLinesV1,
    }
}

pub fn build_router(state: AppState) -> Router {
    let http_trace = TraceLayer::new_for_http()
        .make_span_with(|request: &Request<Body>| {
            let correlation_id = request
                .extensions()
                .get::<RequestCorrelationId>()
                .map_or("unknown", |value| value.0.as_str());
            tracing::span!(
                Level::INFO,
                "http_request",
                correlation_id,
                method = %request.method(),
                uri = %request.uri(),
                version = ?request.version(),
            )
        })
        .on_request(DefaultOnRequest::new().level(Level::INFO))
        .on_response(
            DefaultOnResponse::new()
                .level(Level::INFO)
                .latency_unit(LatencyUnit::Micros),
        )
        .on_failure(
            DefaultOnFailure::new()
                .level(Level::ERROR)
                .latency_unit(LatencyUnit::Micros),
        );

    let protected = Router::new()
        .route(
            "/v1/claims/{id}/audit",
            get(crate::handlers::claim_audit::audit),
        )
        .route(
            "/v1/epistemic/projection",
            get(crate::handlers::claim_audit::projection),
        )
        .route("/v1/cypher/execute", post(execute_cypher))
        .route("/v1/cypher/read", post(execute_read_cypher))
        .route("/v1/cypher/write", post(execute_write_cypher))
        .route("/v1/domains/{domain}/validate", post(validate_domain))
        .route("/v1/stix/validate", post(validate_stix))
        .route("/v1/license/status", get(license_status))
        .route("/v1/seed/search", post(seed_search))
        .route("/v1/memory/operations", post(execute_memory_operation))
        .route("/v1/sessions/start", post(start_session))
        .route("/v1/sessions/{session_id}/stop", post(stop_session))
        .route("/v1/sessions/{session_id}/health", get(session_health))
        .route("/v1/sessions/{session_id}/logs", get(session_logs))
        .route("/v1/explorer/sessions", get(explorer_sessions))
        .route(
            "/v1/explorer/sessions/{session_id}/timeline",
            get(explorer_timeline),
        )
        .route(
            "/v1/explorer/sessions/{session_id}/graph",
            get(explorer_graph),
        )
        .route("/v1/export/stix", get(export_stix))
        // 2.3: standard JSON routes get a tight body limit.
        .layer(DefaultBodyLimit::max(state.config.max_body_bytes));

    // 2.3: import routes accept larger multipart/JSON bundles.
    let import_routes = Router::new()
        .route(
            "/v1/reconciliations",
            post(crate::handlers::reconciliations::submit),
        )
        .route(
            "/v1/reconciliations/{id}",
            get(crate::handlers::reconciliations::inspect),
        )
        .route(
            "/v1/reconciliations/{id}/merge",
            post(crate::handlers::reconciliations::apply),
        )
        .route(
            "/v1/reconciliations/{id}/undo",
            post(crate::handlers::reconciliations::undo),
        )
        .route(
            "/v1/import/candidates",
            post(crate::handlers::candidates::submit),
        )
        .route(
            "/v1/import/candidates/{id}",
            get(crate::handlers::candidates::inspect),
        )
        .route(
            "/v1/import/candidates/{id}/repairs",
            post(crate::handlers::candidates::repair),
        )
        .route(
            "/v1/import/candidates/{id}/promote",
            post(crate::handlers::candidates::promote),
        )
        .route("/v1/import/stix", post(import_stix_bundle))
        .route("/v1/import/stix/file", post(import_stix_bundle_file))
        .layer(DefaultBodyLimit::max(state.config.import_max_body_bytes));

    let opencti_routes = Router::new()
        .route("/v1/opencti/sync/batches", post(apply_opencti_sync_batch))
        .route("/v1/opencti/sync/status", get(opencti_sync_status))
        .route(
            "/v1/opencti/shadow/reads",
            post(execute_opencti_shadow_read),
        )
        .route("/v1/opencti/shadow/reports", get(opencti_shadow_reports))
        .route("/v1/opencti/reads", post(execute_opencti_routed_read))
        .route("/v1/opencti/writes", post(execute_opencti_write))
        .route("/v1/opencti/files", post(execute_opencti_file_command))
        .route("/v1/opencti/writes/status", get(opencti_write_status))
        .route(
            "/v1/opencti/reconciliation",
            post(execute_opencti_reconciliation),
        )
        .route(
            "/v1/opencti/reconciliation/status",
            get(opencti_reconciliation_status),
        )
        .route(
            "/v1/opencti/routing/decisions",
            get(opencti_routing_decisions),
        )
        .route(
            "/v1/opencti/routing/rollback",
            post(opencti_routing_rollback),
        )
        .layer(DefaultBodyLimit::max(state.config.import_max_body_bytes));

    // 2.3: a global token-bucket rate limiter guards all protected routes.
    let governor_config = Arc::new(
        GovernorConfigBuilder::default()
            .period(rate_limit_period(state.config.rate_limit_per_second))
            .burst_size(state.config.rate_limit_burst)
            .key_extractor(GlobalKeyExtractor)
            .finish()
            .expect("rate limiter configuration must be valid"),
    );

    let protected = protected
        .merge(import_routes)
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_bearer_auth,
        ))
        .layer(GovernorLayer::new(governor_config));

    // OpenCTI bootstrapping generates a bounded burst of authenticated provider
    // calls. Isolate that traffic so raising its allowance does not weaken the
    // global limit protecting Corrobore's other APIs.
    let opencti_governor_config = Arc::new(
        GovernorConfigBuilder::default()
            .period(rate_limit_period(
                state.config.opencti_rate_limit_per_second,
            ))
            .burst_size(state.config.opencti_rate_limit_burst)
            .key_extractor(GlobalKeyExtractor)
            .finish()
            .expect("OpenCTI rate limiter configuration must be valid"),
    );
    let opencti_routes = opencti_routes
        .layer(DefaultBodyLimit::max(state.config.import_max_body_bytes))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_bearer_auth,
        ))
        .layer(GovernorLayer::new(opencti_governor_config));

    let operational = Router::new()
        .route("/health", get(health))
        .route("/health/live", get(liveness))
        .route("/health/ready", get(readiness))
        .route("/version", get(version))
        .route("/metrics", get(metrics));
    let operational =
        if state.config.operational_endpoint_policy == OperationalEndpointPolicy::Authenticated {
            operational.route_layer(middleware::from_fn_with_state(
                state.clone(),
                require_bearer_auth,
            ))
        } else {
            operational
        };

    let web_dir = state.config.web_dir.clone();
    let router = Router::new()
        .route("/v1/admin/license/status", get(admin_license_status))
        .route("/v1/admin/storage/snapshots", post(create_snapshot))
        .route("/v1/admin/storage/indexes/rebuild", post(rebuild_indexes))
        .route(
            "/v1/admin/storage/operations",
            get(database_operation_status),
        )
        .route(
            "/v1/admin/domain-providers/status",
            get(admin_domain_provider_status),
        )
        .route(
            "/v1/admin/opencti/projection/drain",
            post(drain_opencti_projection),
        )
        .route(
            "/v1/admin/opencti/reconstruction",
            post(reconstruct_opencti_reference),
        )
        .route(
            "/v1/admin/opencti/authority/suspend",
            post(suspend_opencti_writes),
        )
        .route(
            "/v1/admin/opencti/authority",
            post(transition_opencti_write_authority),
        )
        .merge(protected)
        .merge(opencti_routes)
        .merge(operational)
        .with_state(state.clone());

    attach_web_delivery(router, web_dir.as_deref())
        .layer(middleware::from_fn_with_state(
            state,
            lifecycle_request_gate,
        ))
        .layer(http_trace)
        .layer(middleware::from_fn(correlate_request))
}

fn rate_limit_period(requests_per_second: u64) -> Duration {
    Duration::from_nanos(1_000_000_000_u64.div_ceil(requests_per_second))
}

async fn lifecycle_request_gate(
    axum::extract::State(state): axum::extract::State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if matches!(
        request.uri().path(),
        "/health" | "/health/live" | "/health/ready" | "/version" | "/metrics"
    ) {
        return next.run(request).await;
    }
    let Ok(_activity) = state.lifecycle.try_begin_request() else {
        return ApiError::service_unavailable(
            "SERVICE_DRAINING",
            "server is draining and is not accepting new work",
        )
        .into_response();
    };
    match tokio::time::timeout(
        Duration::from_millis(state.config.request_timeout_ms),
        next.run(request),
    )
    .await
    {
        Ok(response) => response,
        Err(_) => ApiError::timeout(
            "REQUEST_TIMEOUT",
            "request exceeded the configured execution timeout",
        )
        .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, fs};

    use corrobore_engine::{
        AggregateRequest, Aggregation, AggregationPlan, GetByIdRequest, GraphDirection,
        GraphReadPolicy, KnowledgeDataOperation, ReadFilter, ReadFilterOperator, ReadPredicate,
        TraverseRequest,
    };
    use graph_storage::CanonicalPropertyOperator;
    use serde_json::{Value, json};

    use super::{
        AppState, RuntimeStoreProvider, canonical_projection_for_knowledge_data,
        canonical_projection_for_query,
    };
    use crate::config::{ServerConfig, StorageMode};

    fn base_vars() -> HashMap<String, String> {
        HashMap::from([(
            "CORROBORE_HTTP_AUTH_TOKEN".to_owned(),
            "token-123".to_owned(),
        )])
    }

    #[test]
    fn app_state_uses_ephemeral_runtime_provider_by_default() {
        let vars = base_vars();
        let config = ServerConfig::from_map(&vars).expect("config should parse");
        let state = AppState::new(config).expect("ephemeral state should initialize");
        assert!(matches!(
            state.runtime_store,
            RuntimeStoreProvider::Ephemeral
        ));
    }

    #[test]
    fn canonical_projection_selection_distinguishes_create_match_and_traversal() {
        let create = canonical_projection_for_query("CREATE (n:Indicator {name: 'new'}) RETURN n");
        assert!(!create.include_nodes);
        assert!(!create.include_relationships);

        let match_all = canonical_projection_for_query("MATCH (n) RETURN n");
        assert!(match_all.include_nodes);
        assert_eq!(match_all.node_label, None);
        assert!(!match_all.include_relationships);

        let traversal =
            canonical_projection_for_query("MATCH (a:Indicator)-[r:LINKS]->(b) RETURN a, r, b");
        assert!(traversal.include_nodes);
        assert_eq!(traversal.node_label.as_deref(), Some("Indicator"));
        assert_eq!(traversal.relationship_type.as_deref(), Some("LINKS"));
        assert!(traversal.include_relationships);
    }

    #[test]
    fn typed_knowledge_data_projection_selects_compact_read_indexes_and_budgets() {
        let point = canonical_projection_for_knowledge_data(&KnowledgeDataOperation::GetById(
            GetByIdRequest {
                id: "indicator--indexed".to_owned(),
            },
        ))
        .expect("point projection");
        assert_eq!(point.identifiers, ["indicator--indexed"]);
        assert!(point.property_filters.is_empty());

        let traversal = canonical_projection_for_knowledge_data(&KnowledgeDataOperation::Traverse(
            TraverseRequest {
                start_ids: vec!["indicator--indexed".to_owned()],
                max_depth: 2,
                direction: GraphDirection::Outgoing,
                constraints: Value::Null,
                policy: GraphReadPolicy {
                    relationship_types: vec!["indicates".to_owned()],
                    node_kinds: vec!["malware".to_owned()],
                    filters: vec![ReadFilter {
                        field: "confidence".to_owned(),
                        operator: ReadFilterOperator::GreaterThanOrEqual,
                        value: Some(json!(70)),
                    }],
                    max_results: 25,
                    max_expansions: 50,
                    supernode_threshold: 100,
                },
            },
        ))
        .expect("traversal projection");
        let adjacency = traversal.adjacency.expect("persistent adjacency plan");
        assert!(!adjacency.incoming);
        assert!(adjacency.outgoing);
        assert_eq!(adjacency.relationship_types, ["indicates"]);
        assert_eq!(adjacency.max_depth, 2);
        assert_eq!(adjacency.max_relationships, 50);
        assert_eq!(adjacency.supernode_threshold, 100);

        let filtered = super::canonical_record_projection(
            &["indicator".to_owned()],
            &[ReadFilter {
                field: "valid_from".to_owned(),
                operator: ReadFilterOperator::GreaterThanOrEqual,
                value: Some(json!("2026-01-01T00:00:00.000Z")),
            }],
            None,
            false,
        );
        assert_eq!(
            filtered.node_label.as_deref(),
            Some("OpenCtiType_indicator")
        );
        assert_eq!(filtered.property_filters.len(), 1);
        assert_eq!(
            filtered.property_filters[0].operator,
            CanonicalPropertyOperator::GreaterThanOrEqual
        );
        assert_eq!(
            filtered.property_filters[0].field,
            "opencti.field.valid_from"
        );

        let predicate = ReadPredicate::And(vec![
            ReadPredicate::Condition(ReadFilter {
                field: "pattern_type".to_owned(),
                operator: ReadFilterOperator::In,
                value: Some(json!(["stix", "sigma"])),
            }),
            ReadPredicate::Or(vec![
                ReadPredicate::Condition(ReadFilter {
                    field: "name".to_owned(),
                    operator: ReadFilterOperator::Equal,
                    value: Some(json!("alpha")),
                }),
                ReadPredicate::Condition(ReadFilter {
                    field: "name".to_owned(),
                    operator: ReadFilterOperator::Equal,
                    value: Some(json!("beta")),
                }),
            ]),
        ]);
        let structural = super::canonical_record_projection(
            &["indicator".to_owned()],
            &[],
            Some(&predicate),
            false,
        );
        assert_eq!(structural.property_filters.len(), 1);
        assert_eq!(
            structural.property_filters[0].operator,
            CanonicalPropertyOperator::In
        );

        let relationships =
            super::canonical_record_projection(&["uses".to_owned()], &[], None, true);
        assert!(relationships.include_relationships);

        let aggregation = canonical_projection_for_knowledge_data(
            &KnowledgeDataOperation::Aggregate(AggregateRequest {
                plan: AggregationPlan {
                    aggregation: Aggregation::Terms {
                        field: "type".to_owned(),
                        limit: 20,
                    },
                    ..AggregationPlan::default()
                },
            }),
        )
        .expect("aggregation projection");
        assert!(aggregation.include_nodes);
        assert!(aggregation.include_relationships);
    }

    #[test]
    fn app_state_fails_when_domain_provider_manifest_is_missing() {
        let root = std::env::temp_dir().join(format!(
            "corrobore-domain-provider-missing-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).expect("provider root should be created");

        let mut vars = base_vars();
        vars.insert(
            "CORROBORE_DOMAIN_PROVIDER_DIR".to_owned(),
            root.display().to_string(),
        );
        vars.insert(
            "CORROBORE_DOMAIN_PROVIDER_MANIFEST_FILE".to_owned(),
            root.join("missing.json").display().to_string(),
        );
        let config = ServerConfig::from_map(&vars).expect("config should parse");

        let error = match AppState::new(config) {
            Ok(_) => panic!("missing manifest must fail startup"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            super::AppStateInitError::DomainProviderInitFailed { .. }
        ));
        assert!(
            error
                .to_string()
                .contains("failed to read provider manifest")
        );
    }

    #[test]
    fn app_state_fails_before_loading_provider_with_wrong_hash() {
        let root = std::env::temp_dir().join(format!(
            "corrobore-domain-provider-hash-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).expect("provider root should be created");
        fs::write(root.join("libdomain_cti.dylib"), b"not-a-library")
            .expect("provider fixture should be written");
        let manifest_path = root.join("providers.json");
        fs::write(
            &manifest_path,
            r#"{
                "schema_version": "1",
                "providers": [{
                    "domain": "cti",
                    "library": "libdomain_cti.dylib",
                    "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "required": true,
                    "capabilities": [{"name": "node.validate", "version": "1"}]
                }]
            }"#,
        )
        .expect("manifest fixture should be written");

        let mut vars = base_vars();
        vars.insert(
            "CORROBORE_DOMAIN_PROVIDER_DIR".to_owned(),
            root.display().to_string(),
        );
        vars.insert(
            "CORROBORE_DOMAIN_PROVIDER_MANIFEST_FILE".to_owned(),
            manifest_path.display().to_string(),
        );
        let config = ServerConfig::from_map(&vars).expect("config should parse");

        let error = match AppState::new(config) {
            Ok(_) => panic!("hash mismatch must fail startup"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            super::AppStateInitError::DomainProviderInitFailed { .. }
        ));
        assert!(error.to_string().contains("SHA-256 mismatch for cti"));
    }

    #[test]
    fn app_state_persistent_mode_creates_and_opens_storage_root() {
        let storage_dir =
            std::env::temp_dir().join(format!("corrobore-persistent-{}", uuid::Uuid::new_v4()));
        let mut vars = base_vars();
        vars.insert("CORROBORE_STORAGE_MODE".to_owned(), "persistent".to_owned());
        vars.insert(
            "CORROBORE_STORAGE_DIR".to_owned(),
            storage_dir.display().to_string(),
        );

        let config = ServerConfig::from_map(&vars).expect("config should parse");
        assert_eq!(config.storage_mode, StorageMode::Persistent);
        let state = AppState::new(config).expect("persistent state should initialize");

        match state.runtime_store {
            RuntimeStoreProvider::Persistent(runtime) => {
                assert_eq!(runtime.root_path, storage_dir);
                assert!(runtime.root_path.join("manifest.json").is_file());
            }
            RuntimeStoreProvider::Ephemeral => {
                panic!("expected persistent runtime provider");
            }
        }

        let _ = std::fs::remove_dir_all(storage_dir);
    }
}

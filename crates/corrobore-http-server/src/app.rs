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
use corrobore_engine::CorroboreEngine;
use graph_storage::{
    FileBackedGraphStore, GraphId, GraphStorageError, GraphStoreOpenMode, GraphStoreOpenOptions,
    GraphStoreRecoveryReport, RecordFormat, StorageManifest, StorageTimestamp, StorageVersion,
    create_storage_root, open_existing_file_backed_graph_store,
};
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
        domain_provider_status::admin_domain_provider_status,
        domain_validate::validate_domain,
        explorer::{explorer_graph, explorer_sessions, explorer_timeline},
        export::export_stix,
        health::health,
        import::{import_stix_bundle, import_stix_bundle_file},
        license::{admin_license_status, license_status},
        metrics::metrics,
        operational::{liveness, readiness, version},
        seed::seed_search,
        session::{session_health, session_logs, start_session, stop_session},
        stix_validate::validate_stix,
    },
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
    pub(crate) domain_providers: Option<Arc<crate::enterprise::registry::DomainProviderRegistry>>,
    pub started_at: Instant,
}

impl AppState {
    pub fn new(config: ServerConfig) -> Result<Self, AppStateInitError> {
        let runtime_store = initialize_runtime_store(&config)?;
        let domain_providers = initialize_domain_providers(&config)?;
        let timeline = ExplorerTimelineStore::new(&config.session_store_dir);
        let lifecycle = Arc::new(ServerLifecycle::initializing());
        let state = Self {
            engine: Arc::new(Mutex::new(CorroboreEngine::strict_default())),
            runtime_store,
            sessions: Arc::new(Mutex::new(SessionRuntime::new(
                config.session_store_dir.clone(),
                config.session_idle_ttl_ms,
            ))),
            timeline: Arc::new(Mutex::new(timeline)),
            config: Arc::new(config),
            lifecycle: Arc::clone(&lifecycle),
            domain_providers,
            started_at: Instant::now(),
        };
        lifecycle.mark_ready();
        Ok(state)
    }
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
            initialize_persistent_runtime_store(root_path, config.storage_strict_recovery)
                .map(|store| RuntimeStoreProvider::Persistent(Box::new(store)))
        }
    }
}

fn initialize_persistent_runtime_store(
    root_path: PathBuf,
    strict_recovery: bool,
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

    let mut outcome = open_existing_file_backed_graph_store(
        root_path.clone(),
        GraphStoreOpenOptions {
            mode: if strict_recovery {
                GraphStoreOpenMode::RebuildCatalogFromAppendLogs
            } else {
                GraphStoreOpenMode::LoadCatalogWhenAvailable
            },
            ..GraphStoreOpenOptions::default()
        },
    )
    .map_err(|error| classify_storage_open_error(&root_path, error))?;

    if ownership.recovered_stale_owner() {
        outcome.recovery_report.warnings.push(format!(
            "recovered stale ownership metadata from {} for {}",
            ownership.lock_path().display(),
            ownership.root_path().display()
        ));
    }

    Ok(PersistentRuntimeStore {
        root_path,
        store: outcome.store,
        recovery_report: outcome.recovery_report,
        manifest: outcome.manifest,
        _ownership: ownership,
    })
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
        .route("/v1/cypher/execute", post(execute_cypher))
        .route("/v1/cypher/read", post(execute_read_cypher))
        .route("/v1/cypher/write", post(execute_write_cypher))
        .route("/v1/domains/{domain}/validate", post(validate_domain))
        .route("/v1/stix/validate", post(validate_stix))
        .route("/v1/license/status", get(license_status))
        .route("/v1/seed/search", post(seed_search))
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
        .route("/v1/import/stix", post(import_stix_bundle))
        .route("/v1/import/stix/file", post(import_stix_bundle_file))
        .layer(DefaultBodyLimit::max(state.config.import_max_body_bytes));

    // 2.3: a global token-bucket rate limiter guards all protected routes.
    let governor_config = Arc::new(
        GovernorConfigBuilder::default()
            .per_second(state.config.rate_limit_per_second)
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
        .route(
            "/v1/admin/domain-providers/status",
            get(admin_domain_provider_status),
        )
        .merge(protected)
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

    use super::{AppState, RuntimeStoreProvider};
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

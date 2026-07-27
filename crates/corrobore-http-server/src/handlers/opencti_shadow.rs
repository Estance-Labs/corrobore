// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! Authenticated OpenCTI reference reads with asynchronous Corrobore shadowing.

use std::{
    sync::Arc,
    sync::OnceLock,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use axum::{
    Json,
    extract::{Query, State},
};
use corrobore_engine::{
    CorroboreKnowledgeDataProvider, KnowledgeDataRequest, KnowledgeDataResponseEnvelope,
    ProviderDescriptor, ProviderExecution, QueryClass, ShadowComparisonReport, ShadowFailureKind,
    ShadowRequestMetadata, compare_shadow_read, shadow_failure_report,
};
use serde::{Deserialize, Serialize};
use tokio::sync::OwnedSemaphorePermit;

use crate::{
    app::{AppState, RuntimeStoreProvider},
    error::ApiError,
    opencti_shadow::{ShadowAdmission, ShadowCompletion, dispatch_shadowed},
};

const SHADOW_PAGINATION_KEY: &[u8] = b"corrobore-opencti-shadow-pagination-key-v1";
static REFERENCE_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

/// Shadow-read request metadata.
#[derive(Clone, Debug, Deserialize)]
pub struct OpenCtiShadowReadRequest {
    /// Typed provider-neutral read sent unchanged to the reference provider.
    pub request: KnowledgeDataRequest,
    /// Non-sensitive sampling dimensions.
    pub metadata: ShadowRequestMetadata,
}

/// Bounded report query.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct OpenCtiShadowReportQuery {
    /// Optional query-class filter.
    pub query_class: Option<QueryClass>,
    /// Optional bounded release filter.
    pub release: Option<String>,
    /// Maximum reports, capped by configured retention.
    pub limit: Option<usize>,
}

/// Recent privacy-safe reports.
#[derive(Clone, Debug, Serialize)]
pub struct OpenCtiShadowReportsResponse {
    /// Success marker.
    pub ok: bool,
    /// Deterministically filtered newest-first reports.
    pub result: Vec<ShadowComparisonReport>,
}

/// Execute the reference request and detach independently budgeted shadow work.
pub async fn execute_opencti_shadow_read(
    State(state): State<AppState>,
    Json(payload): Json<OpenCtiShadowReadRequest>,
) -> Result<Json<KnowledgeDataResponseEnvelope>, ApiError> {
    if !matches!(state.runtime_store, RuntimeStoreProvider::Persistent(_)) {
        return Err(ApiError::unprocessable(
            "OPENCTI_SHADOW_REQUIRES_PERSISTENT_STORAGE",
            "OpenCTI shadow reads require durable report storage",
        ));
    }
    let query_class = QueryClass::from_operation(&payload.request.operation).ok_or_else(|| {
        ApiError::bad_request(
            "INVALID_OPENCTI_SHADOW_OPERATION",
            "only Knowledge Data Engine read operations may be shadowed",
        )
    })?;
    let _ = query_class;
    let endpoint = state
        .config
        .opencti_shadow
        .reference_endpoint
        .clone()
        .ok_or_else(|| {
            ApiError::service_unavailable(
                "OPENCTI_REFERENCE_NOT_CONFIGURED",
                "OpenCTI reference provider endpoint is not configured",
            )
        })?;
    let synchronization_gate_open = state
        .opencti_sync
        .lock()
        .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "sync state lock poisoned"))?
        .status()
        .shadow_reads_enabled;
    let admission = state
        .opencti_shadow
        .lock()
        .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "shadow state lock poisoned"))?
        .admit(
            &payload.request,
            &payload.metadata,
            synchronization_gate_open,
        );
    let shadow_provider = ProviderDescriptor {
        name: "corrobore".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        release: state.config.opencti_shadow.release.clone(),
    };
    let reference_provider = ProviderDescriptor {
        name: "opensearch".to_owned(),
        version: state.config.opencti_shadow.reference_version.clone(),
        release: state.config.opencti_shadow.reference_version.clone(),
    };
    let request = payload.request;
    let reference_future = execute_reference(
        endpoint,
        state.config.opencti_shadow.reference_auth_token.clone(),
        reference_provider,
        request.clone(),
        Duration::from_millis(state.config.opencti_shadow.timeout_ms),
    );

    let result = match admission {
        ShadowAdmission::Accepted(permit) => {
            let shadow_future = execute_corrobore_shadow(
                Arc::clone(&state.engine),
                shadow_provider.clone(),
                request.clone(),
                permit,
            );
            let report_state = Arc::clone(&state.opencti_shadow);
            let report_request = request.clone();
            let report_shadow_provider = shadow_provider;
            let baselines = report_state
                .lock()
                .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "shadow state lock poisoned"))?
                .baselines();
            dispatch_shadowed(
                reference_future,
                Some(shadow_future),
                Duration::from_millis(state.config.opencti_shadow.timeout_ms),
                move |reference, completion| {
                    let Ok(reference) = reference else {
                        return;
                    };
                    let report = match completion {
                        ShadowCompletion::Completed(shadow) => compare_shadow_read(
                            &report_request,
                            reference,
                            shadow,
                            &baselines,
                            now_unix_ms(),
                        ),
                        ShadowCompletion::Failed => shadow_failure_report(
                            &report_request,
                            reference,
                            report_shadow_provider,
                            ShadowFailureKind::Failed,
                        ),
                        ShadowCompletion::TimedOut => shadow_failure_report(
                            &report_request,
                            reference,
                            report_shadow_provider,
                            ShadowFailureKind::TimedOut,
                        ),
                        ShadowCompletion::Shed => return,
                    };
                    if let Ok(mut runtime) = report_state.lock()
                        && let Err(error) = runtime.record(report)
                    {
                        tracing::error!(
                            error = %error,
                            "failed to persist OpenCTI shadow comparison report"
                        );
                    }
                },
            )
            .await
        }
        ShadowAdmission::Shed(_) => reference_future.await,
    };
    result
        .map(|execution| Json(execution.envelope))
        .map_err(|reason| ApiError::service_unavailable("OPENCTI_REFERENCE_FAILED", reason))
}

/// Query bounded parity reports without raw provider values.
pub async fn opencti_shadow_reports(
    State(state): State<AppState>,
    Query(query): Query<OpenCtiShadowReportQuery>,
) -> Result<Json<OpenCtiShadowReportsResponse>, ApiError> {
    let result = state
        .opencti_shadow
        .lock()
        .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "shadow state lock poisoned"))?
        .reports(
            query.query_class,
            query.release.as_deref(),
            query.limit.unwrap_or(100).min(500),
        );
    Ok(Json(OpenCtiShadowReportsResponse { ok: true, result }))
}

pub(crate) async fn execute_reference(
    endpoint: String,
    token: Option<String>,
    provider: ProviderDescriptor,
    request: KnowledgeDataRequest,
    timeout: Duration,
) -> Result<ProviderExecution, String> {
    let started = Instant::now();
    let client = REFERENCE_CLIENT.get_or_init(reqwest::Client::new);
    // The bound is applied here rather than at each call site: reqwest has no
    // default timeout, so an unresponsive reference deployment would otherwise
    // hold the request open indefinitely wherever a caller forgot to wrap it.
    let mut call = client.post(endpoint).timeout(timeout).json(&request);
    if let Some(token) = token {
        call = call.bearer_auth(token);
    }
    let response = call
        .send()
        .await
        .map_err(|_| "reference provider transport failed".to_owned())?;
    if !response.status().is_success() {
        return Err(format!(
            "reference provider returned HTTP {}",
            response.status().as_u16()
        ));
    }
    let envelope = response
        .json::<KnowledgeDataResponseEnvelope>()
        .await
        .map_err(|_| "reference provider returned an invalid contract envelope".to_owned())?;
    Ok(ProviderExecution {
        provider,
        latency_ms: elapsed_ms(started),
        envelope,
    })
}

pub(crate) async fn execute_corrobore_shadow(
    engine: Arc<std::sync::Mutex<corrobore_engine::CorroboreEngine>>,
    provider: ProviderDescriptor,
    request: KnowledgeDataRequest,
    permit: OwnedSemaphorePermit,
) -> Result<ProviderExecution, String> {
    execute_corrobore(engine, provider, request, Some(permit)).await
}

pub(crate) async fn execute_corrobore_primary(
    engine: Arc<std::sync::Mutex<corrobore_engine::CorroboreEngine>>,
    provider: ProviderDescriptor,
    request: KnowledgeDataRequest,
) -> Result<ProviderExecution, String> {
    execute_corrobore(engine, provider, request, None).await
}

async fn execute_corrobore(
    engine: Arc<std::sync::Mutex<corrobore_engine::CorroboreEngine>>,
    provider: ProviderDescriptor,
    mut request: KnowledgeDataRequest,
    permit: Option<OwnedSemaphorePermit>,
) -> Result<ProviderExecution, String> {
    let started = Instant::now();
    if permit.is_some() {
        request.context.deadline_unix_ms = None;
    }
    let envelope = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let mut engine = engine
            .lock()
            .map_err(|_| "Corrobore shadow engine lock poisoned".to_owned())?;
        let audit_events_before = engine.security_audit_events().len();
        let mut provider = CorroboreKnowledgeDataProvider::new(&mut engine, SHADOW_PAGINATION_KEY)
            .map_err(|_| "failed to initialize Corrobore shadow provider".to_owned())?;
        let envelope = provider.execute(request);
        drop(provider);
        if let Some(event) = engine.security_audit_events().get(audit_events_before) {
            tracing::info!(
                event = "opencti_authorization_decision",
                correlation_id = %event.correlation_id,
                operation = %event.operation,
                policy_version = %event.policy_version,
                allowed = event.allowed,
                authorization_denials = event.authorization_denials,
                decision_reason = ?event.reason,
                "OpenCTI authorization decision"
            );
        }
        Ok::<_, String>(envelope)
    })
    .await
    .map_err(|_| "Corrobore shadow execution task failed".to_owned())??;
    Ok(ProviderExecution {
        provider,
        latency_ms: elapsed_ms(started),
        envelope,
    })
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

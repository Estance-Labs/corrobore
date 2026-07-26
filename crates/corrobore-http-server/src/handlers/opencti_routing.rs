// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! Progressive OpenCTI read routing, audit explanation, and rollback controls.

use std::{future::Future, pin::Pin, sync::Arc, time::Duration};

use axum::{
    Json,
    extract::{Query, State},
};
use corrobore_engine::{
    ConsistencyLevel, KnowledgeDataRequest, KnowledgeDataResponseEnvelope, ProviderDescriptor,
    ProviderExecution, ProviderTarget, ReadRoutingAuditEvent, ReadRoutingGates,
    ReadRoutingMetadata, RoutingSignal, ShadowComparisonGate, ShadowRequestMetadata,
    compare_shadow_read,
};
use serde::{Deserialize, Serialize};

use crate::{
    app::AppState,
    error::ApiError,
    opencti_shadow::{ShadowAdmission, ShadowCompletion, dispatch_shadowed},
};

use super::opencti_shadow::{
    execute_corrobore_primary, execute_corrobore_shadow, execute_reference,
};

type ProviderFuture = Pin<Box<dyn Future<Output = Result<ProviderExecution, String>> + Send>>;

/// Provider-neutral routed read with bounded selection dimensions.
#[derive(Clone, Debug, Deserialize)]
pub struct OpenCtiRoutedReadRequest {
    /// Typed Knowledge Data Engine read.
    pub request: KnowledgeDataRequest,
    /// Non-sensitive routing dimensions and sticky session identity.
    pub metadata: ReadRoutingMetadata,
}

/// Bounded decision-audit query.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct OpenCtiRoutingDecisionQuery {
    /// Optional exact request correlation identity.
    pub correlation_id: Option<String>,
    /// Maximum newest-first events.
    pub limit: Option<usize>,
}

/// Privacy-safe provider-decision evidence.
#[derive(Clone, Debug, Serialize)]
pub struct OpenCtiRoutingDecisionsResponse {
    /// Success marker.
    pub ok: bool,
    /// Current circuit-breaker cause, when rollback is active.
    pub rollback_reason: Option<corrobore_engine::RollbackReason>,
    /// Bounded decision evidence.
    pub result: Vec<ReadRoutingAuditEvent>,
}

/// Execute exactly the policy-selected primary and detach optional shadow work.
pub async fn execute_opencti_routed_read(
    State(state): State<AppState>,
    Json(payload): Json<OpenCtiRoutedReadRequest>,
) -> Result<Json<KnowledgeDataResponseEnvelope>, ApiError> {
    let primary_projection_lag = state
        .opencti_write
        .lock()
        .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "write state lock poisoned"))?
        .status()
        .projection_lag;
    if primary_projection_lag > 0
        && payload.request.context.consistency == ConsistencyLevel::ReadYourWrites
    {
        let execution = tokio::time::timeout(
            Duration::from_millis(state.config.opencti_shadow.timeout_ms),
            provider_future(
                &state,
                ProviderTarget::Corrobore,
                payload.request.clone(),
                None,
            )?,
        )
        .await
        .map_err(|_| {
            ApiError::timeout(
                "OPENCTI_READ_YOUR_WRITES_TIMEOUT",
                "canonical read-your-writes request exceeded the configured deadline",
            )
        })?
        .map_err(|reason| {
            ApiError::service_unavailable("OPENCTI_READ_YOUR_WRITES_FAILED", reason)
        })?;
        return Ok(Json(execution.envelope));
    }
    let sync = state
        .opencti_sync
        .lock()
        .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "sync state lock poisoned"))?
        .status();
    let recent_reports = state
        .opencti_shadow
        .lock()
        .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "shadow state lock poisoned"))?
        .reports(
            None,
            Some(state.config.opencti_shadow.release.as_str()),
            100,
        );
    let gates = gates_from_runtime(
        &sync,
        &recent_reports,
        state.config.opencti_shadow.reference_endpoint.is_some(),
    );
    let decision = state
        .opencti_routing
        .lock()
        .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "routing state lock poisoned"))?
        .decide(&payload.request, &payload.metadata, &gates, now_unix_ms())
        .map_err(|error| {
            ApiError::service_unavailable(
                "OPENCTI_READ_ROUTING_BLOCKED",
                format!("read routing blocked: {:?}", error.reason),
            )
        })?;

    let shadow_metadata = ShadowRequestMetadata {
        environment: payload.metadata.environment.clone(),
        entity_type: payload.metadata.entity_type.clone(),
        user_cohort: payload.metadata.user_cohort.clone(),
    };
    let shadow_permit = if decision.shadow.is_some() {
        match state
            .opencti_shadow
            .lock()
            .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "shadow state lock poisoned"))?
            .admit(
                &payload.request,
                &shadow_metadata,
                sync.shadow_reads_enabled,
            ) {
            ShadowAdmission::Accepted(permit) => Some(permit),
            ShadowAdmission::Shed(_) => None,
        }
    } else {
        None
    };

    let primary = provider_future(&state, decision.primary, payload.request.clone(), None)?;
    let shadow = match (decision.shadow, shadow_permit) {
        (Some(target), Some(permit)) => Some(provider_future(
            &state,
            target,
            payload.request.clone(),
            Some(permit),
        )?),
        _ => None,
    };
    let report_state = Arc::clone(&state.opencti_shadow);
    let baselines = report_state
        .lock()
        .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "shadow state lock poisoned"))?
        .baselines();
    let request = payload.request;
    let primary_target = decision.primary;
    let result = dispatch_shadowed(
        primary,
        shadow,
        Duration::from_millis(state.config.opencti_shadow.timeout_ms),
        move |primary, completion| {
            let Ok(primary) = primary else {
                return;
            };
            let ShadowCompletion::Completed(shadow) = completion else {
                return;
            };
            let (reference, corrobore) = match primary_target {
                ProviderTarget::Reference => (primary, shadow),
                ProviderTarget::Corrobore => (shadow, primary),
            };
            let report =
                compare_shadow_read(&request, reference, corrobore, &baselines, now_unix_ms());
            if let Ok(mut runtime) = report_state.lock()
                && let Err(error) = runtime.record(report)
            {
                tracing::error!(error = %error, "failed to persist routed OpenCTI comparison");
            }
        },
    )
    .await;

    match result {
        Ok(execution) => Ok(Json(execution.envelope)),
        Err(reason) => {
            if decision.primary == ProviderTarget::Corrobore
                && let Ok(mut routing) = state.opencti_routing.lock()
                && let Err(error) =
                    routing.record_signal(RoutingSignal::Unavailability, now_unix_ms())
            {
                tracing::error!(error = %error, "failed to persist automatic OpenCTI rollback");
            }
            Err(ApiError::service_unavailable(
                "OPENCTI_PRIMARY_PROVIDER_FAILED",
                reason,
            ))
        }
    }
}

/// Return bounded decision evidence without request payload or access context.
pub async fn opencti_routing_decisions(
    State(state): State<AppState>,
    Query(query): Query<OpenCtiRoutingDecisionQuery>,
) -> Result<Json<OpenCtiRoutingDecisionsResponse>, ApiError> {
    let runtime = state
        .opencti_routing
        .lock()
        .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "routing state lock poisoned"))?;
    let result = if let Some(correlation_id) = query.correlation_id.as_deref() {
        runtime
            .explain(correlation_id)
            .cloned()
            .into_iter()
            .collect()
    } else {
        runtime.audits(query.limit.unwrap_or(100).min(500))
    };
    Ok(Json(OpenCtiRoutingDecisionsResponse {
        ok: true,
        rollback_reason: runtime.rollback_reason(),
        result,
    }))
}

/// Open the circuit breaker immediately; traffic switches to a fresh reference.
pub async fn opencti_routing_rollback(
    State(state): State<AppState>,
) -> Result<Json<OpenCtiRoutingDecisionsResponse>, ApiError> {
    let mut runtime = state
        .opencti_routing
        .lock()
        .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "routing state lock poisoned"))?;
    runtime
        .record_signal(RoutingSignal::OperatorRollback, now_unix_ms())
        .map_err(|_| {
            ApiError::internal(
                "OPENCTI_ROUTING_STATE_FAILED",
                "failed to persist rollback state",
            )
        })?;
    Ok(Json(OpenCtiRoutingDecisionsResponse {
        ok: true,
        rollback_reason: runtime.rollback_reason(),
        result: Vec::new(),
    }))
}

fn provider_future(
    state: &AppState,
    target: ProviderTarget,
    request: KnowledgeDataRequest,
    permit: Option<tokio::sync::OwnedSemaphorePermit>,
) -> Result<ProviderFuture, ApiError> {
    match target {
        ProviderTarget::Reference => {
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
            let provider = ProviderDescriptor {
                name: "opensearch".to_owned(),
                version: state.config.opencti_shadow.reference_version.clone(),
                release: state.config.opencti_shadow.reference_version.clone(),
            };
            let token = state.config.opencti_shadow.reference_auth_token.clone();
            Ok(Box::pin(execute_reference(
                endpoint, token, provider, request,
            )))
        }
        ProviderTarget::Corrobore => {
            let provider = ProviderDescriptor {
                name: "corrobore".to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
                release: state.config.opencti_shadow.release.clone(),
            };
            let engine = Arc::clone(&state.engine);
            Ok(match permit {
                Some(permit) => {
                    Box::pin(execute_corrobore_shadow(engine, provider, request, permit))
                }
                None => Box::pin(execute_corrobore_primary(engine, provider, request)),
            })
        }
    }
}

fn gates_from_runtime(
    sync: &crate::opencti_sync::OpenCtiSyncStatus,
    reports: &[corrobore_engine::ShadowComparisonReport],
    reference_configured: bool,
) -> ReadRoutingGates {
    let security_divergence = reports
        .iter()
        .any(|report| !report.security_differences.is_empty());
    let parity_breach = reports.iter().any(|report| {
        report.gate == ShadowComparisonGate::Blocked && report.security_differences.is_empty()
    });
    let errors = reports
        .iter()
        .filter(|report| report.shadow_latency_ms.is_none())
        .count();
    let error_rate_basis_points = if reports.is_empty() {
        0
    } else {
        u16::try_from(errors.saturating_mul(10_000) / reports.len()).unwrap_or(10_000)
    };
    let mut latencies: Vec<u64> = reports
        .iter()
        .filter_map(|report| report.shadow_latency_ms)
        .collect();
    latencies.sort_unstable();
    let p95_index = latencies
        .len()
        .saturating_mul(95)
        .div_ceil(100)
        .saturating_sub(1);
    let latency_p95_ms = latencies.get(p95_index).copied().unwrap_or(0);
    ReadRoutingGates {
        synchronization_ready: sync.shadow_reads_enabled && sync.lag == 0 && sync.queue_depth == 0,
        reference_fresh: reference_configured && sync.lag == 0,
        corrobore_available: true,
        corruption_detected: false,
        security_divergence,
        parity_breach,
        error_rate_basis_points,
        latency_p95_ms,
    }
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

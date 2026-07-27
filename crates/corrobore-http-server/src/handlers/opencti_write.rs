// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! Corrobore-authoritative OpenCTI writes and ordered reference projection.

use std::time::Duration;

use axum::{Json, extract::State, http::HeaderMap};
use corrobore_engine::{
    KnowledgeDataOperation, KnowledgeDataOutcome, KnowledgeDataRequest,
    KnowledgeDataResponseEnvelope, ProviderDescriptor,
};
use serde::{Deserialize, Serialize};

use crate::{
    app::{AppState, RuntimeStoreProvider},
    auth::require_admin_auth,
    error::ApiError,
    opencti_write::{
        AuthorityTransitionReadiness, OpenCtiWriteAuditRecord, OpenCtiWriteStatus,
        ProjectionRecordSummary, ReconciliationRecord, ReferenceReconstructionPlan,
        RollbackTrigger, WriteAuthority,
    },
};

use super::opencti_shadow::{execute_corrobore_primary, execute_reference};

/// Authenticated operational state for canonical writes and reference projection.
#[derive(Clone, Debug, Serialize)]
pub struct OpenCtiWriteStatusResponse {
    /// Success marker.
    pub ok: bool,
    /// Bounded write, outbox and authority summary.
    pub result: OpenCtiWriteStatus,
    /// Legacy migration-period reconciliation records.
    pub reconciliations: Vec<ReconciliationRecord>,
    /// Ordered reference projection records.
    pub projections: Vec<ProjectionRecordSummary>,
    /// WAL-bound, payload-free committed mutation receipts.
    pub audits: Vec<OpenCtiWriteAuditRecord>,
}

/// Durably prepare projection, commit Corrobore, then best-effort drain the
/// ordered reference outbox without making reference availability part of the
/// accepted-write acknowledgement.
pub async fn execute_opencti_write(
    State(state): State<AppState>,
    Json(request): Json<KnowledgeDataRequest>,
) -> Result<Json<KnowledgeDataResponseEnvelope>, ApiError> {
    validate_write_request(&request)?;
    let permit = state
        .opencti_write_semaphore
        .clone()
        .try_acquire_owned()
        .map_err(|_| {
            ApiError::service_unavailable(
                "OPENCTI_WRITE_BACKPRESSURE",
                "transactional write concurrency is saturated; retry with the same idempotency key",
            )
        })?;
    if state.config.opencti_elastic_free {
        // In final mode the canonical WAL is the only acknowledgement boundary.
        // Do not create a reference-projection intent that can never be drained.
        let corrobore_provider = ProviderDescriptor {
            name: "corrobore".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            release: state.config.opencti_shadow.release.clone(),
        };
        let execution = tokio::time::timeout(
            Duration::from_millis(state.config.opencti_shadow.timeout_ms),
            execute_corrobore_primary(state.engine.clone(), corrobore_provider, request),
        )
        .await
        .map_err(|_| {
            ApiError::timeout(
                "OPENCTI_PRIMARY_WRITE_TIMEOUT",
                "canonical write exceeded the configured deadline; retry with the same idempotency key",
            )
        })?
        .map_err(|reason| {
            ApiError::service_unavailable("OPENCTI_PRIMARY_WRITE_FAILED", reason)
        })?;
        drop(permit);
        return Ok(Json(execution.envelope));
    }
    let sequence = state
        .opencti_write
        .lock()
        .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "write state lock poisoned"))?
        .prepare_projection(&request)
        .map_err(|error| ApiError::service_unavailable("OPENCTI_PRIMARY_WRITE_SUSPENDED", error))?;

    let corrobore_provider = ProviderDescriptor {
        name: "corrobore".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        release: state.config.opencti_shadow.release.clone(),
    };
    let corrobore = tokio::time::timeout(
        Duration::from_millis(state.config.opencti_shadow.timeout_ms),
        execute_corrobore_primary(state.engine.clone(), corrobore_provider, request),
    )
    .await
    .map_err(|_| {
        ApiError::timeout(
            "OPENCTI_PRIMARY_WRITE_TIMEOUT",
            "canonical write exceeded the configured deadline; retry with the same idempotency key",
        )
    })?
    .map_err(|reason| ApiError::service_unavailable("OPENCTI_PRIMARY_WRITE_FAILED", reason))?;
    drop(permit);

    let expected_response = match &corrobore.envelope.outcome {
        KnowledgeDataOutcome::Success { response } => response.clone(),
        KnowledgeDataOutcome::Failure { .. } => {
            state
                .opencti_write
                .lock()
                .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "write state lock poisoned"))?
                .abort_projection(sequence)
                .map_err(|error| {
                    ApiError::service_unavailable("OPENCTI_PROJECTION_STATE_FAILED", error)
                })?;
            return Ok(Json(corrobore.envelope));
        }
    };
    state
        .opencti_write
        .lock()
        .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "write state lock poisoned"))?
        .activate_projection(sequence, expected_response)
        .map_err(|error| ApiError::service_unavailable("OPENCTI_PROJECTION_STATE_FAILED", error))?;

    drain_reference_projection(&state).await?;
    Ok(Json(corrobore.envelope))
}

async fn drain_reference_projection(state: &AppState) -> Result<(), ApiError> {
    let Some(endpoint) = state.config.opencti_shadow.reference_endpoint.clone() else {
        if let Some(sequence) = pending_projection_sequence(state)? {
            record_projection_failure(state, sequence, "reference provider is not configured")?;
        }
        return Ok(());
    };
    let reference_provider = ProviderDescriptor {
        name: "opensearch".to_owned(),
        version: state.config.opencti_shadow.reference_version.clone(),
        release: state.config.opencti_shadow.reference_version.clone(),
    };
    loop {
        let pending = state
            .opencti_write
            .lock()
            .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "write state lock poisoned"))?
            .pending_projection()
            .cloned();
        let Some(pending) = pending else {
            return Ok(());
        };
        let execution = tokio::time::timeout(
            Duration::from_millis(state.config.opencti_shadow.timeout_ms),
            execute_reference(
                endpoint.clone(),
                state.config.opencti_shadow.reference_auth_token.clone(),
                reference_provider.clone(),
                pending.request,
                Duration::from_millis(state.config.opencti_shadow.timeout_ms),
            ),
        )
        .await;
        let actual = match execution {
            Ok(Ok(execution)) => match execution.envelope.outcome {
                KnowledgeDataOutcome::Success { response } => response,
                KnowledgeDataOutcome::Failure { .. } => {
                    record_projection_failure(
                        state,
                        pending.sequence,
                        "reference provider rejected projection",
                    )?;
                    return Ok(());
                }
            },
            Ok(Err(reason)) => {
                record_projection_failure(state, pending.sequence, &reason)?;
                return Ok(());
            }
            Err(_) => {
                record_projection_failure(
                    state,
                    pending.sequence,
                    "reference projection deadline exceeded",
                )?;
                return Ok(());
            }
        };
        let verification = state
            .opencti_write
            .lock()
            .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "write state lock poisoned"))?
            .verify_projection(pending.sequence, &actual);
        if verification.is_err() {
            return Ok(());
        }
    }
}

fn pending_projection_sequence(state: &AppState) -> Result<Option<u64>, ApiError> {
    Ok(state
        .opencti_write
        .lock()
        .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "write state lock poisoned"))?
        .pending_projection()
        .map(|record| record.sequence))
}

fn record_projection_failure(
    state: &AppState,
    sequence: u64,
    diagnostic: &str,
) -> Result<(), ApiError> {
    state
        .opencti_write
        .lock()
        .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "write state lock poisoned"))?
        .record_projection_failure(sequence, diagnostic)
        .map_err(|error| ApiError::service_unavailable("OPENCTI_PROJECTION_STATE_FAILED", error))
}

fn validate_write_request(request: &KnowledgeDataRequest) -> Result<(), ApiError> {
    if !matches!(
        request.operation,
        KnowledgeDataOperation::Create(_)
            | KnowledgeDataOperation::Update(_)
            | KnowledgeDataOperation::Delete(_)
            | KnowledgeDataOperation::Bulk(_)
            | KnowledgeDataOperation::Merge(_)
    ) {
        return Err(ApiError::bad_request(
            "INVALID_OPENCTI_WRITE_OPERATION",
            "OpenCTI write endpoint accepts create, update, delete, bulk, or merge",
        ));
    }
    request
        .context
        .idempotency_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .ok_or_else(|| {
            ApiError::bad_request(
                "OPENCTI_IDEMPOTENCY_KEY_REQUIRED",
                "transactional mutation requires an idempotency_key",
            )
        })?;
    Ok(())
}

/// Operator request that suspends writes for a bounded rollback trigger.
#[derive(Clone, Debug, Deserialize)]
pub struct SuspendOpenCtiWritesRequest {
    /// Trigger requiring immediate write suspension.
    pub trigger: RollbackTrigger,
}

/// Operator request for a gated authority transition.
#[derive(Clone, Debug, Deserialize)]
pub struct TransitionOpenCtiWriteAuthorityRequest {
    /// Desired exclusive write authority.
    pub target: WriteAuthority,
    /// Health, replay, and parity evidence.
    #[serde(flatten)]
    pub readiness: AuthorityTransitionReadiness,
}

/// Authenticated authority operation response.
#[derive(Clone, Debug, Serialize)]
pub struct OpenCtiWriteAuthorityResponse {
    /// Success marker.
    pub ok: bool,
    /// Durable authority after the operation.
    pub authority: WriteAuthority,
}

/// Authenticated lossless reconstruction response.
#[derive(Clone, Debug, Serialize)]
pub struct OpenCtiReconstructionResponse {
    /// Success marker.
    pub ok: bool,
    /// Reference rebuild records and canonical high-water sequence.
    pub result: ReferenceReconstructionPlan,
}

/// Immediately suspend mutations before a rollback investigation.
pub async fn suspend_opencti_writes(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SuspendOpenCtiWritesRequest>,
) -> Result<Json<OpenCtiWriteAuthorityResponse>, ApiError> {
    require_admin_auth(&state, &headers)?;
    let mut runtime = state
        .opencti_write
        .lock()
        .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "write state lock poisoned"))?;
    runtime
        .suspend_writes(request.trigger)
        .map_err(|error| ApiError::service_unavailable("OPENCTI_AUTHORITY_STATE_FAILED", error))?;
    Ok(Json(OpenCtiWriteAuthorityResponse {
        ok: true,
        authority: runtime.status().write_authority,
    }))
}

/// Transition exclusive write authority after all rollback gates pass.
pub async fn transition_opencti_write_authority(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<TransitionOpenCtiWriteAuthorityRequest>,
) -> Result<Json<OpenCtiWriteAuthorityResponse>, ApiError> {
    require_admin_auth(&state, &headers)?;
    let mut runtime = state
        .opencti_write
        .lock()
        .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "write state lock poisoned"))?;
    runtime
        .transition_authority(request.target, request.readiness)
        .map_err(|error| ApiError::bad_request("OPENCTI_AUTHORITY_GATE_FAILED", error))?;
    Ok(Json(OpenCtiWriteAuthorityResponse {
        ok: true,
        authority: runtime.status().write_authority,
    }))
}

/// Retry the ordered reference outbox after an outage is resolved.
pub async fn drain_opencti_projection(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<OpenCtiWriteStatusResponse>, ApiError> {
    require_admin_auth(&state, &headers)?;
    drain_reference_projection(&state).await?;
    write_status_response(&state)
}

/// Generate a complete lossless reference rebuild from canonical state.
pub async fn reconstruct_opencti_reference(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<OpenCtiReconstructionResponse>, ApiError> {
    require_admin_auth(&state, &headers)?;
    let RuntimeStoreProvider::Persistent(store) = &state.runtime_store else {
        return Err(ApiError::service_unavailable(
            "OPENCTI_RECONSTRUCTION_REQUIRES_PERSISTENCE",
            "reference reconstruction requires persistent canonical storage",
        ));
    };
    let mut runtime = state
        .opencti_write
        .lock()
        .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "write state lock poisoned"))?;
    let mut canonical_store = store
        .canonical_store
        .lock()
        .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "canonical store lock poisoned"))?;
    let result = runtime
        .reconstruction_plan(&mut canonical_store)
        .map_err(|error| ApiError::service_unavailable("OPENCTI_RECONSTRUCTION_FAILED", error))?;
    Ok(Json(OpenCtiReconstructionResponse { ok: true, result }))
}

/// Return bounded primary-write recovery state without secrets.
pub async fn opencti_write_status(
    State(state): State<AppState>,
) -> Result<Json<OpenCtiWriteStatusResponse>, ApiError> {
    write_status_response(&state)
}

fn write_status_response(state: &AppState) -> Result<Json<OpenCtiWriteStatusResponse>, ApiError> {
    let runtime = state
        .opencti_write
        .lock()
        .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "write state lock poisoned"))?;
    Ok(Json(OpenCtiWriteStatusResponse {
        ok: true,
        result: runtime.status(),
        reconciliations: runtime.reconciliation_records().to_vec(),
        projections: runtime.projection_summaries(),
        audits: runtime.audit_records().to_vec(),
    }))
}

// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! Reference-authoritative OpenCTI dual writes and recovery visibility.

use std::time::Duration;

use axum::{Json, extract::State};
use corrobore_engine::{
    KnowledgeDataOperation, KnowledgeDataOutcome, KnowledgeDataRequest,
    KnowledgeDataResponseEnvelope, ProviderDescriptor,
};
use serde::Serialize;

use crate::{
    app::AppState,
    error::ApiError,
    opencti_write::{
        DualWriteOutcome, OpenCtiWriteAuditRecord, OpenCtiWriteStatus, ReconciliationRecord,
    },
};

use super::opencti_shadow::{execute_corrobore_primary, execute_reference};

/// Authenticated operational state for partial dual writes.
#[derive(Clone, Debug, Serialize)]
pub struct OpenCtiWriteStatusResponse {
    /// Success marker.
    pub ok: bool,
    /// Bounded reconciliation summary.
    pub result: OpenCtiWriteStatus,
    /// Oldest-first pending, reconciled, or quarantined records.
    pub reconciliations: Vec<ReconciliationRecord>,
    /// WAL-bound, payload-free committed mutation receipts.
    pub audits: Vec<OpenCtiWriteAuditRecord>,
}

/// Commit against the authoritative reference first, mirror the exact typed
/// request into Corrobore, and durably record any partial or divergent result.
pub async fn execute_opencti_write(
    State(state): State<AppState>,
    Json(request): Json<KnowledgeDataRequest>,
) -> Result<Json<KnowledgeDataResponseEnvelope>, ApiError> {
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
    let idempotency_key = request
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
    let endpoint = state
        .config
        .opencti_shadow
        .reference_endpoint
        .clone()
        .ok_or_else(|| {
            ApiError::service_unavailable(
                "OPENCTI_REFERENCE_NOT_CONFIGURED",
                "reference provider is required before source-of-truth inversion",
            )
        })?;
    let reference_provider = ProviderDescriptor {
        name: "opensearch".to_owned(),
        version: state.config.opencti_shadow.reference_version.clone(),
        release: state.config.opencti_shadow.reference_version.clone(),
    };
    let reference = tokio::time::timeout(
        Duration::from_millis(state.config.opencti_shadow.timeout_ms),
        execute_reference(
            endpoint,
            state.config.opencti_shadow.reference_auth_token.clone(),
            reference_provider,
            request.clone(),
        ),
    )
    .await
    .map_err(|_| {
        ApiError::timeout(
            "OPENCTI_REFERENCE_WRITE_TIMEOUT",
            "reference provider write exceeded the configured deadline",
        )
    })?
    .map_err(|reason| ApiError::bad_gateway("OPENCTI_REFERENCE_WRITE_FAILED", reason))?;

    if matches!(
        reference.envelope.outcome,
        KnowledgeDataOutcome::Failure { .. }
    ) {
        drop(permit);
        return Ok(Json(reference.envelope));
    }

    let corrobore_provider = ProviderDescriptor {
        name: "corrobore".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        release: state.config.opencti_shadow.release.clone(),
    };
    let corrobore = tokio::time::timeout(
        Duration::from_millis(state.config.opencti_shadow.timeout_ms),
        execute_corrobore_primary(state.engine.clone(), corrobore_provider, request.clone()),
    )
    .await;
    drop(permit);

    let (corrobore_applied, diagnostic) = match corrobore {
        Ok(Ok(execution))
            if matches!(
                execution.envelope.outcome,
                KnowledgeDataOutcome::Success { .. }
            ) && execution.envelope.outcome == reference.envelope.outcome =>
        {
            (true, None)
        }
        Ok(Ok(execution))
            if matches!(
                execution.envelope.outcome,
                KnowledgeDataOutcome::Success { .. }
            ) =>
        {
            (false, Some("provider write outcome divergence".to_owned()))
        }
        Ok(Ok(_)) => (false, Some("Corrobore rejected mirrored write".to_owned())),
        Ok(Err(_)) => (false, Some("Corrobore write execution failed".to_owned())),
        Err(_) => (false, Some("Corrobore write deadline exceeded".to_owned())),
    };
    let idempotency_key_hash =
        crate::opencti_write::OpenCtiWriteRuntime::hash_idempotency_key(idempotency_key);
    state
        .opencti_write
        .lock()
        .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "write state lock poisoned"))?
        .record_dual_write(DualWriteOutcome {
            idempotency_key_hash,
            correlation_id: request.context.correlation_id,
            reference_applied: true,
            corrobore_applied,
            diagnostic,
        })
        .map_err(|error| {
            ApiError::service_unavailable("OPENCTI_RECONCILIATION_STATE_FAILED", error)
        })?;

    Ok(Json(reference.envelope))
}

/// Return bounded dual-write recovery state without graph payloads or secrets.
pub async fn opencti_write_status(
    State(state): State<AppState>,
) -> Result<Json<OpenCtiWriteStatusResponse>, ApiError> {
    let runtime = state
        .opencti_write
        .lock()
        .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "write state lock poisoned"))?;
    Ok(Json(OpenCtiWriteStatusResponse {
        ok: true,
        result: runtime.status(),
        reconciliations: runtime.reconciliation_records().to_vec(),
        audits: runtime.audit_records().to_vec(),
    }))
}

// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! Authenticated dry-run, targeted repair, and reconciliation visibility.

use axum::{Json, extract::State};
use opencti_adapter::{OpenCtiReconciliationCommand, ReconciliationReport};
use serde::Serialize;

use crate::{
    app::{AppState, RuntimeStoreProvider},
    error::ApiError,
    opencti_reconciliation::OpenCtiReconciliationStatus,
};

/// Exact payload-free reconciliation result.
#[derive(Clone, Debug, Serialize)]
pub struct OpenCtiReconciliationResponse {
    /// Success marker.
    pub ok: bool,
    /// Dry-run or completed repair report.
    pub result: ReconciliationReport,
}

/// Bounded persisted reports and aggregate operator state.
#[derive(Clone, Debug, Serialize)]
pub struct OpenCtiReconciliationStatusResponse {
    /// Success marker.
    pub ok: bool,
    /// Aggregate bounded state.
    pub result: OpenCtiReconciliationStatus,
    /// Oldest-first payload-free reports.
    pub reports: Vec<ReconciliationReport>,
}

/// Execute one bounded reconciliation command against persistent canonical data.
pub async fn execute_opencti_reconciliation(
    State(state): State<AppState>,
    Json(command): Json<OpenCtiReconciliationCommand>,
) -> Result<Json<OpenCtiReconciliationResponse>, ApiError> {
    let RuntimeStoreProvider::Persistent(persistent) = &state.runtime_store else {
        return Err(ApiError::service_unavailable(
            "OPENCTI_RECONCILIATION_REQUIRES_PERSISTENCE",
            "OpenCTI reconciliation requires persistent canonical storage",
        ));
    };
    let mut runtime = state.opencti_reconciliation.lock().map_err(|_| {
        ApiError::internal("STATE_LOCK_FAILED", "reconciliation state lock poisoned")
    })?;
    let mut store = persistent
        .canonical_store
        .lock()
        .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "canonical storage lock poisoned"))?;
    let report = runtime
        .execute(&mut store, command)
        .map_err(|reason| ApiError::unprocessable("OPENCTI_RECONCILIATION_REJECTED", reason))?;
    Ok(Json(OpenCtiReconciliationResponse {
        ok: true,
        result: report,
    }))
}

/// Return bounded payload-free repair and quarantine evidence.
pub async fn opencti_reconciliation_status(
    State(state): State<AppState>,
) -> Result<Json<OpenCtiReconciliationStatusResponse>, ApiError> {
    let runtime = state.opencti_reconciliation.lock().map_err(|_| {
        ApiError::internal("STATE_LOCK_FAILED", "reconciliation state lock poisoned")
    })?;
    Ok(Json(OpenCtiReconciliationStatusResponse {
        ok: true,
        result: runtime.status(),
        reports: runtime.reports().to_vec(),
    }))
}

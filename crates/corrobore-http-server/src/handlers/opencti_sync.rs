// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

use axum::{Json, extract::State};
use opencti_adapter::{GraphDigest, OpenCtiSyncBatch};
use serde::{Deserialize, Serialize};

use crate::{
    app::{AppState, RuntimeStoreProvider},
    error::ApiError,
    opencti_sync::{DurableSyncBatchResult, OpenCtiSyncStatus, is_client_sync_error},
};

#[derive(Debug, Deserialize)]
pub struct OpenCtiSyncRequest {
    pub batch: OpenCtiSyncBatch,
    pub expected: Option<GraphDigest>,
}

#[derive(Debug, Serialize)]
pub struct OpenCtiSyncResponse {
    pub ok: bool,
    pub result: DurableSyncBatchResult,
}

#[derive(Debug, Serialize)]
pub struct OpenCtiSyncStatusResponse {
    pub ok: bool,
    pub result: OpenCtiSyncStatus,
}

pub async fn apply_opencti_sync_batch(
    State(state): State<AppState>,
    Json(request): Json<OpenCtiSyncRequest>,
) -> Result<Json<OpenCtiSyncResponse>, ApiError> {
    let RuntimeStoreProvider::Persistent(runtime_store) = &state.runtime_store else {
        return Err(ApiError::unprocessable(
            "OPENCTI_SYNC_REQUIRES_PERSISTENT_STORAGE",
            "OpenCTI synchronization requires persistent WAL storage",
        ));
    };
    let mut synchronization = state
        .opencti_sync
        .lock()
        .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "sync state lock poisoned"))?;
    let mut canonical = runtime_store
        .canonical_store
        .lock()
        .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "canonical store lock poisoned"))?;
    let result = synchronization
        .apply(&mut canonical, request.batch, request.expected.as_ref())
        .map_err(|error| {
            if is_client_sync_error(&error) {
                ApiError::bad_request("INVALID_OPENCTI_SYNC_BATCH", error)
            } else {
                ApiError::service_unavailable("OPENCTI_SYNC_COMMIT_FAILED", error)
            }
        })?;
    Ok(Json(OpenCtiSyncResponse { ok: true, result }))
}

pub async fn opencti_sync_status(
    State(state): State<AppState>,
) -> Result<Json<OpenCtiSyncStatusResponse>, ApiError> {
    let result = state
        .opencti_sync
        .lock()
        .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "sync state lock poisoned"))?
        .status();
    Ok(Json(OpenCtiSyncStatusResponse { ok: true, result }))
}

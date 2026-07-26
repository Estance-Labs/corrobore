// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

use axum::{Json, extract::State, http::HeaderMap};
use serde::Serialize;

use crate::{
    app::AppState,
    auth::require_admin_auth,
    database_operations::{
        CreateSnapshotCommand, DatabaseOperationMetrics, create_online_snapshot,
        rebuild_online_indexes,
    },
    error::ApiError,
};

#[derive(Debug, Serialize)]
pub struct DatabaseOperationResponse<T> {
    pub ok: bool,
    pub result: T,
}

pub async fn create_snapshot(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(command): Json<CreateSnapshotCommand>,
) -> Result<Json<DatabaseOperationResponse<graph_storage::SnapshotReport>>, ApiError> {
    require_admin_auth(&state, &headers)?;
    create_online_snapshot(&state, command)
        .map(|result| Json(DatabaseOperationResponse { ok: true, result }))
        .map_err(|reason| ApiError::service_unavailable("DATABASE_OPERATION_FAILED", reason))
}

pub async fn rebuild_indexes(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<DatabaseOperationResponse<graph_storage::IndexRebuildReport>>, ApiError> {
    require_admin_auth(&state, &headers)?;
    rebuild_online_indexes(&state)
        .map(|result| Json(DatabaseOperationResponse { ok: true, result }))
        .map_err(|reason| ApiError::service_unavailable("DATABASE_OPERATION_FAILED", reason))
}

pub async fn database_operation_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<DatabaseOperationResponse<DatabaseOperationMetrics>>, ApiError> {
    require_admin_auth(&state, &headers)?;
    let result = state
        .database_operations
        .lock()
        .map_err(|_| {
            ApiError::service_unavailable(
                "DATABASE_OPERATION_STATE_UNAVAILABLE",
                "database operation metrics lock is poisoned",
            )
        })?
        .clone();
    Ok(Json(DatabaseOperationResponse { ok: true, result }))
}

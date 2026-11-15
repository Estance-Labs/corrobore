// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

use axum::{Json, extract::State, http::HeaderMap};
use serde::{Deserialize, Serialize};

use crate::{
    app::AppState,
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
    crate::auth::require_admin_or_policy(
        &state,
        &headers,
        crate::policy::PolicyRequest::graph_wide(crate::policy::PolicyAction::Export),
    )
    .await?;
    create_online_snapshot(&state, command)
        .map(|result| Json(DatabaseOperationResponse { ok: true, result }))
        .map_err(|reason| ApiError::service_unavailable("DATABASE_OPERATION_FAILED", reason))
}

pub async fn rebuild_indexes(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<DatabaseOperationResponse<graph_storage::IndexRebuildReport>>, ApiError> {
    crate::auth::require_admin_or_policy(
        &state,
        &headers,
        crate::policy::PolicyRequest::on(
            crate::policy::PolicyAction::Maintain,
            graph_storage::MAIN_BRANCH,
        ),
    )
    .await?;
    rebuild_online_indexes(&state)
        .map(|result| Json(DatabaseOperationResponse { ok: true, result }))
        .map_err(|reason| ApiError::service_unavailable("DATABASE_OPERATION_FAILED", reason))
}

pub async fn database_operation_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<DatabaseOperationResponse<DatabaseOperationMetrics>>, ApiError> {
    crate::auth::require_admin_or_policy(
        &state,
        &headers,
        crate::policy::PolicyRequest::graph_wide(crate::policy::PolicyAction::Admin),
    )
    .await?;
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

/// Body of `POST /v1/admin/storage/cleanup`.
#[derive(Debug, Deserialize)]
pub struct HistoryCleanupBody {
    /// Newest first-parent commits kept on every branch. Defaults to 100.
    pub keep_commits: Option<usize>,
    /// Also keep commits created less than this many seconds ago.
    pub keep_seconds: Option<u64>,
    /// Report without writing.
    #[serde(default)]
    pub dry_run: bool,
}

/// Prune graph history outside the retention window and compact payloads.
pub async fn cleanup_history(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<HistoryCleanupBody>,
) -> Result<Json<DatabaseOperationResponse<graph_storage::HistoryCleanupReport>>, ApiError> {
    crate::auth::require_admin_or_policy(
        &state,
        &headers,
        crate::policy::PolicyRequest::graph_wide(crate::policy::PolicyAction::Reclaim),
    )
    .await?;
    let request = graph_storage::HistoryCleanupRequest {
        keep_commits: body.keep_commits.unwrap_or(100).max(1),
        keep_seconds: body.keep_seconds,
        dry_run: body.dry_run,
    };
    let result = crate::handlers::graph_history::with_store(&state, move |store| {
        store.cleanup_graph_history(&request).map_err(|error| {
            ApiError::service_unavailable("DATABASE_OPERATION_FAILED", error.to_string())
        })
    })
    .await?;
    Ok(Json(DatabaseOperationResponse { ok: true, result }))
}

/// Body of `POST /v1/admin/storage/export/parquet`.
#[derive(Debug, Deserialize)]
pub struct ParquetExportBody {
    /// New local export directory on the server.
    pub destination: std::path::PathBuf,
    pub branch: Option<String>,
    pub commit: Option<String>,
}

/// Export one commit to Apache Parquet files on the server.
pub async fn export_parquet(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ParquetExportBody>,
) -> Result<Json<DatabaseOperationResponse<storage_interchange::ParquetExportReport>>, ApiError> {
    crate::auth::require_admin_or_policy(
        &state,
        &headers,
        crate::policy::PolicyRequest::graph_wide(crate::policy::PolicyAction::Export),
    )
    .await?;
    let commit = body
        .commit
        .as_deref()
        .map(crate::handlers::graph_history::parse_commit_id)
        .transpose()?;
    let result = crate::handlers::graph_history::with_store(&state, move |store| {
        let commit_id = store
            .resolve_graph_ref(body.branch.as_deref(), commit.as_ref())
            .map_err(crate::handlers::graph_history::map_history_error)?;
        storage_interchange::export_commit_parquet(store, &commit_id, &body.destination).map_err(
            |error| ApiError::service_unavailable("DATABASE_OPERATION_FAILED", error.to_string()),
        )
    })
    .await?;
    Ok(Json(DatabaseOperationResponse { ok: true, result }))
}

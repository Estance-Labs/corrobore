// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! Durable OpenCTI file-extraction queue integration.

use std::time::{SystemTime, UNIX_EPOCH};

use axum::{Json, extract::State, http::StatusCode};
use opencti_file_search::FileDescriptor;
use serde::{Deserialize, Serialize};

use crate::{
    app::{AppState, RuntimeStoreProvider},
    error::ApiError,
};

/// File lifecycle command emitted by the OpenCTI Corrobore provider.
#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum OpenCtiFileCommand {
    /// Queue one immutable object-storage version for isolated extraction.
    Enqueue {
        /// Canonical file identity, provenance, digest, and access metadata.
        descriptor: Box<FileDescriptor>,
    },
    /// Remove one or more file-content projections synchronously.
    Delete {
        /// Stable OpenCTI file identifiers.
        file_ids: Vec<String>,
    },
}

/// Stable acknowledgement without file content or authorization metadata.
#[derive(Debug, Serialize)]
pub struct OpenCtiFileCommandResponse {
    /// Successful command marker.
    ok: bool,
    /// Stable outcome category.
    result: &'static str,
    /// Deterministic queue identity for an enqueue operation.
    #[serde(skip_serializing_if = "Option::is_none")]
    job_id: Option<String>,
}

/// Enqueue or delete durable file-content work against the canonical store.
pub async fn execute_opencti_file_command(
    State(state): State<AppState>,
    Json(command): Json<OpenCtiFileCommand>,
) -> Result<(StatusCode, Json<OpenCtiFileCommandResponse>), ApiError> {
    let RuntimeStoreProvider::Persistent(runtime) = &state.runtime_store else {
        return Err(ApiError::service_unavailable(
            "OPENCTI_FILE_STORAGE_REQUIRED",
            "OpenCTI file extraction requires persistent canonical storage",
        ));
    };
    let mut store = runtime.canonical_store.lock().map_err(|_| {
        ApiError::internal("STATE_LOCK_FAILED", "canonical graph store lock poisoned")
    })?;
    match command {
        OpenCtiFileCommand::Enqueue { descriptor } => {
            let outcome = store
                .enqueue_file_extraction(*descriptor, now_unix_ms())
                .map_err(|error| {
                    ApiError::bad_request(
                        "OPENCTI_FILE_ENQUEUE_FAILED",
                        format!("file extraction enqueue failed: {error}"),
                    )
                })?;
            Ok((
                StatusCode::ACCEPTED,
                Json(OpenCtiFileCommandResponse {
                    ok: true,
                    result: if outcome.duplicate {
                        "duplicate"
                    } else {
                        "enqueued"
                    },
                    job_id: Some(outcome.job_id),
                }),
            ))
        }
        OpenCtiFileCommand::Delete { file_ids } => {
            if file_ids.is_empty() || file_ids.iter().any(|file_id| file_id.trim().is_empty()) {
                return Err(ApiError::bad_request(
                    "INVALID_OPENCTI_FILE_DELETE",
                    "file_ids must contain at least one non-blank identifier",
                ));
            }
            for file_id in file_ids {
                store.delete_file_content(&file_id).map_err(|error| {
                    ApiError::service_unavailable(
                        "OPENCTI_FILE_DELETE_FAILED",
                        format!("file-content deletion failed: {error}"),
                    )
                })?;
            }
            Ok((
                StatusCode::OK,
                Json(OpenCtiFileCommandResponse {
                    ok: true,
                    result: "deleted",
                    job_id: None,
                }),
            ))
        }
    }
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

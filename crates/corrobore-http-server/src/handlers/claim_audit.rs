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
//! Stored audit and epistemic projection reads. Never execute a resolver or verifier.
use crate::{app::AppState, error::ApiError};
use axum::{
    Json,
    extract::{Path, State},
};
use graph_core::{ClaimId, Graph, GraphError};
use serde_json::Value;

async fn read(
    state: AppState,
    operation: impl FnOnce(&Graph) -> Result<Value, GraphError> + Send + 'static,
) -> Result<Json<Value>, ApiError> {
    // Hydrate canonical persisted records before assembling an immutable view.
    // Bound the request using the same timeout and lock discipline as other reads.
    let timeout = std::time::Duration::from_millis(state.config.request_timeout_ms);
    let value = tokio::time::timeout(
        timeout,
        tokio::task::spawn_blocking(move || {
            let mut engine = state
                .engine
                .lock()
                .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "engine lock poisoned"))?;
            engine
                .hydrate_full_graph()
                .map_err(|e| ApiError::internal("AUDIT_STORAGE_FAILED", e.to_string()))?;
            operation(engine.graph()).map_err(|e| match e {
                GraphError::ClaimNotFound(_) => {
                    ApiError::not_found("CLAIM_NOT_FOUND", "unknown claim")
                }
                other => ApiError::internal("AUDIT_READ_FAILED", other.to_string()),
            })
        }),
    )
    .await
    .map_err(|_| ApiError::timeout("REQUEST_TIMEOUT", "audit read timeout"))?
    .map_err(|e| ApiError::internal("TASK_JOIN_FAILED", e.to_string()))??;
    Ok(Json(value))
}
pub async fn audit(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let id =
        ClaimId::new(id).map_err(|e| ApiError::bad_request("INVALID_CLAIM_ID", e.to_string()))?;
    read(state, move |graph| graph.claim_audit_path(&id)).await
}
pub async fn projection(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    read(state, |graph| {
        serde_json::to_value(graph.epistemic_projection()?.persistence_snapshot())
            .map_err(|e| GraphError::InvalidPropertyValue(e.to_string()))
    })
    .await
}

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
//! Attributed human judgments persisted separately from machine evidence.
use crate::{app::AppState, error::ApiError};
use axum::{
    Json,
    extract::{Path, State},
};
use corrobore_engine::{EngineError, EngineMutationContext};
use graph_core::{
    ActorId, AnalystDecision, AnalystDecisionAction, ClaimId, GraphError, TemporalTimestamp,
};
use serde::Deserialize;
use serde_json::{Value, json};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Submission {
    id: String,
    actor: String,
    recorded_at: String,
    action: AnalystDecisionAction,
}
fn invalid(error: GraphError) -> ApiError {
    match error {
        GraphError::ClaimNotFound(_) => ApiError::not_found("CLAIM_NOT_FOUND", "unknown claim"),
        other => ApiError::bad_request("INVALID_ANALYST_DECISION", other.to_string()),
    }
}
pub async fn submit(
    State(state): State<AppState>,
    Path(claim): Path<String>,
    Json(payload): Json<Submission>,
) -> Result<Json<Value>, ApiError> {
    // Validate attribution, then append through the engine's atomic journal.
    // Graph::record_analyst_decision can change only the human ledger.
    let record = AnalystDecision::new(
        payload.id,
        ClaimId::new(claim).map_err(invalid)?,
        ActorId::new(payload.actor).map_err(invalid)?,
        TemporalTimestamp::new(payload.recorded_at).map_err(invalid)?,
        payload.action,
    )
    .map_err(invalid)?;
    let timeout = std::time::Duration::from_millis(state.config.request_timeout_ms);
    let id = tokio::time::timeout(
        timeout,
        tokio::task::spawn_blocking(move || {
            let mut engine = state
                .engine
                .lock()
                .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "engine lock poisoned"))?;
            engine
                .mutate_graph_atomically(
                    EngineMutationContext::new(
                        "http-default",
                        "http-analyst-decisions",
                        "http-analyst-decisions",
                    ),
                    move |graph| graph.record_analyst_decision(record),
                )
                .map_err(|error| match error {
                    EngineError::Graph(error) => invalid(error),
                    EngineError::InvalidConfiguration { field, reason } => ApiError::bad_request(
                        "ANALYST_MUTATION_FORBIDDEN",
                        format!("{field}: {reason}"),
                    ),
                    other => ApiError::internal("ANALYST_STORAGE_FAILED", other.to_string()),
                })
        }),
    )
    .await
    .map_err(|_| ApiError::timeout("REQUEST_TIMEOUT", "analyst decision timeout"))?
    .map_err(|e| ApiError::internal("TASK_JOIN_FAILED", e.to_string()))??;
    Ok(Json(json!({"ok":true,"decision_id":id})))
}

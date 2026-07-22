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
use std::collections::HashMap;
use std::time::Duration;

use axum::{Json, extract::State};
use graph_core::{SessionId, WorkspaceId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use shared_runtime::{
    CypherBudgetRef, CypherParameters, CypherRequest, CypherRequestMode, CypherResponse,
    CypherResponseData,
};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::{
    app::AppState, error::ApiError, handlers::session::map_session_error,
    session_runtime::SessionServiceStatus,
};

#[derive(Debug, Deserialize)]
pub struct ExecuteCypherRequest {
    pub query: String,
    #[serde(default)]
    pub params: HashMap<String, Value>,
    pub mode: Option<String>,
    pub workspace_id: Option<String>,
    pub session_id: Option<String>,
    pub budget_ref: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ExecuteCypherResponse {
    pub ok: bool,
    pub result: CypherResponse,
}

pub async fn execute_cypher(
    State(state): State<AppState>,
    Json(payload): Json<ExecuteCypherRequest>,
) -> Result<Json<ExecuteCypherResponse>, ApiError> {
    execute_cypher_inner(state, payload, None).await
}

pub async fn execute_read_cypher(
    State(state): State<AppState>,
    Json(payload): Json<ExecuteCypherRequest>,
) -> Result<Json<ExecuteCypherResponse>, ApiError> {
    execute_cypher_inner(state, payload, Some(CypherRequestMode::ReadOnly)).await
}

pub async fn execute_write_cypher(
    State(state): State<AppState>,
    Json(payload): Json<ExecuteCypherRequest>,
) -> Result<Json<ExecuteCypherResponse>, ApiError> {
    execute_cypher_inner(state, payload, Some(CypherRequestMode::Mutation)).await
}

async fn execute_cypher_inner(
    state: AppState,
    mut payload: ExecuteCypherRequest,
    forced_mode: Option<CypherRequestMode>,
) -> Result<Json<ExecuteCypherResponse>, ApiError> {
    let audit_event_id = Uuid::new_v4().to_string();
    let query_len = payload.query.len();
    let workspace_id = payload
        .workspace_id
        .clone()
        .unwrap_or_else(|| "workspace--http-default".to_owned());
    let requested_session_id = payload.session_id.clone();
    let session_id = payload
        .session_id
        .clone()
        .unwrap_or_else(generate_rotating_session_id);
    payload.session_id = Some(session_id.clone());
    debug!(
        query_len,
        workspace_id = %workspace_id,
        session_id = %session_id,
        forced_mode = ?forced_mode,
        "received cypher execution request"
    );

    info!(
        event = "cypher_audit_input",
        audit_event_id = %audit_event_id,
        workspace_id = %workspace_id,
        session_id = %session_id,
        forced_mode = ?forced_mode,
        requested_mode = ?payload.mode,
        budget_ref = ?payload.budget_ref,
        query = %payload.query,
        params = ?payload.params,
        "cypher audit input"
    );

    {
        let mut sessions = state.sessions.lock().map_err(|_| {
            ApiError::internal("STATE_LOCK_FAILED", "session runtime lock poisoned")
        })?;
        sessions
            .expire_inactive_sessions()
            .map_err(map_session_error)?;
    }

    if let Some(session_id) = requested_session_id.as_deref() {
        let mut sessions = state.sessions.lock().map_err(|_| {
            ApiError::internal("STATE_LOCK_FAILED", "session runtime lock poisoned")
        })?;
        sessions
            .transition_to(session_id, SessionServiceStatus::Working)
            .map_err(map_session_error)?;
        sessions
            .transition_to(session_id, SessionServiceStatus::Processing)
            .map_err(map_session_error)?;
    }

    let outcome = match to_runtime_request(payload, forced_mode) {
        Ok(runtime_request) => {
            let timeout = Duration::from_millis(state.config.request_timeout_ms);
            let gateway = state.gateway.clone();
            let execution_result = tokio::time::timeout(
                timeout,
                tokio::task::spawn_blocking(move || {
                    let mut locked = gateway.lock().map_err(|_| {
                        ApiError::internal("STATE_LOCK_FAILED", "cypher gateway lock poisoned")
                    })?;

                    locked.execute(&runtime_request).map_err(|error| {
                        ApiError::bad_request(
                            "RUNTIME_ERROR",
                            format!("cypher execution failed: {error}"),
                        )
                    })
                }),
            )
            .await;

            match execution_result {
                Ok(joined) => match joined {
                    Ok(inner) => match inner {
                        Ok(success) => {
                            if let Some(session_id) = requested_session_id.as_deref() {
                                let mut sessions = state.sessions.lock().map_err(|_| {
                                    ApiError::internal(
                                        "STATE_LOCK_FAILED",
                                        "session runtime lock poisoned",
                                    )
                                })?;
                                sessions
                                    .transition_to(session_id, SessionServiceStatus::Idle)
                                    .map_err(map_session_error)?;
                            }

                            Ok(success)
                        }
                        Err(error) => {
                            warn!(error = ?error, "cypher execution rejected in runtime");
                            if let Some(session_id) = requested_session_id.as_deref() {
                                let mut sessions = state.sessions.lock().map_err(|_| {
                                    ApiError::internal(
                                        "STATE_LOCK_FAILED",
                                        "session runtime lock poisoned",
                                    )
                                })?;
                                sessions
                                    .transition_to(session_id, SessionServiceStatus::Degraded)
                                    .map_err(map_session_error)?;
                            }

                            Err(error)
                        }
                    },
                    Err(error) => {
                        warn!(error = %error, "cypher execution task join failed");
                        if let Some(session_id) = requested_session_id.as_deref() {
                            let mut sessions = state.sessions.lock().map_err(|_| {
                                ApiError::internal(
                                    "STATE_LOCK_FAILED",
                                    "session runtime lock poisoned",
                                )
                            })?;
                            sessions
                                .transition_to(session_id, SessionServiceStatus::Degraded)
                                .map_err(map_session_error)?;
                        }

                        Err(ApiError::internal("TASK_JOIN_FAILED", error.to_string()))
                    }
                },
                Err(_) => {
                    warn!("cypher execution timed out");
                    if let Some(session_id) = requested_session_id.as_deref() {
                        let mut sessions = state.sessions.lock().map_err(|_| {
                            ApiError::internal("STATE_LOCK_FAILED", "session runtime lock poisoned")
                        })?;
                        sessions
                            .transition_to(session_id, SessionServiceStatus::Degraded)
                            .map_err(map_session_error)?;
                    }

                    Err(ApiError::timeout(
                        "REQUEST_TIMEOUT",
                        "cypher execution timeout",
                    ))
                }
            }
        }
        Err(error) => {
            warn!(error = ?error, "cypher request rejected before runtime execution");
            if let Some(session_id) = requested_session_id.as_deref() {
                let mut sessions = state.sessions.lock().map_err(|_| {
                    ApiError::internal("STATE_LOCK_FAILED", "session runtime lock poisoned")
                })?;
                sessions
                    .transition_to(session_id, SessionServiceStatus::Idle)
                    .map_err(map_session_error)?;
            }
            Err(error)
        }
    };

    match &outcome {
        Ok(response) => {
            let (response_data_kind, record_count, mutation_summary) =
                response_audit_summary(response);
            info!(
                event = "cypher_audit_output",
                audit_event_id = %audit_event_id,
                workspace_id = %workspace_id,
                session_id = %session_id,
                status = ?response.status,
                response_data_kind = response_data_kind,
                record_count,
                warnings_count = response.warnings.len(),
                validation_errors_count = response.validation_errors.len(),
                fix_hints_count = response.fix_hints.len(),
                audit_references_count = response.audit_references.len(),
                budget_usage = ?response.budget_usage,
                mutation_summary = ?mutation_summary,
                response = ?response,
                "cypher audit output"
            );
        }
        Err(error) => {
            info!(
                event = "cypher_audit_output",
                audit_event_id = %audit_event_id,
                workspace_id = %workspace_id,
                session_id = %session_id,
                error_code = error.code,
                error_message = %error.message,
                "cypher audit output (error)"
            );
        }
    }

    let result = outcome?;

    Ok(Json(ExecuteCypherResponse { ok: true, result }))
}

fn response_audit_summary(response: &CypherResponse) -> (&'static str, usize, Option<String>) {
    match &response.data {
        CypherResponseData::Records(records) => ("records", records.len(), None),
        CypherResponseData::MutationSummary(summary) => (
            "mutation_summary",
            0,
            Some(format!(
                "created_nodes={},updated_nodes={},deleted_nodes={},created_relationships={},deleted_relationships={},properties_set={}",
                summary.created_nodes,
                summary.updated_nodes,
                summary.deleted_nodes,
                summary.created_relationships,
                summary.deleted_relationships,
                summary.properties_set
            )),
        ),
        CypherResponseData::Empty => ("empty", 0, None),
    }
}

fn to_runtime_request(
    payload: ExecuteCypherRequest,
    forced_mode: Option<CypherRequestMode>,
) -> Result<CypherRequest, ApiError> {
    let query = payload.query;
    if query.trim().is_empty() {
        return Err(ApiError::bad_request(
            "INVALID_QUERY",
            "query cannot be empty",
        ));
    }

    let workspace_id = WorkspaceId::new(
        payload
            .workspace_id
            .unwrap_or_else(|| "workspace--http-default".to_owned()),
    )
    .map_err(|error| ApiError::bad_request("INVALID_WORKSPACE_ID", error.to_string()))?;

    let session_id = SessionId::new(
        payload
            .session_id
            .unwrap_or_else(|| "session--http-default".to_owned()),
    )
    .map_err(|error| ApiError::bad_request("INVALID_SESSION_ID", error.to_string()))?;

    let budget_ref = CypherBudgetRef::new(
        payload
            .budget_ref
            .unwrap_or_else(|| "budget--http-default".to_owned()),
    )
    .map_err(|error| ApiError::bad_request("INVALID_BUDGET_REF", error.to_string()))?;

    let mode = forced_mode.unwrap_or_else(|| parse_mode(payload.mode.as_deref(), &query));
    let parameters = CypherParameters::new(stringify_params(payload.params));

    match mode {
        CypherRequestMode::ReadOnly => CypherRequest::build_read_only_request(
            query,
            parameters,
            workspace_id,
            session_id,
            budget_ref,
        )
        .map_err(|error| ApiError::bad_request("INVALID_REQUEST", error.to_string())),
        CypherRequestMode::Mutation => CypherRequest::build_mutation_request(
            query,
            parameters,
            workspace_id,
            session_id,
            budget_ref,
        )
        .map_err(|error| ApiError::bad_request("INVALID_REQUEST", error.to_string())),
        CypherRequestMode::ValidateOnly => CypherRequest::build_validate_only_request(
            query,
            parameters,
            workspace_id,
            session_id,
            budget_ref,
        )
        .map_err(|error| ApiError::bad_request("INVALID_REQUEST", error.to_string())),
        CypherRequestMode::Explain => Err(ApiError::bad_request(
            "UNSUPPORTED_MODE",
            "mode explain is not supported for execution",
        )),
    }
}

fn parse_mode(mode: Option<&str>, query: &str) -> CypherRequestMode {
    match mode.map(|value| value.trim().to_ascii_lowercase()) {
        Some(value) if value == "read" || value == "readonly" || value == "read_only" => {
            CypherRequestMode::ReadOnly
        }
        Some(value) if value == "write" || value == "mutation" => CypherRequestMode::Mutation,
        Some(value) if value == "validate" || value == "validate_only" => {
            CypherRequestMode::ValidateOnly
        }
        Some(value) if value == "auto" => auto_mode(query),
        Some(_) => auto_mode(query),
        None => auto_mode(query),
    }
}

fn auto_mode(query: &str) -> CypherRequestMode {
    let upper = query.to_ascii_uppercase();
    if [
        " CREATE ", " MERGE ", " SET ", " DELETE ", " REMOVE ", " DROP ",
    ]
    .iter()
    .any(|keyword| upper.contains(keyword))
    {
        CypherRequestMode::Mutation
    } else {
        CypherRequestMode::ReadOnly
    }
}

fn generate_rotating_session_id() -> String {
    format!("session--http-{}", Uuid::new_v4().simple())
}

fn stringify_params(params: HashMap<String, Value>) -> HashMap<String, String> {
    params
        .into_iter()
        .map(|(key, value)| {
            let converted = match value {
                Value::String(inner) => inner,
                _ => value.to_string(),
            };
            (key, converted)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::generate_rotating_session_id;

    #[test]
    fn generated_session_ids_rotate_and_use_http_prefix() {
        let first = generate_rotating_session_id();
        let second = generate_rotating_session_id();

        assert_ne!(first, second);
        assert!(first.starts_with("session--http-"));
        assert!(second.starts_with("session--http-"));
    }
}

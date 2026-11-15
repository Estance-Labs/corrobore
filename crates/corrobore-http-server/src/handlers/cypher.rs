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
use corrobore_engine::{
    CypherResponse, CypherResponseData, CypherValue, EngineError, EngineRequest, EngineRequestMode,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
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
    /// Read the head of this branch instead of `main` (persistent mode).
    pub branch: Option<String>,
    /// Read at this graph commit (persistent mode). Pinned reads are read-only;
    /// a `branch` without `commit` reads and writes that branch's head.
    pub commit: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ExecuteCypherResponse {
    pub ok: bool,
    pub result: CypherResponse,
    /// Commit the request read (pinned reads) or the `main` head right after
    /// the request (persistent mode).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph_commit_id: Option<String>,
}

/// Where a Cypher request reads.
#[derive(Clone, Debug, PartialEq, Eq)]
enum ReadTarget {
    /// The canonical engine on `main`.
    Canonical,
    /// A one-shot read of a commit or branch head.
    Pinned {
        branch: Option<String>,
        commit: Option<String>,
    },
}

fn read_target(payload: &ExecuteCypherRequest) -> ReadTarget {
    let branch = payload
        .branch
        .as_deref()
        .map(str::trim)
        .filter(|branch| !branch.is_empty());
    let commit = payload
        .commit
        .as_deref()
        .map(str::trim)
        .filter(|commit| !commit.is_empty());
    if commit.is_none() && branch.is_none_or(|branch| branch == graph_storage::MAIN_BRANCH) {
        ReadTarget::Canonical
    } else {
        ReadTarget::Pinned {
            branch: branch.map(str::to_owned),
            commit: commit.map(str::to_owned),
        }
    }
}

fn execute_pinned(
    state: &AppState,
    engine_request: EngineRequest,
    query: &str,
    mode: EngineRequestMode,
    branch: Option<&str>,
    commit: Option<&str>,
) -> Result<(CypherResponse, String), ApiError> {
    use crate::handlers::graph_history::{canonical_store, map_history_error, parse_commit_id};
    use crate::pinned_graph::{BranchWriteOutcome, BranchWritePersistence, PinnedGraphPersistence};

    let commit = commit.map(parse_commit_id).transpose()?;
    if commit.is_some() && mode == EngineRequestMode::Mutation {
        return Err(ApiError {
            status: axum::http::StatusCode::CONFLICT,
            code: "GRAPH_COMMIT_READ_ONLY",
            message: "reads pinned to a commit cannot write; target a branch head instead"
                .to_owned(),
        });
    }
    let store = canonical_store(state)?;
    let (commit_id, graph) = {
        let mut store = store.lock().map_err(|_| {
            ApiError::internal("STATE_LOCK_FAILED", "canonical store lock poisoned")
        })?;
        let commit_id = store
            .resolve_graph_ref(branch, commit.as_ref())
            .map_err(map_history_error)?;
        let projection = crate::app::canonical_projection_for_query(query);
        let graph = store
            .load_commit_projection(&commit_id, &projection)
            .map_err(|error| {
                ApiError::bad_request("COMMIT_PROJECTION_FAILED", error.to_string())
            })?;
        (commit_id, graph)
    };
    // A branch head (no explicit commit) accepts writes; everything else is a read.
    let writable_branch = match (commit.as_ref(), branch) {
        (None, Some(branch)) if mode != EngineRequestMode::ReadOnly => Some(branch.to_owned()),
        _ => None,
    };
    let Some(branch) = writable_branch else {
        let mut engine = corrobore_engine::CorroboreEngine::builder()
            .persistence(Box::new(PinnedGraphPersistence::read_only(graph)))
            .read_only(true)
            .build()
            .map_err(map_engine_error)?;
        let response = engine
            .execute_request(engine_request)
            .map_err(map_engine_error)?;
        return Ok((response, commit_id.to_string()));
    };
    let outcome = std::sync::Arc::new(std::sync::Mutex::new(BranchWriteOutcome::default()));
    let mut engine = corrobore_engine::CorroboreEngine::builder()
        .persistence(Box::new(BranchWritePersistence::new(
            graph,
            std::sync::Arc::clone(&store),
            branch,
            commit_id.clone(),
            None,
            std::sync::Arc::clone(&outcome),
        )))
        .build()
        .map_err(map_engine_error)?;
    let response = engine.execute_request(engine_request);
    let mut outcome = outcome.lock().map_err(|_| {
        ApiError::internal("STATE_LOCK_FAILED", "branch write outcome lock poisoned")
    })?;
    if let Some(error) = outcome.error.take() {
        return Err(map_history_error(error));
    }
    let response = response.map_err(map_engine_error)?;
    let head = outcome.commit.take().unwrap_or(commit_id);
    Ok((response, head.to_string()))
}

fn canonical_head(state: &AppState) -> Option<String> {
    let store = crate::handlers::graph_history::canonical_store(state).ok()?;
    let mut store = store.lock().ok()?;
    store
        .graph_head(graph_storage::MAIN_BRANCH)
        .ok()
        .map(|head| head.to_string())
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
    execute_cypher_inner(state, payload, Some(EngineRequestMode::ReadOnly)).await
}

pub async fn execute_write_cypher(
    State(state): State<AppState>,
    Json(payload): Json<ExecuteCypherRequest>,
) -> Result<Json<ExecuteCypherResponse>, ApiError> {
    execute_cypher_inner(state, payload, Some(EngineRequestMode::Mutation)).await
}

async fn execute_cypher_inner(
    state: AppState,
    mut payload: ExecuteCypherRequest,
    forced_mode: Option<EngineRequestMode>,
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

    let target = read_target(&payload);
    let query_text = payload.query.clone();
    let requested_mode =
        forced_mode.unwrap_or_else(|| parse_mode(payload.mode.as_deref(), &payload.query));
    let outcome = match to_engine_request(payload, forced_mode) {
        Ok(engine_request) => {
            let timeout = Duration::from_millis(state.config.request_timeout_ms);
            let engine = state.engine.clone();
            let blocking_state = state.clone();
            let execution_result = tokio::time::timeout(
                timeout,
                tokio::task::spawn_blocking(move || match &target {
                    ReadTarget::Pinned { branch, commit } => execute_pinned(
                        &blocking_state,
                        engine_request,
                        &query_text,
                        requested_mode,
                        branch.as_deref(),
                        commit.as_deref(),
                    )
                    .map(|(response, commit_id)| (response, Some(commit_id))),
                    ReadTarget::Canonical => {
                        let response = {
                            let mut locked = engine.lock().map_err(|_| {
                                ApiError::internal("STATE_LOCK_FAILED", "engine lock poisoned")
                            })?;
                            locked
                                .execute_request(engine_request)
                                .map_err(map_engine_error)?
                        };
                        Ok((response, canonical_head(&blocking_state)))
                    }
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
        Ok((response, _)) => {
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

    let (result, graph_commit_id) = outcome?;

    Ok(Json(ExecuteCypherResponse {
        ok: true,
        result,
        graph_commit_id,
    }))
}

fn response_audit_summary(response: &CypherResponse) -> (&'static str, usize, Option<String>) {
    match &response.data {
        CypherResponseData::Records(records) => ("records", records.len(), None),
        CypherResponseData::MutationSummary(summary) => (
            "mutation_summary",
            0,
            Some(format!(
                "matched_rows={},created_nodes={},updated_nodes={},deleted_nodes={},created_relationships={},updated_relationships={},deleted_relationships={},properties_set={},native_fields_changed={},property_fields_changed={}",
                summary.matched_rows,
                summary.created_nodes,
                summary.updated_nodes,
                summary.deleted_nodes,
                summary.created_relationships,
                summary.updated_relationships,
                summary.deleted_relationships,
                summary.properties_set,
                summary.native_fields_changed,
                summary.property_fields_changed
            )),
        ),
        CypherResponseData::Empty => ("empty", 0, None),
    }
}

fn to_engine_request(
    payload: ExecuteCypherRequest,
    forced_mode: Option<EngineRequestMode>,
) -> Result<EngineRequest, ApiError> {
    let query = payload.query;
    if query.trim().is_empty() {
        return Err(ApiError::bad_request(
            "INVALID_QUERY",
            "query cannot be empty",
        ));
    }

    let mode = forced_mode.unwrap_or_else(|| parse_mode(payload.mode.as_deref(), &query));
    let mut request =
        EngineRequest::new(query, mode).with_typed_parameters(typed_params(payload.params)?);
    if let Some(workspace_id) = payload.workspace_id {
        request = request.with_workspace_id(workspace_id);
    }
    if let Some(session_id) = payload.session_id {
        request = request.with_session_id(session_id);
    }
    if let Some(budget_ref) = payload.budget_ref {
        request = request.with_budget_ref(budget_ref);
    }

    Ok(request)
}

fn parse_mode(mode: Option<&str>, query: &str) -> EngineRequestMode {
    match mode.map(|value| value.trim().to_ascii_lowercase()) {
        Some(value) if value == "read" || value == "readonly" || value == "read_only" => {
            EngineRequestMode::ReadOnly
        }
        Some(value) if value == "write" || value == "mutation" => EngineRequestMode::Mutation,
        Some(value) if value == "validate" || value == "validate_only" => {
            EngineRequestMode::ValidateOnly
        }
        Some(value) if value == "auto" => auto_mode(query),
        Some(_) => auto_mode(query),
        None => auto_mode(query),
    }
}

fn auto_mode(_query: &str) -> EngineRequestMode {
    EngineRequestMode::Auto
}

fn map_engine_error(error: EngineError) -> ApiError {
    match error {
        EngineError::InvalidConfiguration { field, reason } => {
            let code = match field {
                "workspace_id" => "INVALID_WORKSPACE_ID",
                "session_id" => "INVALID_SESSION_ID",
                "budget_ref" => "INVALID_BUDGET_REF",
                _ => "INVALID_REQUEST",
            };
            ApiError::bad_request(code, reason)
        }
        other => {
            ApiError::bad_request("RUNTIME_ERROR", format!("cypher execution failed: {other}"))
        }
    }
}

fn generate_rotating_session_id() -> String {
    format!("session--http-{}", Uuid::new_v4().simple())
}

/// Converts JSON parameters into typed runtime values.
///
/// JSON already carries the scalar type the caller intended, so it is preserved
/// instead of being flattened to text: a number stays a number all the way to the
/// executor, which is what makes `LIMIT $n` and numeric comparisons behave.
/// Homogeneous scalar arrays remain typed lists. Other composite shapes are
/// rejected rather than stringified into values that would silently fail.
fn typed_params(params: HashMap<String, Value>) -> Result<HashMap<String, CypherValue>, ApiError> {
    params
        .into_iter()
        .map(|(key, value)| {
            let converted = match value {
                Value::String(inner) => CypherValue::String(inner),
                Value::Bool(inner) => CypherValue::Boolean(inner),
                Value::Null => CypherValue::Null,
                Value::Number(number) => match number.as_i64() {
                    Some(integer) => CypherValue::Integer(integer),
                    // Decimals keep their source text so the value stays lossless.
                    None => CypherValue::Float(number.to_string()),
                },
                Value::Array(values) => json_array_to_cypher_list(&key, values)?,
                Value::Object(_) => {
                    return Err(ApiError::bad_request(
                        "UNSUPPORTED_PARAMETER_TYPE",
                        format!(
                            "parameter {key:?} must be a scalar or homogeneous scalar array; \
                             objects are not supported"
                        ),
                    ));
                }
            };
            Ok((key, converted))
        })
        .collect()
}

fn json_array_to_cypher_list(key: &str, values: Vec<Value>) -> Result<CypherValue, ApiError> {
    if values.is_empty() || values.len() > cypher_parser::MAX_LIST_ITEMS {
        return Err(unsupported_array(key));
    }
    let converted = values
        .into_iter()
        .map(|value| match value {
            Value::String(inner) => Ok(CypherValue::String(inner)),
            Value::Bool(inner) => Ok(CypherValue::Boolean(inner)),
            Value::Number(number) => match number.as_i64() {
                Some(integer) => Ok(CypherValue::Integer(integer)),
                None => Ok(CypherValue::Float(number.to_string())),
            },
            Value::Null | Value::Array(_) | Value::Object(_) => Err(unsupported_array(key)),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let first_type = converted[0].type_name();
    if converted
        .iter()
        .any(|value| value.type_name() != first_type)
    {
        return Err(unsupported_array(key));
    }
    Ok(CypherValue::List(converted))
}

fn unsupported_array(key: &str) -> ApiError {
    ApiError::bad_request(
        "UNSUPPORTED_PARAMETER_TYPE",
        format!(
            "parameter {key:?} arrays must contain 1..={} strings, integers, decimals, or booleans of one type",
            cypher_parser::MAX_LIST_ITEMS
        ),
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::json;

    use super::{generate_rotating_session_id, typed_params};
    use corrobore_engine::CypherValue;

    #[test]
    fn generated_session_ids_rotate_and_use_http_prefix() {
        let first = generate_rotating_session_id();
        let second = generate_rotating_session_id();

        assert_ne!(first, second);
        assert!(first.starts_with("session--http-"));
        assert!(second.starts_with("session--http-"));
    }

    #[test]
    fn typed_params_preserve_homogeneous_json_arrays_end_to_end() {
        let params = HashMap::from([
            ("names".to_owned(), json!(["alpha", "beta"])),
            ("ports".to_owned(), json!([80, 443])),
            ("scores".to_owned(), json!([0.4, 0.9])),
            ("flags".to_owned(), json!([true, false])),
        ]);

        let converted = typed_params(params).expect("homogeneous arrays should remain typed");

        assert_eq!(
            converted.get("names"),
            Some(&CypherValue::List(vec![
                CypherValue::String("alpha".to_owned()),
                CypherValue::String("beta".to_owned()),
            ]))
        );
        assert_eq!(
            converted.get("ports"),
            Some(&CypherValue::List(vec![
                CypherValue::Integer(80),
                CypherValue::Integer(443),
            ]))
        );
        assert_eq!(
            converted.get("scores"),
            Some(&CypherValue::List(vec![
                CypherValue::Float("0.4".to_owned()),
                CypherValue::Float("0.9".to_owned()),
            ]))
        );
        assert_eq!(
            converted.get("flags"),
            Some(&CypherValue::List(vec![
                CypherValue::Boolean(true),
                CypherValue::Boolean(false),
            ]))
        );
    }

    #[test]
    fn typed_params_reject_mixed_nested_and_object_values_atomically() {
        for value in [
            json!(["alpha", 1]),
            json!([[1, 2]]),
            json!({"key": "value"}),
        ] {
            let error = typed_params(HashMap::from([("value".to_owned(), value)]))
                .expect_err("unsupported composite parameter should be rejected");
            assert_eq!(error.code, "UNSUPPORTED_PARAMETER_TYPE");
        }
    }
}

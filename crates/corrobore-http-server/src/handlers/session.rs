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
use std::collections::{HashMap, HashSet};

use axum::{
    Json,
    extract::{Path, Query, State},
    response::IntoResponse,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{debug, info};

use crate::{
    app::AppState,
    error::ApiError,
    logging::SESSION_LOG_FILE_NAME,
    session_runtime::{SessionRuntimeError, SessionServiceStatus, StartSessionInput},
};

#[derive(Debug, Deserialize)]
pub struct StartSessionRequest {
    pub workspace_id: String,
    pub actor_id: String,
    pub actor_kind: Option<String>,
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Serialize)]
pub struct StartSessionResponse {
    pub ok: bool,
    pub result: StartSessionResult,
}

#[derive(Debug, Serialize)]
pub struct StartSessionResult {
    pub session_id: String,
    pub status: SessionServiceStatus,
}

#[derive(Debug, Serialize)]
pub struct SessionHealthResponse {
    pub ok: bool,
    pub result: SessionHealthResult,
}

#[derive(Debug, Serialize)]
pub struct StopSessionResponse {
    pub ok: bool,
    pub result: StopSessionResult,
}

#[derive(Debug, Serialize)]
pub struct StopSessionResult {
    pub session_id: String,
    pub status: SessionServiceStatus,
    pub updated_at_ms: u64,
}

#[derive(Debug, Serialize)]
pub struct SessionHealthResult {
    pub session_id: String,
    pub workspace_id: String,
    pub actor_id: String,
    pub actor_kind: String,
    pub status: SessionServiceStatus,
    pub started_at_ms: u64,
    pub updated_at_ms: u64,
    pub uptime_ms: u64,
    pub stop_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SessionLogsQuery {
    pub limit: Option<usize>,
    pub from_ms: Option<u64>,
    pub to_ms: Option<u64>,
    pub format: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SessionLogsResponse {
    pub ok: bool,
    pub result: SessionLogsResult,
}

#[derive(Debug, Serialize)]
pub struct SessionLogsResult {
    pub session_id: String,
    pub log_path: String,
    pub matched_entries: usize,
    pub total_matched_entries: usize,
    pub stop_reason: Option<String>,
    pub audit_parity: SessionAuditParity,
    pub entries: Vec<Value>,
}

#[derive(Debug, Serialize)]
pub struct SessionAuditParity {
    pub input_events: usize,
    pub output_events: usize,
    pub missing_output_event_ids: Vec<String>,
    pub orphan_output_event_ids: Vec<String>,
    pub parity_ok: bool,
}

pub async fn start_session(
    State(state): State<AppState>,
    Json(payload): Json<StartSessionRequest>,
) -> Result<Json<StartSessionResponse>, ApiError> {
    let actor_kind = parse_actor_kind(payload.actor_kind.as_deref())?;

    let mut sessions = state
        .sessions
        .lock()
        .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "session runtime lock poisoned"))?;

    sessions
        .expire_inactive_sessions()
        .map_err(map_session_error)?;

    let started = sessions
        .start_session(StartSessionInput {
            workspace_id: payload.workspace_id,
            actor_id: payload.actor_id,
            actor_kind,
            metadata: payload.metadata,
        })
        .map_err(map_session_error)?;

    info!(
        session_id = %started.session_id,
        status = ?started.status,
        "session started"
    );

    Ok(Json(StartSessionResponse {
        ok: true,
        result: StartSessionResult {
            session_id: started.session_id,
            status: started.status,
        },
    }))
}

pub async fn session_health(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<SessionHealthResponse>, ApiError> {
    let mut sessions = state
        .sessions
        .lock()
        .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "session runtime lock poisoned"))?;

    sessions
        .expire_inactive_sessions()
        .map_err(map_session_error)?;

    let health = sessions
        .session_health(&session_id)
        .map_err(map_session_error)?;

    debug!(
        session_id = %health.session_id,
        status = ?health.status,
        "session health requested"
    );

    let stop_reason = stop_reason_from_health(&health);

    Ok(Json(SessionHealthResponse {
        ok: true,
        result: SessionHealthResult {
            session_id: health.session_id,
            workspace_id: health.workspace_id,
            actor_id: health.actor_id,
            actor_kind: format!("{:?}", health.actor_kind),
            status: health.status,
            started_at_ms: health.started_at_ms,
            updated_at_ms: health.updated_at_ms,
            uptime_ms: health.updated_at_ms.saturating_sub(health.started_at_ms),
            stop_reason,
        },
    }))
}

pub async fn stop_session(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<StopSessionResponse>, ApiError> {
    let mut sessions = state
        .sessions
        .lock()
        .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "session runtime lock poisoned"))?;

    sessions
        .expire_inactive_sessions()
        .map_err(map_session_error)?;

    sessions
        .stop_session(&session_id)
        .map_err(map_session_error)?;

    let health = sessions
        .session_health(&session_id)
        .map_err(map_session_error)?;

    info!(
        session_id = %health.session_id,
        status = ?health.status,
        "session stopped"
    );

    Ok(Json(StopSessionResponse {
        ok: true,
        result: StopSessionResult {
            session_id: health.session_id,
            status: health.status,
            updated_at_ms: health.updated_at_ms,
        },
    }))
}

pub async fn session_logs(
    Path(session_id): Path<String>,
    Query(query): Query<SessionLogsQuery>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ApiError> {
    let stop_reason = {
        let mut sessions = state.sessions.lock().map_err(|_| {
            ApiError::internal("STATE_LOCK_FAILED", "session runtime lock poisoned")
        })?;
        sessions
            .expire_inactive_sessions()
            .map_err(map_session_error)?;
        let health = sessions
            .session_health(&session_id)
            .map_err(map_session_error)?;
        stop_reason_from_health(&health)
    };

    let limit = query.limit.unwrap_or(500).clamp(1, 5000);
    if let (Some(from_ms), Some(to_ms)) = (query.from_ms, query.to_ms)
        && from_ms > to_ms
    {
        return Err(ApiError::bad_request(
            "INVALID_TIME_RANGE",
            "from_ms must be lower than or equal to to_ms",
        ));
    }

    let format = query
        .format
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("json")
        .to_ascii_lowercase();
    if format != "json" && format != "ndjson" {
        return Err(ApiError::bad_request(
            "INVALID_LOG_FORMAT",
            "format must be one of: json, ndjson",
        ));
    }

    let log_path = std::path::Path::new(&state.config.log_dir).join(SESSION_LOG_FILE_NAME);

    let content = std::fs::read_to_string(&log_path)
        .map_err(|error| ApiError::internal("SESSION_LOG_READ_FAILED", error.to_string()))?;

    let mut matched_lines_rev = Vec::new();
    let mut parsed_entries_rev = Vec::new();
    for line in content.lines().rev() {
        if !line.contains(&session_id) {
            continue;
        }

        if let Ok(parsed) = serde_json::from_str::<Value>(line) {
            if !timestamp_matches_range(&parsed, query.from_ms, query.to_ms) {
                continue;
            }

            matched_lines_rev.push(line.to_owned());
            parsed_entries_rev.push(parsed);
        }
    }

    let audit_parity = compute_audit_parity(&parsed_entries_rev);
    let total_matched_entries = parsed_entries_rev.len();

    let mut matched_lines: Vec<String> = matched_lines_rev.into_iter().take(limit).collect();
    let mut entries: Vec<Value> = parsed_entries_rev.into_iter().take(limit).collect();
    matched_lines.reverse();
    entries.reverse();

    if format == "ndjson" {
        let body = if matched_lines.is_empty() {
            String::new()
        } else {
            format!("{}\n", matched_lines.join("\n"))
        };
        return Ok((
            [(axum::http::header::CONTENT_TYPE, "application/x-ndjson")],
            body,
        )
            .into_response());
    }

    Ok(Json(SessionLogsResponse {
        ok: true,
        result: SessionLogsResult {
            session_id,
            log_path: log_path.display().to_string(),
            matched_entries: entries.len(),
            total_matched_entries,
            stop_reason,
            audit_parity,
            entries,
        },
    })
    .into_response())
}

fn stop_reason_from_health(health: &crate::session_runtime::SessionHealthView) -> Option<String> {
    if health.status == SessionServiceStatus::Stopped && health.auto_stopped_due_to_idle_ttl {
        Some("idle_ttl_expired".to_owned())
    } else {
        None
    }
}

fn compute_audit_parity(entries: &[Value]) -> SessionAuditParity {
    let mut input_events = 0usize;
    let mut output_events = 0usize;
    let mut input_event_ids = HashSet::new();
    let mut output_event_ids = HashSet::new();

    for entry in entries {
        let Some(fields) = entry.get("fields").and_then(Value::as_object) else {
            continue;
        };

        let Some(event) = fields.get("event").and_then(Value::as_str) else {
            continue;
        };

        let audit_event_id = fields
            .get("audit_event_id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);

        match event {
            "cypher_audit_input" => {
                input_events += 1;
                if let Some(audit_event_id) = audit_event_id {
                    input_event_ids.insert(audit_event_id);
                }
            }
            "cypher_audit_output" => {
                output_events += 1;
                if let Some(audit_event_id) = audit_event_id {
                    output_event_ids.insert(audit_event_id);
                }
            }
            _ => {}
        }
    }

    let mut missing_output_event_ids: Vec<String> = input_event_ids
        .difference(&output_event_ids)
        .cloned()
        .collect();
    let mut orphan_output_event_ids: Vec<String> = output_event_ids
        .difference(&input_event_ids)
        .cloned()
        .collect();
    missing_output_event_ids.sort();
    orphan_output_event_ids.sort();

    SessionAuditParity {
        input_events,
        output_events,
        parity_ok: input_events == output_events
            && missing_output_event_ids.is_empty()
            && orphan_output_event_ids.is_empty(),
        missing_output_event_ids,
        orphan_output_event_ids,
    }
}

fn timestamp_matches_range(entry: &Value, from_ms: Option<u64>, to_ms: Option<u64>) -> bool {
    if from_ms.is_none() && to_ms.is_none() {
        return true;
    }

    let timestamp = entry
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(parse_timestamp_ms);

    match timestamp {
        Some(timestamp_ms) => {
            if let Some(from_ms) = from_ms
                && timestamp_ms < from_ms
            {
                return false;
            }
            if let Some(to_ms) = to_ms
                && timestamp_ms > to_ms
            {
                return false;
            }
            true
        }
        None => false,
    }
}

fn parse_timestamp_ms(timestamp: &str) -> Option<u64> {
    let parsed: DateTime<Utc> = DateTime::parse_from_rfc3339(timestamp)
        .ok()?
        .with_timezone(&Utc);
    let millis = parsed.timestamp_millis();
    u64::try_from(millis).ok()
}

fn parse_actor_kind(value: Option<&str>) -> Result<shared_runtime::ActorKind, ApiError> {
    let normalized = value
        .map(|entry| entry.trim().to_ascii_lowercase())
        .unwrap_or_else(|| "agent".to_owned());

    match normalized.as_str() {
        "user" => Ok(shared_runtime::ActorKind::User),
        "agent" => Ok(shared_runtime::ActorKind::Agent),
        "orchestrator_agent" | "orchestrator-agent" | "orchestratoragent" => {
            Ok(shared_runtime::ActorKind::OrchestratorAgent)
        }
        "worker_agent" | "worker-agent" | "workeragent" => {
            Ok(shared_runtime::ActorKind::WorkerAgent)
        }
        "tool" => Ok(shared_runtime::ActorKind::Tool),
        "system" => Ok(shared_runtime::ActorKind::System),
        "test_fixture" | "test-fixture" | "testfixture" => {
            Ok(shared_runtime::ActorKind::TestFixture)
        }
        _ => Err(ApiError::bad_request(
            "INVALID_ACTOR_KIND",
            "actor_kind is invalid",
        )),
    }
}

pub fn map_session_error(error: SessionRuntimeError) -> ApiError {
    match error {
        SessionRuntimeError::SessionNotFound(session_id) => ApiError::not_found(
            "SESSION_NOT_FOUND",
            format!("session not found: {session_id}"),
        ),
        SessionRuntimeError::InvalidStatusTransition { from, to } => ApiError::bad_request(
            "INVALID_STATUS_TRANSITION",
            format!("invalid transition from {from:?} to {to:?}"),
        ),
        SessionRuntimeError::Persistence(message) => {
            ApiError::internal("SESSION_PERSISTENCE_ERROR", message)
        }
        SessionRuntimeError::Runtime(message) => ApiError::bad_request("RUNTIME_ERROR", message),
    }
}

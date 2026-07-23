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
//! Authenticated read-only HTTP handlers for the temporal graph explorer.

use std::collections::BTreeMap;

use axum::{
    Json,
    extract::{Path, Query, State},
};
use serde::{Deserialize, Serialize};

use crate::{
    app::AppState,
    error::ApiError,
    explorer_timeline::{ExplorerBoundarySelection, ExplorerTimeline, ExplorerTimelineError},
    session_runtime::SessionHealthView,
    visualization::{
        VisualizationProjectionBudget, VisualizationProjectionError,
        VisualizationProjectionRequest, VisualizationProjectionResponse, project_resolved_graph,
    },
};

/// Query for deterministic session listing.
#[derive(Debug, Default, Deserialize)]
pub struct ExplorerSessionsQuery {
    /// Include stopped sessions when true.
    pub include_stopped: Option<bool>,
}

/// Session listing envelope.
#[derive(Debug, Serialize)]
pub struct ExplorerSessionsResponse {
    /// Successful response marker.
    pub ok: bool,
    /// Response body.
    pub result: ExplorerSessionsResult,
}

/// Deterministic session list.
#[derive(Debug, Serialize)]
pub struct ExplorerSessionsResult {
    /// Session read models.
    pub sessions: Vec<SessionHealthView>,
}

/// Timeline response envelope.
#[derive(Debug, Serialize)]
pub struct ExplorerTimelineResponse {
    /// Successful response marker.
    pub ok: bool,
    /// Persisted temporal lineage.
    pub result: ExplorerTimeline,
}

/// Graph projection response envelope.
#[derive(Debug, Serialize)]
pub struct ExplorerGraphResponse {
    /// Successful response marker.
    pub ok: bool,
    /// Bounded visualization projection.
    pub result: VisualizationProjectionResponse,
}

/// Query selecting a temporal boundary and projection budgets.
#[derive(Debug, Default, Deserialize)]
pub struct ExplorerGraphQuery {
    /// `current`, `snapshot`, or `timeshot`.
    pub boundary_kind: Option<String>,
    /// Snapshot or timeshot identifier.
    pub boundary_id: Option<String>,
    /// Optional node budget.
    pub max_nodes: Option<usize>,
    /// Optional relationship budget.
    pub max_relationships: Option<usize>,
    /// Optional property budget.
    pub max_properties_per_record: Option<usize>,
    /// Optional serialized payload budget.
    pub max_payload_bytes: Option<usize>,
    /// Optional computation budget.
    pub max_computation_units: Option<usize>,
}

/// List current or all persisted sessions for the explorer rail.
pub async fn explorer_sessions(
    Query(query): Query<ExplorerSessionsQuery>,
    State(state): State<AppState>,
) -> Result<Json<ExplorerSessionsResponse>, ApiError> {
    let mut sessions = state
        .sessions
        .lock()
        .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "session runtime lock poisoned"))?;
    sessions
        .expire_inactive_sessions()
        .map_err(super::session::map_session_error)?;
    Ok(Json(ExplorerSessionsResponse {
        ok: true,
        result: ExplorerSessionsResult {
            sessions: sessions.list_sessions(query.include_stopped.unwrap_or(false)),
        },
    }))
}

/// Return one session's snapshot/timeshot tree.
pub async fn explorer_timeline(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<ExplorerTimelineResponse>, ApiError> {
    let session = resolve_session(&state, &session_id)?;
    let timeline = state
        .timeline
        .lock()
        .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "timeline store lock poisoned"))?
        .timeline_for_session(&session)
        .map_err(map_timeline_error)?;
    Ok(Json(ExplorerTimelineResponse {
        ok: true,
        result: timeline,
    }))
}

/// Return a bounded graph projection at the selected temporal boundary.
pub async fn explorer_graph(
    Path(session_id): Path<String>,
    Query(query): Query<ExplorerGraphQuery>,
    State(state): State<AppState>,
) -> Result<Json<ExplorerGraphResponse>, ApiError> {
    let session = resolve_session(&state, &session_id)?;
    let selection = parse_selection(&query)?;
    let boundary = state
        .timeline
        .lock()
        .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "timeline store lock poisoned"))?
        .resolve_boundary(&session, &selection)
        .map_err(map_timeline_error)?;
    let budget = VisualizationProjectionBudget::new(
        query.max_nodes.unwrap_or(5_000),
        query.max_relationships.unwrap_or(10_000),
        query.max_properties_per_record.unwrap_or(32),
        query.max_payload_bytes.unwrap_or(2 * 1024 * 1024),
        query.max_computation_units.unwrap_or(100_000),
    )
    .map_err(map_projection_error)?;
    let request = VisualizationProjectionRequest::new(boundary, budget);
    let engine = state
        .engine
        .lock()
        .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "engine lock poisoned"))?;
    let result = project_resolved_graph(engine.graph(), &request, &BTreeMap::new())
        .map_err(map_projection_error)?;
    Ok(Json(ExplorerGraphResponse { ok: true, result }))
}

fn resolve_session(state: &AppState, session_id: &str) -> Result<SessionHealthView, ApiError> {
    let mut sessions = state
        .sessions
        .lock()
        .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "session runtime lock poisoned"))?;
    sessions
        .expire_inactive_sessions()
        .map_err(super::session::map_session_error)?;
    sessions
        .session_health(session_id)
        .map_err(super::session::map_session_error)
}

fn parse_selection(query: &ExplorerGraphQuery) -> Result<ExplorerBoundarySelection, ApiError> {
    let kind = query
        .boundary_kind
        .as_deref()
        .unwrap_or("current")
        .trim()
        .to_ascii_lowercase();
    match kind.as_str() {
        "current" if query.boundary_id.is_none() => Ok(ExplorerBoundarySelection::current()),
        "current" => Err(ApiError::bad_request(
            "INVALID_TEMPORAL_BOUNDARY",
            "current boundary must not include boundary_id",
        )),
        "snapshot" => required_boundary_id(query).map(ExplorerBoundarySelection::snapshot),
        "timeshot" => required_boundary_id(query).map(ExplorerBoundarySelection::timeshot),
        _ => Err(ApiError::bad_request(
            "INVALID_TEMPORAL_BOUNDARY",
            "boundary_kind must be one of: current, snapshot, timeshot",
        )),
    }
}

fn required_boundary_id(query: &ExplorerGraphQuery) -> Result<String, ApiError> {
    query
        .boundary_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            ApiError::bad_request(
                "INVALID_TEMPORAL_BOUNDARY",
                "snapshot and timeshot selections require boundary_id",
            )
        })
}

fn map_timeline_error(error: ExplorerTimelineError) -> ApiError {
    match error {
        ExplorerTimelineError::BoundaryNotFound { .. }
        | ExplorerTimelineError::BoundarySessionMismatch { .. } => ApiError::not_found(
            "TEMPORAL_BOUNDARY_NOT_FOUND",
            "temporal boundary not found for the requested session",
        ),
        ExplorerTimelineError::Persistence(message) => {
            ApiError::internal("EXPLORER_TIMELINE_PERSISTENCE_ERROR", message)
        }
        other => ApiError::bad_request("INVALID_TEMPORAL_LINEAGE", other.to_string()),
    }
}

fn map_projection_error(error: VisualizationProjectionError) -> ApiError {
    match error {
        VisualizationProjectionError::InvalidBudget { .. }
        | VisualizationProjectionError::InvalidTemporalBoundary { .. }
        | VisualizationProjectionError::PayloadBudgetTooSmall { .. } => {
            ApiError::bad_request("INVALID_VISUALIZATION_PROJECTION", error.to_string())
        }
        VisualizationProjectionError::GraphProjection { .. } => {
            ApiError::internal("VISUALIZATION_PROJECTION_FAILED", error.to_string())
        }
    }
}

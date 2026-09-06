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
//! WS-C candidate import: immutable raw proposals and explicit promotion.
use crate::{app::AppState, error::ApiError};
use axum::{
    Json,
    extract::{Path, State},
};
use corrobore_engine::{EngineError, EngineMutationContext};
use graph_core::{
    ActorId, CandidateId, CandidateInput, CandidatePromotionInput, ExtractionRunId, Graph,
    GraphError, GraphTier, NodeId, NodeInput, PropertyMap, RelationshipInput,
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Submission {
    id: String,
    extraction_run_id: String,
    raw_payload: String,
    actor: String,
    tier: Option<GraphTier>,
    workspace_id: Option<String>,
    session_id: Option<String>,
    budget_ref: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Promotion {
    actor: String,
    reason: String,
    record: ReviewedRecord,
    workspace_id: Option<String>,
    session_id: Option<String>,
    budget_ref: Option<String>,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ReviewedRecord {
    Node {
        labels: Vec<String>,
        #[serde(default)]
        properties: PropertyMap,
    },
    Relationship {
        source: String,
        target: String,
        relationship_type: String,
        #[serde(default)]
        properties: PropertyMap,
    },
}
impl ReviewedRecord {
    fn into_input(self) -> Result<CandidatePromotionInput, GraphError> {
        match self {
            Self::Node { labels, properties } => {
                let mut input = NodeInput::new(labels);
                for (name, value) in properties {
                    input = input.with_property(name, value);
                }
                Ok(CandidatePromotionInput::Node(input))
            }
            Self::Relationship {
                source,
                target,
                relationship_type,
                properties,
            } => {
                let mut input = RelationshipInput::new(
                    NodeId::new(source)?,
                    relationship_type,
                    NodeId::new(target)?,
                )?;
                for (name, value) in properties {
                    input = input.with_property(name, value);
                }
                Ok(CandidatePromotionInput::Relationship(input))
            }
        }
    }
}
fn invalid(error: impl std::fmt::Display) -> ApiError {
    ApiError::bad_request("INVALID_CANDIDATE_OPERATION", error.to_string())
}
fn engine_error(error: EngineError) -> ApiError {
    match error {
        EngineError::Graph(error) => invalid(error),
        EngineError::InvalidConfiguration { field, reason } => {
            ApiError::bad_request("CANDIDATE_MUTATION_FORBIDDEN", format!("{field}: {reason}"))
        }
        other => ApiError::internal("CANDIDATE_STORAGE_FAILED", other.to_string()),
    }
}
fn candidate_response(graph: &Graph, id: &CandidateId) -> Result<Value, GraphError> {
    let store = &graph.epistemic_stores().candidates;
    let candidate = store
        .get(id)
        .ok_or_else(|| GraphError::InvalidPropertyValue("unknown candidate".into()))?;
    Ok(
        json!({"ok":true,"candidate":candidate,"tier":store.tier_of(id),"promotions":store.promotions().iter().filter(|p| p.candidate_id() == id).collect::<Vec<_>>() }),
    )
}
async fn mutate(
    state: AppState,
    workspace: Option<String>,
    session: Option<String>,
    budget: Option<String>,
    operation: impl FnOnce(&mut Graph) -> Result<Value, GraphError> + Send + 'static,
) -> Result<Json<Value>, ApiError> {
    let context = EngineMutationContext::new(
        workspace.unwrap_or_else(|| "workspace--http-default".into()),
        session.unwrap_or_else(|| "session--http-candidates".into()),
        budget.unwrap_or_else(|| "budget--http-candidates".into()),
    );
    let timeout = std::time::Duration::from_millis(state.config.request_timeout_ms);
    let result = tokio::time::timeout(
        timeout,
        tokio::task::spawn_blocking(move || {
            let mut engine = state
                .engine
                .lock()
                .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "engine lock poisoned"))?;
            engine
                .mutate_graph_atomically(context, operation)
                .map_err(engine_error)
        }),
    )
    .await
    .map_err(|_| ApiError::timeout("REQUEST_TIMEOUT", "candidate operation timeout"))?
    .map_err(|error| ApiError::internal("TASK_JOIN_FAILED", error.to_string()))??;
    Ok(Json(result))
}
pub async fn submit(
    State(state): State<AppState>,
    Json(payload): Json<Submission>,
) -> Result<Json<Value>, ApiError> {
    let candidate = CandidateInput::new(
        payload.id,
        ExtractionRunId::new(payload.extraction_run_id).map_err(invalid)?,
        payload.raw_payload,
        ActorId::new(payload.actor).map_err(invalid)?,
    )
    .map_err(invalid)?
    .with_tier(payload.tier.unwrap_or(GraphTier::Shadow));
    mutate(
        state,
        payload.workspace_id,
        payload.session_id,
        payload.budget_ref,
        move |graph| {
            let candidate = graph.submit_candidate(candidate)?;
            candidate_response(graph, candidate.id())
        },
    )
    .await
}
pub async fn promote(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<Promotion>,
) -> Result<Json<Value>, ApiError> {
    let id = CandidateId::new(id).map_err(invalid)?;
    let actor = ActorId::new(payload.actor).map_err(invalid)?;
    let input = payload.record.into_input().map_err(invalid)?;
    mutate(
        state,
        payload.workspace_id,
        payload.session_id,
        payload.budget_ref,
        move |graph| {
            let promotion = graph.promote_candidate(&id, actor, payload.reason, input)?;
            Ok(json!({"ok":true,"tier":GraphTier::Canonical,"promotion":promotion}))
        },
    )
    .await
}
pub async fn inspect(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let id = CandidateId::new(id).map_err(invalid)?;
    let timeout = std::time::Duration::from_millis(state.config.request_timeout_ms);
    let response = tokio::time::timeout(
        timeout,
        tokio::task::spawn_blocking(move || {
            let mut engine = state
                .engine
                .lock()
                .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "engine lock poisoned"))?;
            engine.hydrate_full_graph().map_err(engine_error)?;
            candidate_response(engine.graph(), &id).map_err(invalid)
        }),
    )
    .await
    .map_err(|_| ApiError::timeout("REQUEST_TIMEOUT", "candidate inspection timeout"))?
    .map_err(|error| ApiError::internal("TASK_JOIN_FAILED", error.to_string()))??;
    Ok(Json(response))
}

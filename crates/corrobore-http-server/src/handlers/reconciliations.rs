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
//! Analyst inspection and reversible, evidence-cited mention identity merges.
use crate::{app::AppState, error::ApiError};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use corrobore_engine::{EngineError, EngineMutationContext};
use graph_core::{
    ActorId, Graph, GraphError, MergeUndo, ReconciliationInput, ReconciliationRecordId,
    TemporalTimestamp,
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Context {
    workspace_id: Option<String>,
    session_id: Option<String>,
    budget_ref: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Submission {
    record: ReconciliationInput,
    #[serde(default)]
    context: Context,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Undo {
    id: String,
    actor: String,
    undone_at: String,
    rationale: String,
    #[serde(default)]
    context: Context,
}
pub enum Error {
    Api(ApiError),
    Dependent { merge: String, dependent: String },
}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        match self {
            Self::Api(error) => error.into_response(),
            Self::Dependent { merge, dependent } => (StatusCode::CONFLICT, Json(json!({"ok":false,"error":{
                "code":"DEPENDENT_RECONCILIATION", "message":"A later reconciliation depends on this merge",
                "merge_record":merge,"dependent_record":dependent
            }}))).into_response(),
        }
    }
}
impl From<ApiError> for Error {
    fn from(error: ApiError) -> Self {
        Self::Api(error)
    }
}
fn invalid(error: GraphError) -> Error {
    match error {
        GraphError::DependentReconciliation {
            merge_record,
            dependent_record,
        } => Error::Dependent {
            merge: merge_record.as_str().into(),
            dependent: dependent_record.as_str().into(),
        },
        other => {
            ApiError::bad_request("INVALID_RECONCILIATION_OPERATION", other.to_string()).into()
        }
    }
}
fn engine_error(error: EngineError) -> Error {
    match error {
        EngineError::Graph(error) => invalid(error),
        EngineError::InvalidConfiguration { field, reason } => ApiError::bad_request(
            "RECONCILIATION_MUTATION_FORBIDDEN",
            format!("{field}: {reason}"),
        )
        .into(),
        other => ApiError::internal("RECONCILIATION_STORAGE_FAILED", other.to_string()).into(),
    }
}
fn inspection(graph: &Graph, id: &ReconciliationRecordId) -> Result<Value, GraphError> {
    let stores = graph.epistemic_stores();
    let record = stores
        .reconciliations
        .record_by_id(id)
        .ok_or_else(|| GraphError::InvalidPropertyValue("unknown reconciliation".into()))?;
    let roots = graph.resolved_mentions()?;
    let left = roots
        .get(record.left())
        .ok_or_else(|| GraphError::InvalidPropertyValue("missing left mention".into()))?;
    let right = roots
        .get(record.right())
        .ok_or_else(|| GraphError::InvalidPropertyValue("missing right mention".into()))?;
    // Include original observation bindings and relation features for inspection.
    let members = stores
        .mentions
        .mentions()
        .iter()
        .filter(|mention| {
            roots
                .get(mention.id())
                .is_some_and(|root| root == left || root == right)
        })
        .collect::<Vec<_>>();
    Ok(
        json!({"ok":true,"record":record,"active":stores.merges.is_active(id),
        "dependent_record":stores.merges.dependent_record(id).map(|id| id.as_str()),
        "resolved_left":left.as_str(),"resolved_right":right.as_str(),"members":members,
        "undos":stores.merges.undos().into_iter().filter(|undo| undo.reconciliation_id() == id).collect::<Vec<_>>() }),
    )
}
async fn mutate(
    state: AppState,
    context: Context,
    operation: impl FnOnce(&mut Graph) -> Result<Value, GraphError> + Send + 'static,
) -> Result<Json<Value>, Error> {
    let timeout = std::time::Duration::from_millis(state.config.request_timeout_ms);
    let context = EngineMutationContext::new(
        context
            .workspace_id
            .unwrap_or_else(|| "http-default".into()),
        context
            .session_id
            .unwrap_or_else(|| "http-reconciliations".into()),
        context
            .budget_ref
            .unwrap_or_else(|| "http-reconciliations".into()),
    );
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
    .map_err(|_| ApiError::timeout("REQUEST_TIMEOUT", "reconciliation operation timeout"))?
    .map_err(|error| ApiError::internal("TASK_JOIN_FAILED", error.to_string()))??;
    Ok(Json(result))
}
pub async fn submit(
    State(state): State<AppState>,
    Json(payload): Json<Submission>,
) -> Result<Json<Value>, Error> {
    // Recording a judgment never applies a merge implicitly.
    mutate(state, payload.context, move |graph| {
        let id = graph.record_reconciliation(payload.record)?;
        inspection(graph, &id)
    })
    .await
}
pub async fn apply(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(context): Json<Context>,
) -> Result<Json<Value>, Error> {
    let id = ReconciliationRecordId::new(id).map_err(invalid)?;
    mutate(state, context, move |graph| {
        graph.apply_reconciliation_merge(&id)?;
        inspection(graph, &id)
    })
    .await
}
pub async fn undo(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<Undo>,
) -> Result<Json<Value>, Error> {
    let id = ReconciliationRecordId::new(id).map_err(invalid)?;
    let undo = MergeUndo::new(
        payload.id,
        id.clone(),
        ActorId::new(payload.actor).map_err(invalid)?,
        TemporalTimestamp::new(payload.undone_at).map_err(invalid)?,
        payload.rationale,
    )
    .map_err(invalid)?;
    mutate(state, payload.context, move |graph| {
        graph.undo_reconciliation_merge(undo)?;
        inspection(graph, &id)
    })
    .await
}
pub async fn inspect(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, Error> {
    let id = ReconciliationRecordId::new(id).map_err(invalid)?;
    let timeout = std::time::Duration::from_millis(state.config.request_timeout_ms);
    let result = tokio::time::timeout(
        timeout,
        tokio::task::spawn_blocking(move || {
            let mut engine = state
                .engine
                .lock()
                .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "engine lock poisoned"))?;
            engine.hydrate_full_graph().map_err(engine_error)?;
            if engine
                .graph()
                .epistemic_stores()
                .reconciliations
                .record_by_id(&id)
                .is_none()
            {
                return Err(ApiError::not_found(
                    "RECONCILIATION_NOT_FOUND",
                    "unknown reconciliation",
                )
                .into());
            }
            inspection(engine.graph(), &id).map_err(invalid)
        }),
    )
    .await
    .map_err(|_| ApiError::timeout("REQUEST_TIMEOUT", "reconciliation inspection timeout"))?
    .map_err(|error| ApiError::internal("TASK_JOIN_FAILED", error.to_string()))??;
    Ok(Json(result))
}

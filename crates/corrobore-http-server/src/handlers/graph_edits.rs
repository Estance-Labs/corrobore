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

//! `POST /v1/graph/edits` (issues #352 and #366): a batch of node and
//! relationship edits published as one commit on a branch, `main` included.

use std::collections::BTreeMap;

use axum::{Json, extract::State, http::StatusCode};
use graph_core::PropertyValue;
use graph_storage::{GraphEdit, GraphEditRequest, GraphStorageError, MAIN_BRANCH, MAX_GRAPH_EDITS};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    app::AppState,
    error::ApiError,
    handlers::graph_history::{Envelope, map_history_error, ok, parse_commit_id, with_store},
};

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum EditBody {
    SetProperties {
        node_id: String,
        #[serde(default)]
        set: BTreeMap<String, Value>,
        #[serde(default)]
        remove: Vec<String>,
    },
    CreateNode {
        labels: Vec<String>,
        #[serde(default)]
        properties: BTreeMap<String, Value>,
    },
    DeleteNode {
        node_id: String,
    },
    CreateRelationship {
        source_id: String,
        target_id: String,
        #[serde(rename = "type")]
        rel_type: String,
        #[serde(default)]
        properties: BTreeMap<String, Value>,
    },
    SetRelationshipProperties {
        relationship_id: String,
        #[serde(default)]
        set: BTreeMap<String, Value>,
        #[serde(default)]
        remove: Vec<String>,
    },
    DeleteRelationship {
        relationship_id: String,
    },
}

#[derive(Debug, Deserialize)]
pub struct EditsBody {
    pub branch: Option<String>,
    pub expected_head: Option<String>,
    pub message: Option<String>,
    pub actor: Option<String>,
    /// Raw edits, so one malformed edit is reported with its position.
    pub edits: Vec<Value>,
}

#[derive(Debug, Serialize)]
pub struct EditsResult {
    pub commit_id: String,
    pub branch: String,
    pub created: Vec<String>,
    pub created_relationships: Vec<String>,
    pub changed: usize,
}

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::bad_request("INVALID_EDIT", message)
}

/// A JSON value as a typed property: scalars, homogeneous lists (integers
/// and floats mix into a float list), never objects or mixed lists.
fn property_value(key: &str, value: &Value) -> Result<PropertyValue, ApiError> {
    let unsupported = || {
        invalid(format!(
            "property {key:?} must be a scalar or a list of one scalar type"
        ))
    };
    Ok(match value {
        Value::Null => PropertyValue::Null,
        Value::Bool(value) => PropertyValue::Bool(*value),
        Value::Number(number) => match number.as_i64() {
            Some(integer) => PropertyValue::Integer(integer),
            None => PropertyValue::Float(number.as_f64().ok_or_else(unsupported)?),
        },
        Value::String(value) => PropertyValue::String(value.clone()),
        Value::Array(items) if items.iter().all(Value::is_string) => PropertyValue::StringList(
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect(),
        ),
        Value::Array(items) if items.iter().all(Value::is_boolean) => {
            PropertyValue::BoolList(items.iter().filter_map(Value::as_bool).collect())
        }
        Value::Array(items) if items.iter().all(Value::is_i64) => {
            PropertyValue::IntegerList(items.iter().filter_map(Value::as_i64).collect())
        }
        Value::Array(items) if items.iter().all(Value::is_number) => {
            PropertyValue::FloatList(items.iter().filter_map(Value::as_f64).collect())
        }
        _ => return Err(unsupported()),
    })
}

fn properties(map: BTreeMap<String, Value>) -> Result<BTreeMap<String, PropertyValue>, ApiError> {
    map.into_iter()
        .map(|(key, value)| property_value(&key, &value).map(|value| (key, value)))
        .collect()
}

fn edit_error(error: GraphStorageError) -> ApiError {
    match error {
        GraphStorageError::GraphEditInvalid { index, reason } => {
            invalid(format!("edit {index}: {reason}"))
        }
        GraphStorageError::GraphEditNodeNotFound { branch, node_id } => ApiError::not_found(
            "GRAPH_NODE_NOT_FOUND",
            format!("node {node_id} is not on branch {branch}"),
        ),
        GraphStorageError::GraphEditRelationshipNotFound {
            branch,
            relationship_id,
        } => ApiError::not_found(
            "GRAPH_RELATIONSHIP_NOT_FOUND",
            format!("relationship {relationship_id} is not on branch {branch}"),
        ),
        GraphStorageError::GraphEditBlocked {
            branch,
            node_id,
            reason,
        } => ApiError {
            status: StatusCode::CONFLICT,
            code: "GRAPH_EDIT_BLOCKED",
            message: format!("node {node_id} on branch {branch} cannot be deleted: {reason}"),
        },
        other => map_history_error(other),
    }
}

/// `POST /v1/graph/edits`.
pub async fn apply(
    State(state): State<AppState>,
    Json(body): Json<EditsBody>,
) -> Result<Json<Envelope<EditsResult>>, ApiError> {
    if body.edits.is_empty() || body.edits.len() > MAX_GRAPH_EDITS {
        return Err(invalid(format!(
            "edits must hold 1 to {MAX_GRAPH_EDITS} entries"
        )));
    }
    let mut edits = Vec::with_capacity(body.edits.len());
    for (index, raw) in body.edits.into_iter().enumerate() {
        let edit: EditBody = serde_json::from_value(raw)
            .map_err(|error| invalid(format!("edit {index}: {error}")))?;
        edits.push(match edit {
            EditBody::SetProperties {
                node_id,
                set,
                remove,
            } => GraphEdit::SetProperties {
                node_id,
                set: properties(set)?,
                remove,
            },
            EditBody::CreateNode {
                labels,
                properties: values,
            } => GraphEdit::CreateNode {
                labels,
                properties: properties(values)?,
            },
            EditBody::DeleteNode { node_id } => GraphEdit::DeleteNode { node_id },
            EditBody::CreateRelationship {
                source_id,
                target_id,
                rel_type,
                properties: values,
            } => GraphEdit::CreateRelationship {
                source_id,
                target_id,
                rel_type,
                properties: properties(values)?,
            },
            EditBody::SetRelationshipProperties {
                relationship_id,
                set,
                remove,
            } => GraphEdit::SetRelationshipProperties {
                relationship_id,
                set: properties(set)?,
                remove,
            },
            EditBody::DeleteRelationship { relationship_id } => {
                GraphEdit::DeleteRelationship { relationship_id }
            }
        });
    }
    let expected_head = body
        .expected_head
        .as_deref()
        .map(parse_commit_id)
        .transpose()?;
    let branch = body
        .branch
        .filter(|branch| !branch.trim().is_empty())
        .unwrap_or_else(|| MAIN_BRANCH.to_owned());
    let actor = body
        .actor
        .filter(|actor| !actor.trim().is_empty())
        .unwrap_or_else(|| state.config.memory_actor_id.clone());
    with_store(&state, move |store| {
        let report = store
            .apply_graph_edits(GraphEditRequest {
                branch,
                expected_head,
                actor: Some(actor),
                message: body.message,
                edits,
            })
            .map_err(edit_error)?;
        Ok(ok(EditsResult {
            commit_id: report.commit_id.to_string(),
            branch: report.branch,
            created: report.created,
            created_relationships: report.created_relationships,
            changed: report.changed,
        }))
    })
    .await
}

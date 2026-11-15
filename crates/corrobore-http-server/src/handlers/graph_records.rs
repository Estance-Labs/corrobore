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

//! Typed record browsing pinned to one graph commit (issue #318).

use std::collections::BTreeMap;

use axum::{
    Json,
    extract::{Path, Query, State},
};
use graph_core::{Node, PropertyValue, RecordStatus, Relationship};
use graph_storage::{HistoryRecordKind, NeighbourDirection};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    app::AppState,
    error::ApiError,
    handlers::graph_history::{Envelope, map_history_error, ok, parse_commit_id, with_store},
};

const DEFAULT_RECORD_PAGE: usize = 100;
const MAX_RECORD_PAGE: usize = 1_000;
const DEFAULT_GROUP_ITEMS: usize = 8;
const MAX_GROUP_ITEMS: usize = 200;

#[derive(Debug, Default, Deserialize)]
pub struct RecordListQuery {
    pub branch: Option<String>,
    pub commit: Option<String>,
    /// `node` (default) or `relationship`.
    pub kind: Option<String>,
    /// Node label, or relationship type for relationships.
    pub label: Option<String>,
    /// Last record id of the previous page.
    pub cursor: Option<String>,
    /// Page size, 1-1000. Defaults to 100.
    pub limit: Option<usize>,
    /// Include tombstoned versions. Defaults to false.
    pub include_deleted: Option<bool>,
}

/// One node at a commit.
#[derive(Debug, Serialize)]
pub struct NodeRecordView {
    pub id: String,
    pub version_id: String,
    pub version: u64,
    pub labels: Vec<String>,
    pub properties: BTreeMap<String, Value>,
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_seen: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<String>,
}

/// One relationship at a commit.
#[derive(Debug, Serialize)]
pub struct RelationshipRecordView {
    pub id: String,
    pub version_id: String,
    pub version: u64,
    #[serde(rename = "type")]
    pub relationship_type: String,
    pub source: String,
    pub target: String,
    pub properties: BTreeMap<String, Value>,
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct RecordListResult {
    pub commit_id: String,
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub total: usize,
    pub nodes: Vec<NodeRecordView>,
    pub relationships: Vec<RelationshipRecordView>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct NodeQuery {
    pub branch: Option<String>,
    pub commit: Option<String>,
    /// Relationships returned per group, 0-200. Defaults to 8.
    pub group_limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct NeighbourView {
    pub relationship: RelationshipRecordView,
    pub node: NodeRecordView,
}

#[derive(Debug, Serialize)]
pub struct NeighbourGroupView {
    #[serde(rename = "type")]
    pub relationship_type: String,
    pub direction: &'static str,
    pub count: usize,
    pub items: Vec<NeighbourView>,
}

#[derive(Debug, Serialize)]
pub struct NodeDetailResult {
    pub commit_id: String,
    pub node: NodeRecordView,
    pub groups: Vec<NeighbourGroupView>,
}

/// JSON value of a typed property.
pub(crate) fn property_json(value: &PropertyValue) -> Value {
    match value {
        PropertyValue::Null => Value::Null,
        PropertyValue::Bool(value) => Value::Bool(*value),
        PropertyValue::Integer(value) => Value::from(*value),
        PropertyValue::Float(value) => Value::from(*value),
        PropertyValue::String(value) => Value::String(value.clone()),
        PropertyValue::StringList(values) => Value::from(values.clone()),
        PropertyValue::IntegerList(values) => Value::from(values.clone()),
        PropertyValue::FloatList(values) => Value::from(values.clone()),
        PropertyValue::BoolList(values) => Value::from(values.clone()),
        PropertyValue::Json(value) => value.clone(),
    }
}

fn properties_json(properties: &graph_core::PropertyMap) -> BTreeMap<String, Value> {
    properties
        .iter()
        .map(|(key, value)| (key.clone(), property_json(value)))
        .collect()
}

pub(crate) fn status_name(status: RecordStatus) -> &'static str {
    match status {
        RecordStatus::Candidate => "candidate",
        RecordStatus::NeedsEvidence => "needs_evidence",
        RecordStatus::NeedsReview => "needs_review",
        RecordStatus::Validated => "validated",
        RecordStatus::Rejected => "rejected",
        RecordStatus::Exportable => "exportable",
        RecordStatus::Exported => "exported",
        RecordStatus::Tombstoned => "tombstoned",
    }
}

pub(crate) fn node_view(node: &Node) -> NodeRecordView {
    let mut labels = node.labels().to_vec();
    labels.sort();
    NodeRecordView {
        id: node.id().as_str().to_owned(),
        version_id: node.version_id().as_str().to_owned(),
        version: node.version(),
        labels,
        properties: properties_json(node.properties()),
        status: status_name(node.status()),
        confidence: node.confidence().map(|value| value.value()),
        first_seen: node.first_seen().map(str::to_owned),
        last_seen: node.last_seen().map(str::to_owned),
    }
}

pub(crate) fn relationship_view(relationship: &Relationship) -> RelationshipRecordView {
    RelationshipRecordView {
        id: relationship.id().as_str().to_owned(),
        version_id: relationship.version_id().as_str().to_owned(),
        version: relationship.version(),
        relationship_type: relationship.rel_type().as_str().to_owned(),
        source: relationship.source().as_str().to_owned(),
        target: relationship.target().as_str().to_owned(),
        properties: properties_json(relationship.properties()),
        status: status_name(relationship.status()),
        confidence: relationship.confidence().map(|value| value.value()),
    }
}

fn parse_kind(kind: Option<&str>) -> Result<HistoryRecordKind, ApiError> {
    match kind.map(str::trim).unwrap_or("node") {
        "node" | "nodes" => Ok(HistoryRecordKind::Node),
        "relationship" | "relationships" => Ok(HistoryRecordKind::Relationship),
        other => Err(ApiError::bad_request(
            "INVALID_RECORD_KIND",
            format!("kind must be node or relationship, not {other:?}"),
        )),
    }
}

/// `GET /v1/graph/records`: one keyset page of records at a commit.
pub async fn list_records(
    State(state): State<AppState>,
    Query(query): Query<RecordListQuery>,
) -> Result<Json<Envelope<RecordListResult>>, ApiError> {
    let kind = parse_kind(query.kind.as_deref())?;
    let commit = query.commit.as_deref().map(parse_commit_id).transpose()?;
    let limit = query
        .limit
        .unwrap_or(DEFAULT_RECORD_PAGE)
        .clamp(1, MAX_RECORD_PAGE);
    with_store(&state, move |store| {
        let commit_id = store
            .resolve_graph_ref(query.branch.as_deref(), commit.as_ref())
            .map_err(map_history_error)?;
        let page = store
            .list_commit_records(
                &commit_id,
                kind,
                query.label.as_deref(),
                query.cursor.as_deref(),
                limit,
                query.include_deleted.unwrap_or(false),
            )
            .map_err(map_history_error)?;
        Ok(ok(RecordListResult {
            commit_id: commit_id.to_string(),
            kind: crate::handlers::graph_history::kind_name(kind),
            label: query.label,
            total: page.total,
            nodes: page.nodes.iter().map(node_view).collect(),
            relationships: page.relationships.iter().map(relationship_view).collect(),
            next_cursor: page.next_cursor,
        }))
    })
    .await
}

/// `GET /v1/graph/nodes/{node_id}`: a node and its relationship groups at a commit.
pub async fn get_node(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
    Query(query): Query<NodeQuery>,
) -> Result<Json<Envelope<NodeDetailResult>>, ApiError> {
    let commit = query.commit.as_deref().map(parse_commit_id).transpose()?;
    let group_limit = query
        .group_limit
        .unwrap_or(DEFAULT_GROUP_ITEMS)
        .min(MAX_GROUP_ITEMS);
    with_store(&state, move |store| {
        let commit_id = store
            .resolve_graph_ref(query.branch.as_deref(), commit.as_ref())
            .map_err(map_history_error)?;
        let Some(neighbourhood) = store
            .commit_node(&commit_id, &node_id, group_limit)
            .map_err(map_history_error)?
        else {
            return Err(ApiError::not_found(
                "GRAPH_NODE_NOT_FOUND",
                format!("node {node_id} is not visible at {commit_id}"),
            ));
        };
        Ok(ok(NodeDetailResult {
            commit_id: commit_id.to_string(),
            node: node_view(&neighbourhood.node),
            groups: neighbourhood
                .groups
                .iter()
                .map(|group| NeighbourGroupView {
                    relationship_type: group.relationship_type.clone(),
                    direction: match group.direction {
                        NeighbourDirection::Outgoing => "outgoing",
                        NeighbourDirection::Incoming => "incoming",
                    },
                    count: group.count,
                    items: group
                        .items
                        .iter()
                        .map(|(relationship, node)| NeighbourView {
                            relationship: relationship_view(relationship),
                            node: node_view(node),
                        })
                        .collect(),
                })
                .collect(),
        }))
    })
    .await
}

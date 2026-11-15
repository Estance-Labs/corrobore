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

//! Schema route (issue #323).

use axum::{
    Json,
    extract::{Query, State},
};
use graph_storage::{GraphSchema, SchemaProperty};
use serde::{Deserialize, Serialize};

use crate::{
    app::AppState,
    error::ApiError,
    handlers::graph_history::{Envelope, map_history_error, ok, parse_commit_id, with_store},
};

#[derive(Debug, Default, Deserialize)]
pub struct SchemaQuery {
    pub branch: Option<String>,
    pub commit: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SchemaPropertyView {
    pub id: u64,
    pub key: String,
    pub value_types: Vec<String>,
    pub count: usize,
}

#[derive(Debug, Serialize)]
pub struct SchemaLabelView {
    pub id: u64,
    pub name: String,
    pub count: usize,
    pub properties: Vec<SchemaPropertyView>,
}

#[derive(Debug, Serialize)]
pub struct SchemaEndpointView {
    pub source_label: String,
    pub target_label: String,
    pub count: usize,
}

#[derive(Debug, Serialize)]
pub struct SchemaTypeView {
    pub id: u64,
    pub name: String,
    pub count: usize,
    pub endpoints: Vec<SchemaEndpointView>,
    pub properties: Vec<SchemaPropertyView>,
}

#[derive(Debug, Serialize)]
pub struct SchemaResult {
    pub commit_id: String,
    pub labels: Vec<SchemaLabelView>,
    pub relationship_types: Vec<SchemaTypeView>,
}

fn properties(properties: &[SchemaProperty]) -> Vec<SchemaPropertyView> {
    properties
        .iter()
        .map(|property| SchemaPropertyView {
            id: property.id,
            key: property.key.clone(),
            value_types: property.value_types.clone(),
            count: property.count,
        })
        .collect()
}

fn view(schema: &GraphSchema) -> SchemaResult {
    SchemaResult {
        commit_id: schema.commit_id.to_string(),
        labels: schema
            .labels
            .iter()
            .map(|label| SchemaLabelView {
                id: label.id,
                name: label.name.clone(),
                count: label.count,
                properties: properties(&label.properties),
            })
            .collect(),
        relationship_types: schema
            .relationship_types
            .iter()
            .map(|kind| SchemaTypeView {
                id: kind.id,
                name: kind.name.clone(),
                count: kind.count,
                endpoints: kind
                    .endpoints
                    .iter()
                    .map(|endpoint| SchemaEndpointView {
                        source_label: endpoint.source_label.clone(),
                        target_label: endpoint.target_label.clone(),
                        count: endpoint.count,
                    })
                    .collect(),
                properties: properties(&kind.properties),
            })
            .collect(),
    }
}

/// `GET /v1/graph/schema`: labels, relationship types and property keys at a commit.
pub async fn schema(
    State(state): State<AppState>,
    Query(query): Query<SchemaQuery>,
) -> Result<Json<Envelope<SchemaResult>>, ApiError> {
    let commit = query.commit.as_deref().map(parse_commit_id).transpose()?;
    with_store(&state, move |store| {
        let commit_id = store
            .resolve_graph_ref(query.branch.as_deref(), commit.as_ref())
            .map_err(map_history_error)?;
        let schema = store.graph_schema(&commit_id).map_err(map_history_error)?;
        Ok(ok(view(&schema)))
    })
    .await
}

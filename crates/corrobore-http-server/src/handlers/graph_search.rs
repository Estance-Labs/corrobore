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

//! Vector and hybrid retrieval routes (issue #326).

use std::collections::BTreeMap;

use axum::{Json, extract::State};
use graph_storage::{
    CanonicalAccessContext, DEFAULT_EMBEDDING_PROPERTY, GraphSearchHit, HybridSearchRequest,
    VectorMetric,
};
use serde::{Deserialize, Serialize};

use crate::{
    app::AppState,
    error::ApiError,
    handlers::graph_history::{Envelope, map_history_error, ok, parse_commit_id, with_store},
};

const DEFAULT_K: usize = 10;
const MAX_K: usize = 1_000;
const MAX_DIMENSIONS: usize = 8_192;

#[derive(Debug, Deserialize)]
pub struct VectorSearchBody {
    pub vector: Vec<f32>,
    /// Embedding property; defaults to `embedding`.
    pub property: Option<String>,
    pub k: Option<usize>,
    /// `cosine` (default) or `l2`.
    pub metric: Option<String>,
    pub label: Option<String>,
    pub branch: Option<String>,
    pub commit: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct HybridSearchBody {
    pub text: Option<String>,
    pub vector: Option<Vec<f32>>,
    pub property: Option<String>,
    pub k: Option<usize>,
    #[serde(default = "default_true")]
    pub graph_expansion: bool,
    pub branch: Option<String>,
    pub commit: Option<String>,
}

const fn default_true() -> bool {
    true
}

#[derive(Debug, Serialize)]
pub struct SearchHitView {
    pub node_id: String,
    pub score: f64,
    pub labels: Vec<String>,
    pub ranks: BTreeMap<&'static str, usize>,
}

#[derive(Debug, Serialize)]
pub struct SearchResult {
    pub commit_id: String,
    pub hits: Vec<SearchHitView>,
}

fn hit_view(hit: GraphSearchHit) -> SearchHitView {
    SearchHitView {
        node_id: hit.node_id,
        score: hit.score,
        labels: hit.labels,
        ranks: hit.ranks,
    }
}

fn search_error(error: graph_storage::GraphStorageError) -> ApiError {
    match error {
        graph_storage::GraphStorageError::OperationFailed {
            operation: "graph_search",
            message,
        } => ApiError::bad_request("INVALID_SEARCH", message),
        other => map_history_error(other),
    }
}

fn check_vector(vector: &[f32]) -> Result<(), ApiError> {
    if vector.is_empty() || vector.len() > MAX_DIMENSIONS {
        return Err(ApiError::bad_request(
            "INVALID_SEARCH",
            format!("vectors need 1 to {MAX_DIMENSIONS} dimensions"),
        ));
    }
    Ok(())
}

/// `POST /v1/graph/search/vector`: nodes whose embedding is closest to a vector.
pub async fn vector_search(
    State(state): State<AppState>,
    Json(body): Json<VectorSearchBody>,
) -> Result<Json<Envelope<SearchResult>>, ApiError> {
    check_vector(&body.vector)?;
    let metric = match body.metric.as_deref().unwrap_or("cosine") {
        "cosine" => VectorMetric::Cosine,
        "l2" => VectorMetric::L2,
        other => {
            return Err(ApiError::bad_request(
                "INVALID_SEARCH",
                format!("metric must be cosine or l2, not {other:?}"),
            ));
        }
    };
    let commit = body.commit.as_deref().map(parse_commit_id).transpose()?;
    let k = body.k.unwrap_or(DEFAULT_K).clamp(1, MAX_K);
    with_store(&state, move |store| {
        let commit_id = store
            .resolve_graph_ref(body.branch.as_deref(), commit.as_ref())
            .map_err(map_history_error)?;
        let hits = store
            .vector_search(
                &commit_id,
                body.property
                    .as_deref()
                    .unwrap_or(DEFAULT_EMBEDDING_PROPERTY),
                &body.vector,
                k,
                metric,
                body.label.as_deref(),
            )
            .map_err(search_error)?;
        Ok(ok(SearchResult {
            commit_id: commit_id.to_string(),
            hits: hits.into_iter().map(hit_view).collect(),
        }))
    })
    .await
}

/// `POST /v1/graph/search/hybrid`: BM25, vector and graph rankings fused with RRF.
pub async fn hybrid_search(
    State(state): State<AppState>,
    Json(body): Json<HybridSearchBody>,
) -> Result<Json<Envelope<SearchResult>>, ApiError> {
    if let Some(vector) = &body.vector {
        check_vector(vector)?;
    }
    let commit = body.commit.as_deref().map(parse_commit_id).transpose()?;
    let request = HybridSearchRequest {
        text: body.text,
        vector: body.vector,
        property: body.property,
        k: body.k.unwrap_or(DEFAULT_K).clamp(1, MAX_K),
        graph_expansion: body.graph_expansion,
        // Full text is access-aware; without a caller identity only records
        // visible to an empty access context are returned.
        access: CanonicalAccessContext::default(),
    };
    with_store(&state, move |store| {
        let commit_id = store
            .resolve_graph_ref(body.branch.as_deref(), commit.as_ref())
            .map_err(map_history_error)?;
        let hits = store
            .hybrid_search(&commit_id, &request)
            .map_err(search_error)?;
        Ok(ok(SearchResult {
            commit_id: commit_id.to_string(),
            hits: hits.into_iter().map(hit_view).collect(),
        }))
    })
    .await
}

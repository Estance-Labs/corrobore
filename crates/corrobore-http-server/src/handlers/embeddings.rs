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

//! `POST /v1/embeddings`: vectors from the configured provider (issue #343),
//! so clients store node embeddings comparable with seed-search objectives.

use std::sync::Arc;

use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};

use crate::{
    app::AppState,
    embedding::{EmbeddingError, EmbeddingProvider},
    error::ApiError,
    handlers::graph_history::{Envelope, ok},
};

/// Most texts one request embeds.
pub const MAX_TEXTS: usize = 256;

#[derive(Debug, Deserialize)]
pub struct EmbeddingsBody {
    pub texts: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct EmbeddingsResult {
    pub provider: String,
    pub dimensions: usize,
    pub vectors: Vec<Vec<f32>>,
}

/// The configured provider, or `503 EMBEDDING_PROVIDER_NOT_CONFIGURED`.
pub(crate) fn provider(state: &AppState) -> Result<Arc<dyn EmbeddingProvider>, ApiError> {
    state.embedding.clone().ok_or_else(|| {
        ApiError::service_unavailable(
            "EMBEDDING_PROVIDER_NOT_CONFIGURED",
            "set CORROBORE_EMBEDDING_PROVIDER to hashing or http",
        )
    })
}

pub(crate) fn embedding_error(error: EmbeddingError) -> ApiError {
    match error {
        EmbeddingError::Invalid(message) => {
            ApiError::bad_request("INVALID_EMBEDDING_REQUEST", message)
        }
        EmbeddingError::Provider(message) => {
            ApiError::bad_gateway("EMBEDDING_PROVIDER_FAILED", message)
        }
    }
}

/// Embed texts on a blocking thread.
pub(crate) async fn embed_texts(
    provider: Arc<dyn EmbeddingProvider>,
    texts: Vec<String>,
) -> Result<Vec<Vec<f32>>, ApiError> {
    tokio::task::spawn_blocking(move || provider.embed(&texts).map_err(embedding_error))
        .await
        .map_err(|error| ApiError::internal("TASK_JOIN_FAILED", error.to_string()))?
}

/// `POST /v1/embeddings`.
pub async fn embed(
    State(state): State<AppState>,
    Json(body): Json<EmbeddingsBody>,
) -> Result<Json<Envelope<EmbeddingsResult>>, ApiError> {
    let provider = provider(&state)?;
    if body.texts.is_empty() || body.texts.len() > MAX_TEXTS {
        return Err(ApiError::bad_request(
            "INVALID_EMBEDDING_REQUEST",
            format!("texts must hold 1 to {MAX_TEXTS} entries"),
        ));
    }
    let name = provider.name();
    let vectors = embed_texts(provider, body.texts).await?;
    Ok(ok(EmbeddingsResult {
        provider: name,
        dimensions: vectors.first().map_or(0, Vec::len),
        vectors,
    }))
}

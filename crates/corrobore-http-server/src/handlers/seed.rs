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
use std::{sync::Arc, time::Duration};

use axum::{Json, extract::State};
use corrobore_engine::EngineError;
use graph_core::{
    GraphError, SemanticDomainProfile, SemanticSeedQueryRequest, SemanticSeedResolutionErrorCode,
    SemanticSeedRetrievalMode, WorkspaceId,
};
use graph_storage::{
    CanonicalAccessContext, DEFAULT_EMBEDDING_PROPERTY, HybridSearchRequest, VectorMetric,
};
use serde::{Deserialize, Serialize};

use crate::{
    app::{AppState, RuntimeStoreProvider},
    embedding::EmbeddingProvider,
    error::ApiError,
    handlers::{
        embeddings::embed_texts,
        graph_history::{canonical_store, map_history_error},
    },
};

const DEFAULT_TOP_K: usize = 10;
const DEFAULT_SCORE_THRESHOLD: f64 = 0.0;

#[derive(Debug, Deserialize)]
pub struct SeedSearchRequest {
    pub objective: String,
    pub workspace_id: Option<String>,
    pub domain_profile: Option<String>,
    pub mode: Option<String>,
    pub top_k: Option<usize>,
    pub score_threshold: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct SeedSearchResponse {
    pub ok: bool,
    pub result: SeedSearchResult,
}

#[derive(Debug, Serialize)]
pub struct SeedSearchResult {
    /// `vector`, `hybrid` (embedding-backed, issue #343) or `lexical`.
    pub retrieval: &'static str,
    /// Commit the embedding-backed retrieval read; `null` for lexical.
    pub commit_id: Option<String>,
    pub candidates: Vec<SeedCandidateView>,
}

#[derive(Debug, Serialize)]
pub struct SeedCandidateView {
    pub node_id: String,
    pub score: f64,
    pub explanation: SeedExplanationView,
}

#[derive(Debug, Serialize)]
pub struct SeedExplanationView {
    pub rationale: String,
    pub source_refs: Vec<String>,
    pub boundary_notes: Vec<String>,
}

pub async fn seed_search(
    State(state): State<AppState>,
    Json(payload): Json<SeedSearchRequest>,
) -> Result<Json<SeedSearchResponse>, ApiError> {
    let workspace_id = WorkspaceId::new(
        payload
            .workspace_id
            .unwrap_or_else(|| "workspace--http-default".to_owned()),
    )
    .map_err(|error| ApiError::bad_request("INVALID_WORKSPACE_ID", error.to_string()))?;

    let domain_profile = parse_domain_profile(payload.domain_profile.as_deref())?;
    enforce_profile_availability(&domain_profile)?;
    let mode = parse_retrieval_mode(payload.mode.as_deref())?;
    let top_k = payload.top_k.unwrap_or(DEFAULT_TOP_K);
    let score_threshold = payload.score_threshold.unwrap_or(DEFAULT_SCORE_THRESHOLD);

    // Issue #343: with an embedding provider and persistent storage, the
    // vector, semantic and hybrid modes go through the canonical vector and
    // hybrid retrieval at the head of main.
    let embedded_mode = matches!(
        mode,
        SemanticSeedRetrievalMode::Vector
            | SemanticSeedRetrievalMode::Semantic
            | SemanticSeedRetrievalMode::Hybrid
    );
    let mut fallback_note = None;
    let mut lexical_mode = mode;
    if embedded_mode && let Some(provider) = state.embedding.clone() {
        // The lexical resolver would otherwise claim vector retrieval is not
        // configured; say instead why it was not used.
        lexical_mode = SemanticSeedRetrievalMode::Hybrid;
        if matches!(state.runtime_store, RuntimeStoreProvider::Persistent(_)) {
            let outcome = embedded_seed_search(
                &state,
                provider,
                &payload.objective,
                mode,
                top_k,
                score_threshold,
            )
            .await?;
            match outcome {
                EmbeddedOutcome::Resolved(result) => {
                    return Ok(Json(SeedSearchResponse { ok: true, result }));
                }
                EmbeddedOutcome::NoEmbeddings => {
                    fallback_note = Some(
                        "no node carries an `embedding` vector at main; lexical fallback applied",
                    );
                }
            }
        } else {
            fallback_note =
                Some("vector retrieval needs persistent storage; lexical fallback applied");
        }
    }

    let request = SemanticSeedQueryRequest::new(
        payload.objective,
        workspace_id,
        domain_profile,
        lexical_mode,
        top_k,
        score_threshold,
    )
    .map_err(|error| ApiError::bad_request("INVALID_SEED_REQUEST", error.to_string()))?;

    let timeout = Duration::from_millis(state.config.request_timeout_ms);
    let engine = state.engine.clone();

    let candidates = tokio::time::timeout(
        timeout,
        tokio::task::spawn_blocking(move || {
            let locked = engine
                .lock()
                .map_err(|_| ApiError::internal("STATE_LOCK_FAILED", "engine lock poisoned"))?;

            let response = locked
                .seed_search_with_request(&request)
                .map_err(map_seed_engine_error)?;

            let views = response
                .seed_candidates()
                .iter()
                .map(|candidate| {
                    let mut boundary_notes = candidate.explanation().boundary_notes().to_vec();
                    if let Some(note) = fallback_note {
                        boundary_notes.push(note.to_owned());
                    }
                    SeedCandidateView {
                        node_id: candidate.node_id().as_str().to_owned(),
                        score: candidate.score(),
                        explanation: SeedExplanationView {
                            rationale: candidate.explanation().rationale().to_owned(),
                            source_refs: candidate.explanation().source_refs().to_vec(),
                            boundary_notes,
                        },
                    }
                })
                .collect::<Vec<_>>();

            Ok::<_, ApiError>(views)
        }),
    )
    .await
    .map_err(|_| ApiError::timeout("REQUEST_TIMEOUT", "seed search timeout"))?
    .map_err(|error| ApiError::internal("TASK_JOIN_FAILED", error.to_string()))??;

    Ok(Json(SeedSearchResponse {
        ok: true,
        result: SeedSearchResult {
            retrieval: "lexical",
            commit_id: None,
            candidates,
        },
    }))
}

enum EmbeddedOutcome {
    Resolved(SeedSearchResult),
    /// No node at main carries an embedding to compare with.
    NoEmbeddings,
}

/// Embed the objective and rank nodes by vector or hybrid retrieval at main.
async fn embedded_seed_search(
    state: &AppState,
    provider: Arc<dyn EmbeddingProvider>,
    objective: &str,
    mode: SemanticSeedRetrievalMode,
    top_k: usize,
    score_threshold: f64,
) -> Result<EmbeddedOutcome, ApiError> {
    let provider_name = provider.name();
    let vector = embed_texts(provider, vec![objective.to_owned()])
        .await?
        .pop()
        .ok_or_else(|| {
            ApiError::bad_gateway("EMBEDDING_PROVIDER_FAILED", "no embedding returned")
        })?;
    let store = canonical_store(state)?;
    let objective = objective.to_owned();
    let hybrid = mode == SemanticSeedRetrievalMode::Hybrid;
    let timeout = Duration::from_millis(state.config.request_timeout_ms);
    tokio::time::timeout(
        timeout,
        tokio::task::spawn_blocking(move || {
            let mut store = store.lock().map_err(|_| {
                ApiError::internal("STATE_LOCK_FAILED", "canonical store lock poisoned")
            })?;
            let commit_id = store
                .resolve_graph_ref(None, None)
                .map_err(map_history_error)?;
            let hits = if hybrid {
                store.hybrid_search(
                    &commit_id,
                    &HybridSearchRequest {
                        text: Some(objective),
                        vector: Some(vector),
                        property: None,
                        k: top_k,
                        graph_expansion: false,
                        access: CanonicalAccessContext::default(),
                    },
                )
            } else {
                store.vector_search(
                    &commit_id,
                    DEFAULT_EMBEDDING_PROPERTY,
                    &vector,
                    top_k,
                    VectorMetric::Cosine,
                    None,
                )
            }
            .map_err(embedded_search_error)?;
            if !hits.iter().any(|hit| hit.ranks.contains_key("vector")) {
                return Ok(EmbeddedOutcome::NoEmbeddings);
            }
            let note = format!("resolved at main commit {commit_id}");
            let candidates: Vec<SeedCandidateView> = hits
                .into_iter()
                .filter(|hit| hit.score >= score_threshold)
                .map(|hit| {
                    let rationale = if hybrid {
                        let ranks = hit
                            .ranks
                            .iter()
                            .map(|(ranking, rank)| format!("{ranking} #{rank}"))
                            .collect::<Vec<_>>()
                            .join(", ");
                        format!(
                            "reciprocal rank fusion {:.4} of {ranks}; objective embedded with {provider_name}",
                            hit.score
                        )
                    } else {
                        format!(
                            "vector cosine {:.4} on `{DEFAULT_EMBEDDING_PROPERTY}`; objective embedded with {provider_name}",
                            hit.score
                        )
                    };
                    SeedCandidateView {
                        node_id: hit.node_id,
                        score: hit.score,
                        explanation: SeedExplanationView {
                            rationale,
                            source_refs: Vec::new(),
                            boundary_notes: vec![note.clone()],
                        },
                    }
                })
                .collect();
            if candidates.is_empty() {
                return Err(ApiError::unprocessable(
                    "NO_SEED",
                    "No seed candidate scored above the threshold. (fix: Broaden objective wording or lower the score threshold.)",
                ));
            }
            Ok(EmbeddedOutcome::Resolved(SeedSearchResult {
                retrieval: if hybrid { "hybrid" } else { "vector" },
                commit_id: Some(commit_id.to_string()),
                candidates,
            }))
        }),
    )
    .await
    .map_err(|_| ApiError::timeout("REQUEST_TIMEOUT", "seed search timeout"))?
    .map_err(|error| ApiError::internal("TASK_JOIN_FAILED", error.to_string()))?
}

fn embedded_search_error(error: graph_storage::GraphStorageError) -> ApiError {
    match error {
        graph_storage::GraphStorageError::OperationFailed {
            operation: "graph_search",
            message,
        } => ApiError::unprocessable(
            "EMBEDDING_MISMATCH",
            format!("{message}; node embeddings must come from the configured provider"),
        ),
        other => map_history_error(other),
    }
}

fn map_seed_engine_error(error: EngineError) -> ApiError {
    match error {
        EngineError::Graph(error) => map_seed_error(error),
        other => ApiError::internal("SEED_RESOLUTION_FAILED", other.to_string()),
    }
}

/// Rejects the domain-scoped profiles: their seed semantics come from the CTI,
/// crisis, and FIMI packs, which Corrobore does not distribute. Only the
/// cross-domain profile resolves against the core runtime.
fn enforce_profile_availability(profile: &SemanticDomainProfile) -> Result<(), ApiError> {
    let module = match profile {
        SemanticDomainProfile::CtiInvestigation => "cti",
        SemanticDomainProfile::CrisisInvestigation => "crisis",
        SemanticDomainProfile::FimiInvestigation => "fimi",
        SemanticDomainProfile::CrossDomainInvestigation => return Ok(()),
    };

    Err(ApiError::forbidden(
        "FEATURE_NOT_AVAILABLE",
        format!("domain profile '{module}' is not distributed with Corrobore"),
    ))
}

fn map_seed_error(error: GraphError) -> ApiError {
    match error {
        GraphError::SemanticSeedResolutionFailed(details) => {
            let code = match details.code {
                SemanticSeedResolutionErrorCode::NoSeed => "NO_SEED",
                SemanticSeedResolutionErrorCode::AmbiguousSeed => "AMBIGUOUS_SEED",
                SemanticSeedResolutionErrorCode::OverbroadObjective => "OVERBROAD_OBJECTIVE",
            };

            ApiError::unprocessable(
                code,
                format!("{} (fix: {})", details.message, details.fix_hint),
            )
        }
        other => ApiError::internal("SEED_RESOLUTION_FAILED", other.to_string()),
    }
}

fn parse_domain_profile(value: Option<&str>) -> Result<SemanticDomainProfile, ApiError> {
    let Some(raw) = value else {
        return Ok(SemanticDomainProfile::CrossDomainInvestigation);
    };

    match raw.trim().to_ascii_lowercase().as_str() {
        "cti" => Ok(SemanticDomainProfile::CtiInvestigation),
        "fimi" => Ok(SemanticDomainProfile::FimiInvestigation),
        "crisis" => Ok(SemanticDomainProfile::CrisisInvestigation),
        "cross_domain" | "cross-domain" => Ok(SemanticDomainProfile::CrossDomainInvestigation),
        other => Err(ApiError::bad_request(
            "INVALID_DOMAIN_PROFILE",
            format!("unknown domain profile '{other}'; use cti, fimi, crisis, or cross_domain"),
        )),
    }
}

fn parse_retrieval_mode(value: Option<&str>) -> Result<SemanticSeedRetrievalMode, ApiError> {
    let Some(raw) = value else {
        return Ok(SemanticSeedRetrievalMode::Hybrid);
    };

    match raw.trim().to_ascii_lowercase().as_str() {
        "hybrid" => Ok(SemanticSeedRetrievalMode::Hybrid),
        "full_text" | "fulltext" | "full-text" => Ok(SemanticSeedRetrievalMode::FullText),
        "semantic" => Ok(SemanticSeedRetrievalMode::Semantic),
        "vector" => Ok(SemanticSeedRetrievalMode::Vector),
        other => Err(ApiError::bad_request(
            "INVALID_RETRIEVAL_MODE",
            format!("unknown retrieval mode '{other}'; use hybrid, full_text, semantic, or vector"),
        )),
    }
}

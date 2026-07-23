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
use std::time::Duration;

use axum::{Json, extract::State};
use corrobore_engine::EngineError;
use domain_provider_abi::DomainName;
use graph_core::{
    GraphError, SemanticDomainProfile, SemanticSeedQueryRequest, SemanticSeedResolutionErrorCode,
    SemanticSeedRetrievalMode, WorkspaceId,
};
use serde::{Deserialize, Serialize};

use crate::{app::AppState, error::ApiError};

const DEFAULT_TOP_K: usize = 10;
const DEFAULT_SCORE_THRESHOLD: f64 = 0.0;

#[cfg(feature = "enterprise-cti")]
const ENTERPRISE_CTI_ENABLED: bool = true;
#[cfg(not(feature = "enterprise-cti"))]
const ENTERPRISE_CTI_ENABLED: bool = false;

#[cfg(feature = "enterprise-crisis")]
const ENTERPRISE_CRISIS_ENABLED: bool = true;
#[cfg(not(feature = "enterprise-crisis"))]
const ENTERPRISE_CRISIS_ENABLED: bool = false;

#[cfg(feature = "enterprise-fimi")]
const ENTERPRISE_FIMI_ENABLED: bool = true;
#[cfg(not(feature = "enterprise-fimi"))]
const ENTERPRISE_FIMI_ENABLED: bool = false;

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
    enforce_profile_availability(&state, &domain_profile)?;
    let mode = parse_retrieval_mode(payload.mode.as_deref())?;
    let top_k = payload.top_k.unwrap_or(DEFAULT_TOP_K);
    let score_threshold = payload.score_threshold.unwrap_or(DEFAULT_SCORE_THRESHOLD);

    let request = SemanticSeedQueryRequest::new(
        payload.objective,
        workspace_id,
        domain_profile,
        mode,
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
                .map(|candidate| SeedCandidateView {
                    node_id: candidate.node_id().as_str().to_owned(),
                    score: candidate.score(),
                    explanation: SeedExplanationView {
                        rationale: candidate.explanation().rationale().to_owned(),
                        source_refs: candidate.explanation().source_refs().to_vec(),
                        boundary_notes: candidate.explanation().boundary_notes().to_vec(),
                    },
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
        result: SeedSearchResult { candidates },
    }))
}

fn map_seed_engine_error(error: EngineError) -> ApiError {
    match error {
        EngineError::Graph(error) => map_seed_error(error),
        other => ApiError::internal("SEED_RESOLUTION_FAILED", other.to_string()),
    }
}

fn enforce_profile_availability(
    state: &AppState,
    profile: &SemanticDomainProfile,
) -> Result<(), ApiError> {
    let (module, domain, feature_enabled) = match profile {
        SemanticDomainProfile::CtiInvestigation => ("cti", DomainName::Cti, ENTERPRISE_CTI_ENABLED),
        SemanticDomainProfile::CrisisInvestigation => {
            ("crisis", DomainName::Crisis, ENTERPRISE_CRISIS_ENABLED)
        }
        SemanticDomainProfile::FimiInvestigation => {
            ("fimi", DomainName::Fimi, ENTERPRISE_FIMI_ENABLED)
        }
        SemanticDomainProfile::CrossDomainInvestigation => return Ok(()),
    };

    if !feature_enabled {
        return Err(ApiError::forbidden(
            "FEATURE_NOT_AVAILABLE",
            format!("domain profile '{module}' requires enterprise-{module}"),
        ));
    }

    if !state.config.is_module_licensed(module) {
        return Err(ApiError::forbidden(
            "LICENSE_MODULE_MISSING",
            format!("domain profile '{module}' requires a valid {module} enterprise license"),
        ));
    }

    let provider_ready = state
        .domain_providers
        .as_deref()
        .and_then(|registry| registry.status(domain))
        .is_some_and(|status| status.ready);
    if !provider_ready {
        return Err(ApiError::service_unavailable(
            "DOMAIN_PROVIDER_NOT_READY",
            format!("domain profile '{module}' requires a ready enterprise provider"),
        ));
    }

    Ok(())
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

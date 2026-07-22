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
//! Semantic seed resolution contracts for semantic or hybrid graph entry.
//!
//! This module defines request/response data models and the resolver trait used
//! to map natural-language objectives to ranked graph seed node IDs.
//!
//! Boundaries:
//!
//! - Define typed contracts only.
//! - Validate deterministic request and response invariants.
//! - Do not implement production vector or semantic retrieval internals.

use serde::{Deserialize, Serialize};

use crate::{GraphError, NodeId, WorkspaceId};

/// Stable machine-readable semantic seed resolution error code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SemanticSeedResolutionErrorCode {
    /// No seed candidate reached resolver acceptance thresholds.
    #[serde(rename = "NO_SEED")]
    NoSeed,

    /// Candidate set is too ambiguous to pick reliable seed IDs.
    #[serde(rename = "AMBIGUOUS_SEED")]
    AmbiguousSeed,

    /// Objective is too broad to safely seed a bounded working set.
    #[serde(rename = "OVERBROAD_OBJECTIVE")]
    OverbroadObjective,
}

/// Typed semantic seed resolution error payload.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SemanticSeedResolutionError {
    /// Stable machine-readable error code.
    pub code: SemanticSeedResolutionErrorCode,

    /// Human-readable deterministic message.
    pub message: String,

    /// Human-readable deterministic remediation hint.
    pub fix_hint: String,

    /// Optional candidate count observed when resolution failed.
    pub candidate_count: Option<usize>,

    /// Optional score threshold in effect when resolution failed.
    pub threshold: Option<f64>,
}

impl SemanticSeedResolutionError {
    /// Creates a new instance.
    pub fn new(
        code: SemanticSeedResolutionErrorCode,
        message: impl Into<String>,
        fix_hint: impl Into<String>,
    ) -> Self {
        Self {
            code,
            // Message.
            message: message.into(),
            // Fix hint.
            fix_hint: fix_hint.into(),
            // Candidate count.
            candidate_count: None,
            // Threshold.
            threshold: None,
        }
    }

    /// Sets the candidate count.
    pub fn with_candidate_count(mut self, candidate_count: usize) -> Self {
        self.candidate_count = Some(candidate_count);
        self
    }

    /// Sets the threshold.
    pub fn with_threshold(mut self, threshold: f64) -> Self {
        self.threshold = Some(threshold);
        self
    }
}

/// Domain profile used to scope semantic seed resolution behavior.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SemanticDomainProfile {
    /// Cyber threat intelligence investigations.
    CtiInvestigation,

    /// Foreign information manipulation and interference investigations.
    FimiInvestigation,

    /// Crisis and emergency investigations.
    CrisisInvestigation,

    /// Cross-domain investigations spanning multiple profiles.
    CrossDomainInvestigation,
}

/// Retrieval mode requested for semantic seed resolution.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SemanticSeedRetrievalMode {
    /// Pure semantic retrieval behavior.
    Semantic,

    /// Vector-index based retrieval behavior.
    Vector,

    /// Full-text retrieval behavior.
    FullText,

    /// Hybrid retrieval behavior combining multiple retrieval signals.
    Hybrid,
}

/// Permission and retention policy toggle for contract-level boundary hooks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SemanticBoundaryPolicy {
    /// Respect configured boundaries.
    Enforce,

    /// Keep boundaries explicit in the request but allow relaxed execution.
    Relaxed,
}

/// Visibility scope used to constrain which sources may contribute seeds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceVisibilityScope {
    /// Only sources currently visible in the active workspace context.
    WorkspaceVisible,

    /// Only sources marked as trusted by caller policies.
    TrustedOnly,

    /// Explicit source references allowed for this request.
    ExplicitSources(Vec<String>),
}

/// Semantic seed query request contract.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SemanticSeedQueryRequest {
    objective: String,
    workspace_id: WorkspaceId,
    domain_profile: SemanticDomainProfile,
    retrieval_mode: SemanticSeedRetrievalMode,
    top_k: usize,
    score_threshold: f64,
    permission_boundary: SemanticBoundaryPolicy,
    retention_boundary: SemanticBoundaryPolicy,
    source_visibility: SourceVisibilityScope,
}

impl SemanticSeedQueryRequest {
    /// Build a semantic seed query request with deterministic validation.
    pub fn new(
        objective: impl Into<String>,
        workspace_id: WorkspaceId,
        domain_profile: SemanticDomainProfile,
        retrieval_mode: SemanticSeedRetrievalMode,
        top_k: usize,
        score_threshold: f64,
    ) -> Result<Self, GraphError> {
        let objective = objective.into();

        if objective.trim().is_empty() {
            return Err(GraphError::InvalidPropertyValue(
                "semantic seed objective must not be empty".to_owned(),
            ));
        }

        if top_k == 0 {
            return Err(GraphError::InvalidPropertyValue(
                "semantic seed top_k must be greater than zero".to_owned(),
            ));
        }

        if !score_threshold.is_finite() || !(0.0..=1.0).contains(&score_threshold) {
            return Err(GraphError::InvalidPropertyValue(
                "semantic seed score threshold must be finite and in [0, 1]".to_owned(),
            ));
        }

        Ok(Self {
            objective,
            workspace_id,
            domain_profile,
            retrieval_mode,
            top_k,
            score_threshold,
            permission_boundary: SemanticBoundaryPolicy::Enforce,
            retention_boundary: SemanticBoundaryPolicy::Enforce,
            source_visibility: SourceVisibilityScope::WorkspaceVisible,
        })
    }

    /// Set explicit permission boundary policy.
    pub fn with_permission_boundary(mut self, policy: SemanticBoundaryPolicy) -> Self {
        self.permission_boundary = policy;
        self
    }

    /// Set explicit retention boundary policy.
    pub fn with_retention_boundary(mut self, policy: SemanticBoundaryPolicy) -> Self {
        self.retention_boundary = policy;
        self
    }

    /// Set source visibility scope.
    pub fn with_source_visibility(mut self, scope: SourceVisibilityScope) -> Self {
        self.source_visibility = scope;
        self
    }

    /// Objective.
    pub fn objective(&self) -> &str {
        self.objective.as_str()
    }

    /// Workspace id.
    pub fn workspace_id(&self) -> &WorkspaceId {
        &self.workspace_id
    }

    /// Domain profile.
    pub fn domain_profile(&self) -> SemanticDomainProfile {
        self.domain_profile
    }

    /// Retrieval mode.
    pub fn retrieval_mode(&self) -> SemanticSeedRetrievalMode {
        self.retrieval_mode
    }

    /// Top k.
    pub fn top_k(&self) -> usize {
        self.top_k
    }

    /// Score threshold.
    pub fn score_threshold(&self) -> f64 {
        self.score_threshold
    }

    /// Permission boundary.
    pub fn permission_boundary(&self) -> SemanticBoundaryPolicy {
        self.permission_boundary
    }

    /// Retention boundary.
    pub fn retention_boundary(&self) -> SemanticBoundaryPolicy {
        self.retention_boundary
    }

    /// Source visibility.
    pub fn source_visibility(&self) -> &SourceVisibilityScope {
        &self.source_visibility
    }
}

/// Explanation metadata describing why a node became a semantic seed candidate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticSeedExplanationMetadata {
    rationale: String,
    source_refs: Vec<String>,
    boundary_notes: Vec<String>,
}

impl SemanticSeedExplanationMetadata {
    /// Creates a new instance.
    pub fn new(rationale: impl Into<String>, source_refs: Vec<String>) -> Self {
        Self {
            // Rationale.
            rationale: rationale.into(),
            source_refs,
            // Boundary notes.
            boundary_notes: Vec::new(),
        }
    }

    /// Sets the boundary note.
    pub fn with_boundary_note(mut self, note: impl Into<String>) -> Self {
        self.boundary_notes.push(note.into());
        self
    }

    /// Rationale.
    pub fn rationale(&self) -> &str {
        self.rationale.as_str()
    }

    /// Source refs.
    pub fn source_refs(&self) -> &[String] {
        &self.source_refs
    }

    /// Boundary notes.
    pub fn boundary_notes(&self) -> &[String] {
        &self.boundary_notes
    }
}

/// Ranked semantic seed candidate contract.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticSeedCandidate {
    node_id: NodeId,
    score: OrderedScore,
    explanation: SemanticSeedExplanationMetadata,
}

impl SemanticSeedCandidate {
    /// Creates a new instance.
    pub fn new(
        node_id: NodeId,
        score: f64,
        explanation: SemanticSeedExplanationMetadata,
    ) -> Result<Self, GraphError> {
        Ok(Self {
            node_id,
            score: OrderedScore::new(score)?,
            explanation,
        })
    }

    /// Node id.
    pub fn node_id(&self) -> &NodeId {
        &self.node_id
    }

    /// Score.
    pub fn score(&self) -> f64 {
        self.score.value
    }

    /// Explanation.
    pub fn explanation(&self) -> &SemanticSeedExplanationMetadata {
        &self.explanation
    }
}

/// Semantic seed query response contract.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SemanticSeedQueryResponse {
    request: SemanticSeedQueryRequest,
    seed_candidates: Vec<SemanticSeedCandidate>,
}

impl SemanticSeedQueryResponse {
    /// Creates a new instance.
    pub fn new(
        request: SemanticSeedQueryRequest,
        seed_candidates: Vec<SemanticSeedCandidate>,
    ) -> Result<Self, GraphError> {
        if seed_candidates.len() > request.top_k {
            return Err(GraphError::InvalidPropertyValue(format!(
                "semantic seed candidate count {} exceeds top_k {}",
                seed_candidates.len(),
                request.top_k
            )));
        }

        for candidate in &seed_candidates {
            if candidate.score() < request.score_threshold {
                return Err(GraphError::InvalidPropertyValue(format!(
                    "semantic seed candidate score {} is below score threshold {}",
                    candidate.score(),
                    request.score_threshold
                )));
            }
        }

        for pair in seed_candidates.windows(2) {
            if pair[0].score() < pair[1].score() {
                return Err(GraphError::InvalidPropertyValue(
                    "semantic seed candidates must be ranked by descending score".to_owned(),
                ));
            }
        }

        Ok(Self {
            request,
            seed_candidates,
        })
    }

    /// Request.
    pub fn request(&self) -> &SemanticSeedQueryRequest {
        &self.request
    }

    /// Seed candidates.
    pub fn seed_candidates(&self) -> &[SemanticSeedCandidate] {
        &self.seed_candidates
    }
}

/// Contract for components that resolve semantic objectives to seed node IDs.
pub trait SemanticSeedResolver {
    /// Resolves a semantic seed query into candidate graph seed nodes.
    fn resolve(
        &self,
        request: &SemanticSeedQueryRequest,
    ) -> Result<SemanticSeedQueryResponse, GraphError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
struct OrderedScore {
    value: f64,
}

impl Eq for OrderedScore {}

impl OrderedScore {
    fn new(value: f64) -> Result<Self, GraphError> {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(GraphError::InvalidPropertyValue(
                "semantic seed candidate score must be finite and in [0, 1]".to_owned(),
            ));
        }

        Ok(Self { value })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace_id(value: &str) -> WorkspaceId {
        WorkspaceId::new(value).expect("test workspace ID should be valid")
    }

    fn node_id(value: &str) -> NodeId {
        NodeId::new(value).expect("test node ID should be valid")
    }

    #[test]
    fn request_defaults_to_enforced_boundaries_and_workspace_visibility() {
        let request = SemanticSeedQueryRequest::new(
            "find seed nodes",
            workspace_id("workspace--1"),
            SemanticDomainProfile::CrossDomainInvestigation,
            SemanticSeedRetrievalMode::Hybrid,
            10,
            0.3,
        )
        .expect("request should be valid");

        assert_eq!(
            request.permission_boundary(),
            SemanticBoundaryPolicy::Enforce
        );
        assert_eq!(
            request.retention_boundary(),
            SemanticBoundaryPolicy::Enforce
        );
        assert_eq!(
            request.source_visibility(),
            &SourceVisibilityScope::WorkspaceVisible
        );
    }

    #[test]
    fn response_rejects_ascending_rank_order() {
        let request = SemanticSeedQueryRequest::new(
            "find seeds",
            workspace_id("workspace--1"),
            SemanticDomainProfile::CtiInvestigation,
            SemanticSeedRetrievalMode::Hybrid,
            2,
            0.1,
        )
        .expect("request should be valid");

        let low = SemanticSeedCandidate::new(
            node_id("node--low"),
            0.2,
            SemanticSeedExplanationMetadata::new("low", Vec::new()),
        )
        .expect("candidate should be valid");
        let high = SemanticSeedCandidate::new(
            node_id("node--high"),
            0.9,
            SemanticSeedExplanationMetadata::new("high", Vec::new()),
        )
        .expect("candidate should be valid");

        let error = SemanticSeedQueryResponse::new(request, vec![low, high])
            .expect_err("ascending scores should be rejected");

        assert!(matches!(
        error,
        GraphError::InvalidPropertyValue(message)
        if message.contains("ranked by descending score")
        ));
    }

    #[test]
    fn request_builder_accessors_and_validation_errors_are_explicit() {
        let request = SemanticSeedQueryRequest::new(
            "find narrative seeds",
            workspace_id("workspace--semantic-accessors"),
            SemanticDomainProfile::FimiInvestigation,
            SemanticSeedRetrievalMode::Vector,
            5,
            0.6,
        )
        .expect("request should be valid")
        .with_permission_boundary(SemanticBoundaryPolicy::Relaxed)
        .with_retention_boundary(SemanticBoundaryPolicy::Relaxed)
        .with_source_visibility(SourceVisibilityScope::ExplicitSources(vec![
            "source--1".to_owned(),
            "source--2".to_owned(),
        ]));

        assert_eq!(request.objective(), "find narrative seeds");
        assert_eq!(
            request.workspace_id(),
            &workspace_id("workspace--semantic-accessors")
        );
        assert_eq!(
            request.domain_profile(),
            SemanticDomainProfile::FimiInvestigation
        );
        assert_eq!(request.retrieval_mode(), SemanticSeedRetrievalMode::Vector);
        assert_eq!(request.top_k(), 5);
        assert_eq!(request.score_threshold(), 0.6);
        assert_eq!(
            request.permission_boundary(),
            SemanticBoundaryPolicy::Relaxed
        );
        assert_eq!(
            request.retention_boundary(),
            SemanticBoundaryPolicy::Relaxed
        );
        assert!(matches!(
        request.source_visibility(),
        SourceVisibilityScope::ExplicitSources(sources)
        if sources == &vec!["source--1".to_owned(), "source--2".to_owned()]
        ));

        assert!(matches!(
        SemanticSeedQueryRequest::new(
        " ",
        workspace_id("workspace--invalid-objective"),
        SemanticDomainProfile::CrossDomainInvestigation,
        SemanticSeedRetrievalMode::Hybrid,
        1,
        0.1,
        ),
        Err(GraphError::InvalidPropertyValue(message))
        if message.contains("objective must not be empty")
        ));

        assert!(matches!(
        SemanticSeedQueryRequest::new(
        "objective",
        workspace_id("workspace--invalid-top-k"),
        SemanticDomainProfile::CrossDomainInvestigation,
        SemanticSeedRetrievalMode::Hybrid,
        0,
        0.1,
        ),
        Err(GraphError::InvalidPropertyValue(message))
        if message.contains("top_k must be greater than zero")
        ));

        assert!(matches!(
        SemanticSeedQueryRequest::new(
        "objective",
        workspace_id("workspace--invalid-threshold"),
        SemanticDomainProfile::CrossDomainInvestigation,
        SemanticSeedRetrievalMode::Hybrid,
        1,
        1.5,
        ),
        Err(GraphError::InvalidPropertyValue(message))
        if message.contains("score threshold must be finite and in [0, 1]")
        ));
    }

    #[test]
    fn explanation_error_and_candidate_builders_expose_expected_fields() {
        let error = SemanticSeedResolutionError::new(
            SemanticSeedResolutionErrorCode::AmbiguousSeed,
            "ambiguous candidates",
            "narrow objective",
        )
        .with_candidate_count(4)
        .with_threshold(0.75);
        assert_eq!(error.code, SemanticSeedResolutionErrorCode::AmbiguousSeed);
        assert_eq!(error.message, "ambiguous candidates");
        assert_eq!(error.fix_hint, "narrow objective");
        assert_eq!(error.candidate_count, Some(4));
        assert_eq!(error.threshold, Some(0.75));

        let explanation = SemanticSeedExplanationMetadata::new(
            "matched campaign objective",
            vec!["source--a".to_owned()],
        )
        .with_boundary_note("permission boundary enforced")
        .with_boundary_note("retention boundary enforced");
        assert_eq!(explanation.rationale(), "matched campaign objective");
        assert_eq!(explanation.source_refs(), &["source--a".to_owned()]);
        assert_eq!(
            explanation.boundary_notes(),
            &[
                "permission boundary enforced".to_owned(),
                "retention boundary enforced".to_owned(),
            ]
        );

        let candidate = SemanticSeedCandidate::new(
            node_id("node--semantic-candidate"),
            0.88,
            explanation.clone(),
        )
        .expect("candidate should be valid");
        assert_eq!(candidate.node_id(), &node_id("node--semantic-candidate"));
        assert_eq!(candidate.score(), 0.88);
        assert_eq!(candidate.explanation(), &explanation);

        assert!(matches!(
        SemanticSeedCandidate::new(
        node_id("node--semantic-invalid"),
        -0.01,
        SemanticSeedExplanationMetadata::new("bad", Vec::new()),
        ),
        Err(GraphError::InvalidPropertyValue(message))
        if message.contains("candidate score must be finite and in [0, 1]")
        ));
    }

    #[test]
    fn response_validates_top_k_and_threshold_and_exposes_accessors() {
        let request = SemanticSeedQueryRequest::new(
            "seed objective",
            workspace_id("workspace--response-validation"),
            SemanticDomainProfile::CrisisInvestigation,
            SemanticSeedRetrievalMode::Semantic,
            1,
            0.7,
        )
        .expect("request should be valid");

        let high = SemanticSeedCandidate::new(
            node_id("node--high"),
            0.9,
            SemanticSeedExplanationMetadata::new("high", Vec::new()),
        )
        .expect("high candidate should be valid");
        let also_high = SemanticSeedCandidate::new(
            node_id("node--also-high"),
            0.8,
            SemanticSeedExplanationMetadata::new("also-high", Vec::new()),
        )
        .expect("second candidate should be valid");
        let low = SemanticSeedCandidate::new(
            node_id("node--low-threshold"),
            0.4,
            SemanticSeedExplanationMetadata::new("low", Vec::new()),
        )
        .expect("low candidate should still be constructible");

        let too_many =
            SemanticSeedQueryResponse::new(request.clone(), vec![high.clone(), also_high])
                .expect_err("candidate count above top_k should be rejected");
        assert!(matches!(
        too_many,
        GraphError::InvalidPropertyValue(message)
        if message.contains("candidate count") && message.contains("exceeds top_k")
        ));

        let below_threshold = SemanticSeedQueryResponse::new(request.clone(), vec![low])
            .expect_err("candidate below threshold should be rejected");
        assert!(matches!(
        below_threshold,
        GraphError::InvalidPropertyValue(message)
        if message.contains("is below score threshold")
        ));

        let response = SemanticSeedQueryResponse::new(request.clone(), vec![high])
            .expect("valid response should be accepted");
        assert_eq!(response.request(), &request);
        assert_eq!(response.seed_candidates().len(), 1);
    }
}

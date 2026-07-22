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
use graph_core::{
    GraphError, NodeId, SemanticBoundaryPolicy, SemanticDomainProfile, SemanticSeedCandidate,
    SemanticSeedExplanationMetadata, SemanticSeedQueryRequest, SemanticSeedQueryResponse,
    SemanticSeedResolutionError, SemanticSeedResolutionErrorCode, SemanticSeedResolver,
    SemanticSeedRetrievalMode, SourceVisibilityScope, WorkspaceId,
};

fn workspace_id(value: &str) -> WorkspaceId {
    WorkspaceId::new(value).expect("test workspace ID should be valid")
}

fn node_id(value: &str) -> NodeId {
    NodeId::new(value).expect("test node ID should be valid")
}

fn candidate(node_ref: &str, score: f64, rationale: &str, source: &str) -> SemanticSeedCandidate {
    SemanticSeedCandidate::new(
        node_id(node_ref),
        score,
        SemanticSeedExplanationMetadata::new(rationale, vec![source.to_owned()]),
    )
    .expect("fixture candidate should be valid")
}

fn cti_seed_fixtures() -> Vec<SemanticSeedCandidate> {
    vec![
        candidate(
            "campaign--cti-4",
            0.94,
            "semantic and keyword overlap on campaign reports",
            "source://cti/report-12",
        ),
        candidate(
            "indicator--cti-11",
            0.88,
            "indicator IOC co-occurrence in trusted CTI source",
            "source://cti/report-9",
        ),
    ]
}

fn fimi_seed_fixtures() -> Vec<SemanticSeedCandidate> {
    vec![
        candidate(
            "narrative--fimi-17",
            0.93,
            "narrative match with amplification and claim patterns",
            "source://fimi/corpus-7",
        ),
        candidate(
            "campaign--fimi-4",
            0.86,
            "hybrid rank across posts and campaign references",
            "source://fimi/corpus-5",
        ),
    ]
}

fn crisis_seed_fixtures() -> Vec<SemanticSeedCandidate> {
    vec![
        candidate(
            "event--crisis-9",
            0.91,
            "event and location semantic overlap with incident objective",
            "source://crisis/feed-3",
        ),
        candidate(
            "location--crisis-2",
            0.83,
            "location mention overlap with humanitarian context",
            "source://crisis/feed-6",
        ),
    ]
}

fn cross_domain_seed_fixtures() -> Vec<SemanticSeedCandidate> {
    vec![
        candidate(
            "campaign--cti-4",
            0.90,
            "cross-domain hybrid operation anchor in CTI campaign",
            "source://cross/analysis-2",
        ),
        candidate(
            "narrative--fimi-17",
            0.89,
            "cross-domain hybrid operation anchor in FIMI narrative",
            "source://cross/analysis-2",
        ),
    ]
}

struct FixtureSemanticSeedResolver;

impl FixtureSemanticSeedResolver {
    fn fixtures_for(
        domain_profile: SemanticDomainProfile,
        objective: &str,
    ) -> Result<Vec<SemanticSeedCandidate>, GraphError> {
        if objective.contains("no-seed") {
            return Err(GraphError::SemanticSeedResolutionFailed(
                SemanticSeedResolutionError::new(
                    SemanticSeedResolutionErrorCode::NoSeed,
                    "No seed candidate matched the semantic objective.",
                    "Narrow objective scope or lower score threshold.",
                ),
            ));
        }

        if objective.contains("ambiguous") {
            return Err(GraphError::SemanticSeedResolutionFailed(
                SemanticSeedResolutionError::new(
                    SemanticSeedResolutionErrorCode::AmbiguousSeed,
                    "Multiple seed candidates have near-identical ranking.",
                    "Add stronger qualifiers or constrain the domain profile.",
                )
                .with_candidate_count(12)
                .with_threshold(0.80),
            ));
        }

        if objective.contains("overbroad") {
            return Err(GraphError::SemanticSeedResolutionFailed(
                SemanticSeedResolutionError::new(
                    SemanticSeedResolutionErrorCode::OverbroadObjective,
                    "Objective matched too many high-degree entities.",
                    "Add stronger objective qualifiers before loading a working set.",
                )
                .with_candidate_count(1_500)
                .with_threshold(0.25),
            ));
        }

        let fixtures = match domain_profile {
            SemanticDomainProfile::CtiInvestigation => cti_seed_fixtures(),
            SemanticDomainProfile::FimiInvestigation => fimi_seed_fixtures(),
            SemanticDomainProfile::CrisisInvestigation => crisis_seed_fixtures(),
            SemanticDomainProfile::CrossDomainInvestigation => cross_domain_seed_fixtures(),
        };

        Ok(fixtures)
    }
}

impl SemanticSeedResolver for FixtureSemanticSeedResolver {
    fn resolve(
        &self,
        request: &SemanticSeedQueryRequest,
    ) -> Result<SemanticSeedQueryResponse, GraphError> {
        let fixtures = Self::fixtures_for(request.domain_profile(), request.objective())?;
        let filtered = fixtures
            .into_iter()
            .filter(|candidate| candidate.score() >= request.score_threshold())
            .take(request.top_k())
            .collect();

        SemanticSeedQueryResponse::new(request.clone(), filtered)
    }
}

//
// Validate acceptance flow where semantic or hybrid objectives resolve
// into ranked seed node IDs with explanation metadata across supported domains.
#[test]
fn epic_0012_acceptance_resolves_ranked_seed_ids_with_explanations_across_domains() {
    let resolver = FixtureSemanticSeedResolver;

    let scenarios = vec![
        (
            SemanticDomainProfile::CtiInvestigation,
            "find infrastructure tied to this CTI campaign",
            "workspace--cti",
        ),
        (
            SemanticDomainProfile::FimiInvestigation,
            "find narratives associated with this influence operation",
            "workspace--fimi",
        ),
        (
            SemanticDomainProfile::CrisisInvestigation,
            "find events and locations in this humanitarian emergency",
            "workspace--crisis",
        ),
        (
            SemanticDomainProfile::CrossDomainInvestigation,
            "find hybrid operation links across FIMI and CTI",
            "workspace--cross",
        ),
    ];

    for (profile, objective, workspace_ref) in scenarios {
        let request = SemanticSeedQueryRequest::new(
            objective,
            workspace_id(workspace_ref),
            profile,
            SemanticSeedRetrievalMode::Hybrid,
            5,
            0.80,
        )
        .expect("request should be valid")
        .with_permission_boundary(SemanticBoundaryPolicy::Enforce)
        .with_retention_boundary(SemanticBoundaryPolicy::Enforce)
        .with_source_visibility(SourceVisibilityScope::WorkspaceVisible);

        let response = resolver
            .resolve(&request)
            .expect("fixture resolver should produce ranked seeds");

        assert_eq!(response.request().domain_profile(), profile);
        assert!(!response.seed_candidates().is_empty());

        for ranked_pair in response.seed_candidates().windows(2) {
            assert!(ranked_pair[0].score() >= ranked_pair[1].score());
        }

        for seed in response.seed_candidates() {
            assert!(!seed.explanation().rationale().is_empty());
            assert!(!seed.explanation().source_refs().is_empty());
        }
    }
}

//
// Validate that no-seed and ambiguous-seed outcomes are explicit typed errors
// with machine-readable codes for deterministic handling.
#[test]
fn epic_0012_acceptance_returns_typed_semantic_seed_errors() {
    let resolver = FixtureSemanticSeedResolver;

    let no_seed_request = SemanticSeedQueryRequest::new(
        "no-seed objective for acceptance validation",
        workspace_id("workspace--cti"),
        SemanticDomainProfile::CtiInvestigation,
        SemanticSeedRetrievalMode::Semantic,
        5,
        0.50,
    )
    .expect("request should be valid");

    let no_seed_error = resolver
        .resolve(&no_seed_request)
        .expect_err("no-seed objective should return typed error");
    assert!(matches!(
        no_seed_error,
        GraphError::SemanticSeedResolutionFailed(SemanticSeedResolutionError {
            code: SemanticSeedResolutionErrorCode::NoSeed,
            ..
        })
    ));

    let ambiguous_request = SemanticSeedQueryRequest::new(
        "ambiguous objective for acceptance validation",
        workspace_id("workspace--cross"),
        SemanticDomainProfile::CrossDomainInvestigation,
        SemanticSeedRetrievalMode::Hybrid,
        5,
        0.50,
    )
    .expect("request should be valid");

    let ambiguous_error = resolver
        .resolve(&ambiguous_request)
        .expect_err("ambiguous objective should return typed error");
    assert!(matches!(
        ambiguous_error,
        GraphError::SemanticSeedResolutionFailed(SemanticSeedResolutionError {
            code: SemanticSeedResolutionErrorCode::AmbiguousSeed,
            candidate_count: Some(12),
            ..
        })
    ));
}

//
// Validate request-level workspace, permission, retention, and source visibility
// hooks remain explicit for boundary-aware resolver behavior.
#[test]
fn epic_0012_acceptance_exposes_workspace_and_boundary_hooks_in_request_contract() {
    let request = SemanticSeedQueryRequest::new(
        "find hybrid operation path",
        workspace_id("workspace--cross"),
        SemanticDomainProfile::CrossDomainInvestigation,
        SemanticSeedRetrievalMode::Hybrid,
        10,
        0.35,
    )
    .expect("request should be valid")
    .with_permission_boundary(SemanticBoundaryPolicy::Relaxed)
    .with_retention_boundary(SemanticBoundaryPolicy::Enforce)
    .with_source_visibility(SourceVisibilityScope::ExplicitSources(vec![
        "source://cross/analysis-2".to_owned(),
        "source://cti/report-12".to_owned(),
    ]));

    assert_eq!(request.workspace_id().as_str(), "workspace--cross");
    assert_eq!(
        request.domain_profile(),
        SemanticDomainProfile::CrossDomainInvestigation
    );
    assert_eq!(
        request.permission_boundary(),
        SemanticBoundaryPolicy::Relaxed
    );
    assert_eq!(
        request.retention_boundary(),
        SemanticBoundaryPolicy::Enforce
    );
    assert_eq!(
        request.source_visibility(),
        &SourceVisibilityScope::ExplicitSources(vec![
            "source://cross/analysis-2".to_owned(),
            "source://cti/report-12".to_owned(),
        ])
    );
}

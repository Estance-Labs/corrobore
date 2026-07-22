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
    GraphError, NodeId, SemanticDomainProfile, SemanticSeedCandidate,
    SemanticSeedExplanationMetadata, SemanticSeedQueryRequest, SemanticSeedQueryResponse,
    SemanticSeedResolver, SemanticSeedRetrievalMode, WorkspaceId,
};

fn workspace_id(value: &str) -> WorkspaceId {
    WorkspaceId::new(value).expect("test workspace ID should be valid")
}

fn node_id(value: &str) -> NodeId {
    NodeId::new(value).expect("test node ID should be valid")
}

//
// Verify the request contract validates required fields and preserves explicit
// retrieval policy controls before any production retrieval implementation.
#[test]
fn semantic_seed_query_request_enforces_objective_top_k_and_threshold_contract() {
    let request = SemanticSeedQueryRequest::new(
        "find infrastructure linked to this campaign",
        workspace_id("workspace--cti"),
        SemanticDomainProfile::CtiInvestigation,
        SemanticSeedRetrievalMode::Hybrid,
        5,
        0.42,
    )
    .expect("valid semantic seed request should be accepted");

    assert_eq!(
        request.objective(),
        "find infrastructure linked to this campaign"
    );
    assert_eq!(request.top_k(), 5);
    assert_eq!(request.score_threshold(), 0.42);
    assert_eq!(
        request.domain_profile(),
        SemanticDomainProfile::CtiInvestigation
    );
    assert_eq!(request.retrieval_mode(), SemanticSeedRetrievalMode::Hybrid);

    let empty_objective_error = SemanticSeedQueryRequest::new(
        " ",
        workspace_id("workspace--cti"),
        SemanticDomainProfile::CtiInvestigation,
        SemanticSeedRetrievalMode::Semantic,
        5,
        0.5,
    )
    .expect_err("whitespace objective should be rejected");
    assert!(matches!(
    empty_objective_error,
    GraphError::InvalidPropertyValue(message) if message.contains("objective must not be empty")
    ));

    let top_k_error = SemanticSeedQueryRequest::new(
        "objective",
        workspace_id("workspace--cti"),
        SemanticDomainProfile::CtiInvestigation,
        SemanticSeedRetrievalMode::Semantic,
        0,
        0.5,
    )
    .expect_err("top_k equal to zero should be rejected");
    assert!(matches!(
    top_k_error,
    GraphError::InvalidPropertyValue(message) if message.contains("top_k must be greater than zero")
    ));

    let threshold_error = SemanticSeedQueryRequest::new(
        "objective",
        workspace_id("workspace--cti"),
        SemanticDomainProfile::CtiInvestigation,
        SemanticSeedRetrievalMode::Semantic,
        5,
        1.1,
    )
    .expect_err("score threshold above one should be rejected");
    assert!(matches!(
    threshold_error,
    GraphError::InvalidPropertyValue(message) if message.contains("score threshold")
    ));
}

//
// Verify ranked seed output includes node IDs, score values, and explanation
// metadata while preserving deterministic rank order.
#[test]
fn semantic_seed_query_response_preserves_ranked_seed_candidates() {
    let request = SemanticSeedQueryRequest::new(
        "identify narratives tied to this actor",
        workspace_id("workspace--fimi"),
        SemanticDomainProfile::FimiInvestigation,
        SemanticSeedRetrievalMode::Hybrid,
        3,
        0.3,
    )
    .expect("request should be valid");

    let first = SemanticSeedCandidate::new(
        node_id("narrative--17"),
        0.94,
        SemanticSeedExplanationMetadata::new(
            "semantic similarity + graph keyword overlap",
            vec!["source://intel/report-12".to_owned()],
        ),
    )
    .expect("candidate should be valid");
    let second = SemanticSeedCandidate::new(
        node_id("campaign--4"),
        0.88,
        SemanticSeedExplanationMetadata::new(
            "hybrid rank with source trust weighting",
            vec!["source://intel/report-9".to_owned()],
        ),
    )
    .expect("candidate should be valid");

    let response =
        SemanticSeedQueryResponse::new(request.clone(), vec![first.clone(), second.clone()])
            .expect("response should be valid");

    assert_eq!(response.request(), &request);
    assert_eq!(response.seed_candidates(), &[first, second]);

    let out_of_range = SemanticSeedCandidate::new(
        node_id("narrative--19"),
        1.5,
        SemanticSeedExplanationMetadata::new("bad score", Vec::new()),
    )
    .expect_err("score must be clamped to [0,1]");
    assert!(matches!(
    out_of_range,
    GraphError::InvalidPropertyValue(message) if message.contains("candidate score")
    ));
}

//
// Verify the resolver contract can be implemented by deterministic callers and
// returns typed request and ranked response models through the public facade.
#[test]
fn semantic_seed_resolver_trait_supports_typed_request_and_response_models() {
    struct StubResolver;

    impl SemanticSeedResolver for StubResolver {
        fn resolve(
            &self,
            request: &SemanticSeedQueryRequest,
        ) -> Result<SemanticSeedQueryResponse, GraphError> {
            let candidate = SemanticSeedCandidate::new(
                node_id("indicator--seed"),
                0.9,
                SemanticSeedExplanationMetadata::new(
                    "deterministic stub match",
                    vec!["source://stub".to_owned()],
                ),
            )?;
            SemanticSeedQueryResponse::new(request.clone(), vec![candidate])
        }
    }

    let request = SemanticSeedQueryRequest::new(
        "find indicator related to actor",
        workspace_id("workspace--cti"),
        SemanticDomainProfile::CtiInvestigation,
        SemanticSeedRetrievalMode::Semantic,
        1,
        0.2,
    )
    .expect("request should be valid");

    let response = StubResolver
        .resolve(&request)
        .expect("stub resolver should return response");

    assert_eq!(response.request(), &request);
    assert_eq!(response.seed_candidates().len(), 1);
    assert_eq!(
        response.seed_candidates()[0].node_id().as_str(),
        "indicator--seed"
    );
}

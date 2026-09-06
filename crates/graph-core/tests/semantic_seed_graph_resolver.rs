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
    Confidence, Graph, GraphError, GraphSemanticSeedResolver, NodeId, NodeInput, PropertyValue,
    RecordStatus, RelationshipInput, SemanticDomainProfile, SemanticSeedQueryRequest,
    SemanticSeedResolutionErrorCode, SemanticSeedResolver, SemanticSeedRetrievalMode, WorkspaceId,
};

fn workspace_id() -> WorkspaceId {
    WorkspaceId::new("workspace--seed-resolver-tests").expect("workspace id should be valid")
}

fn request(
    objective: &str,
    mode: SemanticSeedRetrievalMode,
    top_k: usize,
    score_threshold: f64,
) -> SemanticSeedQueryRequest {
    SemanticSeedQueryRequest::new(
        objective,
        workspace_id(),
        SemanticDomainProfile::CtiInvestigation,
        mode,
        top_k,
        score_threshold,
    )
    .expect("test request should be valid")
}

fn named_node(graph: &mut Graph, label: &str, name: &str) -> NodeId {
    graph
        .create_node(
            NodeInput::new([label]).with_property("name", PropertyValue::String(name.to_owned())),
        )
        .expect("test node creation should succeed")
}

fn link(graph: &mut Graph, source: &NodeId, rel_type: &str, target: &NodeId) {
    let input = RelationshipInput::new(source.clone(), rel_type, target.clone())
        .expect("test relationship input should be valid");
    graph
        .create_relationship(input)
        .expect("test relationship creation should succeed");
}

#[test]
fn resolver_ranks_lexical_matches_above_nonmatches() {
    let mut graph = Graph::new();
    let matching = named_node(&mut graph, "Campaign", "acme phishing campaign");
    let _unrelated = named_node(&mut graph, "Identity", "logistics vendor directory");

    let resolver = GraphSemanticSeedResolver::new(&graph);
    let response = resolver
        .resolve(&request(
            "phishing campaign",
            SemanticSeedRetrievalMode::FullText,
            5,
            0.0,
        ))
        .expect("resolution should succeed");

    let candidates = response.seed_candidates();
    assert_eq!(candidates.len(), 1, "only the lexical match may seed");
    assert_eq!(candidates[0].node_id(), &matching);
    assert!(candidates[0].score() > 0.0);
}

#[test]
fn resolver_requires_at_least_one_matched_term() {
    let mut graph = Graph::new();
    let hub = named_node(&mut graph, "Infrastructure", "central command server");
    let spoke_one = named_node(&mut graph, "Indicator", "beacon endpoint alpha");
    let spoke_two = named_node(&mut graph, "Indicator", "beacon endpoint beta");
    link(&mut graph, &spoke_one, "CONNECTS_TO", &hub);
    link(&mut graph, &spoke_two, "CONNECTS_TO", &hub);

    let resolver = GraphSemanticSeedResolver::new(&graph);
    let error = resolver
        .resolve(&request(
            "ransomware negotiation",
            SemanticSeedRetrievalMode::Hybrid,
            5,
            0.0,
        ))
        .expect_err("centrality alone must never seed");

    match error {
        GraphError::SemanticSeedResolutionFailed(details) => {
            assert_eq!(details.code, SemanticSeedResolutionErrorCode::NoSeed);
        }
        other => panic!("expected semantic seed resolution failure, got {other:?}"),
    }
}

#[test]
fn resolver_hybrid_mode_boosts_higher_degree_nodes() {
    let mut graph = Graph::new();
    let connected = named_node(&mut graph, "Campaign", "winter phishing campaign");
    let isolated = named_node(&mut graph, "Campaign", "summer phishing campaign");
    let infra_one = named_node(&mut graph, "Infrastructure", "relay one");
    let infra_two = named_node(&mut graph, "Infrastructure", "relay two");
    link(&mut graph, &connected, "USES", &infra_one);
    link(&mut graph, &connected, "USES", &infra_two);

    let resolver = GraphSemanticSeedResolver::new(&graph);
    let response = resolver
        .resolve(&request(
            "phishing campaign",
            SemanticSeedRetrievalMode::Hybrid,
            5,
            0.0,
        ))
        .expect("resolution should succeed");

    let candidates = response.seed_candidates();
    assert_eq!(candidates.len(), 2);
    assert_eq!(
        candidates[0].node_id(),
        &connected,
        "the connected campaign must outrank the isolated one in hybrid mode"
    );
    assert_eq!(candidates[1].node_id(), &isolated);
    assert!(candidates[0].score() > candidates[1].score());
}

#[test]
fn resolver_full_text_mode_ignores_graph_signals() {
    let mut graph = Graph::new();
    let connected = named_node(&mut graph, "Campaign", "winter phishing campaign");
    let _isolated = named_node(&mut graph, "Campaign", "summer phishing campaign");
    let infra = named_node(&mut graph, "Infrastructure", "relay one");
    link(&mut graph, &connected, "USES", &infra);

    let resolver = GraphSemanticSeedResolver::new(&graph);
    let response = resolver
        .resolve(&request(
            "phishing campaign",
            SemanticSeedRetrievalMode::FullText,
            5,
            0.0,
        ))
        .expect("resolution should succeed");

    let candidates = response.seed_candidates();
    assert_eq!(candidates.len(), 2);
    assert!(
        (candidates[0].score() - candidates[1].score()).abs() < 1e-9,
        "full-text scores must not depend on degree"
    );
}

#[test]
fn resolver_legacy_scalar_does_not_contribute_to_hybrid_ranking() {
    let mut graph = Graph::new();
    let trusted = graph
        .create_node(
            NodeInput::new(["Campaign"])
                .with_property(
                    "name",
                    PropertyValue::String("spring phishing campaign".to_owned()),
                )
                .with_confidence(Confidence::new(0.95).expect("confidence should be valid")),
        )
        .expect("node creation should succeed");
    let untrusted = graph
        .create_node(
            NodeInput::new(["Campaign"])
                .with_property(
                    "name",
                    PropertyValue::String("autumn phishing campaign".to_owned()),
                )
                .with_confidence(Confidence::new(0.05).expect("confidence should be valid")),
        )
        .expect("node creation should succeed");

    let resolver = GraphSemanticSeedResolver::new(&graph);
    let response = resolver
        .resolve(&request(
            "phishing campaign",
            SemanticSeedRetrievalMode::Hybrid,
            5,
            0.0,
        ))
        .expect("resolution should succeed");

    let candidates = response.seed_candidates();
    assert_eq!(candidates.len(), 2);
    assert_eq!(candidates[0].node_id(), &trusted);
    assert_eq!(candidates[1].node_id(), &untrusted);
    assert_eq!(candidates[0].score(), candidates[1].score());
}

#[test]
fn resolver_excludes_rejected_and_deleted_records() {
    let mut graph = Graph::new();
    let _rejected = graph
        .create_node(
            NodeInput::new(["Campaign"])
                .with_property(
                    "name",
                    PropertyValue::String("rejected phishing campaign".to_owned()),
                )
                .with_status(RecordStatus::Rejected),
        )
        .expect("node creation should succeed");
    let kept = named_node(&mut graph, "Campaign", "confirmed phishing campaign");

    let resolver = GraphSemanticSeedResolver::new(&graph);
    let response = resolver
        .resolve(&request(
            "phishing campaign",
            SemanticSeedRetrievalMode::Hybrid,
            5,
            0.0,
        ))
        .expect("resolution should succeed");

    let candidates = response.seed_candidates();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].node_id(), &kept);
}

#[test]
fn resolver_applies_top_k_with_deterministic_ordering() {
    let mut graph = Graph::new();
    let first = named_node(&mut graph, "Campaign", "phishing campaign alpha");
    let second = named_node(&mut graph, "Campaign", "phishing campaign beta");
    let _third = named_node(&mut graph, "Campaign", "phishing campaign gamma delta");

    let resolver = GraphSemanticSeedResolver::new(&graph);
    let response = resolver
        .resolve(&request(
            "phishing campaign",
            SemanticSeedRetrievalMode::FullText,
            2,
            0.0,
        ))
        .expect("resolution should succeed");

    let candidates = response.seed_candidates();
    assert_eq!(candidates.len(), 2, "top_k must cap the candidate list");
    // alpha and beta share the same lexical score and shorter documents than
    // gamma delta; equal scores must fall back to node id order.
    assert_eq!(candidates[0].node_id(), &first);
    assert_eq!(candidates[1].node_id(), &second);
}

#[test]
fn resolver_returns_no_seed_when_threshold_filters_everything() {
    let mut graph = Graph::new();
    let _node = named_node(&mut graph, "Campaign", "acme phishing campaign");

    let resolver = GraphSemanticSeedResolver::new(&graph);
    let error = resolver
        .resolve(&request(
            "phishing",
            SemanticSeedRetrievalMode::FullText,
            5,
            1.0,
        ))
        .expect_err("an unreachable threshold must produce NO_SEED");

    match error {
        GraphError::SemanticSeedResolutionFailed(details) => {
            assert_eq!(details.code, SemanticSeedResolutionErrorCode::NoSeed);
            assert_eq!(details.threshold, Some(1.0));
            assert!(!details.fix_hint.is_empty());
        }
        other => panic!("expected semantic seed resolution failure, got {other:?}"),
    }
}

#[test]
fn resolver_rejects_objective_without_informative_terms() {
    let mut graph = Graph::new();
    let _node = named_node(&mut graph, "Campaign", "acme phishing campaign");

    let resolver = GraphSemanticSeedResolver::new(&graph);
    let error = resolver
        .resolve(&request(
            "find all the",
            SemanticSeedRetrievalMode::Hybrid,
            5,
            0.0,
        ))
        .expect_err("stopword-only objectives must be rejected as overbroad");

    match error {
        GraphError::SemanticSeedResolutionFailed(details) => {
            assert_eq!(
                details.code,
                SemanticSeedResolutionErrorCode::OverbroadObjective
            );
        }
        other => panic!("expected semantic seed resolution failure, got {other:?}"),
    }
}

#[test]
fn resolver_reports_ambiguous_tie_across_top_k_cut() {
    let mut graph = Graph::new();
    let _one = named_node(&mut graph, "Campaign", "phishing campaign alpha");
    let _two = named_node(&mut graph, "Campaign", "phishing campaign beta");
    let _three = named_node(&mut graph, "Campaign", "phishing campaign gamma");

    let resolver = GraphSemanticSeedResolver::new(&graph);
    let error = resolver
        .resolve(&request(
            "phishing campaign",
            SemanticSeedRetrievalMode::FullText,
            2,
            0.0,
        ))
        .expect_err("an exact tie across the top_k cut must be ambiguous");

    match error {
        GraphError::SemanticSeedResolutionFailed(details) => {
            assert_eq!(details.code, SemanticSeedResolutionErrorCode::AmbiguousSeed);
            assert_eq!(details.candidate_count, Some(3));
        }
        other => panic!("expected semantic seed resolution failure, got {other:?}"),
    }
}

#[test]
fn resolver_explanations_carry_matched_terms_and_signals() {
    let mut graph = Graph::new();
    let seed = named_node(&mut graph, "Campaign", "acme phishing campaign");
    let infra = named_node(&mut graph, "Infrastructure", "relay one");
    link(&mut graph, &seed, "USES", &infra);

    let resolver = GraphSemanticSeedResolver::new(&graph);
    let response = resolver
        .resolve(&request(
            "phishing campaign",
            SemanticSeedRetrievalMode::Hybrid,
            5,
            0.0,
        ))
        .expect("resolution should succeed");

    let candidate = response
        .seed_candidates()
        .iter()
        .find(|candidate| candidate.node_id() == &seed)
        .expect("seed candidate should be present");

    let rationale = candidate.explanation().rationale();
    assert!(
        rationale.contains("phishing") && rationale.contains("campaign"),
        "rationale must list matched terms, got: {rationale}"
    );
    assert!(
        rationale.contains("lexical"),
        "rationale must expose the lexical contribution, got: {rationale}"
    );
    assert!(
        rationale.contains("degree"),
        "rationale must expose the degree contribution, got: {rationale}"
    );
}

#[test]
fn resolver_semantic_mode_falls_back_with_boundary_note() {
    let mut graph = Graph::new();
    let _node = named_node(&mut graph, "Campaign", "acme phishing campaign");

    let resolver = GraphSemanticSeedResolver::new(&graph);
    let response = resolver
        .resolve(&request(
            "phishing campaign",
            SemanticSeedRetrievalMode::Semantic,
            5,
            0.0,
        ))
        .expect("semantic mode should fall back instead of failing");

    let candidates = response.seed_candidates();
    assert_eq!(candidates.len(), 1);
    assert!(
        candidates[0]
            .explanation()
            .boundary_notes()
            .iter()
            .any(|note| note.contains("fallback")),
        "semantic mode must disclose the lexical fallback in boundary notes"
    );
}

#[test]
fn resolver_matches_labels_not_only_properties() {
    let mut graph = Graph::new();
    let by_label = named_node(&mut graph, "Infrastructure", "relay node oslo");
    let _other = named_node(&mut graph, "Identity", "shipping company");

    let resolver = GraphSemanticSeedResolver::new(&graph);
    let response = resolver
        .resolve(&request(
            "infrastructure",
            SemanticSeedRetrievalMode::FullText,
            5,
            0.0,
        ))
        .expect("resolution should succeed");

    let candidates = response.seed_candidates();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].node_id(), &by_label);
}

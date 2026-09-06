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
    AgentStance, Claim, ClaimLinkKind, EpistemicNodeKind, EpistemicPrimitive,
    EpistemicRelationKind, EvidenceRecord, Graph, HypothesisWorkspace, NodeId, NodeInput,
    RelationshipType, classify_epistemic_node, epistemic_nodes_of_kind,
};

fn rel_type(value: &str) -> RelationshipType {
    RelationshipType::new(value).expect("vocabulary relationship type should be valid")
}

fn create_node(graph: &mut Graph, labels: &[&str]) -> NodeId {
    graph
        .create_node(NodeInput::new(labels.iter().copied()))
        .expect("vocabulary node should be created")
}

//
// Verify that the epistemic node vocabulary is exactly the epic's kinds plus EntityMention
// with stable canonical labels, so classification never depends on free text.
//
// Given the exported node-kind set,
// when canonical labels round-trip through classification,
// then every kind should map to itself and unknown labels to none.
#[test]
fn node_vocabulary_is_complete_with_stable_labels() {
    assert_eq!(EpistemicNodeKind::ALL.len(), 12);
    for kind in [
        EpistemicNodeKind::EntityMention,
        EpistemicNodeKind::Entity,
        EpistemicNodeKind::Event,
        EpistemicNodeKind::Observation,
        EpistemicNodeKind::Claim,
        EpistemicNodeKind::Hypothesis,
        EpistemicNodeKind::Evidence,
        EpistemicNodeKind::Source,
        EpistemicNodeKind::Assessment,
        EpistemicNodeKind::Contradiction,
        EpistemicNodeKind::Inference,
        EpistemicNodeKind::Decision,
    ] {
        assert!(EpistemicNodeKind::ALL.contains(&kind));
        assert_eq!(
            EpistemicNodeKind::from_label(kind.canonical_label()),
            Some(kind)
        );
    }
    assert!(EpistemicNodeKind::from_label("Campaign").is_none());
}

//
// Verify that the epistemic relation vocabulary carries stable canonical
// relationship types that round-trip through classification.
//
// Given the exported relation-kind set,
// when canonical relationship types round-trip through classification,
// then every kind should map to itself and unknown types to none.
#[test]
fn relation_vocabulary_is_complete_with_stable_types() {
    // Eight Epic 0018 kinds, four evidence-link kinds, and mention containment.
    assert_eq!(EpistemicRelationKind::ALL.len(), 13);
    for kind in [
        EpistemicRelationKind::HasMention,
        EpistemicRelationKind::Reports,
        EpistemicRelationKind::Supports,
        EpistemicRelationKind::Refutes,
        EpistemicRelationKind::Contradicts,
        EpistemicRelationKind::Supersedes,
        EpistemicRelationKind::Assesses,
        EpistemicRelationKind::Infers,
        EpistemicRelationKind::Decides,
        EpistemicRelationKind::ContextFor,
        EpistemicRelationKind::Duplicates,
        EpistemicRelationKind::DerivedFrom,
        EpistemicRelationKind::DependsOn,
    ] {
        assert!(EpistemicRelationKind::ALL.contains(&kind));
        assert_eq!(
            EpistemicRelationKind::from_relationship_type(&kind.canonical_relationship_type()),
            Some(kind)
        );
    }
    assert!(EpistemicRelationKind::from_relationship_type(&rel_type("PROMOTES")).is_none());
}

//
// Verify that the relation vocabulary reuses the Epic 0005 claim-link
// semantics instead of duplicating them: every claim-link kind embeds into the
// vocabulary, and only the claim subset maps back.
//
// Given the four claim-link kinds,
// when they convert into relation kinds and back,
// then the round trip should be lossless, and the non-claim relation kinds
// should have no claim-link equivalent.
#[test]
fn relation_vocabulary_aligns_with_claim_links() {
    for link_kind in [
        ClaimLinkKind::Supports,
        ClaimLinkKind::Refutes,
        ClaimLinkKind::Contradicts,
        ClaimLinkKind::Supersedes,
    ] {
        let relation_kind = EpistemicRelationKind::from(link_kind);
        assert_eq!(relation_kind.claim_link_kind(), Some(link_kind));
    }

    for non_claim_kind in [
        EpistemicRelationKind::Reports,
        EpistemicRelationKind::Assesses,
        EpistemicRelationKind::Infers,
        EpistemicRelationKind::Decides,
    ] {
        assert!(non_claim_kind.claim_link_kind().is_none());
    }
}

//
// Verify node classification by canonical label with a documented precedence:
// the first matching kind in vocabulary order wins when a node carries several
// epistemic labels.
//
// Given nodes with epistemic, mixed, and non-epistemic labels,
// when each node is classified,
// then classification should follow the canonical labels and the vocabulary
// order, and non-epistemic nodes should classify as none.
#[test]
fn nodes_classify_by_canonical_label_with_stable_precedence() {
    let mut graph = Graph::new();
    let claim = create_node(&mut graph, &["Claim"]);
    let source = create_node(&mut graph, &["Source", "Organization"]);
    // Vocabulary order puts Claim before Source: the first kind wins.
    let mixed = create_node(&mut graph, &["Source", "Claim"]);
    let plain = create_node(&mut graph, &["Campaign"]);

    let classify = |node_id: &NodeId| {
        let node = graph
            .get_node(node_id)
            .expect("node lookup should succeed")
            .expect("node should exist");
        classify_epistemic_node(&node)
    };

    assert_eq!(classify(&claim), Some(EpistemicNodeKind::Claim));
    assert_eq!(classify(&source), Some(EpistemicNodeKind::Source));
    assert_eq!(classify(&mixed), Some(EpistemicNodeKind::Claim));
    assert_eq!(classify(&plain), None);
}

//
// Verify that each epistemic kind is independently queryable over the graph:
// per-kind queries return exactly the matching nodes in insertion order.
//
// Given a graph mixing observations, claims, sources, and plain nodes,
// when each kind is queried,
// then only that kind's nodes should return, in creation order, and kinds with
// no nodes should return empty.
#[test]
fn kinds_are_independently_queryable_over_the_graph() {
    let mut graph = Graph::new();
    let first_observation = create_node(&mut graph, &["Observation"]);
    let claim = create_node(&mut graph, &["Claim"]);
    let source = create_node(&mut graph, &["Source"]);
    let second_observation = create_node(&mut graph, &["Observation"]);
    create_node(&mut graph, &["Campaign"]);

    let observations = epistemic_nodes_of_kind(&graph, EpistemicNodeKind::Observation)
        .expect("observation query should succeed");
    assert_eq!(observations, vec![first_observation, second_observation]);

    let claims = epistemic_nodes_of_kind(&graph, EpistemicNodeKind::Claim)
        .expect("claim query should succeed");
    assert_eq!(claims, vec![claim]);

    let sources = epistemic_nodes_of_kind(&graph, EpistemicNodeKind::Source)
        .expect("source query should succeed");
    assert_eq!(sources, vec![source]);

    let decisions = epistemic_nodes_of_kind(&graph, EpistemicNodeKind::Decision)
        .expect("decision query should succeed");
    assert!(decisions.is_empty());
}

//
// Verify that the Epic 0005 primitives keep their contracts and gain explicit
// epistemic kinds through the primitive trait.
//
// Given the claim, evidence, stance, and hypothesis-workspace types,
// when their associated epistemic kinds are read,
// then each should map onto its vocabulary kind.
#[test]
fn existing_primitives_map_onto_the_vocabulary() {
    assert_eq!(Claim::KIND, EpistemicNodeKind::Claim);
    assert_eq!(EvidenceRecord::KIND, EpistemicNodeKind::Evidence);
    assert_eq!(AgentStance::KIND, EpistemicNodeKind::Assessment);
    assert_eq!(HypothesisWorkspace::KIND, EpistemicNodeKind::Hypothesis);
}

//
// Verify determinism: identical graphs yield identical per-kind query results,
// which the epic's reproducibility posture requires of every read surface.
//
// Given two graphs built by the same construction sequence,
// when the same kind is queried on both,
// then the results should be exactly equal.
#[test]
fn identical_graphs_yield_identical_kind_queries() {
    let build = || {
        let mut graph = Graph::new();
        create_node(&mut graph, &["Observation"]);
        create_node(&mut graph, &["Claim"]);
        create_node(&mut graph, &["Observation"]);
        graph
    };

    let first = build();
    let second = build();

    assert_eq!(
        epistemic_nodes_of_kind(&first, EpistemicNodeKind::Observation)
            .expect("first query should succeed"),
        epistemic_nodes_of_kind(&second, EpistemicNodeKind::Observation)
            .expect("second query should succeed")
    );
}

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
    Graph, NodeId, NodeInput, RelationshipId, RelationshipInput, RelationshipType,
    ValidationErrorSeverity, ValidationTarget, validate_graph_structure,
};

fn rel_type(value: &str) -> RelationshipType {
    RelationshipType::new(value).expect("structural relationship type should be valid")
}

fn create_node(graph: &mut Graph, labels: &[&str]) -> NodeId {
    graph
        .create_node(NodeInput::new(labels.iter().copied()))
        .expect("structural node should be created")
}

fn create_relationship(
    graph: &mut Graph,
    source: &NodeId,
    relationship_type: &str,
    target: &NodeId,
) -> RelationshipId {
    graph
        .create_relationship(
            RelationshipInput::new(source.clone(), relationship_type, target.clone())
                .expect("structural relationship input should be valid"),
        )
        .expect("structural relationship should be created")
}

//
// Verify that a structurally sound graph produces no findings: validators must
// never invent defects.
//
// Given a valid campaign graph with epistemic and plain relations,
// when structural validation runs,
// then no findings should be reported.
#[test]
fn clean_graphs_produce_no_findings() {
    let mut graph = Graph::new();
    let source = create_node(&mut graph, &["Source"]);
    let observation = create_node(&mut graph, &["Observation"]);
    let claim = create_node(&mut graph, &["Claim"]);
    create_relationship(&mut graph, &source, "REPORTS", &observation);
    create_relationship(&mut graph, &observation, "SUPPORTS", &claim);

    let findings = validate_graph_structure(&graph, &[rel_type("SUPERSEDES")])
        .expect("structural validation should run");

    assert!(findings.is_empty());
}

//
// Verify dangling-link detection: a current relationship whose endpoint was
// tombstoned is a structural defect reported as a typed validation record.
//
// Given a relationship whose target node is tombstoned afterwards,
// when structural validation runs,
// then one dangling-link finding should target that relationship at error
// severity.
#[test]
fn dangling_links_are_detected_after_endpoint_tombstone() {
    let mut graph = Graph::new();
    let campaign = create_node(&mut graph, &["Campaign"]);
    let narrative = create_node(&mut graph, &["Narrative"]);
    let promotes = create_relationship(&mut graph, &campaign, "PROMOTES", &narrative);
    graph
        .tombstone_node(&narrative)
        .expect("narrative should be tombstoned");

    let findings = validate_graph_structure(&graph, &[]).expect("structural validation should run");

    assert_eq!(findings.len(), 1);
    let finding = &findings[0];
    assert_eq!(finding.code(), "immune-structural--dangling-link");
    assert_eq!(finding.severity(), ValidationErrorSeverity::Error);
    assert_eq!(
        finding.target(),
        &ValidationTarget::relationship(promotes.as_str())
    );
    assert!(finding.message().contains(narrative.as_str()));
}

//
// Verify impossible-cycle detection: relation types declared acyclic must not
// close cycles, and undeclared types are never checked.
//
// Given two PART_OF relationships closing a cycle,
// when validation runs with and without declaring PART_OF acyclic,
// then the declared run should report one impossible-cycle finding and the
// undeclared run none.
#[test]
fn impossible_cycles_are_detected_for_declared_acyclic_types() {
    let mut graph = Graph::new();
    let parent = create_node(&mut graph, &["Campaign"]);
    let child = create_node(&mut graph, &["Narrative"]);
    create_relationship(&mut graph, &parent, "PART_OF", &child);
    create_relationship(&mut graph, &child, "PART_OF", &parent);

    let declared = validate_graph_structure(&graph, &[rel_type("PART_OF")])
        .expect("structural validation should run");
    assert_eq!(declared.len(), 1);
    assert_eq!(declared[0].code(), "immune-structural--impossible-cycle");
    assert_eq!(declared[0].severity(), ValidationErrorSeverity::Error);
    assert!(matches!(
        declared[0].target(),
        ValidationTarget::Relationship(_)
    ));

    let undeclared =
        validate_graph_structure(&graph, &[]).expect("structural validation should run");
    assert!(undeclared.is_empty());
}

//
// Verify schema-violation detection: epistemic relations must connect the
// endpoint kinds declared by the vocabulary, and valid epistemic edges pass.
//
// Given a REPORTS relation from a non-Source node,
// when structural validation runs,
// then one schema-violation finding should target that relation while the
// valid REPORTS relation stays clean.
#[test]
fn schema_violations_are_detected_on_epistemic_relations() {
    let mut graph = Graph::new();
    let source = create_node(&mut graph, &["Source"]);
    let post = create_node(&mut graph, &["Post"]);
    let valid_observation = create_node(&mut graph, &["Observation"]);
    let invalid_observation = create_node(&mut graph, &["Observation"]);
    create_relationship(&mut graph, &source, "REPORTS", &valid_observation);
    let invalid = create_relationship(&mut graph, &post, "REPORTS", &invalid_observation);

    let findings = validate_graph_structure(&graph, &[]).expect("structural validation should run");

    assert_eq!(findings.len(), 1);
    let finding = &findings[0];
    assert_eq!(finding.code(), "immune-structural--schema-violation");
    assert_eq!(
        finding.target(),
        &ValidationTarget::relationship(invalid.as_str())
    );
}

//
// Verify that validation is a pure read: the graph is unchanged by running the
// validators over a defective graph.
//
// Given a graph with a dangling link,
// when validation runs,
// then node and relationship listings should be identical before and after.
#[test]
fn validation_never_mutates_the_graph() {
    let mut graph = Graph::new();
    let campaign = create_node(&mut graph, &["Campaign"]);
    let narrative = create_node(&mut graph, &["Narrative"]);
    create_relationship(&mut graph, &campaign, "PROMOTES", &narrative);
    graph
        .tombstone_node(&narrative)
        .expect("narrative should be tombstoned");

    let nodes_before = graph.list_nodes().expect("nodes should list");
    let relationships_before = graph
        .list_relationships()
        .expect("relationships should list");

    validate_graph_structure(&graph, &[rel_type("PART_OF")])
        .expect("structural validation should run");

    assert_eq!(graph.list_nodes().expect("nodes should list"), nodes_before);
    assert_eq!(
        graph
            .list_relationships()
            .expect("relationships should list"),
        relationships_before
    );
}

//
// Verify deterministic reporting: findings follow the documented order —
// dangling links, impossible cycles, then schema violations — and identical
// graphs yield identical findings.
//
// Given a graph seeded with one defect of each class,
// when validation runs twice on identical builds,
// then both runs should agree and follow the documented order.
#[test]
fn findings_are_deterministically_ordered() {
    let build = || {
        let mut graph = Graph::new();
        let campaign = create_node(&mut graph, &["Campaign"]);
        let narrative = create_node(&mut graph, &["Narrative"]);
        create_relationship(&mut graph, &campaign, "PROMOTES", &narrative);
        graph
            .tombstone_node(&narrative)
            .expect("narrative should be tombstoned");

        let parent = create_node(&mut graph, &["Campaign"]);
        let child = create_node(&mut graph, &["Narrative"]);
        create_relationship(&mut graph, &parent, "PART_OF", &child);
        create_relationship(&mut graph, &child, "PART_OF", &parent);

        let post = create_node(&mut graph, &["Post"]);
        let observation = create_node(&mut graph, &["Observation"]);
        create_relationship(&mut graph, &post, "REPORTS", &observation);
        graph
    };

    let first = validate_graph_structure(&build(), &[rel_type("PART_OF")])
        .expect("first validation should run");
    let second = validate_graph_structure(&build(), &[rel_type("PART_OF")])
        .expect("second validation should run");

    assert_eq!(first, second);
    let codes: Vec<&str> = first.iter().map(|finding| finding.code()).collect();
    assert_eq!(
        codes,
        vec![
            "immune-structural--dangling-link",
            "immune-structural--impossible-cycle",
            "immune-structural--schema-violation",
        ]
    );
}

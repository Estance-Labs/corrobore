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
    Confidence, Graph, GraphError, NodeId, NodeInput, PropertyValue, RecordStatus, RelationshipId,
    RelationshipInput, RelationshipType,
};

fn node(label: &str, name: &str) -> NodeInput {
    NodeInput::new([label])
        .with_property("name", PropertyValue::String(name.to_owned()))
        .with_status(RecordStatus::Candidate)
}

//
// Verify that relationship type validation rejects meaningless relationship
// labels before they can reach graph storage. A blank relationship type cannot
// describe an edge between two intelligence objects.
//
// Given a whitespace-only relationship type,
// when `RelationshipType::new` is called,
// then construction should fail with `GraphError::InvalidRelationshipType`.
#[test]
fn relationship_type_rejects_whitespace_only_values() {
    let error =
        RelationshipType::new(" ").expect_err("blank relationship types should be rejected");

    assert!(matches!(error, GraphError::InvalidRelationshipType(value) if value == " "));
}

//
// Verify the full happy path for creating and reading a relationship between two
// existing nodes. This locks the acceptance criteria before the storage
// implementation is written.
//
// Given two existing nodes and a relationship input with type, property, status,
// and confidence,
// when `Graph::create_relationship` is called and the relationship is retrieved,
// then the relationship should be current version `1` and preserve the input
// payload.
#[test]
fn create_relationship_between_existing_nodes_stores_current_version_one_payload() {
    let mut graph = Graph::new();
    let source = graph
        .create_node(node("ThreatActor", "APT28"))
        .expect("source node creation should succeed");
    let target = graph
        .create_node(node("Malware", "Sofacy"))
        .expect("target node creation should succeed");
    let confidence = Confidence::new(0.82).expect("bounded confidence should be accepted");
    let input = RelationshipInput::new(source.clone(), "uses", target.clone())
        .expect("valid relationship input should be accepted")
        .with_property(
            "source_report",
            PropertyValue::String("report--1".to_owned()),
        )
        .with_status(RecordStatus::NeedsReview)
        .with_confidence(confidence);

    let relationship_id = graph
        .create_relationship(input)
        .expect("relationship creation between existing nodes should succeed");
    let relationship = graph
        .get_relationship(&relationship_id)
        .expect("relationship lookup should not fail")
        .expect("created relationship should exist");

    assert_eq!(relationship.id(), &relationship_id);
    assert_eq!(relationship.source(), &source);
    assert_eq!(relationship.target(), &target);
    assert_eq!(relationship.rel_type().as_str(), "uses");
    assert_eq!(relationship.version(), 1);
    assert!(relationship.is_current());
    assert!(relationship.previous_version_id().is_none());
    assert_eq!(relationship.status(), RecordStatus::NeedsReview);
    assert_eq!(relationship.confidence(), Some(confidence));
    assert_eq!(
        relationship.property("source_report"),
        Some(&PropertyValue::String("report--1".to_owned()))
    );
}

//
// Verify that relationship creation cannot create an orphan edge when the source
// node is missing. Relationships must always point from an existing source node
// to an existing target node.
//
// Given an existing target node and a missing source node ID,
// when `Graph::create_relationship` is called,
// then creation should fail with `GraphError::SourceNodeNotFound` for the source
// ID.
#[test]
fn create_relationship_rejects_missing_source_node() {
    let mut graph = Graph::new();
    let missing_source =
        NodeId::new("node--missing-source").expect("valid node ID should be accepted");
    let target = graph
        .create_node(node("Malware", "Sofacy"))
        .expect("target node creation should succeed");
    let input = RelationshipInput::new(missing_source.clone(), "uses", target)
        .expect("valid relationship input should be accepted");

    let error = graph
        .create_relationship(input)
        .expect_err("missing source nodes should be rejected");

    assert!(matches!(error, GraphError::SourceNodeNotFound(id) if id == missing_source));
}

//
// Verify that relationship creation cannot create an orphan edge when the target
// node is missing. The graph must reject dangling target references instead of
// storing an incomplete edge.
//
// Given an existing source node and a missing target node ID,
// when `Graph::create_relationship` is called,
// then creation should fail with `GraphError::TargetNodeNotFound` for the target
// ID.
#[test]
fn create_relationship_rejects_missing_target_node() {
    let mut graph = Graph::new();
    let source = graph
        .create_node(node("ThreatActor", "APT28"))
        .expect("source node creation should succeed");
    let missing_target =
        NodeId::new("node--missing-target").expect("valid node ID should be accepted");
    let input = RelationshipInput::new(source, "uses", missing_target.clone())
        .expect("valid relationship input should be accepted");

    let error = graph
        .create_relationship(input)
        .expect_err("missing target nodes should be rejected");

    assert!(matches!(error, GraphError::TargetNodeNotFound(id) if id == missing_target));
}

//
// Verify that missing relationship reads use absence semantics, just like missing
// node reads. A caller should be able to check for a relationship without turning
// absence into an operational error.
//
// Given an empty graph and a valid missing relationship ID,
// when `Graph::get_relationship` is called,
// then it should return `Ok(None)`.
#[test]
fn get_relationship_returns_none_for_missing_relationships() {
    let graph = Graph::new();
    let missing = RelationshipId::new("relationship--missing")
        .expect("valid relationship ID should be accepted");

    let result = graph
        .get_relationship(&missing)
        .expect("missing relationship lookup should not fail");

    assert!(result.is_none());
}

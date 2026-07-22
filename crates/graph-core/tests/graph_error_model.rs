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
    Confidence, Graph, GraphError, NodeId, NodeInput, NodePatch, RecordStatus, RelationshipId,
    RelationshipInput, RelationshipPatch,
};

fn node_id(value: &str) -> NodeId {
    NodeId::new(value).expect("test node ID should be valid")
}

fn relationship_id(value: &str) -> RelationshipId {
    RelationshipId::new(value).expect("test relationship ID should be valid")
}

fn create_node(graph: &mut Graph, label: &str) -> NodeId {
    graph
        .create_node(NodeInput::new([label]))
        .expect("test node should be created")
}

fn relationship_input(source: NodeId, rel_type: &str, target: NodeId) -> RelationshipInput {
    RelationshipInput::new(source, rel_type, target)
        .expect("test relationship input should be valid")
}

//
// Verify that update operations on missing nodes expose the typed not-found
// branch required by the graph error model instead of returning a string-only
// failure or creating an implicit node.
//
// Given an empty graph and a valid missing node ID,
// when `Graph::update_node` is called,
// then it should fail with `GraphError::NodeNotFound` carrying that ID.
#[test]
fn update_unknown_node_returns_node_not_found() {
    let mut graph = Graph::new();
    let missing = node_id("node--missing");

    let error = graph
        .update_node(&missing, NodePatch::default())
        .expect_err("updating a missing node should fail");

    assert!(matches!(error, GraphError::NodeNotFound(id) if id == missing));
}

//
// Verify that logical deletion cannot create orphan node history. Tombstoning a
// missing node should use the same explicit typed not-found branch as updates.
//
// Given an empty graph and a valid missing node ID,
// when `Graph::tombstone_node` is called,
// then it should fail with `GraphError::NodeNotFound` carrying that ID.
#[test]
fn tombstone_unknown_node_returns_node_not_found() {
    let mut graph = Graph::new();
    let missing = node_id("node--missing");

    let error = graph
        .tombstone_node(&missing)
        .expect_err("tombstoning a missing node should fail");

    assert!(matches!(error, GraphError::NodeNotFound(id) if id == missing));
}

//
// Verify that relationship creation validates the source endpoint against graph
// state and reports the dedicated source-node branch.
//
// Given an existing target node and a missing source node,
// when `Graph::create_relationship` is called,
// then it should fail with `GraphError::SourceNodeNotFound` for the source ID.
#[test]
fn create_relationship_returns_source_node_not_found_for_missing_source() {
    let mut graph = Graph::new();
    let missing_source = node_id("node--missing-source");
    let target = create_node(&mut graph, "Indicator");
    let input = relationship_input(missing_source.clone(), "indicates", target);

    let error = graph
        .create_relationship(input)
        .expect_err("missing source node should be rejected");

    assert!(matches!(error, GraphError::SourceNodeNotFound(id) if id == missing_source));
}

//
// Verify that relationship creation validates the target endpoint independently
// from the source endpoint and reports the dedicated target-node branch.
//
// Given an existing source node and a missing target node,
// when `Graph::create_relationship` is called,
// then it should fail with `GraphError::TargetNodeNotFound` for the target ID.
#[test]
fn create_relationship_returns_target_node_not_found_for_missing_target() {
    let mut graph = Graph::new();
    let source = create_node(&mut graph, "Indicator");
    let missing_target = node_id("node--missing-target");
    let input = relationship_input(source, "indicates", missing_target.clone());

    let error = graph
        .create_relationship(input)
        .expect_err("missing target node should be rejected");

    assert!(matches!(error, GraphError::TargetNodeNotFound(id) if id == missing_target));
}

//
// Verify that relationship updates on unknown stable relationship IDs expose the
// typed relationship not-found branch.
//
// Given an empty graph and a valid missing relationship ID,
// when `Graph::update_relationship` is called,
// then it should fail with `GraphError::RelationshipNotFound` carrying that ID.
#[test]
fn update_unknown_relationship_returns_relationship_not_found() {
    let mut graph = Graph::new();
    let missing = relationship_id("relationship--missing");

    let error = graph
        .update_relationship(&missing, RelationshipPatch::default())
        .expect_err("updating a missing relationship should fail");

    assert!(matches!(error, GraphError::RelationshipNotFound(id) if id == missing));
}

//
// Verify that relationship tombstones cannot create orphan relationship history.
// Missing relationship tombstones should fail with the typed relationship
// not-found branch.
//
// Given an empty graph and a valid missing relationship ID,
// when `Graph::tombstone_relationship` is called,
// then it should fail with `GraphError::RelationshipNotFound` carrying that ID.
#[test]
fn tombstone_unknown_relationship_returns_relationship_not_found() {
    let mut graph = Graph::new();
    let missing = relationship_id("relationship--missing");

    let error = graph
        .tombstone_relationship(&missing)
        .expect_err("tombstoning a missing relationship should fail");

    assert!(matches!(error, GraphError::RelationshipNotFound(id) if id == missing));
}

//
// Verify that node-label validation is surfaced through the graph write path as
// the dedicated invalid-label branch.
//
// Given a node input with a whitespace-only label,
// when `Graph::create_node` is called,
// then it should fail with `GraphError::InvalidLabel` carrying that label.
#[test]
fn create_node_returns_invalid_label_for_whitespace_label() {
    let mut graph = Graph::new();

    let error = graph
        .create_node(NodeInput::new([" "]))
        .expect_err("whitespace labels should be rejected");

    assert!(matches!(error, GraphError::InvalidLabel(label) if label == " "));
}

//
// Verify that relationship-type validation remains typed before graph storage is
// attempted.
//
// Given a relationship input with a whitespace-only type,
// when `RelationshipInput::new` is called,
// then it should fail with `GraphError::InvalidRelationshipType`.
#[test]
fn relationship_input_returns_invalid_relationship_type_for_whitespace_type() {
    let source = node_id("node--source");
    let target = node_id("node--target");

    let error = RelationshipInput::new(source, "\t\n", target)
        .expect_err("whitespace relationship types should be rejected");

    assert!(matches!(error, GraphError::InvalidRelationshipType(value) if value == "\t\n"));
}

//
// Verify that invalid confidence values are rejected by the typed confidence
// primitive before they can be attached to graph records.
//
// Given a confidence value outside the accepted range,
// when `Confidence::new` is called,
// then it should fail with `GraphError::InvalidConfidence`.
#[test]
fn confidence_returns_invalid_confidence_for_out_of_range_value() {
    let error = Confidence::new(1.5).expect_err("confidence above one should be rejected");

    assert!(matches!(error, GraphError::InvalidConfidence(value) if value == 1.5));
}

//
// Verify that node lifecycle operations reject writes once the current node state
// is tombstoned.
//
// Given a node that has already been tombstoned,
// when `Graph::update_node` is called,
// then it should fail with `GraphError::RecordAlreadyTombstoned`.
#[test]
fn update_tombstoned_node_returns_record_already_tombstoned() {
    let mut graph = Graph::new();
    let node_id = create_node(&mut graph, "ThreatActor");
    graph
        .tombstone_node(&node_id)
        .expect("initial tombstone should succeed");

    let error = graph
        .update_node(
            &node_id,
            NodePatch::default().set_status(RecordStatus::NeedsReview),
        )
        .expect_err("updating a tombstoned node should fail");

    assert!(matches!(error, GraphError::RecordAlreadyTombstoned(id) if id == node_id.as_str()));
}

//
// Verify that relationship lifecycle operations reject writes once the current
// relationship state is tombstoned.
//
// Given a relationship that has already been tombstoned,
// when `Graph::update_relationship` is called,
// then it should fail with `GraphError::RecordAlreadyTombstoned`.
#[test]
fn update_tombstoned_relationship_returns_record_already_tombstoned() {
    let mut graph = Graph::new();
    let source = create_node(&mut graph, "Indicator");
    let target = create_node(&mut graph, "ThreatActor");
    let relationship_id = graph
        .create_relationship(relationship_input(source, "indicates", target))
        .expect("test relationship should be created");
    graph
        .tombstone_relationship(&relationship_id)
        .expect("initial relationship tombstone should succeed");

    let error = graph
        .update_relationship(
            &relationship_id,
            RelationshipPatch::default().set_status(RecordStatus::NeedsReview),
        )
        .expect_err("updating a tombstoned relationship should fail");

    assert!(
        matches!(error, GraphError::RecordAlreadyTombstoned(id) if id == relationship_id.as_str())
    );
}

//
// Verify that broken version-transition failures remain a typed branch even
// though public graph APIs should not normally allow callers to create this
// state directly.
//
// Given an explicit invalid-version-state error,
// when callers match on `GraphError`,
// then the branch should be matchable without inspecting formatted messages.
#[test]
fn invalid_version_state_is_typed_and_matchable() {
    let error = GraphError::InvalidVersionState("missing current pointer".to_owned());

    assert!(matches!(
    error,
    GraphError::InvalidVersionState(message) if message == "missing current pointer"
    ));
}

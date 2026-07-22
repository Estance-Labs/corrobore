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
    Graph, NodeId, NodeInput, PropertyValue, RecordStatus, RelationshipId, RelationshipInput,
};

fn threat_actor(name: &str) -> NodeInput {
    NodeInput::new(["ThreatActor"])
        .with_property("name", PropertyValue::String(name.to_owned()))
        .with_status(RecordStatus::Candidate)
}

fn observable(value: &str) -> NodeInput {
    NodeInput::new(["Observable"])
        .with_property("value", PropertyValue::String(value.to_owned()))
        .with_status(RecordStatus::Candidate)
}

fn create_node(graph: &mut Graph, input: NodeInput) -> NodeId {
    graph
        .create_node(input)
        .expect("test fixture node creation should succeed")
}

fn create_relationship(
    graph: &mut Graph,
    source: &NodeId,
    rel_type: &str,
    target: &NodeId,
) -> RelationshipId {
    let input = RelationshipInput::new(source.clone(), rel_type, target.clone())
        .expect("test fixture relationship input should be valid");

    graph
        .create_relationship(input)
        .expect("test fixture relationship creation should succeed")
}

//
// Verify that outgoing adjacency returns relationships whose source is the
// requested node without requiring callers to scan every relationship.
//
// Given a graph with two relationships leaving the source node and one unrelated
// relationship entering the same node,
// when `Graph::outgoing` is called for the source node,
// then only the two relationships whose source is that node should be returned
// in deterministic creation order.
#[test]
fn outgoing_lookup_returns_relationships_from_source_node() {
    let mut graph = Graph::new();
    let source = create_node(&mut graph, threat_actor("APT28"));
    let first_target = create_node(&mut graph, observable("malware.exe"));
    let second_target = create_node(&mut graph, observable("198.51.100.10"));
    let unrelated_source = create_node(&mut graph, threat_actor("APT29"));

    let first_outgoing = create_relationship(&mut graph, &source, "USES", &first_target);
    let second_outgoing = create_relationship(&mut graph, &source, "TARGETS", &second_target);
    let _unrelated_incoming =
        create_relationship(&mut graph, &unrelated_source, "OBSERVED_WITH", &source);

    let outgoing = graph
        .outgoing(&source)
        .expect("outgoing adjacency lookup should not fail");
    let outgoing_ids: Vec<_> = outgoing
        .iter()
        .map(|relationship| relationship.id().clone())
        .collect();

    assert_eq!(outgoing_ids, vec![first_outgoing, second_outgoing]);
    assert!(
        outgoing
            .iter()
            .all(|relationship| relationship.source() == &source)
    );
}

//
// Verify that incoming adjacency returns relationships whose target is the
// requested node without exposing internal adjacency storage.
//
// Given a graph with two relationships entering the target node and one unrelated
// relationship leaving the same node,
// when `Graph::incoming` is called for the target node,
// then only the two relationships whose target is that node should be returned
// in deterministic creation order.
#[test]
fn incoming_lookup_returns_relationships_to_target_node() {
    let mut graph = Graph::new();
    let target = create_node(&mut graph, observable("malware.exe"));
    let first_source = create_node(&mut graph, threat_actor("APT28"));
    let second_source = create_node(&mut graph, threat_actor("APT29"));
    let unrelated_target = create_node(&mut graph, observable("203.0.113.77"));

    let first_incoming = create_relationship(&mut graph, &first_source, "USES", &target);
    let second_incoming = create_relationship(&mut graph, &second_source, "TARGETS", &target);
    let _unrelated_outgoing =
        create_relationship(&mut graph, &target, "RESOLVES_TO", &unrelated_target);

    let incoming = graph
        .incoming(&target)
        .expect("incoming adjacency lookup should not fail");
    let incoming_ids: Vec<_> = incoming
        .iter()
        .map(|relationship| relationship.id().clone())
        .collect();

    assert_eq!(incoming_ids, vec![first_incoming, second_incoming]);
    assert!(
        incoming
            .iter()
            .all(|relationship| relationship.target() == &target)
    );
}

//
// Verify that pairwise adjacency returns relationships from a specific source to
// a specific target without mixing reverse or unrelated edges.
//
// Given a graph with a relationship from source to target, a reverse relationship,
// and a relationship from the source to another target,
// when `Graph::relationships_between` is called with the source and target,
// then only the relationship in that exact direction should be returned.
#[test]
fn relationships_between_returns_relationships_for_exact_direction() {
    let mut graph = Graph::new();
    let source = create_node(&mut graph, threat_actor("APT28"));
    let target = create_node(&mut graph, observable("malware.exe"));
    let other_target = create_node(&mut graph, observable("198.51.100.10"));

    let expected = create_relationship(&mut graph, &source, "USES", &target);
    let _reverse = create_relationship(&mut graph, &target, "OBSERVED_WITH", &source);
    let _other_target = create_relationship(&mut graph, &source, "TARGETS", &other_target);

    let between = graph
        .relationships_between(&source, &target)
        .expect("between-node adjacency lookup should not fail");
    let between_ids: Vec<_> = between
        .iter()
        .map(|relationship| relationship.id().clone())
        .collect();

    assert_eq!(between_ids, vec![expected]);
    assert!(
        between.iter().all(
            |relationship| relationship.source() == &source && relationship.target() == &target
        )
    );
}

//
// Verify that adjacency preserves multiple relationships between the same pair of
// nodes instead of collapsing them into a single edge.
//
// Given a graph with two different relationships from the same source node to the
// same target node,
// when `Graph::relationships_between` is called for that pair,
// then both relationship IDs should be returned in deterministic creation order.
#[test]
fn relationships_between_preserves_multiple_relationships_between_same_nodes() {
    let mut graph = Graph::new();
    let source = create_node(&mut graph, threat_actor("APT28"));
    let target = create_node(&mut graph, observable("malware.exe"));

    let first = create_relationship(&mut graph, &source, "USES", &target);
    let second = create_relationship(&mut graph, &source, "ATTRIBUTED_TO", &target);

    let between = graph
        .relationships_between(&source, &target)
        .expect("between-node adjacency lookup should not fail");
    let between_ids: Vec<_> = between
        .iter()
        .map(|relationship| relationship.id().clone())
        .collect();

    assert_eq!(between_ids, vec![first, second]);
}

//
// Verify that default adjacency reads hide tombstoned relationships while keeping
// surviving relationships visible.
//
// Given a graph with two outgoing relationships from the same source and one of
// those relationships has been tombstoned,
// when `Graph::outgoing` is called for the source node,
// then only the non-tombstoned relationship should be returned by default.
#[test]
fn outgoing_lookup_hides_tombstoned_relationships_by_default() {
    let mut graph = Graph::new();
    let source = create_node(&mut graph, threat_actor("APT28"));
    let first_target = create_node(&mut graph, observable("malware.exe"));
    let second_target = create_node(&mut graph, observable("198.51.100.10"));

    let tombstoned = create_relationship(&mut graph, &source, "USES", &first_target);
    let active = create_relationship(&mut graph, &source, "TARGETS", &second_target);

    graph
        .tombstone_relationship(&tombstoned)
        .expect("relationship tombstone should succeed");

    let outgoing = graph
        .outgoing(&source)
        .expect("outgoing adjacency lookup should not fail");
    let outgoing_ids: Vec<_> = outgoing
        .iter()
        .map(|relationship| relationship.id().clone())
        .collect();

    assert_eq!(outgoing_ids, vec![active]);
    assert!(
        outgoing
            .iter()
            .all(|relationship| relationship.status() != RecordStatus::Tombstoned)
    );
}

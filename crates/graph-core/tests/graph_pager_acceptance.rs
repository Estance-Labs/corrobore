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
    AdjacencyDirection, Graph, GraphPager, GraphPagerError, GraphRecordRef, NodeId, NodeInput,
    PropertyValue, RelationshipId, RelationshipInput, StorageRef,
};

fn sample_graph() -> (Graph, NodeId, NodeId, RelationshipId) {
    let mut graph = Graph::new();

    let campaign_id = graph
        .create_node(NodeInput::new(["Campaign", "FIMI"]).with_property(
            "name",
            PropertyValue::String("Operation Overcast".to_owned()),
        ))
        .expect("campaign node should be created");
    let narrative_id = graph
        .create_node(NodeInput::new(["Narrative"]).with_property(
            "topic",
            PropertyValue::String("election integrity".to_owned()),
        ))
        .expect("narrative node should be created");
    let relationship_id = graph
        .create_relationship(
            RelationshipInput::new(campaign_id.clone(), "AMPLIFIES", narrative_id.clone())
                .expect("relationship input should be valid")
                .with_property("weight", PropertyValue::Integer(3)),
        )
        .expect("relationship should be created");

    (graph, campaign_id, narrative_id, relationship_id)
}

//
// Validate the acceptance path where the graph pager can load current node and
// relationship payloads from the public in-memory graph API.
//
// Given a graph containing two nodes and one relationship,
// when node and relationship payloads are loaded through `GraphPager`,
// then the returned payloads should preserve IDs, indexed properties, endpoint data, and storage refs.
#[test]
fn graph_pager_loads_node_and_relationship_payloads_from_graph() {
    let (graph, campaign_id, narrative_id, relationship_id) = sample_graph();

    let paged_node = graph
        .load_node_payload(&campaign_id)
        .expect("campaign node should be loaded through pager");
    let paged_relationship = graph
        .load_relationship_payload(&relationship_id)
        .expect("relationship should be loaded through pager");

    assert_eq!(paged_node.node.id(), &campaign_id);
    assert!(paged_node.node.has_label("Campaign"));
    assert!(matches!(
    paged_node.node.property("name"),
    Some(PropertyValue::String(value)) if value == "Operation Overcast"
    ));
    assert!(matches!(
    &paged_node.storage_ref,
    Some(StorageRef::External { uri }) if uri == &format!("memory://graph/nodes/{}", campaign_id.as_str())
    ));

    assert_eq!(paged_relationship.relationship.id(), &relationship_id);
    assert_eq!(paged_relationship.relationship.source(), &campaign_id);
    assert_eq!(paged_relationship.relationship.target(), &narrative_id);
    assert_eq!(
        paged_relationship.relationship.rel_type().as_str(),
        "AMPLIFIES"
    );
    assert!(matches!(
    paged_relationship.relationship.property("weight"),
    Some(PropertyValue::Integer(value)) if *value == 3
    ));
    assert!(matches!(
    &paged_relationship.storage_ref,
    Some(StorageRef::External { uri })
    if uri == &format!("memory://graph/relationships/{}", relationship_id.as_str())
    ));
}

//
// Validate that the pager exposes incoming and outgoing adjacency as lightweight
// frontier data rather than forcing callers to read graph internals.
//
// Given a graph containing a relationship from campaign to narrative,
// when outgoing adjacency is loaded from campaign and incoming adjacency is loaded from narrative,
// then each result should expose owner, direction, relationship ID, neighbor ID, and lazy-load refs.
#[test]
fn graph_pager_loads_incoming_and_outgoing_adjacency_frontiers() {
    let (graph, campaign_id, narrative_id, relationship_id) = sample_graph();

    let outgoing = graph
        .load_outgoing_adjacency(&campaign_id)
        .expect("outgoing adjacency should be loaded");
    let incoming = graph
        .load_incoming_adjacency(&narrative_id)
        .expect("incoming adjacency should be loaded");

    assert_eq!(outgoing.owner_node_id, campaign_id);
    assert_eq!(outgoing.direction, AdjacencyDirection::Outgoing);
    assert_eq!(outgoing.entries.len(), 1);
    assert_eq!(outgoing.entries[0].relationship_id, relationship_id);
    assert_eq!(outgoing.entries[0].neighbor_node_id, narrative_id);
    assert_eq!(
        outgoing.entries[0]
            .relationship_type
            .as_ref()
            .map(|rel_type| rel_type.as_str()),
        Some("AMPLIFIES")
    );
    assert!(matches!(
    &outgoing.storage_ref,
    Some(StorageRef::External { uri }) if uri.contains("memory://graph/adjacency/outgoing/")
    ));
    assert!(outgoing.entries[0].relationship_storage_ref.is_some());
    assert!(outgoing.entries[0].neighbor_storage_ref.is_some());

    assert_eq!(incoming.owner_node_id, narrative_id);
    assert_eq!(incoming.direction, AdjacencyDirection::Incoming);
    assert_eq!(incoming.entries.len(), 1);
    assert_eq!(incoming.entries[0].relationship_id, relationship_id);
    assert_eq!(incoming.entries[0].neighbor_node_id, campaign_id);
    assert!(matches!(
    &incoming.storage_ref,
    Some(StorageRef::External { uri }) if uri.contains("memory://graph/adjacency/incoming/")
    ));
}

//
// Validate that metadata can be loaded through the pager without requiring the
// caller to request full payload wrappers first.
//
// Given a graph containing a campaign node and an amplifies relationship,
// when metadata is requested for both records,
// then node metadata should expose labels/properties and relationship metadata should expose type/properties.
#[test]
fn graph_pager_loads_indexed_metadata_without_full_payload_request() {
    let (graph, campaign_id, _narrative_id, relationship_id) = sample_graph();

    let node_metadata = graph
        .load_indexed_metadata(&GraphRecordRef::Node(campaign_id.clone()))
        .expect("node metadata should be loaded");
    let relationship_metadata = graph
        .load_indexed_metadata(&GraphRecordRef::Relationship(relationship_id.clone()))
        .expect("relationship metadata should be loaded");

    assert_eq!(node_metadata.record_ref, GraphRecordRef::Node(campaign_id));
    assert_eq!(node_metadata.relationship_type, None);
    assert!(node_metadata.labels.contains(&"Campaign".to_owned()));
    assert!(matches!(
    node_metadata.indexed_properties.get("name"),
    Some(PropertyValue::String(value)) if value == "Operation Overcast"
    ));
    assert!(node_metadata.storage_ref.is_some());

    assert_eq!(
        relationship_metadata.record_ref,
        GraphRecordRef::Relationship(relationship_id)
    );
    assert_eq!(
        relationship_metadata
            .relationship_type
            .as_ref()
            .map(|rel_type| rel_type.as_str()),
        Some("AMPLIFIES")
    );
    assert!(matches!(
    relationship_metadata.indexed_properties.get("weight"),
    Some(PropertyValue::Integer(value)) if *value == 3
    ));
    assert!(relationship_metadata.storage_ref.is_some());
}

//
// Validate the acceptance error path for unavailable graph records. Missing
// payloads and missing adjacency owners should be typed pager errors.
//
// Given an empty graph and stable IDs that are not present,
// when payload, adjacency, and metadata calls are made through `GraphPager`,
// then the pager should return `UnavailableRecord` with the requested logical record ref.
#[test]
fn graph_pager_reports_unavailable_records_as_typed_errors() {
    let graph = Graph::new();
    let missing_node_id = NodeId::new("node--missing").expect("missing node ID should be valid");
    let missing_relationship_id = RelationshipId::new("relationship--missing")
        .expect("missing relationship ID should be valid");

    assert!(matches!(
    graph.load_node_payload(&missing_node_id),
    Err(GraphPagerError::UnavailableRecord { record_ref })
    if record_ref == GraphRecordRef::Node(missing_node_id.clone())
    ));
    assert!(matches!(
    graph.load_relationship_payload(&missing_relationship_id),
    Err(GraphPagerError::UnavailableRecord { record_ref })
    if record_ref == GraphRecordRef::Relationship(missing_relationship_id.clone())
    ));
    assert!(matches!(
    graph.load_outgoing_adjacency(&missing_node_id),
    Err(GraphPagerError::UnavailableRecord { record_ref })
    if record_ref == GraphRecordRef::Node(missing_node_id.clone())
    ));
    assert!(matches!(
    graph.load_indexed_metadata(&GraphRecordRef::Relationship(missing_relationship_id.clone())),
    Err(GraphPagerError::UnavailableRecord { record_ref })
    if record_ref == GraphRecordRef::Relationship(missing_relationship_id)
    ));
}

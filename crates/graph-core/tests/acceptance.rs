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
#![allow(clippy::unwrap_used)]
// acceptance contract.
//
// This suite is the source of truth for the public in-memory graph-core MVP contract.
// It intentionally exercises the `graph_core` public API instead of private storage internals,
// so future hot, warm, streaming, or paged backends can preserve the same observable behavior
// without inheriting a full-graph-only implementation constraint.

use graph_core::{
    Confidence, Graph, GraphError, NodeId, NodeInput, NodePatch, PropertyValue, RecordStatus,
    RelationshipId, RelationshipInput, RelationshipPatch, RelationshipType,
};

// Build a minimal ThreatActor fixture for public Graph acceptance scenarios.
// The fixture carries a stable label, a `name` property, and Candidate status.
// Validation errors are intentionally left to the acceptance tests, not hidden here.
fn threat_actor(name: &str) -> NodeInput {
    NodeInput::new(["ThreatActor"])
        .with_property("name", PropertyValue::String(name.to_owned()))
        .with_status(RecordStatus::Candidate)
}

// Build a minimal Malware fixture for relationship and adjacency scenarios.
// The fixture carries a stable label, a `name` property, and Candidate status.
// Validation errors are intentionally left to the acceptance tests, not hidden here.
fn malware(name: &str) -> NodeInput {
    NodeInput::new(["Malware"])
        .with_property("name", PropertyValue::String(name.to_owned()))
        .with_status(RecordStatus::Candidate)
}

// Prove that a node created through the public API can be retrieved by ID.
// Given: a fresh Graph and a valid ThreatActor node input.
// When: the node is created and looked up through `get_node`.
// Then: the returned node exposes its ID, initial version, current marker, label, properties, and status.
#[test]
fn create_and_retrieve_node_by_id() {
    let mut graph = Graph::new();

    let node_id = graph
        .create_node(threat_actor("APT28"))
        .expect("node creation should succeed");

    let node = graph
        .get_node(&node_id)
        .expect("node lookup should not fail")
        .expect("node should exist");

    assert_eq!(node.id(), &node_id);
    assert_eq!(node.version(), 1);
    assert!(node.is_current());
    assert!(node.has_label("ThreatActor"));
    assert_eq!(
        node.property("name"),
        Some(&PropertyValue::String("APT28".to_owned()))
    );
    assert_eq!(node.status(), RecordStatus::Candidate);
}

// Prove that missing node lookup is a successful empty read, not an error.
// Given: a fresh Graph and a syntactically valid but unknown NodeId.
// When: the ID is looked up through `get_node`.
// Then: the public API returns `Ok(None)`.
#[test]
fn get_missing_node_returns_none() {
    let graph = Graph::new();
    let missing = NodeId::new("node--missing").expect("valid node ID");

    let result = graph
        .get_node(&missing)
        .expect("missing lookup should not fail");

    assert!(result.is_none());
}

// Prove that list-node reads return only visible current node records.
// Given: two visible nodes and one tombstoned node in the same graph.
// When: `list_nodes` is called through the public Graph API.
// Then: only the non-tombstoned current nodes are returned, independent of storage ordering.
#[test]
fn list_nodes_returns_only_visible_current_nodes() {
    let mut graph = Graph::new();
    let actor_id = graph
        .create_node(threat_actor("APT28"))
        .expect("actor creation should succeed");
    let malware_id = graph
        .create_node(malware("X-Agent"))
        .expect("malware creation should succeed");
    let tombstoned_id = graph
        .create_node(threat_actor("APT29"))
        .expect("second actor creation should succeed");

    graph
        .tombstone_node(&tombstoned_id)
        .expect("node tombstone should succeed");

    let mut returned_ids = graph
        .list_nodes()
        .expect("list_nodes should succeed")
        .into_iter()
        .map(|node| node.id().clone())
        .collect::<Vec<_>>();
    returned_ids.sort_by(|left, right| left.as_str().cmp(right.as_str()));

    let mut expected_ids = vec![actor_id, malware_id];
    expected_ids.sort_by(|left, right| left.as_str().cmp(right.as_str()));

    assert_eq!(returned_ids, expected_ids);
}

// Prove that node updates create immutable, readable versions.
// Given: an existing node and its first version identifier.
// When: the node is updated through the public Graph API.
// Then: the current version is incremented and linked to the previous readable version.
#[test]
fn update_node_creates_new_version_and_keeps_previous_version_readable() {
    let mut graph = Graph::new();
    let node_id = graph
        .create_node(threat_actor("APT28"))
        .expect("node creation should succeed");

    let first_version_id = graph
        .get_node(&node_id)
        .expect("node lookup should not fail")
        .expect("node should exist")
        .version_id()
        .clone();

    graph
        .update_node(
            &node_id,
            NodePatch::default()
                .set_property("name", PropertyValue::String("Fancy Bear".to_owned()))
                .set_status(RecordStatus::NeedsReview),
        )
        .expect("node update should succeed");

    let current = graph
        .get_node(&node_id)
        .expect("node lookup should not fail")
        .expect("node should still exist");

    assert_eq!(current.version(), 2);
    assert!(current.is_current());
    assert_eq!(current.previous_version_id(), Some(&first_version_id));
    assert_eq!(
        current.property("name"),
        Some(&PropertyValue::String("Fancy Bear".to_owned()))
    );
    assert_eq!(current.status(), RecordStatus::NeedsReview);

    let first_version = graph
        .get_node_version(&node_id, &first_version_id)
        .expect("version lookup should not fail")
        .expect("first version should be readable");

    assert_eq!(first_version.version(), 1);
    assert!(!first_version.is_current());
    assert_eq!(
        first_version.property("name"),
        Some(&PropertyValue::String("APT28".to_owned()))
    );

    let versions = graph
        .list_node_versions(&node_id)
        .expect("version listing should succeed");

    assert_eq!(versions.len(), 2);
    assert_eq!(
        versions
            .iter()
            .filter(|version| version.is_current())
            .count(),
        1
    );
}

// Prove that missing node updates fail with a typed not-found error.
// Given: a fresh Graph and a syntactically valid but unknown NodeId.
// When: `update_node` is called for that missing ID.
// Then: the public API returns `GraphError::NodeNotFound` with the missing ID.
#[test]
fn update_missing_node_returns_node_not_found() {
    let mut graph = Graph::new();
    let missing = NodeId::new("node--missing-update").expect("valid node ID");

    let error = graph
        .update_node(
            &missing,
            NodePatch::default().set_status(RecordStatus::NeedsReview),
        )
        .expect_err("missing node update should fail");

    assert!(matches!(error, GraphError::NodeNotFound(id) if id == missing));
}

// Prove that a tombstoned node cannot be updated again through default lifecycle APIs.
// Given: an existing node that has already been tombstoned.
// When: `update_node` is called for that stable node ID.
// Then: the public API returns `GraphError::RecordAlreadyTombstoned`.
#[test]
fn update_tombstoned_node_returns_record_already_tombstoned() {
    let mut graph = Graph::new();
    let node_id = graph
        .create_node(threat_actor("APT28"))
        .expect("node creation should succeed");

    graph
        .tombstone_node(&node_id)
        .expect("node tombstone should succeed");

    let error = graph
        .update_node(
            &node_id,
            NodePatch::default().set_status(RecordStatus::NeedsReview),
        )
        .expect_err("tombstoned node update should fail");

    assert!(matches!(error, GraphError::RecordAlreadyTombstoned(id) if id == node_id.as_str()));
}

// Prove that node tombstoning is modeled as a new current version.
// Given: an existing node visible through default reads.
// When: the node is tombstoned through the public Graph API.
// Then: default reads hide it while version listing exposes the tombstone version.
#[test]
fn tombstone_node_creates_tombstone_version_and_hides_default_read() {
    let mut graph = Graph::new();
    let node_id = graph
        .create_node(threat_actor("APT28"))
        .expect("node creation should succeed");

    graph
        .tombstone_node(&node_id)
        .expect("node tombstone should succeed");

    let current_read = graph
        .get_node(&node_id)
        .expect("node lookup should not fail");

    assert!(
        current_read.is_none(),
        "default reads hide tombstoned records"
    );

    let versions = graph
        .list_node_versions(&node_id)
        .expect("version listing should succeed");

    assert_eq!(versions.len(), 2);

    let tombstone = versions
        .iter()
        .find(|version| version.is_current())
        .expect("one current tombstone version should exist");

    assert_eq!(tombstone.status(), RecordStatus::Tombstoned);
    assert_eq!(tombstone.version(), 2);
}

// Prove that missing node tombstones fail with a typed not-found error.
// Given: a fresh Graph and a syntactically valid but unknown NodeId.
// When: `tombstone_node` is called for that missing ID.
// Then: the public API returns `GraphError::NodeNotFound` with the missing ID.
#[test]
fn tombstone_missing_node_returns_node_not_found() {
    let mut graph = Graph::new();
    let missing = NodeId::new("node--missing-tombstone").expect("valid node ID");

    let error = graph
        .tombstone_node(&missing)
        .expect_err("missing node tombstone should fail");

    assert!(matches!(error, GraphError::NodeNotFound(id) if id == missing));
}

// Prove that tombstoning an already tombstoned node is rejected deterministically.
// Given: an existing node that has already been tombstoned once.
// When: `tombstone_node` is called again for the same stable node ID.
// Then: the public API returns `GraphError::RecordAlreadyTombstoned`.
#[test]
fn tombstone_already_tombstoned_node_returns_record_already_tombstoned() {
    let mut graph = Graph::new();
    let node_id = graph
        .create_node(threat_actor("APT28"))
        .expect("node creation should succeed");

    graph
        .tombstone_node(&node_id)
        .expect("first node tombstone should succeed");

    let error = graph
        .tombstone_node(&node_id)
        .expect_err("second node tombstone should fail");

    assert!(matches!(error, GraphError::RecordAlreadyTombstoned(id) if id == node_id.as_str()));
}

// Prove that relationships connect existing nodes and are visible through adjacency reads.
// Given: an existing source node, target node, relationship type, and confidence.
// When: a relationship is created and read through public relationship and adjacency methods.
// Then: the relationship is retrievable by ID and appears in outgoing, incoming, and pairwise adjacency.
#[test]
fn create_relationship_between_existing_nodes_and_read_adjacency() {
    let mut graph = Graph::new();
    let actor_id = graph
        .create_node(threat_actor("APT28"))
        .expect("actor creation should succeed");
    let malware_id = graph
        .create_node(malware("X-Agent"))
        .expect("malware creation should succeed");

    let rel_id = graph
        .create_relationship(
            RelationshipInput::new(actor_id.clone(), "USES", malware_id.clone())
                .expect("relationship input should be valid")
                .with_confidence(Confidence::new(0.82).expect("valid confidence")),
        )
        .expect("relationship creation should succeed");

    let relationship = graph
        .get_relationship(&rel_id)
        .expect("relationship lookup should not fail")
        .expect("relationship should exist");

    assert_eq!(relationship.id(), &rel_id);
    assert_eq!(relationship.source(), &actor_id);
    assert_eq!(relationship.target(), &malware_id);
    assert_eq!(
        relationship.rel_type(),
        &RelationshipType::new("USES").unwrap()
    );
    assert_eq!(relationship.version(), 1);
    assert!(relationship.is_current());
    assert_eq!(
        relationship.confidence(),
        Some(Confidence::new(0.82).unwrap())
    );

    let outgoing = graph
        .outgoing(&actor_id)
        .expect("outgoing adjacency should succeed");
    assert_eq!(outgoing.len(), 1);
    assert_eq!(outgoing[0].id(), &rel_id);

    let incoming = graph
        .incoming(&malware_id)
        .expect("incoming adjacency should succeed");
    assert_eq!(incoming.len(), 1);
    assert_eq!(incoming[0].id(), &rel_id);

    let between = graph
        .relationships_between(&actor_id, &malware_id)
        .expect("relationships_between should succeed");
    assert_eq!(between.len(), 1);
    assert_eq!(between[0].id(), &rel_id);
}

// Prove that missing relationship lookup is a successful empty read, not an error.
// Given: a fresh Graph and a syntactically valid but unknown RelationshipId.
// When: the ID is looked up through `get_relationship`.
// Then: the public API returns `Ok(None)`.
#[test]
fn get_missing_relationship_returns_none() {
    let graph = Graph::new();
    let missing = RelationshipId::new("relationship--missing").expect("valid relationship ID");

    let result = graph
        .get_relationship(&missing)
        .expect("missing relationship lookup should not fail");

    assert!(result.is_none());
}

// Prove that relationship creation rejects missing source nodes deterministically.
// Given: a missing source node ID and an existing target node.
// When: a relationship is created from the missing source to the existing target.
// Then: the public API returns `GraphError::SourceNodeNotFound` with the missing ID.
#[test]
fn reject_relationship_when_source_node_is_missing() {
    let mut graph = Graph::new();
    let missing_actor_id = NodeId::new("node--missing-actor").expect("valid node ID");
    let malware_id = graph
        .create_node(malware("X-Agent"))
        .expect("malware creation should succeed");

    let error = graph
        .create_relationship(
            RelationshipInput::new(missing_actor_id.clone(), "USES", malware_id)
                .expect("relationship input should be valid"),
        )
        .expect_err("missing source node should fail");

    assert!(matches!(error, GraphError::SourceNodeNotFound(id) if id == missing_actor_id));
}

// Prove that relationship creation rejects missing target nodes deterministically.
// Given: an existing source node and a missing target node ID.
// When: a relationship is created from the existing source to the missing target.
// Then: the public API returns `GraphError::TargetNodeNotFound` with the missing ID.
#[test]
fn reject_relationship_when_target_node_is_missing() {
    let mut graph = Graph::new();
    let actor_id = graph
        .create_node(threat_actor("APT28"))
        .expect("actor creation should succeed");
    let missing_malware_id = NodeId::new("node--missing-malware").expect("valid node ID");

    let error = graph
        .create_relationship(
            RelationshipInput::new(actor_id, "USES", missing_malware_id.clone())
                .expect("relationship input should be valid"),
        )
        .expect_err("missing target node should fail");

    assert!(matches!(error, GraphError::TargetNodeNotFound(id) if id == missing_malware_id));
}

// Prove that tombstoned source nodes cannot be used for new relationships.
// Given: a tombstoned source node and a visible target node.
// When: a relationship is created from the tombstoned source to the target.
// Then: the public API treats the source as not found for creation purposes.
#[test]
fn reject_relationship_when_source_node_is_tombstoned() {
    let mut graph = Graph::new();
    let actor_id = graph
        .create_node(threat_actor("APT28"))
        .expect("actor creation should succeed");
    let malware_id = graph
        .create_node(malware("X-Agent"))
        .expect("malware creation should succeed");

    graph
        .tombstone_node(&actor_id)
        .expect("source node tombstone should succeed");

    let error = graph
        .create_relationship(
            RelationshipInput::new(actor_id.clone(), "USES", malware_id)
                .expect("relationship input should be valid"),
        )
        .expect_err("relationship with tombstoned source should fail");

    assert!(matches!(error, GraphError::SourceNodeNotFound(id) if id == actor_id));
}

// Prove that tombstoned target nodes cannot be used for new relationships.
// Given: a visible source node and a tombstoned target node.
// When: a relationship is created from the source to the tombstoned target.
// Then: the public API treats the target as not found for creation purposes.
#[test]
fn reject_relationship_when_target_node_is_tombstoned() {
    let mut graph = Graph::new();
    let actor_id = graph
        .create_node(threat_actor("APT28"))
        .expect("actor creation should succeed");
    let malware_id = graph
        .create_node(malware("X-Agent"))
        .expect("malware creation should succeed");

    graph
        .tombstone_node(&malware_id)
        .expect("target node tombstone should succeed");

    let error = graph
        .create_relationship(
            RelationshipInput::new(actor_id, "USES", malware_id.clone())
                .expect("relationship input should be valid"),
        )
        .expect_err("relationship with tombstoned target should fail");

    assert!(matches!(error, GraphError::TargetNodeNotFound(id) if id == malware_id));
}

// Prove that relationship updates create immutable, readable versions.
// Given: an existing relationship and its first version identifier.
// When: the relationship status and confidence are updated through the public Graph API.
// Then: the current version is incremented and linked to the previous readable version.
#[test]
fn update_relationship_creates_new_version_and_keeps_previous_version_readable() {
    let mut graph = Graph::new();
    let actor_id = graph
        .create_node(threat_actor("APT28"))
        .expect("actor creation should succeed");
    let malware_id = graph
        .create_node(malware("X-Agent"))
        .expect("malware creation should succeed");

    let rel_id = graph
        .create_relationship(
            RelationshipInput::new(actor_id, "USES", malware_id)
                .expect("relationship input should be valid")
                .with_status(RecordStatus::Candidate),
        )
        .expect("relationship creation should succeed");

    let first_version_id = graph
        .get_relationship(&rel_id)
        .expect("relationship lookup should not fail")
        .expect("relationship should exist")
        .version_id()
        .clone();

    graph
        .update_relationship(
            &rel_id,
            RelationshipPatch::default()
                .set_status(RecordStatus::Validated)
                .set_confidence(Confidence::new(0.91).expect("valid confidence")),
        )
        .expect("relationship update should succeed");

    let current = graph
        .get_relationship(&rel_id)
        .expect("relationship lookup should not fail")
        .expect("relationship should exist");

    assert_eq!(current.version(), 2);
    assert!(current.is_current());
    assert_eq!(current.previous_version_id(), Some(&first_version_id));
    assert_eq!(current.status(), RecordStatus::Validated);
    assert_eq!(current.confidence(), Some(Confidence::new(0.91).unwrap()));

    let first_version = graph
        .get_relationship_version(&rel_id, &first_version_id)
        .expect("version lookup should not fail")
        .expect("first version should be readable");

    assert_eq!(first_version.version(), 1);
    assert!(!first_version.is_current());
    assert_eq!(first_version.status(), RecordStatus::Candidate);

    let versions = graph
        .list_relationship_versions(&rel_id)
        .expect("version listing should succeed");

    assert_eq!(versions.len(), 2);
    assert_eq!(
        versions
            .iter()
            .filter(|version| version.is_current())
            .count(),
        1
    );
}

// Prove that missing relationship updates fail with a typed not-found error.
// Given: a fresh Graph and a syntactically valid but unknown RelationshipId.
// When: `update_relationship` is called for that missing ID.
// Then: the public API returns `GraphError::RelationshipNotFound` with the missing ID.
#[test]
fn update_missing_relationship_returns_relationship_not_found() {
    let mut graph = Graph::new();
    let missing =
        RelationshipId::new("relationship--missing-update").expect("valid relationship ID");

    let error = graph
        .update_relationship(
            &missing,
            RelationshipPatch::default().set_status(RecordStatus::Validated),
        )
        .expect_err("missing relationship update should fail");

    assert!(matches!(error, GraphError::RelationshipNotFound(id) if id == missing));
}

// Prove that a tombstoned relationship cannot be updated again through default lifecycle APIs.
// Given: an existing relationship that has already been tombstoned.
// When: `update_relationship` is called for that stable relationship ID.
// Then: the public API returns `GraphError::RecordAlreadyTombstoned`.
#[test]
fn update_tombstoned_relationship_returns_record_already_tombstoned() {
    let mut graph = Graph::new();
    let actor_id = graph
        .create_node(threat_actor("APT28"))
        .expect("actor creation should succeed");
    let malware_id = graph
        .create_node(malware("X-Agent"))
        .expect("malware creation should succeed");
    let rel_id = graph
        .create_relationship(
            RelationshipInput::new(actor_id, "USES", malware_id)
                .expect("relationship input should be valid"),
        )
        .expect("relationship creation should succeed");

    graph
        .tombstone_relationship(&rel_id)
        .expect("relationship tombstone should succeed");

    let error = graph
        .update_relationship(
            &rel_id,
            RelationshipPatch::default().set_status(RecordStatus::Validated),
        )
        .expect_err("tombstoned relationship update should fail");

    assert!(matches!(error, GraphError::RecordAlreadyTombstoned(id) if id == rel_id.as_str()));
}

// Prove that relationship tombstoning hides default reads and adjacency results.
// Given: an existing relationship visible through relationship and adjacency reads.
// When: the relationship is tombstoned through the public Graph API.
// Then: default reads hide it while version listing exposes the tombstone version.
#[test]
fn tombstone_relationship_creates_tombstone_version_and_hides_from_default_adjacency() {
    let mut graph = Graph::new();
    let actor_id = graph
        .create_node(threat_actor("APT28"))
        .expect("actor creation should succeed");
    let malware_id = graph
        .create_node(malware("X-Agent"))
        .expect("malware creation should succeed");

    let rel_id = graph
        .create_relationship(
            RelationshipInput::new(actor_id.clone(), "USES", malware_id.clone())
                .expect("relationship input should be valid"),
        )
        .expect("relationship creation should succeed");

    graph
        .tombstone_relationship(&rel_id)
        .expect("relationship tombstone should succeed");

    let relationship_read = graph
        .get_relationship(&rel_id)
        .expect("relationship lookup should not fail");
    assert!(
        relationship_read.is_none(),
        "default reads hide tombstoned relationships"
    );

    let outgoing = graph
        .outgoing(&actor_id)
        .expect("outgoing adjacency should succeed");
    assert!(outgoing.is_empty());

    let incoming = graph
        .incoming(&malware_id)
        .expect("incoming adjacency should succeed");
    assert!(incoming.is_empty());

    let versions = graph
        .list_relationship_versions(&rel_id)
        .expect("version listing should succeed");
    assert_eq!(versions.len(), 2);

    let tombstone = versions
        .iter()
        .find(|version| version.is_current())
        .expect("one current tombstone version should exist");

    assert_eq!(tombstone.status(), RecordStatus::Tombstoned);
    assert_eq!(tombstone.version(), 2);
}

// Prove that missing relationship tombstones fail with a typed not-found error.
// Given: a fresh Graph and a syntactically valid but unknown RelationshipId.
// When: `tombstone_relationship` is called for that missing ID.
// Then: the public API returns `GraphError::RelationshipNotFound` with the missing ID.
#[test]
fn tombstone_missing_relationship_returns_relationship_not_found() {
    let mut graph = Graph::new();
    let missing =
        RelationshipId::new("relationship--missing-tombstone").expect("valid relationship ID");

    let error = graph
        .tombstone_relationship(&missing)
        .expect_err("missing relationship tombstone should fail");

    assert!(matches!(error, GraphError::RelationshipNotFound(id) if id == missing));
}

// Prove that tombstoning an already tombstoned relationship is rejected deterministically.
// Given: an existing relationship that has already been tombstoned once.
// When: `tombstone_relationship` is called again for the same stable relationship ID.
// Then: the public API returns `GraphError::RecordAlreadyTombstoned`.
#[test]
fn tombstone_already_tombstoned_relationship_returns_record_already_tombstoned() {
    let mut graph = Graph::new();
    let actor_id = graph
        .create_node(threat_actor("APT28"))
        .expect("actor creation should succeed");
    let malware_id = graph
        .create_node(malware("X-Agent"))
        .expect("malware creation should succeed");
    let rel_id = graph
        .create_relationship(
            RelationshipInput::new(actor_id, "USES", malware_id)
                .expect("relationship input should be valid"),
        )
        .expect("relationship creation should succeed");

    graph
        .tombstone_relationship(&rel_id)
        .expect("first relationship tombstone should succeed");

    let error = graph
        .tombstone_relationship(&rel_id)
        .expect_err("second relationship tombstone should fail");

    assert!(matches!(error, GraphError::RecordAlreadyTombstoned(id) if id == rel_id.as_str()));
}

// Prove that missing version listings are empty public reads.
// Given: a fresh Graph with syntactically valid but unknown stable record IDs.
// When: node and relationship version lists are requested for those IDs.
// Then: both public APIs return empty vectors instead of errors.
#[test]
fn missing_version_lists_return_empty_vectors() {
    let graph = Graph::new();
    let missing_node = NodeId::new("node--missing-versions").expect("valid node ID");
    let missing_relationship =
        RelationshipId::new("relationship--missing-versions").expect("valid relationship ID");

    assert!(
        graph
            .list_node_versions(&missing_node)
            .expect("missing node version listing should not fail")
            .is_empty()
    );
    assert!(
        graph
            .list_relationship_versions(&missing_relationship)
            .expect("missing relationship version listing should not fail")
            .is_empty()
    );
}

// Prove that MVP property values survive public node creation and retrieval.
// Given: a node input containing scalar values and list values supported by the MVP.
// When: the node is created and retrieved through the public Graph API.
// Then: the retrieved node exposes the expected property values without lossy conversion.
#[test]
fn property_values_support_mvp_scalar_and_list_types() {
    let mut graph = Graph::new();

    let node_id = graph
        .create_node(
            NodeInput::new(["Indicator"])
                .with_property("string", PropertyValue::String("value".to_owned()))
                .with_property("integer", PropertyValue::Integer(42))
                .with_property("float", PropertyValue::Float(42.5))
                .with_property("bool", PropertyValue::Bool(true))
                .with_property("null", PropertyValue::Null)
                .with_property(
                    "strings",
                    PropertyValue::StringList(vec!["a".to_owned(), "b".to_owned()]),
                )
                .with_property("integers", PropertyValue::IntegerList(vec![1, 2]))
                .with_property("floats", PropertyValue::FloatList(vec![1.0, 2.0]))
                .with_property("bools", PropertyValue::BoolList(vec![true, false])),
        )
        .expect("node creation should succeed");

    let node = graph
        .get_node(&node_id)
        .expect("node lookup should not fail")
        .expect("node should exist");

    assert_eq!(
        node.property("string"),
        Some(&PropertyValue::String("value".to_owned()))
    );
    assert_eq!(node.property("integer"), Some(&PropertyValue::Integer(42)));
    assert_eq!(node.property("float"), Some(&PropertyValue::Float(42.5)));
    assert_eq!(node.property("bool"), Some(&PropertyValue::Bool(true)));
    assert_eq!(node.property("null"), Some(&PropertyValue::Null));
    assert_eq!(
        node.property("strings"),
        Some(&PropertyValue::StringList(vec![
            "a".to_owned(),
            "b".to_owned()
        ]))
    );
    assert_eq!(
        node.property("integers"),
        Some(&PropertyValue::IntegerList(vec![1, 2]))
    );
    assert_eq!(
        node.property("floats"),
        Some(&PropertyValue::FloatList(vec![1.0, 2.0]))
    );
    assert_eq!(
        node.property("bools"),
        Some(&PropertyValue::BoolList(vec![true, false]))
    );
}

// Prove that confidence validation accepts only bounded, finite scores.
// Given: valid boundary values, a middle value, out-of-range values, and NaN.
// When: each value is passed through `Confidence::new`.
// Then: only values in the inclusive [0.0, 1.0] range are accepted.
#[test]
fn confidence_accepts_only_values_between_zero_and_one() {
    assert!(Confidence::new(0.0).is_ok());
    assert!(Confidence::new(0.5).is_ok());
    assert!(Confidence::new(1.0).is_ok());

    assert!(Confidence::new(-0.1).is_err());
    assert!(Confidence::new(1.1).is_err());
    assert!(Confidence::new(f64::NAN).is_err());
}

// Prove that labels and relationship types reject empty or whitespace-only values.
// Given: valid labels/types plus empty and whitespace-only labels/types.
// When: node input and relationship type validation are executed through public constructors.
// Then: valid values are accepted and meaningless values are rejected deterministically.
#[test]
fn labels_and_relationship_types_reject_empty_values() {
    assert!(NodeInput::new(["ThreatActor"]).validate().is_ok());
    assert!(
        NodeInput::new(std::iter::empty::<&str>())
            .validate()
            .is_err()
    );
    assert!(NodeInput::new([""]).validate().is_err());
    assert!(NodeInput::new([" \t\n"]).validate().is_err());

    assert!(RelationshipType::new("USES").is_ok());
    assert!(RelationshipType::new("").is_err());
    assert!(RelationshipType::new(" \t\n").is_err());
}

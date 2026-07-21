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
    Graph, GraphError, NodeId, NodeInput, PropertyValue, RecordStatus, RelationshipId,
    RelationshipInput, RelationshipPatch, RelationshipVersionId,
};

fn graph_with_relationship() -> (Graph, NodeId, NodeId, RelationshipId, RelationshipVersionId) {
    let mut graph = Graph::new();

    let source_id = graph
        .create_node(
            NodeInput::new(["ThreatActor"])
                .with_property("name", PropertyValue::String("APT28".to_owned())),
        )
        .expect("source node creation should succeed");
    let target_id = graph
        .create_node(
            NodeInput::new(["Malware"])
                .with_property("name", PropertyValue::String("X-Agent".to_owned())),
        )
        .expect("target node creation should succeed");

    let relationship_id = graph
        .create_relationship(
            RelationshipInput::new(source_id.clone(), "USES", target_id.clone())
                .expect("valid relationship input should be accepted")
                .with_property("source", PropertyValue::String("initial-report".to_owned()))
                .with_status(RecordStatus::Candidate),
        )
        .expect("relationship creation should succeed");

    let first_version_id = graph
        .get_relationship(&relationship_id)
        .expect("relationship lookup should not fail")
        .expect("created relationship should exist")
        .version_id()
        .clone();

    (
        graph,
        source_id,
        target_id,
        relationship_id,
        first_version_id,
    )
}

//
// Verify that updating a relationship appends a new current version while keeping
// the previous version readable. Historical relationship versions are the core
// guarantee of and must not be overwritten in place.
//
// Given an existing relationship and its first version ID,
// when `Graph::update_relationship` applies a property and status patch,
// then the current relationship should be version `2`, the first version should
// stay readable as non-current, and exactly one relationship version should be
// current.
#[test]
fn update_relationship_creates_new_current_version_and_preserves_previous_version() {
    let (mut graph, _source_id, _target_id, relationship_id, first_version_id) =
        graph_with_relationship();

    graph
        .update_relationship(
            &relationship_id,
            RelationshipPatch::default()
                .set_property(
                    "source",
                    PropertyValue::String("enriched-report".to_owned()),
                )
                .set_status(RecordStatus::Validated),
        )
        .expect("relationship update should succeed");

    let current = graph
        .get_relationship(&relationship_id)
        .expect("relationship lookup should not fail")
        .expect("updated relationship should exist");
    assert_eq!(current.version(), 2);
    assert!(current.is_current());
    assert_eq!(current.previous_version_id(), Some(&first_version_id));
    assert_eq!(
        current.property("source"),
        Some(&PropertyValue::String("enriched-report".to_owned()))
    );
    assert_eq!(current.status(), RecordStatus::Validated);

    let first_version = graph
        .get_relationship_version(&relationship_id, &first_version_id)
        .expect("relationship version lookup should not fail")
        .expect("first relationship version should remain readable");
    assert_eq!(first_version.version(), 1);
    assert!(!first_version.is_current());
    assert_eq!(
        first_version.property("source"),
        Some(&PropertyValue::String("initial-report".to_owned()))
    );

    let versions = graph
        .list_relationship_versions(&relationship_id)
        .expect("relationship version listing should not fail");
    assert_eq!(versions.len(), 2);
    assert_eq!(
        versions
            .iter()
            .filter(|version| version.is_current())
            .count(),
        1
    );
}

//
// Verify that historical relationship version lookup uses absence semantics for
// missing versions. A missing version ID should not be confused with an
// operational failure when the relationship itself exists.
//
// Given an existing relationship and a valid but unknown relationship version ID,
// when `Graph::get_relationship_version` is called,
// then it should return `Ok(None)`.
#[test]
fn get_relationship_version_returns_none_for_missing_versions() {
    let (graph, _source_id, _target_id, relationship_id, _first_version_id) =
        graph_with_relationship();
    let missing_version = RelationshipVersionId::new("relationship-version--missing")
        .expect("valid relationship version ID should be accepted");

    let result = graph
        .get_relationship_version(&relationship_id, &missing_version)
        .expect("missing relationship version lookup should not fail");

    assert!(result.is_none());
}

//
// Verify that relationship version listing returns the complete lifecycle history
// in deterministic version order. Analysts and acceptance tests need the full
// chain, including historical updates and tombstones.
//
// Given an existing relationship that is updated and then tombstoned,
// when `Graph::list_relationship_versions` is called,
// then it should return versions `1`, `2`, and `3` in order with exactly one
// current tombstone version.
#[test]
fn list_relationship_versions_returns_all_versions_in_version_order() {
    let (mut graph, _source_id, _target_id, relationship_id, _first_version_id) =
        graph_with_relationship();

    graph
        .update_relationship(
            &relationship_id,
            RelationshipPatch::default().set_status(RecordStatus::Validated),
        )
        .expect("relationship update should succeed");
    graph
        .tombstone_relationship(&relationship_id)
        .expect("relationship tombstone should succeed");

    let versions = graph
        .list_relationship_versions(&relationship_id)
        .expect("relationship version listing should not fail");

    assert_eq!(versions.len(), 3);
    assert_eq!(
        versions
            .iter()
            .map(|relationship| relationship.version())
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert_eq!(
        versions
            .iter()
            .filter(|version| version.is_current())
            .count(),
        1
    );
    assert_eq!(
        versions
            .iter()
            .find(|version| version.is_current())
            .expect("one current version should exist")
            .status(),
        RecordStatus::Tombstoned
    );
}

//
// Verify that tombstoning a relationship is represented as a new current version
// and that default relationship reads hide the logically deleted record.
// Tombstones must preserve history rather than physically removing the
// relationship.
//
// Given an existing relationship and its first version ID,
// when `Graph::tombstone_relationship` is called,
// then `Graph::get_relationship` should return `None`, version history should
// contain the original and tombstone versions, and the tombstone should be
// current.
#[test]
fn tombstone_relationship_creates_current_tombstone_version_and_hides_default_read() {
    let (mut graph, _source_id, _target_id, relationship_id, first_version_id) =
        graph_with_relationship();

    graph
        .tombstone_relationship(&relationship_id)
        .expect("relationship tombstone should succeed");

    let current_read = graph
        .get_relationship(&relationship_id)
        .expect("relationship lookup should not fail");
    assert!(
        current_read.is_none(),
        "default reads hide tombstoned relationships"
    );

    let versions = graph
        .list_relationship_versions(&relationship_id)
        .expect("relationship version listing should not fail");
    assert_eq!(versions.len(), 2);

    let tombstone = versions
        .iter()
        .find(|version| version.is_current())
        .expect("one current tombstone version should exist");
    assert_eq!(tombstone.status(), RecordStatus::Tombstoned);
    assert_eq!(tombstone.version(), 2);
    assert_eq!(tombstone.previous_version_id(), Some(&first_version_id));
}

//
// Verify that default adjacency reads hide tombstoned relationships. Logical
// deletion should not require removing IDs from adjacency indexes because default
// traversal can filter current tombstone versions.
//
// Given an existing source-to-target relationship,
// when the relationship is tombstoned,
// then outgoing, incoming, and pairwise adjacency reads should all return empty
// results for that tombstoned relationship.
#[test]
fn tombstoned_relationships_are_hidden_from_default_adjacency_reads() {
    let (mut graph, source_id, target_id, relationship_id, _first_version_id) =
        graph_with_relationship();

    graph
        .tombstone_relationship(&relationship_id)
        .expect("relationship tombstone should succeed");

    assert!(
        graph
            .outgoing(&source_id)
            .expect("outgoing traversal should not fail")
            .is_empty()
    );
    assert!(
        graph
            .incoming(&target_id)
            .expect("incoming traversal should not fail")
            .is_empty()
    );
    assert!(
        graph
            .relationships_between(&source_id, &target_id)
            .expect("pairwise traversal should not fail")
            .is_empty()
    );
}

//
// Verify that updating an unknown relationship fails explicitly instead of
// creating an orphan version or returning a generic implementation error.
// Relationship version history must always be rooted in an existing stable
// relationship ID.
//
// Given an empty graph and a valid missing relationship ID,
// when `Graph::update_relationship` is called,
// then it should fail with `GraphError::RelationshipNotFound` for that ID.
#[test]
fn update_relationship_returns_relationship_not_found_for_unknown_relationships() {
    let mut graph = Graph::new();
    let missing = RelationshipId::new("relationship--missing")
        .expect("valid relationship ID should be accepted");

    let error = graph
        .update_relationship(&missing, RelationshipPatch::default())
        .expect_err("updating an unknown relationship should fail");

    assert!(matches!(error, GraphError::RelationshipNotFound(id) if id == missing));
}

//
// Verify that tombstoning an unknown relationship fails explicitly instead of
// creating an orphan tombstone version. Logical deletion only makes sense when
// there is an existing stable relationship history to append to.
//
// Given an empty graph and a valid missing relationship ID,
// when `Graph::tombstone_relationship` is called,
// then it should fail with `GraphError::RelationshipNotFound` for that ID.
#[test]
fn tombstone_relationship_returns_relationship_not_found_for_unknown_relationships() {
    let mut graph = Graph::new();
    let missing = RelationshipId::new("relationship--missing")
        .expect("valid relationship ID should be accepted");

    let error = graph
        .tombstone_relationship(&missing)
        .expect_err("tombstoning an unknown relationship should fail");

    assert!(matches!(error, GraphError::RelationshipNotFound(id) if id == missing));
}

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
// Optional dataset harness template for .
//
// Show how to consume the synthetic JSON fixtures without making
// graph-core depend on domain-specific rules or private storage internals.
//
//
// - The harness resolves dataset-local refs to generated NodeId/RelationshipId values.
// - Assertions use the public graph_core API only.
// - Ordering is normalized before equality checks where ordering is not a public contract.
//
//
// - Missing logical refs should fail the harness setup, not graph-core.
// - Graph operation errors should be asserted through typed GraphError variants.
//
// Note:
// This is a template, not a drop-in test. Adapt the JSON loading layer to the
// crate's current dev-dependencies. If serde_json is not yet available in
// graph-core tests, either add it as a dev-dependency in the owning issue or
// inline the fixtures as Rust builders.

use std::collections::HashMap;

use graph_core::{
    Confidence, Graph, GraphError, NodeId, NodeInput, NodePatch, PropertyValue, RecordStatus,
    RelationshipId, RelationshipInput,
};

#[test]
fn synthetic_dataset_happy_path_observable_contract() {
    // Given: the synthetic happy-path fixture records.
    let mut graph = Graph::new();
    let mut node_refs: HashMap<&str, NodeId> = HashMap::new();
    let mut relationship_refs: HashMap<&str, RelationshipId> = HashMap::new();

    // When: create nodes from fixtures/entities.json.
    // TODO: replace these examples with fixture-driven construction.
    let actor_id = graph
        .create_node(
            NodeInput::new(["ThreatActor"])
                .with_property("name", PropertyValue::String("APT28".to_owned()))
                .with_status(RecordStatus::Candidate),
        )
        .expect("actor node creation should succeed");
    node_refs.insert("actor_apt28", actor_id.clone());

    let malware_id = graph
        .create_node(
            NodeInput::new(["Malware"])
                .with_property("name", PropertyValue::String("X-Agent".to_owned()))
                .with_status(RecordStatus::Candidate),
        )
        .expect("malware node creation should succeed");
    node_refs.insert("malware_xagent", malware_id.clone());

    // When: create a relationship and validate adjacency.
    let rel_id = graph
        .create_relationship(
            RelationshipInput::new(actor_id.clone(), "USES", malware_id.clone())
                .expect("relationship input should be valid")
                .with_confidence(Confidence::new(0.82).expect("confidence should be valid")),
        )
        .expect("relationship creation should succeed");
    relationship_refs.insert("rel_apt28_uses_xagent", rel_id.clone());

    // Then: the relationship is visible from public adjacency APIs.
    let outgoing = graph.outgoing(&actor_id).expect("outgoing should succeed");
    assert_eq!(outgoing.len(), 1);
    assert_eq!(outgoing[0].id(), &rel_id);

    let incoming = graph
        .incoming(&malware_id)
        .expect("incoming should succeed");
    assert_eq!(incoming.len(), 1);
    assert_eq!(incoming[0].id(), &rel_id);

    let between = graph
        .relationships_between(&actor_id, &malware_id)
        .expect("between should succeed");
    assert_eq!(between.len(), 1);
    assert_eq!(between[0].id(), &rel_id);
}

#[test]
fn synthetic_dataset_error_examples_match_typed_variants() {
    // Given: a fresh Graph and a syntactically valid missing node ID.
    let mut graph = Graph::new();
    let missing = NodeId::new("node--missing-update").expect("valid missing node ID");

    // When: a missing node is updated.
    let error = graph
        .update_node(
            &missing,
            NodePatch::default().set_status(RecordStatus::NeedsReview),
        )
        .expect_err("missing node update should fail");

    // Then: the public API returns a typed error variant.
    assert!(matches!(error, GraphError::NodeNotFound(id) if id == missing));
}

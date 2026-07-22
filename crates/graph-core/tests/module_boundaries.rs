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
use std::collections::HashMap;

use graph_core::{
    ActorId, Confidence, EvidenceId, ExtractionRunId, Graph, GraphError, LabelSet, Node, NodeId,
    NodeInput, NodePatch, NodeVersionId, PropertyMap, PropertyValue, RecordStatus, Relationship,
    RelationshipId, RelationshipInput, RelationshipPatch, RelationshipType, RelationshipVersionId,
    RequestId, SessionId, TemporalMetadata, TransactionId, TransactionMetadata, WorkspaceId,
};

fn assert_clone_debug<T: Clone + std::fmt::Debug>() {
    let _ = std::any::type_name::<T>();
}

//
// Verify that graph-core's stable public API can be imported from the crate
// facade instead of private implementation modules. This protects `lib.rs` as a
// small public boundary while allowing internals to stay organized in focused
// files.
//
// Given the stable graph-core public types re-exported by `graph_core`,
// when an integration test imports and uses those types through the facade,
// then the test should compile and should not need any private module imports.
#[test]
fn public_facade_exports_stable_graph_core_types() {
    assert_clone_debug::<Graph>();
    assert_clone_debug::<Node>();
    assert_clone_debug::<Relationship>();

    let node_id = NodeId::new("node--facade").expect("node ID should be valid");
    let node_version_id =
        NodeVersionId::new("node-version--facade").expect("node version ID should be valid");
    let relationship_id =
        RelationshipId::new("relationship--facade").expect("relationship ID should be valid");
    let relationship_version_id = RelationshipVersionId::new("relationship-version--facade")
        .expect("relationship version ID should be valid");
    let evidence_id = EvidenceId::new("evidence--facade").expect("evidence ID should be valid");

    let mut properties: PropertyMap = HashMap::new();
    properties.insert(
        "name".to_owned(),
        PropertyValue::String("facade import".to_owned()),
    );
    let labels: LabelSet = vec!["ThreatActor".to_owned(), "Indicator".to_owned()];
    let confidence = Confidence::new(0.7).expect("confidence should be in range");
    let relationship_type =
        RelationshipType::new("indicates").expect("relationship type should be valid");

    let _temporal = TemporalMetadata {
        created_at: Some("2026-06-28T00:00:00Z".to_owned()),
        ..TemporalMetadata::default()
    };
    let _transaction = TransactionMetadata {
        transaction_id: Some(
            TransactionId::new("transaction--facade").expect("transaction ID should be valid"),
        ),
        workspace_id: Some(
            WorkspaceId::new("workspace--facade").expect("workspace ID should be valid"),
        ),
        actor_id: Some(ActorId::new("actor--facade").expect("actor ID should be valid")),
        session_id: Some(SessionId::new("session--facade").expect("session ID should be valid")),
        request_id: Some(RequestId::new("request--facade").expect("request ID should be valid")),
        extraction_run_id: Some(
            ExtractionRunId::new("extraction-run--facade")
                .expect("extraction run ID should be valid"),
        ),
    };

    let node_input = NodeInput::new(labels.iter().map(String::as_str))
        .with_property("name", PropertyValue::String("APT28".to_owned()))
        .with_status(RecordStatus::Candidate)
        .with_confidence(confidence);
    let _node_patch = NodePatch::default()
        .set_property("score", PropertyValue::Integer(90))
        .set_status(RecordStatus::NeedsReview)
        .set_confidence(confidence);
    let _relationship_input = RelationshipInput::new(
        node_id.clone(),
        relationship_type.as_str().to_owned(),
        NodeId::new("node--target").expect("target node ID should be valid"),
    )
    .expect("relationship input should be valid")
    .with_property("source", PropertyValue::String("facade".to_owned()))
    .with_status(RecordStatus::Validated)
    .with_confidence(confidence);
    let _relationship_patch = RelationshipPatch::default()
        .set_property("status", PropertyValue::String("reviewed".to_owned()))
        .set_status(RecordStatus::Exportable)
        .set_confidence(confidence);

    assert_eq!(node_input.validate(), Ok(()));
    assert_eq!(
        properties.get("name"),
        Some(&PropertyValue::String("facade import".to_owned()))
    );
    assert_eq!(node_version_id.as_str(), "node-version--facade");
    assert_eq!(relationship_id.as_str(), "relationship--facade");
    assert_eq!(
        relationship_version_id.as_str(),
        "relationship-version--facade"
    );
    assert_eq!(evidence_id.as_str(), "evidence--facade");
    assert!(matches!(
    GraphError::NodeNotFound(node_id.clone()),
    GraphError::NodeNotFound(id) if id == node_id
    ));
}

//
// Verify that graph operations can be exercised through public `graph_core`
// imports only. This is the acceptance-level signal that tests do not need to
// reach into private modules for node creation, relationship creation, or
// traversal reads.
//
// Given a graph built through public facade types,
// when nodes and a relationship are created and queried,
// then the public API should expose the expected node, relationship, and
// adjacency behavior without private module imports.
#[test]
fn graph_operations_are_testable_through_public_facade_imports() {
    let mut graph = Graph::new();
    let source = graph
        .create_node(
            NodeInput::new(["ThreatActor"])
                .with_property("name", PropertyValue::String("APT28".to_owned())),
        )
        .expect("source node creation should succeed");
    let target = graph
        .create_node(
            NodeInput::new(["Indicator"])
                .with_property("value", PropertyValue::String("203.0.113.10".to_owned())),
        )
        .expect("target node creation should succeed");

    let relationship_id = graph
        .create_relationship(
            RelationshipInput::new(source.clone(), "indicates", target.clone())
                .expect("relationship input should be valid")
                .with_property("source", PropertyValue::String("unit-test".to_owned())),
        )
        .expect("relationship creation should succeed");

    let relationship = graph
        .get_relationship(&relationship_id)
        .expect("relationship lookup should not fail")
        .expect("relationship should exist");
    let outgoing = graph
        .outgoing(&source)
        .expect("outgoing traversal should not fail");
    let incoming = graph
        .incoming(&target)
        .expect("incoming traversal should not fail");
    let between = graph
        .relationships_between(&source, &target)
        .expect("pairwise traversal should not fail");

    assert_eq!(relationship.source(), &source);
    assert_eq!(relationship.target(), &target);
    assert_eq!(relationship.rel_type().as_str(), "indicates");
    assert_eq!(outgoing.len(), 1);
    assert_eq!(incoming.len(), 1);
    assert_eq!(between.len(), 1);
    assert_eq!(between[0].id(), &relationship_id);
}

//
// Verify that public error and absence semantics remain available through the
// crate facade. Public callers and integration tests should be able to match
// graph-core error variants without importing the private `error` module.
//
// Given missing graph records and invalid public primitive values,
// when facade-level APIs are called,
// then absence and error outcomes should be observable through public
// `GraphError` variants.
#[test]
fn public_facade_exposes_matchable_absence_and_error_semantics() {
    let mut graph = Graph::new();
    let missing = NodeId::new("node--missing").expect("missing node ID should be valid");

    let missing_read = graph
        .get_node(&missing)
        .expect("missing node lookup should not fail");
    let update_error = graph
        .update_node(&missing, NodePatch::default())
        .expect_err("updating a missing node should fail");
    let relationship_type_error = RelationshipType::new(" \t\n")
        .expect_err("whitespace-only relationship type should be rejected");
    let confidence_error =
        Confidence::new(1.01).expect_err("confidence above one should be rejected");

    assert!(missing_read.is_none());
    assert!(matches!(update_error, GraphError::NodeNotFound(id) if id == missing));
    assert!(matches!(
    relationship_type_error,
    GraphError::InvalidRelationshipType(value) if value == " \t\n"
    ));
    assert!(matches!(
    confidence_error,
    GraphError::InvalidConfidence(value) if value == 1.01
    ));
}

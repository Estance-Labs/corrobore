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
use graph_core::{AdjacencyDirection, Graph, NodeInput, RelationshipInput};
use graph_storage::{
    GraphRecordVersion, GraphStorageError, PersistedRecordEnvelope, PersistedRecordId,
    PersistedRecordKind, RecordChecksum, RecordFormat, StorageRef, StorageSegment, StorageVersion,
    create_adjacency_record_envelope, create_node_record_envelope,
    create_relationship_record_envelope, validate_persisted_record_envelope, validate_storage_ref,
};

fn checksum() -> RecordChecksum {
    RecordChecksum {
        algorithm: "sha256".to_owned(),
        value: "a4f2cc7f0e5b9712c1e7c2f8b4f0b91d".to_owned(),
    }
}

fn storage_ref(segment: StorageSegment, offset: u64) -> StorageRef {
    StorageRef {
        segment,
        offset,
        length: 512,
        checksum: Some(checksum()),
    }
}

//
// Validate the public graph-storage API against the acceptance criteria:
// node, relationship, and adjacency records must become independently loadable
// persisted envelopes with explicit storage refs, storage versions, record formats,
// checksums, and graph record versions where applicable.
//
// Given graph-core node and relationship records plus separate storage references,
// when the graph-storage public envelope helpers are used,
// then each returned envelope should be valid and carry the expected loadable-unit metadata.
#[test]
fn public_api_creates_valid_node_relationship_and_adjacency_envelopes() {
    let mut graph = Graph::new();
    let source = graph
        .create_node(NodeInput::new(["Actor"]))
        .expect("source node should be created");
    let target = graph
        .create_node(NodeInput::new(["Infrastructure"]))
        .expect("target node should be created");
    let relationship_id = graph
        .create_relationship(
            RelationshipInput::new(source.clone(), "USES", target)
                .expect("relationship input should be valid"),
        )
        .expect("relationship should be created");
    let node = graph
        .get_node(&source)
        .expect("node read should succeed")
        .expect("node should be visible");
    let relationship = graph
        .get_relationship(&relationship_id)
        .expect("relationship read should succeed")
        .expect("relationship should be visible");

    let node_envelope = create_node_record_envelope(
        &node,
        storage_ref(StorageSegment::NodeRecords, 0),
        StorageVersion::V1,
        RecordFormat::JsonLinesV1,
        Some(checksum()),
    )
    .expect("node envelope should be valid");
    let relationship_envelope = create_relationship_record_envelope(
        &relationship,
        storage_ref(StorageSegment::RelationshipRecords, 512),
        StorageVersion::V1,
        RecordFormat::JsonLinesV1,
        Some(checksum()),
    )
    .expect("relationship envelope should be valid");
    let outgoing_adjacency_envelope = create_adjacency_record_envelope(
        &source,
        AdjacencyDirection::Outgoing,
        storage_ref(StorageSegment::OutgoingAdjacency, 1024),
        StorageVersion::V1,
        RecordFormat::JsonLinesV1,
        None,
    )
    .expect("outgoing adjacency envelope should be valid");

    assert_eq!(
        node_envelope.record_id,
        PersistedRecordId::Node(source.clone())
    );
    assert_eq!(node_envelope.kind, PersistedRecordKind::Node);
    assert!(matches!(
        node_envelope.graph_record_version,
        Some(GraphRecordVersion::Node {
            version: 1,
            current: true,
            ..
        })
    ));
    assert_eq!(
        relationship_envelope.record_id,
        PersistedRecordId::Relationship(relationship_id)
    );
    assert_eq!(
        relationship_envelope.kind,
        PersistedRecordKind::Relationship
    );
    assert!(matches!(
        relationship_envelope.graph_record_version,
        Some(GraphRecordVersion::Relationship {
            version: 1,
            current: true,
            ..
        })
    ));
    assert_eq!(
        outgoing_adjacency_envelope.record_id,
        PersistedRecordId::Adjacency {
            owner_node_id: source,
            direction: AdjacencyDirection::Outgoing,
        }
    );
    assert_eq!(
        outgoing_adjacency_envelope.kind,
        PersistedRecordKind::OutgoingAdjacency
    );
    assert_eq!(outgoing_adjacency_envelope.graph_record_version, None);
}

//
// Validate that the public storage reference guard rejects unsafe physical ranges.
// This exercises the future catalog/pager failure mode through the crate facade,
// not through the internal module layout.
//
// Given a zero-length public storage reference,
// when the public validation function is called,
// then it should return the explicit `InvalidStorageRef` error.
#[test]
fn public_api_rejects_invalid_storage_references() {
    let invalid_ref = StorageRef {
        segment: StorageSegment::NodeRecords,
        offset: 42,
        length: 0,
        checksum: None,
    };

    let error = validate_storage_ref(&invalid_ref)
        .expect_err("zero-length public storage refs should be rejected");

    assert!(matches!(
    error,
    GraphStorageError::InvalidStorageRef { storage_ref, reason }
    if storage_ref == invalid_ref && reason.contains("length")
    ));
}

//
// Validate that the public envelope validator rejects inconsistent catalog/pager
// metadata before any payload is decoded. This is the acceptance guard for future
// page-in calls.
//
// Given a node envelope whose physical reference points to the relationship segment,
// when the public envelope validator is called,
// then it should return the explicit `InvalidEnvelope` error.
#[test]
fn public_api_rejects_invalid_envelopes_before_page_in() {
    let mut graph = Graph::new();
    let node_id = graph
        .create_node(NodeInput::new(["Campaign"]))
        .expect("node should be created");
    let node = graph
        .get_node(&node_id)
        .expect("node read should succeed")
        .expect("node should be visible");
    let invalid_envelope = PersistedRecordEnvelope {
        record_id: PersistedRecordId::Node(node_id),
        kind: PersistedRecordKind::Node,
        storage_version: StorageVersion::V1,
        record_format: RecordFormat::JsonLinesV1,
        graph_record_version: Some(GraphRecordVersion::Node {
            version_id: node.version_id().clone(),
            version: node.version(),
            current: node.is_current(),
            previous_version_id: node.previous_version_id().cloned(),
        }),
        storage_ref: storage_ref(StorageSegment::RelationshipRecords, 2048),
        record_checksum: Some(checksum()),
    };

    let error = validate_persisted_record_envelope(&invalid_envelope)
        .expect_err("kind/segment mismatches should be rejected before page-in");

    assert!(matches!(
    error,
    GraphStorageError::InvalidEnvelope { reason } if reason.contains("segment")
    ));
}

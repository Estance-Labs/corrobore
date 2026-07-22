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
//! Property-based tests for the persisted-record codec (audit §6 — testing depth).
//!
//! The on-disk format is checksummed JSON Lines, so two properties must hold for
//! *any* validly constructed record envelope:
//!
//! 1. **Encode/decode identity.** `decode(encode(envelope)) == envelope`. No
//!    metadata may be lost or mutated across a persistence round-trip, otherwise
//!    reopened graphs would silently diverge from what was written.
//! 2. **Corruption is always detected.** Tampering with a single byte of an
//!    encoded record must surface `GraphStorageError::ChecksumMismatch` on decode
//!    (never a silently-accepted or partially-decoded record). This is the
//!    integrity guarantee the storage layer promises to callers.

#![allow(clippy::unwrap_used)]

use graph_core::{
    AdjacencyDirection, Graph, Node, NodeId, NodeInput, Relationship, RelationshipInput,
};
use graph_storage::{
    GraphStorageError, JsonLinesRecordCodec, PersistedRecordEnvelope, PersistedRecordKind,
    RecordChecksum, RecordFormat, StorageRef, StorageSegment, StorageVersion,
    create_adjacency_record_envelope, create_node_record_envelope,
    create_relationship_record_envelope, decode_persisted_record_envelope,
    encode_persisted_record_envelope, validate_encoded_record_checksum,
};
use proptest::prelude::*;

/// Capitalized label / relationship-type token accepted by graph-core.
fn label() -> impl Strategy<Value = String> {
    "[A-Z][A-Za-z0-9]{0,7}"
}

/// Generate a validated `StorageRef` for a segment with a non-zero length and no
/// offset/length overflow.
fn storage_ref_strategy(segment: StorageSegment) -> impl Strategy<Value = StorageRef> {
    (0u64..1_000_000, 1u64..1_000_000, "[0-9a-f]{1,64}").prop_map(
        move |(offset, length, checksum_value)| StorageRef {
            segment: segment.clone(),
            offset,
            length,
            checksum: Some(RecordChecksum {
                algorithm: "sha256".to_owned(),
                value: checksum_value,
            }),
        },
    )
}

fn node_from_labels(labels: &[String]) -> Node {
    let mut graph = Graph::new();
    let node_id = graph
        .create_node(NodeInput::new(labels.iter().map(String::as_str)))
        .unwrap();
    graph.get_node(&node_id).unwrap().unwrap()
}

fn relationship_from(source_label: &str, rel_type: &str, target_label: &str) -> Relationship {
    let mut graph = Graph::new();
    let source = graph.create_node(NodeInput::new([source_label])).unwrap();
    let target = graph.create_node(NodeInput::new([target_label])).unwrap();
    let relationship_id = graph
        .create_relationship(RelationshipInput::new(source, rel_type, target).unwrap())
        .unwrap();
    graph.get_relationship(&relationship_id).unwrap().unwrap()
}

/// Round-trip a fully-built envelope: encode, decode requiring `kind`, assert the
/// decoded envelope equals the original, then assert that corrupting the encoded
/// bytes is always rejected with a checksum mismatch.
fn assert_round_trip_and_corruption(envelope: &PersistedRecordEnvelope, kind: PersistedRecordKind) {
    let codec = JsonLinesRecordCodec::default();
    let encoded = encode_persisted_record_envelope(&codec, envelope).unwrap();

    let decoded = decode_persisted_record_envelope(&codec, &encoded, Some(kind)).unwrap();
    assert_eq!(&decoded, envelope);

    // The freshly-encoded bytes validate against their own checksum...
    validate_encoded_record_checksum(&codec, &encoded.bytes, &encoded.checksum).unwrap();

    // ...but any tampering is detected on both the standalone validator and decode.
    let mut corrupted = encoded.clone();
    corrupted.bytes.extend_from_slice(b"!");

    assert!(matches!(
        validate_encoded_record_checksum(&codec, &corrupted.bytes, &encoded.checksum),
        Err(GraphStorageError::ChecksumMismatch { .. })
    ));
    assert!(matches!(
        decode_persisted_record_envelope(&codec, &corrupted, Some(kind)),
        Err(GraphStorageError::ChecksumMismatch { .. })
    ));
}

proptest! {
    #[test]
    fn node_envelope_round_trips_and_detects_corruption(
        labels in proptest::collection::vec(label(), 1..4),
        storage_ref in storage_ref_strategy(StorageSegment::NodeRecords),
    ) {
        let node = node_from_labels(&labels);
        let envelope = create_node_record_envelope(
            &node,
            storage_ref,
            StorageVersion::V1,
            RecordFormat::JsonLinesV1,
            None,
        )
        .unwrap();

        assert_round_trip_and_corruption(&envelope, PersistedRecordKind::Node);
    }

    #[test]
    fn relationship_envelope_round_trips_and_detects_corruption(
        source_label in label(),
        rel_type in label(),
        target_label in label(),
        storage_ref in storage_ref_strategy(StorageSegment::RelationshipRecords),
    ) {
        let relationship = relationship_from(&source_label, &rel_type, &target_label);
        let envelope = create_relationship_record_envelope(
            &relationship,
            storage_ref,
            StorageVersion::V1,
            RecordFormat::JsonLinesV1,
            None,
        )
        .unwrap();

        assert_round_trip_and_corruption(&envelope, PersistedRecordKind::Relationship);
    }

    #[test]
    fn adjacency_envelope_round_trips_and_detects_corruption(
        owner in "[a-z0-9-]{1,16}",
        outgoing in any::<bool>(),
    ) {
        let (direction, segment, kind) = if outgoing {
            (
                AdjacencyDirection::Outgoing,
                StorageSegment::OutgoingAdjacency,
                PersistedRecordKind::OutgoingAdjacency,
            )
        } else {
            (
                AdjacencyDirection::Incoming,
                StorageSegment::IncomingAdjacency,
                PersistedRecordKind::IncomingAdjacency,
            )
        };
        let owner_node_id = NodeId::new(format!("node--{owner}")).unwrap();

        // Adjacency uses a fixed valid storage ref; identity/corruption behavior
        // is independent of the exact offset.
        let storage_ref = StorageRef {
            segment,
            offset: 64,
            length: 128,
            checksum: None,
        };
        let envelope = create_adjacency_record_envelope(
            &owner_node_id,
            direction,
            storage_ref,
            StorageVersion::V1,
            RecordFormat::JsonLinesV1,
            None,
        )
        .unwrap();

        assert_round_trip_and_corruption(&envelope, kind);
    }
}

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
use graph_core::{
    AdjacencyDirection, Graph, Node, NodeId, NodeInput, Relationship, RelationshipInput,
};
use graph_storage::{
    EncodedRecord, GraphRecordVersion, GraphStorageError, JsonLinesRecordCodec,
    PersistedRecordEnvelope, PersistedRecordId, PersistedRecordKind, RecordChecksum, RecordFormat,
    StorageRef, StorageSegment, StorageVersion, calculate_encoded_record_checksum,
    create_adjacency_record_envelope, create_node_record_envelope,
    create_relationship_record_envelope, decode_persisted_record_envelope,
    encode_persisted_record_envelope, validate_encoded_record_checksum,
};

fn checksum(value: &str) -> RecordChecksum {
    RecordChecksum {
        algorithm: "sha256".to_owned(),
        value: value.to_owned(),
    }
}

fn storage_ref(segment: StorageSegment) -> StorageRef {
    StorageRef {
        segment,
        offset: 128,
        length: 256,
        checksum: Some(checksum("storage-ref-checksum")),
    }
}

fn node_fixture() -> Node {
    let mut graph = Graph::new();
    let node_id = graph
        .create_node(NodeInput::new(["Campaign", "FIMI"]))
        .unwrap();
    graph.get_node(&node_id).unwrap().unwrap()
}

fn relationship_fixture() -> Relationship {
    let mut graph = Graph::new();
    let source = graph.create_node(NodeInput::new(["Actor"])).unwrap();
    let target = graph
        .create_node(NodeInput::new(["Infrastructure"]))
        .unwrap();
    let relationship_id = graph
        .create_relationship(RelationshipInput::new(source, "USES", target).unwrap())
        .unwrap();
    graph.get_relationship(&relationship_id).unwrap().unwrap()
}

fn node_envelope() -> PersistedRecordEnvelope {
    let node = node_fixture();
    create_node_record_envelope(
        &node,
        storage_ref(StorageSegment::NodeRecords),
        StorageVersion::V1,
        RecordFormat::JsonLinesV1,
        Some(checksum("node-record-checksum")),
    )
    .unwrap()
}

fn relationship_envelope() -> PersistedRecordEnvelope {
    let relationship = relationship_fixture();
    create_relationship_record_envelope(
        &relationship,
        storage_ref(StorageSegment::RelationshipRecords),
        StorageVersion::V1,
        RecordFormat::JsonLinesV1,
        Some(checksum("relationship-record-checksum")),
    )
    .unwrap()
}

fn adjacency_envelope(direction: AdjacencyDirection) -> PersistedRecordEnvelope {
    let owner_node_id = NodeId::new("node--owner").unwrap();
    let segment = match direction {
        AdjacencyDirection::Outgoing => StorageSegment::OutgoingAdjacency,
        AdjacencyDirection::Incoming => StorageSegment::IncomingAdjacency,
    };

    create_adjacency_record_envelope(
        &owner_node_id,
        direction,
        storage_ref(segment),
        StorageVersion::V1,
        RecordFormat::JsonLinesV1,
        Some(checksum("adjacency-record-checksum")),
    )
    .unwrap()
}

// Specify that record encoding is deterministic for stable fixtures.
// Given: the same codec and the same validated node envelope.
// When: the envelope is encoded twice.
// Then: both encoded records expose the same bytes, checksum, kind, version, and format.
#[test]
fn json_lines_codec_encodes_same_envelope_deterministically() {
    let codec = JsonLinesRecordCodec::default();
    let envelope = node_envelope();

    let first = encode_persisted_record_envelope(&codec, &envelope).unwrap();
    let second = encode_persisted_record_envelope(&codec, &envelope).unwrap();

    assert_eq!(first.bytes, second.bytes);
    assert_eq!(first.checksum, second.checksum);
    assert_eq!(first.kind, PersistedRecordKind::Node);
    assert_eq!(first.storage_version, StorageVersion::V1);
    assert_eq!(first.record_format, RecordFormat::JsonLinesV1);
}

// Specify that encoded node envelopes can be decoded without metadata loss.
// Given: a validated node envelope encoded with the JSON Lines codec.
// When: the encoded record is decoded while requiring a node record kind.
// Then: the decoded envelope is identical to the original envelope.
#[test]
fn json_lines_codec_decodes_node_envelope_deterministically() {
    let codec = JsonLinesRecordCodec::default();
    let envelope = node_envelope();
    let encoded = encode_persisted_record_envelope(&codec, &envelope).unwrap();

    let decoded =
        decode_persisted_record_envelope(&codec, &encoded, Some(PersistedRecordKind::Node))
            .unwrap();

    assert_eq!(decoded, envelope);
}

// Specify that encoded relationship envelopes are supported by the codec.
// Given: a validated relationship envelope encoded with the JSON Lines codec.
// When: the encoded record is decoded while requiring a relationship record kind.
// Then: the decoded envelope preserves relationship identity and graph version metadata.
#[test]
fn json_lines_codec_decodes_relationship_envelope_deterministically() {
    let codec = JsonLinesRecordCodec::default();
    let envelope = relationship_envelope();
    let encoded = encode_persisted_record_envelope(&codec, &envelope).unwrap();

    let decoded =
        decode_persisted_record_envelope(&codec, &encoded, Some(PersistedRecordKind::Relationship))
            .unwrap();

    assert_eq!(decoded, envelope);
    assert!(matches!(
        decoded.graph_record_version,
        Some(GraphRecordVersion::Relationship { .. })
    ));
}

// Specify that outgoing adjacency envelopes are supported by the codec.
// Given: a validated outgoing adjacency envelope encoded with the JSON Lines codec.
// When: the encoded record is decoded while requiring an outgoing adjacency kind.
// Then: the decoded envelope keeps the outgoing adjacency identity and no graph version metadata.
#[test]
fn json_lines_codec_decodes_outgoing_adjacency_envelope_deterministically() {
    let codec = JsonLinesRecordCodec::default();
    let envelope = adjacency_envelope(AdjacencyDirection::Outgoing);
    let encoded = encode_persisted_record_envelope(&codec, &envelope).unwrap();

    let decoded = decode_persisted_record_envelope(
        &codec,
        &encoded,
        Some(PersistedRecordKind::OutgoingAdjacency),
    )
    .unwrap();

    assert_eq!(decoded, envelope);
    assert_eq!(decoded.graph_record_version, None);
    assert!(matches!(
        decoded.record_id,
        PersistedRecordId::Adjacency {
            direction: AdjacencyDirection::Outgoing,
            ..
        }
    ));
}

// Specify that incoming adjacency envelopes are supported by the codec.
// Given: a validated incoming adjacency envelope encoded with the JSON Lines codec.
// When: the encoded record is decoded while requiring an incoming adjacency kind.
// Then: the decoded envelope keeps the incoming adjacency identity and no graph version metadata.
#[test]
fn json_lines_codec_decodes_incoming_adjacency_envelope_deterministically() {
    let codec = JsonLinesRecordCodec::default();
    let envelope = adjacency_envelope(AdjacencyDirection::Incoming);
    let encoded = encode_persisted_record_envelope(&codec, &envelope).unwrap();

    let decoded = decode_persisted_record_envelope(
        &codec,
        &encoded,
        Some(PersistedRecordKind::IncomingAdjacency),
    )
    .unwrap();

    assert_eq!(decoded, envelope);
    assert_eq!(decoded.graph_record_version, None);
    assert!(matches!(
        decoded.record_id,
        PersistedRecordId::Adjacency {
            direction: AdjacencyDirection::Incoming,
            ..
        }
    ));
}

// Specify that checksum calculation is deterministic for encoded fixtures.
// Given: the same canonical encoded byte sequence.
// When: the checksum is calculated twice by the same codec.
// Then: the checksum values are equal and contain complete algorithm metadata.
#[test]
fn json_lines_codec_calculates_same_checksum_for_same_bytes() {
    let codec = JsonLinesRecordCodec::default();
    let bytes = b"canonical persisted record bytes";

    let first = calculate_encoded_record_checksum(&codec, bytes).unwrap();
    let second = calculate_encoded_record_checksum(&codec, bytes).unwrap();

    assert_eq!(first, second);
    assert!(!first.algorithm.trim().is_empty());
    assert!(!first.value.trim().is_empty());
}

// Specify that matching checksums are accepted before decode.
// Given: canonical encoded bytes and their checksum from the same codec.
// When: the checksum validator is called with the matching checksum.
// Then: validation succeeds without changing record data.
#[test]
fn json_lines_codec_accepts_matching_checksum() {
    let codec = JsonLinesRecordCodec::default();
    let bytes = b"canonical persisted record bytes";
    let expected_checksum = calculate_encoded_record_checksum(&codec, bytes).unwrap();

    validate_encoded_record_checksum(&codec, bytes, &expected_checksum).unwrap();
}

// Specify that corrupted encoded records fail with an explicit checksum error.
// Given: a valid encoded relationship record and the checksum calculated for its original bytes.
// When: the encoded bytes are modified before decode.
// Then: decode fails with `GraphStorageError::ChecksumMismatch` rather than returning a record.
#[test]
fn json_lines_codec_rejects_corrupted_bytes_with_checksum_mismatch() {
    let codec = JsonLinesRecordCodec::default();
    let envelope = relationship_envelope();
    let mut encoded = encode_persisted_record_envelope(&codec, &envelope).unwrap();
    encoded.bytes.extend_from_slice(b"corruption");

    let error =
        decode_persisted_record_envelope(&codec, &encoded, Some(PersistedRecordKind::Relationship))
            .unwrap_err();

    assert!(matches!(error, GraphStorageError::ChecksumMismatch { .. }));
}

// Specify that typed readers reject records of the wrong logical kind.
// Given: a valid encoded node record.
// When: the caller decodes it while requiring a relationship record kind.
// Then: decode fails with `GraphStorageError::UnexpectedRecordKind`.
#[test]
fn json_lines_codec_rejects_unexpected_record_kind() {
    let codec = JsonLinesRecordCodec::default();
    let envelope = node_envelope();
    let encoded = encode_persisted_record_envelope(&codec, &envelope).unwrap();

    let error =
        decode_persisted_record_envelope(&codec, &encoded, Some(PersistedRecordKind::Relationship))
            .unwrap_err();

    assert!(matches!(
        error,
        GraphStorageError::UnexpectedRecordKind {
            expected: PersistedRecordKind::Relationship,
            actual: PersistedRecordKind::Node,
        }
    ));
}

// Specify that unsupported storage versions are rejected explicitly.
// Given: encoded bytes with a valid checksum but unsupported storage version metadata.
// When: the encoded record is decoded.
// Then: decode fails with `GraphStorageError::UnsupportedStorageVersion`.
#[test]
fn json_lines_codec_rejects_unsupported_storage_version() {
    let codec = JsonLinesRecordCodec::default();
    let bytes = b"{\"storage_version\":\"V999\"}\n".to_vec();
    let checksum = calculate_encoded_record_checksum(&codec, &bytes).unwrap();
    let encoded = EncodedRecord {
        storage_version: StorageVersion::Unsupported("V999".to_owned()),
        record_format: RecordFormat::JsonLinesV1,
        kind: PersistedRecordKind::Node,
        bytes,
        checksum,
    };

    let error = decode_persisted_record_envelope(&codec, &encoded, Some(PersistedRecordKind::Node))
        .unwrap_err();

    assert!(matches!(
    error,
    GraphStorageError::UnsupportedStorageVersion { version } if version == "V999"
    ));
}

// Specify that malformed bytes are not swallowed as empty records.
// Given: malformed bytes protected by a matching checksum.
// When: the encoded record is decoded.
// Then: decode fails with `GraphStorageError::DecodeFailed`.
#[test]
fn json_lines_codec_rejects_malformed_bytes_with_decode_failure() {
    let codec = JsonLinesRecordCodec::default();
    let bytes = b"not a persisted record envelope".to_vec();
    let checksum = calculate_encoded_record_checksum(&codec, &bytes).unwrap();
    let encoded = EncodedRecord {
        storage_version: StorageVersion::V1,
        record_format: RecordFormat::JsonLinesV1,
        kind: PersistedRecordKind::Node,
        bytes,
        checksum,
    };

    let error = decode_persisted_record_envelope(&codec, &encoded, Some(PersistedRecordKind::Node))
        .unwrap_err();

    assert!(matches!(error, GraphStorageError::DecodeFailed { .. }));
}

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
use graph_core::{Graph, Node, NodeInput, Relationship, RelationshipInput};
use graph_storage::{
    EncodedRecord, GraphId, GraphStorageError, JsonLinesRecordCodec, PersistedRecordEnvelope,
    PersistedRecordKind, RecordCodec, RecordFormat, StorageManifest, StorageRef, StorageSegment,
    StorageTimestamp, StorageVersion, append_encoded_node_record_envelope,
    append_encoded_relationship_record_envelope, create_node_record_envelope,
    create_relationship_record_envelope, create_storage_root, decode_persisted_record_envelope,
    flush_append_only_record_log, open_append_only_node_record_log,
    open_append_only_relationship_record_log, open_storage_root,
};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_temp_path(test_name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "intelligence_graph_engine_issue_54_acceptance_{test_name}_{}_{}",
        std::process::id(),
        unique
    ))
}

fn manifest() -> StorageManifest {
    StorageManifest {
        storage_version: StorageVersion::V1,
        graph_id: GraphId {
            value: "graph--issue-54-acceptance".to_owned(),
        },
        created_at: StorageTimestamp {
            value: "2026-07-05T00:00:00Z".to_owned(),
        },
        updated_at: StorageTimestamp {
            value: "2026-07-05T00:00:00Z".to_owned(),
        },
        record_format: RecordFormat::JsonLinesV1,
    }
}

fn storage_ref(segment: StorageSegment) -> StorageRef {
    StorageRef {
        segment,
        offset: 0,
        length: 1,
        checksum: None,
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

fn encoded_node_record() -> (PersistedRecordEnvelope, EncodedRecord) {
    let node = node_fixture();
    let envelope = create_node_record_envelope(
        &node,
        storage_ref(StorageSegment::NodeRecords),
        StorageVersion::V1,
        RecordFormat::JsonLinesV1,
        None,
    )
    .unwrap();
    let encoded = JsonLinesRecordCodec.encode_envelope(&envelope).unwrap();
    (envelope, encoded)
}

fn encoded_relationship_record() -> (PersistedRecordEnvelope, EncodedRecord) {
    let relationship = relationship_fixture();
    let envelope = create_relationship_record_envelope(
        &relationship,
        storage_ref(StorageSegment::RelationshipRecords),
        StorageVersion::V1,
        RecordFormat::JsonLinesV1,
        None,
    )
    .unwrap();
    let encoded = JsonLinesRecordCodec.encode_envelope(&envelope).unwrap();
    (envelope, encoded)
}

/// Validate the acceptance path from public APIs only for
/// node and relationship append-only logs.
///
/// Given: a fresh storage root, one encoded node envelope, and one encoded
/// relationship envelope.
/// When: both typed append-only logs are opened, appended to, flushed, and read
/// back from disk.
/// Then: node and relationship bytes are stored in separate durable logs, storage
/// refs are deterministic, decoded envelopes match the original records, and no
/// catalog or adjacency persistence is created by this issue.
#[test]
fn acceptance_appends_node_and_relationship_records_to_separate_durable_logs() {
    let root_path = unique_temp_path("separate_durable_logs");
    let root = create_storage_root(root_path.clone(), manifest()).unwrap();
    let mut node_log = open_append_only_node_record_log(&root).unwrap();
    let mut relationship_log = open_append_only_relationship_record_log(&root).unwrap();
    let (node_envelope, encoded_node) = encoded_node_record();
    let (relationship_envelope, encoded_relationship) = encoded_relationship_record();

    let node_ref =
        append_encoded_node_record_envelope(&mut node_log, &node_envelope, &encoded_node).unwrap();
    let relationship_ref = append_encoded_relationship_record_envelope(
        &mut relationship_log,
        &relationship_envelope,
        &encoded_relationship,
    )
    .unwrap();
    flush_append_only_record_log(&mut node_log).unwrap();
    flush_append_only_record_log(&mut relationship_log).unwrap();

    assert_eq!(node_ref.segment, StorageSegment::NodeRecords);
    assert_eq!(node_ref.offset, 0);
    assert_eq!(node_ref.length, encoded_node.bytes.len() as u64);
    assert_eq!(node_ref.checksum, Some(encoded_node.checksum.clone()));
    assert_eq!(
        relationship_ref.segment,
        StorageSegment::RelationshipRecords
    );
    assert_eq!(relationship_ref.offset, 0);
    assert_eq!(
        relationship_ref.length,
        encoded_relationship.bytes.len() as u64
    );
    assert_eq!(
        relationship_ref.checksum,
        Some(encoded_relationship.checksum.clone())
    );

    let persisted_node_bytes = fs::read(&node_log.path).unwrap();
    let persisted_relationship_bytes = fs::read(&relationship_log.path).unwrap();
    assert_eq!(persisted_node_bytes, encoded_node.bytes);
    assert_eq!(persisted_relationship_bytes, encoded_relationship.bytes);

    let decoded_node = decode_persisted_record_envelope(
        &JsonLinesRecordCodec,
        &EncodedRecord {
            bytes: persisted_node_bytes,
            ..encoded_node.clone()
        },
        Some(PersistedRecordKind::Node),
    )
    .unwrap();
    let decoded_relationship = decode_persisted_record_envelope(
        &JsonLinesRecordCodec,
        &EncodedRecord {
            bytes: persisted_relationship_bytes,
            ..encoded_relationship.clone()
        },
        Some(PersistedRecordKind::Relationship),
    )
    .unwrap();

    assert_eq!(decoded_node.record_id, node_envelope.record_id);
    assert_eq!(
        decoded_relationship.record_id,
        relationship_envelope.record_id
    );
    assert!(!root.path().join("catalog").exists());
    assert!(!root.path().join("adjacency").exists());
    let _ = fs::remove_dir_all(root_path);
}

/// Validate that append-only offsets survive reopening the storage
/// boundary instead of depending on in-memory state.
///
/// Given: a storage root with a node envelope already appended to the node log.
/// When: the storage root and node log are reopened and the same encoded envelope
/// is appended again.
/// Then: the second `StorageRef` starts exactly after the first encoded byte range
/// and the durable file keeps both encoded units in append order.
#[test]
fn integration_reopened_storage_root_continues_append_offsets_without_overwrite() {
    let root_path = unique_temp_path("reopen_continues_offsets");
    let root = create_storage_root(root_path.clone(), manifest()).unwrap();
    let mut first_log = open_append_only_node_record_log(&root).unwrap();
    let (node_envelope, encoded_node) = encoded_node_record();

    let first_ref =
        append_encoded_node_record_envelope(&mut first_log, &node_envelope, &encoded_node).unwrap();
    flush_append_only_record_log(&mut first_log).unwrap();

    let reopened_root = open_storage_root(root_path.clone()).unwrap();
    let mut reopened_log = open_append_only_node_record_log(&reopened_root).unwrap();
    let second_ref =
        append_encoded_node_record_envelope(&mut reopened_log, &node_envelope, &encoded_node)
            .unwrap();
    flush_append_only_record_log(&mut reopened_log).unwrap();

    assert_eq!(first_ref.offset, 0);
    assert_eq!(first_ref.length, encoded_node.bytes.len() as u64);
    assert_eq!(second_ref.offset, first_ref.length);
    assert_eq!(second_ref.length, encoded_node.bytes.len() as u64);

    let mut expected_bytes = encoded_node.bytes.clone();
    expected_bytes.extend_from_slice(&encoded_node.bytes);
    assert_eq!(fs::read(&reopened_log.path).unwrap(), expected_bytes);
    let _ = fs::remove_dir_all(root_path);
}

/// Validate that typed append APIs protect segment boundaries at the
/// public integration boundary.
///
/// Given: an opened relationship append-only log and an encoded node envelope.
/// When: the node envelope is passed to the relationship append API.
/// Then: the operation fails with a typed record-kind error and the relationship
/// log remains empty.
#[test]
fn acceptance_relationship_append_rejects_node_envelope_without_writing_bytes() {
    let root_path = unique_temp_path("reject_node_in_relationship_log");
    let root = create_storage_root(root_path.clone(), manifest()).unwrap();
    let mut relationship_log = open_append_only_relationship_record_log(&root).unwrap();
    let (node_envelope, encoded_node) = encoded_node_record();

    let error = append_encoded_relationship_record_envelope(
        &mut relationship_log,
        &node_envelope,
        &encoded_node,
    )
    .unwrap_err();

    assert!(matches!(
        error,
        GraphStorageError::UnexpectedRecordKind { .. }
    ));
    assert_eq!(fs::read(&relationship_log.path).unwrap(), Vec::<u8>::new());
    let _ = fs::remove_dir_all(root_path);
}

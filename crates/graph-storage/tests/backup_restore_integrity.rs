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

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use graph_core::{
    AdjacencyDirection, Graph, NodeId, NodeInput, PropertyValue, RelationshipId, RelationshipInput,
    RelationshipType,
};
use graph_storage::{
    AtomicPersistentMutationAdjacencyRecord, AtomicPersistentMutationBatch,
    AtomicPersistentMutationNodeRecord, AtomicPersistentMutationRelationshipRecord,
    AtomicPersistentRuntimeState, DurableTransactionId, GraphId, GraphStorageError, RecordCodec,
    RecordFormat, StorageManifest, StorageSegment, StorageTimestamp, StorageVersion,
    apply_atomic_persistent_mutation_batch, create_atomic_persistent_backup,
    create_node_record_envelope, create_relationship_record_envelope, create_storage_root,
    recover_atomic_persistent_runtime_state, restore_atomic_persistent_backup,
    validate_atomic_persistent_backup,
};

fn unique_temp_path(test_name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "intelligence_graph_engine_issue_392_{test_name}_{}_{}",
        std::process::id(),
        unique
    ))
}

fn manifest() -> StorageManifest {
    StorageManifest {
        storage_version: StorageVersion::V1,
        graph_id: GraphId {
            value: "graph--issue-392".to_owned(),
        },
        created_at: StorageTimestamp {
            value: "2026-07-20T00:00:00Z".to_owned(),
        },
        updated_at: StorageTimestamp {
            value: "2026-07-20T00:00:00Z".to_owned(),
        },
        record_format: RecordFormat::JsonLinesV1,
    }
}

fn storage_root(test_name: &str) -> graph_storage::StorageRoot {
    let path = unique_temp_path(test_name);
    let _ = fs::remove_dir_all(&path);
    create_storage_root(path, manifest()).unwrap()
}

fn fixture_graph() -> (Graph, NodeId, NodeId, RelationshipId) {
    let mut graph = Graph::new();
    let source = graph
        .create_node(
            NodeInput::new(["Campaign", "FIMI"])
                .with_property("name", PropertyValue::String("campaign-alpha".to_owned())),
        )
        .unwrap();
    let target = graph
        .create_node(NodeInput::new(["Infrastructure"]))
        .unwrap();
    let relationship = graph
        .create_relationship(
            RelationshipInput::new(source.clone(), "PROMOTES", target.clone())
                .unwrap()
                .with_property("confidence", PropertyValue::Integer(97)),
        )
        .unwrap();
    (graph, source, target, relationship)
}

fn mutation_batch(
    transaction_id: &str,
) -> (
    AtomicPersistentMutationBatch,
    NodeId,
    NodeId,
    RelationshipId,
) {
    let (graph, source_id, target_id, relationship_id) = fixture_graph();
    let source = graph.get_node(&source_id).unwrap().unwrap();
    let target = graph.get_node(&target_id).unwrap().unwrap();
    let relationship = graph.get_relationship(&relationship_id).unwrap().unwrap();

    let source_envelope = create_node_record_envelope(
        &source,
        graph_storage::StorageRef {
            segment: StorageSegment::NodeRecords,
            offset: 0,
            length: 1,
            checksum: None,
        },
        StorageVersion::V1,
        RecordFormat::JsonLinesV1,
        None,
    )
    .unwrap();
    let target_envelope = create_node_record_envelope(
        &target,
        graph_storage::StorageRef {
            segment: StorageSegment::NodeRecords,
            offset: 0,
            length: 1,
            checksum: None,
        },
        StorageVersion::V1,
        RecordFormat::JsonLinesV1,
        None,
    )
    .unwrap();
    let relationship_envelope = create_relationship_record_envelope(
        &relationship,
        graph_storage::StorageRef {
            segment: StorageSegment::RelationshipRecords,
            offset: 0,
            length: 1,
            checksum: None,
        },
        StorageVersion::V1,
        RecordFormat::JsonLinesV1,
        None,
    )
    .unwrap();

    let source_encoded = graph_storage::JsonLinesRecordCodec
        .encode_envelope(&source_envelope)
        .unwrap();
    let target_encoded = graph_storage::JsonLinesRecordCodec
        .encode_envelope(&target_envelope)
        .unwrap();
    let relationship_encoded = graph_storage::JsonLinesRecordCodec
        .encode_envelope(&relationship_envelope)
        .unwrap();

    (
        AtomicPersistentMutationBatch {
            transaction_id: DurableTransactionId::new(transaction_id).unwrap(),
            node_records: vec![
                AtomicPersistentMutationNodeRecord {
                    envelope: source_envelope,
                    encoded_record: source_encoded,
                    labels: vec!["Campaign".to_owned(), "FIMI".to_owned()],
                    read_index: Default::default(),
                },
                AtomicPersistentMutationNodeRecord {
                    envelope: target_envelope,
                    encoded_record: target_encoded,
                    labels: vec!["Infrastructure".to_owned()],
                    read_index: Default::default(),
                },
            ],
            relationship_records: vec![AtomicPersistentMutationRelationshipRecord {
                envelope: relationship_envelope,
                encoded_record: relationship_encoded,
                relationship_type: RelationshipType::new("PROMOTES").unwrap(),
            }],
            outgoing_adjacency: vec![AtomicPersistentMutationAdjacencyRecord {
                owner_node_id: source_id.clone(),
                direction: AdjacencyDirection::Outgoing,
                entries: Vec::new(),
            }],
            incoming_adjacency: vec![AtomicPersistentMutationAdjacencyRecord {
                owner_node_id: target_id.clone(),
                direction: AdjacencyDirection::Incoming,
                entries: Vec::new(),
            }],
            audit_events: vec!["backup fixture mutation".to_owned()],
        },
        source_id,
        target_id,
        relationship_id,
    )
}

#[test]
fn backup_restore_roundtrip_is_semantically_equivalent_to_source_checkpoint() {
    let source_root = storage_root("backup_restore_roundtrip_source");
    let mut state = AtomicPersistentRuntimeState::default();
    let (batch, _, _, _) = mutation_batch("tx--392-roundtrip");
    apply_atomic_persistent_mutation_batch(&source_root, &mut state, batch, None)
        .expect("source mutation should commit");

    let backup_root = unique_temp_path("backup_restore_roundtrip_backup");
    let target_root = unique_temp_path("backup_restore_roundtrip_target");

    create_atomic_persistent_backup(&source_root, backup_root.clone())
        .expect("backup should be created");
    validate_atomic_persistent_backup(backup_root.clone())
        .expect("backup should pass integrity validation");
    restore_atomic_persistent_backup(backup_root.clone(), target_root.clone())
        .expect("backup should restore into target root");

    let source_recovery = recover_atomic_persistent_runtime_state(&source_root)
        .expect("source recovery should succeed");
    let restored_root =
        graph_storage::open_storage_root(target_root.clone()).expect("restored root should open");
    let restored_recovery = recover_atomic_persistent_runtime_state(&restored_root)
        .expect("restored recovery should succeed");
    assert_eq!(source_recovery, restored_recovery);

    let _ = fs::remove_dir_all(source_root.path());
    let _ = fs::remove_dir_all(backup_root);
    let _ = fs::remove_dir_all(target_root);
}

#[test]
fn backup_validation_reports_corruption_explicitly() {
    let source_root = storage_root("backup_corruption_source");
    let mut state = AtomicPersistentRuntimeState::default();
    let (batch, _, _, _) = mutation_batch("tx--392-corruption");
    apply_atomic_persistent_mutation_batch(&source_root, &mut state, batch, None)
        .expect("source mutation should commit");

    let backup_root = unique_temp_path("backup_corruption_backup");
    create_atomic_persistent_backup(&source_root, backup_root.clone())
        .expect("backup should be created");

    let manifest_path = backup_root.join("manifest.json");
    fs::write(&manifest_path, b"{invalid-manifest").expect("corruption fixture should be written");

    let error = validate_atomic_persistent_backup(backup_root.clone())
        .expect_err("corrupted backup should fail explicit validation");
    assert!(matches!(
        error,
        GraphStorageError::ManifestCorrupted { .. } | GraphStorageError::OperationFailed { .. }
    ));

    let _ = fs::remove_dir_all(source_root.path());
    let _ = fs::remove_dir_all(backup_root);
}

#[test]
fn restore_requires_empty_target_root() {
    let source_root = storage_root("restore_requires_empty_target_source");
    let mut state = AtomicPersistentRuntimeState::default();
    let (batch, _, _, _) = mutation_batch("tx--392-empty-target");
    apply_atomic_persistent_mutation_batch(&source_root, &mut state, batch, None)
        .expect("source mutation should commit");

    let backup_root = unique_temp_path("restore_requires_empty_target_backup");
    create_atomic_persistent_backup(&source_root, backup_root.clone())
        .expect("backup should be created");

    let target_root = unique_temp_path("restore_requires_empty_target_target");
    fs::create_dir_all(&target_root).expect("target root should be creatable");
    fs::write(target_root.join("already-there.txt"), b"non-empty")
        .expect("target root fixture should be written");

    let error = restore_atomic_persistent_backup(backup_root.clone(), target_root.clone())
        .expect_err("restore should reject non-empty target root");
    assert!(matches!(
        error,
        GraphStorageError::StorageRootAlreadyExists { .. }
            | GraphStorageError::OperationFailed { .. }
    ));

    let _ = fs::remove_dir_all(source_root.path());
    let _ = fs::remove_dir_all(backup_root);
    let _ = fs::remove_dir_all(target_root);
}

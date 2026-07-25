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
    AdjacencyStorageLookupMode, AtomicPersistentCompactionRequest, AtomicPersistentCompactionScope,
    AtomicPersistentMutationAdjacencyRecord, AtomicPersistentMutationBatch,
    AtomicPersistentMutationNodeRecord, AtomicPersistentMutationRelationshipRecord,
    AtomicPersistentRecoveryPath, AtomicPersistentRuntimeState, DurableTransactionId, GraphId,
    GraphStorageError, MutationCrashStage, PersistedAdjacencyEntry, RecordCodec, RecordFormat,
    StorageManifest, StorageSegment, StorageTimestamp, StorageVersion, WalSequenceNumber,
    apply_atomic_persistent_mutation_batch, compact_atomic_persistent_segments,
    create_node_record_envelope, create_relationship_record_envelope, create_storage_root,
    recover_atomic_persistent_runtime_state, recover_atomic_persistent_runtime_state_with_report,
    resolve_latest_node_storage_ref, resolve_latest_relationship_storage_ref,
    resolve_outgoing_adjacency_storage_ref,
};

fn unique_temp_path(test_name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "intelligence_graph_engine_issue_388_{test_name}_{}_{}",
        std::process::id(),
        unique
    ))
}

fn manifest() -> StorageManifest {
    StorageManifest {
        storage_version: StorageVersion::V1,
        graph_id: GraphId {
            value: "graph--issue-388".to_owned(),
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
                active: true,
                access: Default::default(),
            }],
            outgoing_adjacency: vec![AtomicPersistentMutationAdjacencyRecord {
                owner_node_id: source_id.clone(),
                direction: AdjacencyDirection::Outgoing,
                entries: vec![PersistedAdjacencyEntry {
                    relationship_id: relationship_id.clone(),
                    source_node_id: source_id.clone(),
                    target_node_id: target_id.clone(),
                    relationship_type: RelationshipType::new("PROMOTES").unwrap(),
                    direction: AdjacencyDirection::Outgoing,
                    relationship_storage_ref: None,
                    source_node_storage_ref: None,
                    target_node_storage_ref: None,
                }],
            }],
            incoming_adjacency: vec![AtomicPersistentMutationAdjacencyRecord {
                owner_node_id: target_id.clone(),
                direction: AdjacencyDirection::Incoming,
                entries: vec![PersistedAdjacencyEntry {
                    relationship_id: relationship_id.clone(),
                    source_node_id: source_id.clone(),
                    target_node_id: target_id.clone(),
                    relationship_type: RelationshipType::new("PROMOTES").unwrap(),
                    direction: AdjacencyDirection::Incoming,
                    relationship_storage_ref: None,
                    source_node_storage_ref: None,
                    target_node_storage_ref: None,
                }],
            }],
            audit_events: vec!["created campaign promotion relation".to_owned()],
        },
        source_id,
        target_id,
        relationship_id,
    )
}

fn adjacency_only_batch(
    transaction_id: &str,
    source_id: &NodeId,
    target_id: &NodeId,
) -> AtomicPersistentMutationBatch {
    AtomicPersistentMutationBatch {
        transaction_id: DurableTransactionId::new(transaction_id).unwrap(),
        node_records: Vec::new(),
        relationship_records: Vec::new(),
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
        audit_events: vec!["adjacency-only mutation".to_owned()],
    }
}

#[test]
fn committed_atomic_mutation_is_fully_visible_after_recovery() {
    let root = storage_root("commit_visibility");
    let mut state = AtomicPersistentRuntimeState::default();
    let (batch, source_id, _target_id, relationship_id) = mutation_batch("tx--388-commit");

    apply_atomic_persistent_mutation_batch(&root, &mut state, batch, None)
        .expect("committed mutation should persist and apply atomically");

    let recovered = recover_atomic_persistent_runtime_state(&root)
        .expect("recovery should reconstruct committed mutation state");

    assert!(resolve_latest_node_storage_ref(&recovered.catalog, &source_id).is_ok());
    assert!(resolve_latest_relationship_storage_ref(&recovered.catalog, &relationship_id).is_ok());
    assert!(
        resolve_outgoing_adjacency_storage_ref(
            &recovered.catalog,
            &source_id,
            AdjacencyStorageLookupMode::Strict
        )
        .is_ok()
    );

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn incomplete_atomic_mutation_is_ignored_after_recovery() {
    let root = storage_root("incomplete_ignored");
    let mut state = AtomicPersistentRuntimeState::default();
    let (batch, source_id, _target_id, relationship_id) = mutation_batch("tx--388-incomplete");

    let error = apply_atomic_persistent_mutation_batch(
        &root,
        &mut state,
        batch,
        Some(MutationCrashStage::BeforeAppliedMarker),
    )
    .expect_err("injected crash should fail before applied marker");
    assert!(matches!(
        error,
        GraphStorageError::OperationFailed { operation, .. }
        if operation == "apply_atomic_persistent_mutation_batch"
    ));

    let recovered = recover_atomic_persistent_runtime_state(&root)
        .expect("recovery should ignore incomplete transaction");

    assert!(resolve_latest_node_storage_ref(&recovered.catalog, &source_id).is_err());
    assert!(resolve_latest_relationship_storage_ref(&recovered.catalog, &relationship_id).is_err());

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn replaying_same_transaction_id_is_observably_exactly_once() {
    let root = storage_root("exactly_once_replay");
    let mut state = AtomicPersistentRuntimeState::default();
    let (batch, source_id, _target_id, relationship_id) = mutation_batch("tx--388-idempotent");

    let first = apply_atomic_persistent_mutation_batch(&root, &mut state, batch.clone(), None)
        .expect("first commit should succeed");
    assert!(first.applied);

    let second = apply_atomic_persistent_mutation_batch(&root, &mut state, batch, None)
        .expect("duplicate replay should be accepted as idempotent");
    assert!(!second.applied);

    let recovered = recover_atomic_persistent_runtime_state(&root)
        .expect("recovery should keep observable state exactly once");

    assert!(resolve_latest_node_storage_ref(&recovered.catalog, &source_id).is_ok());
    assert!(resolve_latest_relationship_storage_ref(&recovered.catalog, &relationship_id).is_ok());

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn recovery_uses_checkpoint_and_replays_only_newer_transactions() {
    let root = storage_root("checkpoint_bounded_replay");
    let mut state = AtomicPersistentRuntimeState::default();
    let (batch_one, source_id, target_id, relationship_id) =
        mutation_batch("tx--390-checkpoint-base");
    let batch_two = adjacency_only_batch("tx--390-checkpoint-replay", &source_id, &target_id);

    apply_atomic_persistent_mutation_batch(&root, &mut state, batch_one, None)
        .expect("first transaction should commit and checkpoint");

    let error = apply_atomic_persistent_mutation_batch(
        &root,
        &mut state,
        batch_two,
        Some(MutationCrashStage::BeforeCheckpointWrite),
    )
    .expect_err("second transaction should fail after durable applied marker");
    assert!(matches!(
        error,
        GraphStorageError::OperationFailed { operation, .. }
        if operation == "apply_atomic_persistent_mutation_batch"
    ));

    let recovered = recover_atomic_persistent_runtime_state_with_report(&root)
        .expect("recovery should load checkpoint and replay newer committed transaction");
    assert_eq!(
        recovered.report.recovery_path,
        AtomicPersistentRecoveryPath::CheckpointAndBoundedReplay
    );
    assert_eq!(recovered.report.replayed_transaction_count, 1);
    assert!(recovered.report.checkpoint_sequence_number.is_some());
    assert!(resolve_latest_node_storage_ref(&recovered.state.catalog, &source_id).is_ok());
    assert!(
        resolve_latest_relationship_storage_ref(&recovered.state.catalog, &relationship_id).is_ok()
    );

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn recovery_falls_back_to_full_replay_with_diagnostics_when_checkpoint_is_corrupted() {
    let root = storage_root("checkpoint_corruption_fallback");
    let mut state = AtomicPersistentRuntimeState::default();
    let (batch, source_id, _target_id, relationship_id) = mutation_batch("tx--390-corrupt");
    apply_atomic_persistent_mutation_batch(&root, &mut state, batch, None)
        .expect("transaction should commit and create checkpoint");

    let checkpoints = root.path().join("transactions").join("checkpoints");
    let mut checkpoint_files: Vec<_> = fs::read_dir(&checkpoints)
        .expect("checkpoint directory should exist")
        .map(|entry| entry.expect("checkpoint entry should be readable").path())
        .collect();
    checkpoint_files.sort();
    let latest = checkpoint_files
        .last()
        .expect("at least one checkpoint file should exist");
    fs::write(latest, b"{ not-valid-json")
        .expect("checkpoint corruption fixture should be written");

    let recovered = recover_atomic_persistent_runtime_state_with_report(&root)
        .expect("fallback full replay should recover from committed logs");
    assert_eq!(
        recovered.report.recovery_path,
        AtomicPersistentRecoveryPath::FullReplay
    );
    assert!(!recovered.report.warnings.is_empty());
    assert!(
        recovered.report.warnings[0].contains("ignored"),
        "expected corruption diagnostic warning"
    );
    assert!(resolve_latest_node_storage_ref(&recovered.state.catalog, &source_id).is_ok());
    assert!(
        resolve_latest_relationship_storage_ref(&recovered.state.catalog, &relationship_id).is_ok()
    );

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn checkpoint_recovery_state_matches_full_replay_state() {
    let root = storage_root("checkpoint_full_replay_equivalence");
    let mut state = AtomicPersistentRuntimeState::default();
    let (batch_one, source_id, target_id, _) = mutation_batch("tx--390-equivalence-1");
    let batch_two = adjacency_only_batch("tx--390-equivalence-2", &source_id, &target_id);
    apply_atomic_persistent_mutation_batch(&root, &mut state, batch_one, None)
        .expect("first transaction should commit");
    apply_atomic_persistent_mutation_batch(&root, &mut state, batch_two, None)
        .expect("second transaction should commit");

    let checkpoint_recovery = recover_atomic_persistent_runtime_state_with_report(&root)
        .expect("checkpoint recovery should succeed");
    assert_eq!(
        checkpoint_recovery.report.recovery_path,
        AtomicPersistentRecoveryPath::CheckpointAndBoundedReplay
    );

    let checkpoints = root.path().join("transactions").join("checkpoints");
    fs::remove_dir_all(&checkpoints).expect("checkpoint directory should be removable");

    let full_replay_recovery = recover_atomic_persistent_runtime_state_with_report(&root)
        .expect("full replay recovery should succeed");
    assert_eq!(
        full_replay_recovery.report.recovery_path,
        AtomicPersistentRecoveryPath::FullReplay
    );
    assert_eq!(
        checkpoint_recovery.state.catalog.latest_node_records.len(),
        full_replay_recovery.state.catalog.latest_node_records.len()
    );
    assert_eq!(
        checkpoint_recovery
            .state
            .catalog
            .latest_relationship_records
            .len(),
        full_replay_recovery
            .state
            .catalog
            .latest_relationship_records
            .len()
    );
    assert_eq!(
        checkpoint_recovery.state.catalog.metadata_indexes.labels,
        full_replay_recovery.state.catalog.metadata_indexes.labels
    );
    assert_eq!(
        checkpoint_recovery
            .state
            .catalog
            .metadata_indexes
            .relationship_types,
        full_replay_recovery
            .state
            .catalog
            .metadata_indexes
            .relationship_types
    );

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn compaction_reclaims_obsolete_storage_without_recovery_data_loss() {
    let root = storage_root("compaction_reclaims_obsolete_storage");
    let mut state = AtomicPersistentRuntimeState::default();
    let (batch_one, source_id, target_id, relationship_id) = mutation_batch("tx--391-compact-1");
    let batch_two = adjacency_only_batch("tx--391-compact-2", &source_id, &target_id);

    apply_atomic_persistent_mutation_batch(&root, &mut state, batch_one, None)
        .expect("first transaction should commit");
    apply_atomic_persistent_mutation_batch(&root, &mut state, batch_two, None)
        .expect("second transaction should commit");

    let before = recover_atomic_persistent_runtime_state_with_report(&root)
        .expect("recovery before compaction should succeed");
    let outcome = compact_atomic_persistent_segments(
        &root,
        AtomicPersistentCompactionRequest {
            scope: AtomicPersistentCompactionScope::TransactionsAndIndexes,
            snapshot_protected_sequences: Vec::new(),
            retention_protected_sequences: Vec::new(),
        },
    )
    .expect("compaction should succeed with safe checkpoint available");
    assert!(outcome.reclaimed_bytes > 0);

    let after = recover_atomic_persistent_runtime_state_with_report(&root)
        .expect("recovery after compaction should succeed");
    assert_eq!(before.state, after.state);
    assert!(resolve_latest_node_storage_ref(&after.state.catalog, &source_id).is_ok());
    assert!(
        resolve_latest_relationship_storage_ref(&after.state.catalog, &relationship_id).is_ok()
    );

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn compaction_preserves_snapshot_and_retention_protected_sequences() {
    let root = storage_root("compaction_preserves_protected_sequences");
    let mut state = AtomicPersistentRuntimeState::default();
    let (batch_one, source_id, target_id, _) = mutation_batch("tx--391-protected-1");
    let batch_two = adjacency_only_batch("tx--391-protected-2", &source_id, &target_id);

    let first = apply_atomic_persistent_mutation_batch(&root, &mut state, batch_one, None)
        .expect("first transaction should commit");
    apply_atomic_persistent_mutation_batch(&root, &mut state, batch_two, None)
        .expect("second transaction should commit");

    let protected_snapshot = first
        .mutation_sequence_number
        .expect("applied transaction should expose mutation sequence");
    let protected_retention =
        WalSequenceNumber::new(protected_snapshot.0 + 1000).expect("fixture sequence is valid");

    let outcome = compact_atomic_persistent_segments(
        &root,
        AtomicPersistentCompactionRequest {
            scope: AtomicPersistentCompactionScope::TransactionsAndIndexes,
            snapshot_protected_sequences: vec![protected_snapshot],
            retention_protected_sequences: vec![protected_retention],
        },
    )
    .expect("compaction should honor protected sequence inputs");

    assert!(
        outcome
            .retained_protected_sequences
            .contains(&protected_snapshot)
    );
    assert!(
        !outcome
            .retained_protected_sequences
            .contains(&protected_retention)
    );

    let _ = fs::remove_dir_all(root.path());
}

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
    AdjacencyDirection, Graph, NodeInput, PropertyValue, RelationshipInput, RelationshipType,
};
use graph_storage::{
    AtomicPersistentMutationAdjacencyRecord, AtomicPersistentMutationBatch,
    AtomicPersistentMutationNodeRecord, AtomicPersistentMutationRelationshipRecord,
    AtomicPersistentRecoveryPath, AtomicPersistentRuntimeState, CatalogRebuildOptions,
    DurableTransactionId, GraphId, GraphStoreOpenMode, GraphStoreOpenOptions, MutationCrashStage,
    PersistedAdjacencyEntry, RecordCodec, RecordFormat, StorageManifest, StorageSegment,
    StorageTimestamp, StorageVersion, apply_atomic_persistent_mutation_batch,
    create_node_record_envelope, create_relationship_record_envelope, create_storage_root,
    open_existing_file_backed_graph_store, recover_atomic_persistent_runtime_state,
    recover_atomic_persistent_runtime_state_with_report,
};

fn unique_temp_path(test_name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "corrobore_issue_394_matrix_{test_name}_{}_{}",
        std::process::id(),
        unique
    ))
}

fn manifest() -> StorageManifest {
    StorageManifest {
        storage_version: StorageVersion::V1,
        graph_id: GraphId {
            value: "graph--issue-394-matrix".to_owned(),
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

fn mutation_batch(
    transaction_id: &str,
) -> (
    AtomicPersistentMutationBatch,
    graph_storage::StorageRoot,
    AtomicPersistentRuntimeState,
) {
    let root = storage_root(transaction_id);
    let mut graph = Graph::new();
    let source = graph
        .create_node(
            NodeInput::new(["Campaign"])
                .with_property("name", PropertyValue::String("campaign-alpha".to_owned())),
        )
        .unwrap();
    let target = graph
        .create_node(NodeInput::new(["Infrastructure"]))
        .unwrap();
    let relationship = graph
        .create_relationship(
            RelationshipInput::new(source.clone(), "PROMOTES", target.clone()).unwrap(),
        )
        .unwrap();

    let source_node = graph.get_node(&source).unwrap().unwrap();
    let target_node = graph.get_node(&target).unwrap().unwrap();
    let relationship_record = graph.get_relationship(&relationship).unwrap().unwrap();
    let source_envelope = create_node_record_envelope(
        &source_node,
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
        &target_node,
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
        &relationship_record,
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

    let batch = AtomicPersistentMutationBatch {
        transaction_id: DurableTransactionId::new(transaction_id).unwrap(),
        node_records: vec![
            AtomicPersistentMutationNodeRecord {
                envelope: source_envelope.clone(),
                encoded_record: graph_storage::JsonLinesRecordCodec
                    .encode_envelope(&source_envelope)
                    .unwrap(),
                labels: vec!["Campaign".to_owned()],
                read_index: Default::default(),
            },
            AtomicPersistentMutationNodeRecord {
                envelope: target_envelope.clone(),
                encoded_record: graph_storage::JsonLinesRecordCodec
                    .encode_envelope(&target_envelope)
                    .unwrap(),
                labels: vec!["Infrastructure".to_owned()],
                read_index: Default::default(),
            },
        ],
        relationship_records: vec![AtomicPersistentMutationRelationshipRecord {
            envelope: relationship_envelope.clone(),
            encoded_record: graph_storage::JsonLinesRecordCodec
                .encode_envelope(&relationship_envelope)
                .unwrap(),
            relationship_type: RelationshipType::new("PROMOTES").unwrap(),
        }],
        outgoing_adjacency: vec![AtomicPersistentMutationAdjacencyRecord {
            owner_node_id: source.clone(),
            direction: AdjacencyDirection::Outgoing,
            entries: vec![PersistedAdjacencyEntry {
                relationship_id: relationship.clone(),
                source_node_id: source.clone(),
                target_node_id: target.clone(),
                relationship_type: RelationshipType::new("PROMOTES").unwrap(),
                direction: AdjacencyDirection::Outgoing,
                relationship_storage_ref: None,
                source_node_storage_ref: None,
                target_node_storage_ref: None,
            }],
        }],
        incoming_adjacency: vec![AtomicPersistentMutationAdjacencyRecord {
            owner_node_id: target,
            direction: AdjacencyDirection::Incoming,
            entries: vec![PersistedAdjacencyEntry {
                relationship_id: relationship,
                source_node_id: source,
                target_node_id: target_node.id().clone(),
                relationship_type: RelationshipType::new("PROMOTES").unwrap(),
                direction: AdjacencyDirection::Incoming,
                relationship_storage_ref: None,
                source_node_storage_ref: None,
                target_node_storage_ref: None,
            }],
        }],
        audit_events: vec!["acceptance matrix mutation".to_owned()],
    };
    (batch, root, AtomicPersistentRuntimeState::default())
}

#[test]
fn persistent_restart_reconstructs_equivalent_runtime_state() {
    let (batch, root, mut state) = mutation_batch("tx--394-restart");
    apply_atomic_persistent_mutation_batch(&root, &mut state, batch, None)
        .expect("mutation should commit in persistent mode");

    let recovered =
        recover_atomic_persistent_runtime_state(&root).expect("recovery should reconstruct state");
    assert_eq!(recovered, state);

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn deterministic_crash_stages_preserve_recovery_contract() {
    let (batch, root, mut state) = mutation_batch("tx--394-crash-before-applied");
    let err = apply_atomic_persistent_mutation_batch(
        &root,
        &mut state,
        batch,
        Some(MutationCrashStage::BeforeAppliedMarker),
    )
    .expect_err("before-applied crash should fail deterministically");
    assert!(
        err.to_string()
            .contains("injected crash before applied mutation marker")
    );
    let recovered = recover_atomic_persistent_runtime_state_with_report(&root)
        .expect("recovery should ignore incomplete mutation");
    assert_eq!(recovered.report.replayed_transaction_count, 0);
    assert_eq!(recovered.state, AtomicPersistentRuntimeState::default());
    let _ = fs::remove_dir_all(root.path());

    let (batch, root, mut state) = mutation_batch("tx--394-crash-before-checkpoint");
    let err = apply_atomic_persistent_mutation_batch(
        &root,
        &mut state,
        batch,
        Some(MutationCrashStage::BeforeCheckpointWrite),
    )
    .expect_err("before-checkpoint crash should fail deterministically");
    assert!(
        err.to_string()
            .contains("injected crash before checkpoint write")
    );
    let recovered = recover_atomic_persistent_runtime_state_with_report(&root)
        .expect("recovery should replay committed+applied mutation");
    assert_eq!(
        recovered.report.recovery_path,
        AtomicPersistentRecoveryPath::FullReplay
    );
    assert_eq!(recovered.report.replayed_transaction_count, 1);
    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn corrupted_checkpoint_falls_back_to_full_replay_with_warning() {
    let (batch, root, mut state) = mutation_batch("tx--394-corruption");
    apply_atomic_persistent_mutation_batch(&root, &mut state, batch, None)
        .expect("mutation should materialize a checkpoint");

    let checkpoints_dir = root.path().join("transactions").join("checkpoints");
    let checkpoint = fs::read_dir(&checkpoints_dir)
        .expect("checkpoint directory should exist")
        .flatten()
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.starts_with("checkpoint-") && name.ends_with(".json"))
                .unwrap_or(false)
        })
        .expect("checkpoint should exist");
    fs::write(&checkpoint, b"{\"corrupted\":true").expect("corrupted checkpoint should write");

    let recovered = recover_atomic_persistent_runtime_state_with_report(&root)
        .expect("recovery should fallback to full replay");
    assert_eq!(
        recovered.report.recovery_path,
        AtomicPersistentRecoveryPath::FullReplay
    );
    assert!(!recovered.report.warnings.is_empty());

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn missing_catalog_metadata_triggers_derived_state_rebuild_in_load_mode() {
    let (batch, root, mut state) = mutation_batch("tx--394-derived-rebuild");
    apply_atomic_persistent_mutation_batch(&root, &mut state, batch, None)
        .expect("mutation should produce persisted graph logs");

    let _ = fs::remove_file(root.path().join("catalog").join("catalog_metadata.json"));
    let outcome = open_existing_file_backed_graph_store(
        root.path().to_path_buf(),
        GraphStoreOpenOptions {
            mode: GraphStoreOpenMode::LoadCatalogWhenAvailable,
            catalog_rebuild_options: CatalogRebuildOptions {
                include_node_records: true,
                include_relationship_records: false,
                include_outgoing_adjacency: false,
                include_incoming_adjacency: false,
                fail_fast: true,
            },
            require_node_record_log: true,
            require_relationship_record_log: false,
            require_outgoing_adjacency_log: false,
            require_incoming_adjacency_log: false,
        },
    )
    .expect("load mode should rebuild when persisted metadata is missing");

    assert!(outcome.recovery_report.catalog_rebuild_report.is_some());
    assert!(outcome.recovery_report.catalog_recovered);
    assert!(
        outcome
            .recovery_report
            .warnings
            .iter()
            .any(|warning| warning.contains("rebuilt catalog from append logs"))
    );

    let _ = fs::remove_dir_all(root.path());
}

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

use graph_core::{AdjacencyDirection, Graph, NodeInput, RelationshipInput, RelationshipType};
use graph_storage::{
    AdjacencyStorageLookupMode, CatalogIndexLookupMode, CatalogRebuildOptions, GraphId,
    GraphStorageError, PersistedAdjacencyEntry, PersistedAdjacencyRecord, RecordChecksum,
    RecordFormat, StorageManifest, StorageRef, StorageSegment, StorageTimestamp, StorageVersion,
    create_node_record_envelope, create_relationship_record_envelope, create_storage_root,
    detect_corrupted_catalog_rebuild_records, read_relationship_record_log_for_catalog_rebuild,
    rebuild_catalog_from_append_logs, reconstruct_catalog_from_rebuild_records,
    resolve_incoming_adjacency_storage_ref, resolve_latest_node_storage_ref,
    resolve_latest_relationship_storage_ref, resolve_node_ids_by_label,
    resolve_outgoing_adjacency_storage_ref, resolve_relationship_ids_by_type,
};
use serde::Serialize;
use serde_json::json;

#[derive(Serialize)]
struct NodeRebuildLogRecord {
    envelope: graph_storage::PersistedRecordEnvelope,
    storage_ref: StorageRef,
    labels: Vec<String>,
}

#[derive(Serialize)]
struct RelationshipRebuildLogRecord {
    envelope: graph_storage::PersistedRecordEnvelope,
    storage_ref: StorageRef,
    relationship_type: RelationshipType,
}

#[derive(Serialize)]
struct AdjacencyRebuildLogRecord {
    record: PersistedAdjacencyRecord,
    storage_ref: StorageRef,
}

fn unique_temp_path(test_name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be valid")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "intelligence_graph_engine_issue_58_{test_name}_{}_{}",
        std::process::id(),
        unique
    ))
}

fn manifest() -> StorageManifest {
    StorageManifest {
        storage_version: StorageVersion::V1,
        graph_id: GraphId {
            value: "graph--issue-58".to_owned(),
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

fn checksum(value: impl Into<String>) -> RecordChecksum {
    RecordChecksum {
        algorithm: "sha256".to_owned(),
        value: value.into(),
    }
}

fn storage_ref(segment: StorageSegment, offset: u64) -> StorageRef {
    StorageRef {
        segment,
        offset,
        length: 128,
        checksum: Some(checksum(format!("issue-58-checksum-{offset}"))),
    }
}

fn relationship_type(value: &str) -> RelationshipType {
    RelationshipType::new(value).expect("relationship type should be valid")
}

fn write_json_line(path: PathBuf, value: &impl Serialize) {
    fs::create_dir_all(path.parent().expect("path should have parent"))
        .expect("log parent should be created");
    let mut existing = if path.exists() {
        fs::read(&path).expect("existing log should be readable")
    } else {
        Vec::new()
    };
    let mut bytes = serde_json::to_vec(value).expect("record should serialize");
    bytes.push(b'\n');
    existing.extend_from_slice(&bytes);
    fs::write(path, existing).expect("log should be written");
}

fn adjacency_entry(
    source_node_id: &graph_core::NodeId,
    target_node_id: &graph_core::NodeId,
    relationship_id: &graph_core::RelationshipId,
    relationship_type: RelationshipType,
    direction: AdjacencyDirection,
) -> PersistedAdjacencyEntry {
    PersistedAdjacencyEntry {
        relationship_id: relationship_id.clone(),
        source_node_id: source_node_id.clone(),
        target_node_id: target_node_id.clone(),
        relationship_type,
        direction,
        relationship_storage_ref: Some(storage_ref(StorageSegment::RelationshipRecords, 512)),
        source_node_storage_ref: Some(storage_ref(StorageSegment::NodeRecords, 640)),
        target_node_storage_ref: Some(storage_ref(StorageSegment::NodeRecords, 768)),
    }
}

//
// Validate the acceptance path through public APIs only. The catalog is
// recoverable metadata and can be rebuilt from persisted append-only logs without
// loading a complete graph payload.
//
// Given a storage root containing node, relationship, outgoing adjacency, and
// incoming adjacency rebuild logs with lightweight metadata,
// when `rebuild_catalog_from_append_logs` is called,
// then latest ID lookups, label lookups, relationship-type lookups, adjacency
// lookups, and diagnostic counters should all resolve from the rebuilt catalog.
#[test]
fn acceptance_rebuilds_catalog_from_append_logs_without_full_payload_loading() {
    let root_path = unique_temp_path("full_rebuild");
    let root = create_storage_root(root_path.clone(), manifest()).expect("root should be created");
    let mut graph = Graph::new();
    let source_id = graph
        .create_node(NodeInput::new(["Campaign", "FIMI"]))
        .expect("source node should be created");
    let target_id = graph
        .create_node(NodeInput::new(["Infrastructure"]))
        .expect("target node should be created");
    let relationship_id = graph
        .create_relationship(
            RelationshipInput::new(source_id.clone(), "PROMOTES", target_id.clone())
                .expect("relationship input should be valid"),
        )
        .expect("relationship should be created");
    let source = graph.get_node(&source_id).unwrap().unwrap();
    let target = graph.get_node(&target_id).unwrap().unwrap();
    let relationship = graph.get_relationship(&relationship_id).unwrap().unwrap();
    let source_node_ref = storage_ref(StorageSegment::NodeRecords, 0);
    let target_node_ref = storage_ref(StorageSegment::NodeRecords, 64);
    let relationship_ref = storage_ref(StorageSegment::RelationshipRecords, 128);
    let outgoing_ref = storage_ref(StorageSegment::OutgoingAdjacency, 256);
    let incoming_ref = storage_ref(StorageSegment::IncomingAdjacency, 384);
    let rel_type = relationship_type("PROMOTES");
    let source_node_envelope = create_node_record_envelope(
        &source,
        source_node_ref.clone(),
        StorageVersion::V1,
        RecordFormat::JsonLinesV1,
        Some(checksum("source-node-record")),
    )
    .expect("source node envelope should be valid");
    let target_node_envelope = create_node_record_envelope(
        &target,
        target_node_ref.clone(),
        StorageVersion::V1,
        RecordFormat::JsonLinesV1,
        Some(checksum("target-node-record")),
    )
    .expect("target node envelope should be valid");
    let relationship_envelope = create_relationship_record_envelope(
        &relationship,
        relationship_ref.clone(),
        StorageVersion::V1,
        RecordFormat::JsonLinesV1,
        Some(checksum("relationship-record")),
    )
    .expect("relationship envelope should be valid");

    let node_log_path = root.path().join("nodes").join("node_records.log");
    write_json_line(
        node_log_path.clone(),
        &NodeRebuildLogRecord {
            envelope: source_node_envelope,
            storage_ref: source_node_ref.clone(),
            labels: vec!["Campaign".to_owned(), "FIMI".to_owned()],
        },
    );
    write_json_line(
        node_log_path,
        &NodeRebuildLogRecord {
            envelope: target_node_envelope,
            storage_ref: target_node_ref,
            labels: Vec::new(),
        },
    );
    write_json_line(
        root.path()
            .join("relationships")
            .join("relationship_records.log"),
        &RelationshipRebuildLogRecord {
            envelope: relationship_envelope,
            storage_ref: relationship_ref.clone(),
            relationship_type: rel_type.clone(),
        },
    );
    write_json_line(
        root.path().join("adjacency").join("outgoing_adjacency.log"),
        &AdjacencyRebuildLogRecord {
            record: PersistedAdjacencyRecord {
                owner_node_id: source_id.clone(),
                direction: AdjacencyDirection::Outgoing,
                entries: vec![adjacency_entry(
                    &source_id,
                    &target_id,
                    &relationship_id,
                    rel_type.clone(),
                    AdjacencyDirection::Outgoing,
                )],
                storage_ref: Some(outgoing_ref.clone()),
            },
            storage_ref: outgoing_ref.clone(),
        },
    );
    write_json_line(
        root.path().join("adjacency").join("incoming_adjacency.log"),
        &AdjacencyRebuildLogRecord {
            record: PersistedAdjacencyRecord {
                owner_node_id: target_id.clone(),
                direction: AdjacencyDirection::Incoming,
                entries: vec![adjacency_entry(
                    &source_id,
                    &target_id,
                    &relationship_id,
                    rel_type.clone(),
                    AdjacencyDirection::Incoming,
                )],
                storage_ref: Some(incoming_ref.clone()),
            },
            storage_ref: incoming_ref.clone(),
        },
    );

    let outcome = rebuild_catalog_from_append_logs(&root, CatalogRebuildOptions::default())
        .expect("catalog should rebuild from append-only logs");

    assert_eq!(
        resolve_latest_node_storage_ref(&outcome.catalog, &source_id),
        Ok(source_node_ref)
    );
    assert_eq!(
        resolve_latest_relationship_storage_ref(&outcome.catalog, &relationship_id),
        Ok(relationship_ref)
    );
    assert_eq!(
        resolve_node_ids_by_label(&outcome.catalog, "Campaign", CatalogIndexLookupMode::Strict),
        Ok(vec![source_id.clone()])
    );
    assert_eq!(
        resolve_relationship_ids_by_type(
            &outcome.catalog,
            &rel_type,
            CatalogIndexLookupMode::Strict,
        ),
        Ok(vec![relationship_id])
    );
    assert_eq!(
        resolve_outgoing_adjacency_storage_ref(
            &outcome.catalog,
            &source_id,
            AdjacencyStorageLookupMode::Strict,
        ),
        Ok(Some(outgoing_ref))
    );
    assert_eq!(
        resolve_incoming_adjacency_storage_ref(
            &outcome.catalog,
            &target_id,
            AdjacencyStorageLookupMode::Strict,
        ),
        Ok(Some(incoming_ref))
    );
    assert_eq!(outcome.report.records_read.node_records, 2);
    assert_eq!(outcome.report.records_read.relationship_records, 1);
    assert_eq!(outcome.report.records_read.outgoing_adjacency_records, 1);
    assert_eq!(outcome.report.records_read.incoming_adjacency_records, 1);
    assert_eq!(outcome.report.label_index_entries_reconstructed, 2);
    assert_eq!(
        outcome.report.relationship_type_index_entries_reconstructed,
        1
    );
    assert_eq!(outcome.report.adjacency_catalog_entries_reconstructed, 2);

    let _ = fs::remove_dir_all(root_path);
}

#[test]
fn relationship_rebuild_log_requires_relationship_type_metadata() {
    let root_path = unique_temp_path("missing_relationship_type_metadata");
    let root = create_storage_root(root_path.clone(), manifest()).expect("root should be created");
    let mut graph = Graph::new();
    let source_id = graph
        .create_node(NodeInput::new(["Campaign"]))
        .expect("source node should be created");
    let target_id = graph
        .create_node(NodeInput::new(["Narrative"]))
        .expect("target node should be created");
    let relationship_id = graph
        .create_relationship(
            RelationshipInput::new(source_id.clone(), "PROMOTES", target_id.clone())
                .expect("relationship input should be valid"),
        )
        .expect("relationship should be created");
    let relationship = graph
        .get_relationship(&relationship_id)
        .expect("relationship read should succeed")
        .expect("relationship should exist");
    let relationship_ref = storage_ref(StorageSegment::RelationshipRecords, 128);
    let relationship_envelope = create_relationship_record_envelope(
        &relationship,
        relationship_ref.clone(),
        StorageVersion::V1,
        RecordFormat::JsonLinesV1,
        Some(checksum("relationship-record")),
    )
    .expect("relationship envelope should be valid");

    write_json_line(
        root.path()
            .join("relationships")
            .join("relationship_records.log"),
        &json!({
        "envelope": relationship_envelope,
        "storage_ref": relationship_ref
        }),
    );

    let error = read_relationship_record_log_for_catalog_rebuild(&root)
        .expect_err("relationship rebuild records without relationship_type must fail");

    assert!(matches!(
    error,
    GraphStorageError::CatalogRebuildFailed { stage, .. }
    if stage == "read_relationship_record_log_for_catalog_rebuild"
    ));

    let _ = fs::remove_dir_all(root_path);
}

#[test]
fn reconstruct_catalog_can_skip_node_rebuild_when_option_disabled() {
    let root_path = unique_temp_path("skip_node_rebuild");
    let _root = create_storage_root(root_path.clone(), manifest()).expect("root should be created");
    let mut graph = Graph::new();
    let source_id = graph
        .create_node(NodeInput::new(["Campaign", "FIMI"]))
        .expect("source node should be created");
    let target_id = graph
        .create_node(NodeInput::new(["Narrative"]))
        .expect("target node should be created");
    let relationship_id = graph
        .create_relationship(
            RelationshipInput::new(source_id.clone(), "PROMOTES", target_id.clone())
                .expect("relationship input should be valid"),
        )
        .expect("relationship should be created");
    let source = graph.get_node(&source_id).unwrap().unwrap();
    let relationship = graph.get_relationship(&relationship_id).unwrap().unwrap();

    let source_node_ref = storage_ref(StorageSegment::NodeRecords, 0);
    let relationship_ref = storage_ref(StorageSegment::RelationshipRecords, 128);
    let rel_type = relationship_type("PROMOTES");
    let source_node_envelope = create_node_record_envelope(
        &source,
        source_node_ref.clone(),
        StorageVersion::V1,
        RecordFormat::JsonLinesV1,
        Some(checksum("source-node-record")),
    )
    .expect("source node envelope should be valid");
    let relationship_envelope = create_relationship_record_envelope(
        &relationship,
        relationship_ref.clone(),
        StorageVersion::V1,
        RecordFormat::JsonLinesV1,
        Some(checksum("relationship-record")),
    )
    .expect("relationship envelope should be valid");

    let records = vec![
        graph_storage::CatalogRebuildRecord::Node {
            envelope: source_node_envelope,
            storage_ref: source_node_ref,
            labels: vec!["Campaign".to_owned(), "FIMI".to_owned()],
        },
        graph_storage::CatalogRebuildRecord::Relationship {
            envelope: relationship_envelope,
            storage_ref: relationship_ref,
            relationship_type: rel_type.clone(),
        },
    ];

    let outcome = reconstruct_catalog_from_rebuild_records(
        &records,
        CatalogRebuildOptions {
            include_node_records: false,
            include_relationship_records: true,
            include_outgoing_adjacency: false,
            include_incoming_adjacency: false,
            fail_fast: true,
        },
    )
    .expect("catalog reconstruction should succeed with node rebuild disabled");

    assert!(matches!(
        resolve_latest_node_storage_ref(&outcome.catalog, &source_id),
        Err(GraphStorageError::MissingNodeCatalogEntry { .. })
    ));
    assert_eq!(
        resolve_relationship_ids_by_type(
            &outcome.catalog,
            &rel_type,
            CatalogIndexLookupMode::Strict
        ),
        Ok(vec![relationship_id])
    );
    assert_eq!(outcome.report.latest_node_records_reconstructed, 0);
    assert_eq!(outcome.report.latest_relationship_records_reconstructed, 1);
    assert_eq!(outcome.report.label_index_entries_reconstructed, 0);

    let _ = fs::remove_dir_all(root_path);
}

#[test]
fn detect_corrupted_records_rejects_empty_node_labels() {
    let id = graph_core::NodeId::new("node--empty-label").expect("node id should be valid");
    let storage_ref = storage_ref(StorageSegment::NodeRecords, 2048);
    let envelope = graph_storage::PersistedRecordEnvelope {
        record_id: graph_storage::PersistedRecordId::Node(id),
        kind: graph_storage::PersistedRecordKind::Node,
        storage_version: StorageVersion::V1,
        record_format: RecordFormat::JsonLinesV1,
        graph_record_version: Some(graph_storage::GraphRecordVersion::Node {
            version_id: graph_core::NodeVersionId::new("node-version--empty-label")
                .expect("version id should be valid"),
            version: 1,
            current: true,
            previous_version_id: None,
        }),
        storage_ref: storage_ref.clone(),
        record_checksum: Some(checksum("empty-label")),
    };
    let records = vec![graph_storage::CatalogRebuildRecord::Node {
        envelope,
        storage_ref: storage_ref.clone(),
        labels: vec![" ".to_owned()],
    }];

    let error = detect_corrupted_catalog_rebuild_records(&records)
        .expect_err("node labels with empty values must be rejected");

    assert!(matches!(
    error,
    GraphStorageError::CatalogRebuildCorruptedRecord {
    segment: StorageSegment::NodeRecords,
    storage_ref: Some(actual_ref),
    ..
    } if actual_ref.as_ref() == &storage_ref
    ));
}

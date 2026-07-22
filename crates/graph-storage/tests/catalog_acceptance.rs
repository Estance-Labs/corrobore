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
use graph_core::{NodeId, NodeVersionId, RelationshipId, RelationshipVersionId};
use graph_storage::{
    GraphRecordVersion, GraphStorageError, LatestRecordCatalogEntry, PersistedRecordEnvelope,
    PersistedRecordId, PersistedRecordKind, RecordChecksum, RecordFormat, StorageRef,
    StorageSegment, StorageVersion, check_duplicate_latest_record_conflict,
    create_empty_graph_catalog, index_appended_node_record, index_appended_relationship_record,
    resolve_latest_node_storage_ref, resolve_latest_relationship_storage_ref,
};

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
        length: 96,
        checksum: Some(checksum(format!("acceptance-checksum-{offset}"))),
    }
}

fn node_id(value: &str) -> NodeId {
    NodeId::new(value).unwrap()
}

fn relationship_id(value: &str) -> RelationshipId {
    RelationshipId::new(value).unwrap()
}

fn node_version(
    version_id: &str,
    version: u64,
    current: bool,
    previous_version_id: Option<&str>,
) -> GraphRecordVersion {
    GraphRecordVersion::Node {
        version_id: NodeVersionId::new(version_id).unwrap(),
        version,
        current,
        previous_version_id: previous_version_id.map(|value| NodeVersionId::new(value).unwrap()),
    }
}

fn relationship_version(
    version_id: &str,
    version: u64,
    current: bool,
    previous_version_id: Option<&str>,
) -> GraphRecordVersion {
    GraphRecordVersion::Relationship {
        version_id: RelationshipVersionId::new(version_id).unwrap(),
        version,
        current,
        previous_version_id: previous_version_id
            .map(|value| RelationshipVersionId::new(value).unwrap()),
    }
}

fn node_envelope(
    node_id: &NodeId,
    graph_record_version: GraphRecordVersion,
    storage_ref: StorageRef,
) -> PersistedRecordEnvelope {
    PersistedRecordEnvelope {
        record_id: PersistedRecordId::Node(node_id.clone()),
        kind: PersistedRecordKind::Node,
        storage_version: StorageVersion::V1,
        record_format: RecordFormat::JsonLinesV1,
        graph_record_version: Some(graph_record_version),
        storage_ref,
        record_checksum: Some(checksum("node-acceptance-record")),
    }
}

fn relationship_envelope(
    relationship_id: &RelationshipId,
    graph_record_version: GraphRecordVersion,
    storage_ref: StorageRef,
) -> PersistedRecordEnvelope {
    PersistedRecordEnvelope {
        record_id: PersistedRecordId::Relationship(relationship_id.clone()),
        kind: PersistedRecordKind::Relationship,
        storage_version: StorageVersion::V1,
        record_format: RecordFormat::JsonLinesV1,
        graph_record_version: Some(graph_record_version),
        storage_ref,
        record_checksum: Some(checksum("relationship-acceptance-record")),
    }
}

fn latest_node_entry(node_id: &NodeId, storage_ref: StorageRef) -> LatestRecordCatalogEntry {
    LatestRecordCatalogEntry {
        record_id: PersistedRecordId::Node(node_id.clone()),
        kind: PersistedRecordKind::Node,
        graph_record_version: Some(node_version("node-version--duplicate", 1, true, None)),
        storage_ref,
    }
}

#[test]
fn catalog_acceptance_resolves_latest_node_and_relationship_refs() {
    let mut catalog = create_empty_graph_catalog().unwrap();
    let campaign_id = node_id("node--campaign-acceptance");
    let rel_id = relationship_id("relationship--campaign-actor");
    let node_ref = storage_ref(StorageSegment::NodeRecords, 1_024);
    let rel_ref = storage_ref(StorageSegment::RelationshipRecords, 2_048);
    let node_envelope = node_envelope(
        &campaign_id,
        node_version("node-version--campaign-1", 1, true, None),
        node_ref.clone(),
    );
    let rel_envelope = relationship_envelope(
        &rel_id,
        relationship_version("relationship-version--campaign-actor-1", 1, true, None),
        rel_ref.clone(),
    );

    index_appended_node_record(&mut catalog, &node_envelope, node_ref.clone()).unwrap();
    index_appended_relationship_record(&mut catalog, &rel_envelope, rel_ref.clone()).unwrap();

    assert_eq!(
        resolve_latest_node_storage_ref(&catalog, &campaign_id),
        Ok(node_ref)
    );
    assert_eq!(
        resolve_latest_relationship_storage_ref(&catalog, &rel_id),
        Ok(rel_ref)
    );
    assert!(catalog.historical_records.is_empty());
}

#[test]
fn catalog_acceptance_keeps_historical_records_out_of_latest_lookup() {
    let mut catalog = create_empty_graph_catalog().unwrap();
    let id = node_id("node--historical-acceptance");
    let first_ref = storage_ref(StorageSegment::NodeRecords, 4_096);
    let second_ref = storage_ref(StorageSegment::NodeRecords, 8_192);
    let first_envelope = node_envelope(
        &id,
        node_version("node-version--historical-1", 1, true, None),
        first_ref.clone(),
    );
    let second_envelope = node_envelope(
        &id,
        node_version(
            "node-version--historical-2",
            2,
            true,
            Some("node-version--historical-1"),
        ),
        second_ref.clone(),
    );

    index_appended_node_record(&mut catalog, &first_envelope, first_ref.clone()).unwrap();
    index_appended_node_record(&mut catalog, &second_envelope, second_ref.clone()).unwrap();

    assert_eq!(
        resolve_latest_node_storage_ref(&catalog, &id),
        Ok(second_ref.clone())
    );
    assert_eq!(catalog.historical_records.len(), 1);
    assert_eq!(catalog.historical_records[0].storage_ref, first_ref);
    assert_eq!(
        catalog.historical_records[0].superseded_by,
        Some(second_ref)
    );
}

#[test]
fn catalog_acceptance_reports_missing_entries_explicitly() {
    let catalog = create_empty_graph_catalog().unwrap();
    let missing_node = node_id("node--missing-acceptance");
    let missing_rel = relationship_id("relationship--missing-acceptance");

    assert!(matches!(
    resolve_latest_node_storage_ref(&catalog, &missing_node),
    Err(GraphStorageError::MissingNodeCatalogEntry { node_id }) if node_id == missing_node
    ));
    assert!(matches!(
    resolve_latest_relationship_storage_ref(&catalog, &missing_rel),
    Err(GraphStorageError::MissingRelationshipCatalogEntry { relationship_id })
    if relationship_id == missing_rel
    ));
}

#[test]
fn catalog_acceptance_reports_duplicate_latest_conflicts() {
    let id = node_id("node--duplicate-acceptance");
    let record_id = PersistedRecordId::Node(id.clone());
    let existing_ref = storage_ref(StorageSegment::NodeRecords, 16_384);
    let conflicting_ref = storage_ref(StorageSegment::NodeRecords, 32_768);
    let existing = latest_node_entry(&id, existing_ref.clone());
    let candidate = latest_node_entry(&id, conflicting_ref.clone());

    let error = check_duplicate_latest_record_conflict(&record_id, &existing, &candidate)
        .expect_err("duplicate latest records should be explicit");

    assert!(matches!(
    error,
    GraphStorageError::DuplicateLatestRecordConflict {
    record_id: actual_record_id,
    existing_ref: actual_existing_ref,
    conflicting_ref: actual_conflicting_ref,
    } if actual_record_id == record_id
    && actual_existing_ref.as_ref() == &existing_ref
    && actual_conflicting_ref.as_ref() == &conflicting_ref
    ));
}

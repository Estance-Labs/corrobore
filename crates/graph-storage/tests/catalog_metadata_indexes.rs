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
use graph_core::{Graph, NodeInput, RelationshipInput, RelationshipType};
use graph_storage::{
    CatalogIndexLookupMode, GraphStorageError, LabelIndexNodeMetadata, RecordChecksum,
    RecordFormat, RelationshipTypeIndexRelationshipMetadata, StorageRef, StorageSegment,
    StorageVersion, create_empty_graph_catalog, create_node_record_envelope,
    create_relationship_record_envelope, index_appended_node_record,
    index_appended_relationship_record, index_node_labels, index_relationship_type,
    resolve_label_index_entries, resolve_latest_node_storage_ref,
    resolve_latest_relationship_storage_ref, resolve_node_ids_by_label,
    resolve_relationship_ids_by_type, resolve_relationship_type_index_entries,
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
        length: 512,
        checksum: Some(checksum(format!("issue-56-checksum-{offset}"))),
    }
}

fn relationship_type(value: &str) -> RelationshipType {
    RelationshipType::new(value).expect("test relationship type should be valid")
}

//
// Validate the acceptance path through public APIs only. The catalog
// should resolve common graph access paths from metadata indexes while keeping
// existing latest-ID catalog lookups intact.
//
// Given a graph-core node and relationship persisted through graph-storage
// envelopes plus lightweight label and relationship-type metadata,
// when the latest ID catalog and the metadata indexes are populated,
// then callers can resolve node IDs, relationship IDs, and lightweight metadata
// without loading full node or relationship payloads.
#[test]
fn public_api_indexes_labels_and_relationship_types_without_payload_loading() {
    let mut graph = Graph::new();
    let source_id = graph
        .create_node(NodeInput::new(["Campaign", "FIMI"]))
        .expect("source node should be created");
    let target_id = graph
        .create_node(NodeInput::new(["Infrastructure"]))
        .expect("target node should be created");
    let relationship_id = graph
        .create_relationship(
            RelationshipInput::new(source_id.clone(), "USES", target_id)
                .expect("relationship input should be valid"),
        )
        .expect("relationship should be created");
    let source = graph
        .get_node(&source_id)
        .expect("source node read should succeed")
        .expect("source node should be visible");
    let relationship = graph
        .get_relationship(&relationship_id)
        .expect("relationship read should succeed")
        .expect("relationship should be visible");
    let node_ref = storage_ref(StorageSegment::NodeRecords, 0);
    let relationship_ref = storage_ref(StorageSegment::RelationshipRecords, 512);
    let node_envelope = create_node_record_envelope(
        &source,
        node_ref.clone(),
        StorageVersion::V1,
        RecordFormat::JsonLinesV1,
        Some(checksum("node-record")),
    )
    .expect("node envelope should be valid");
    let relationship_envelope = create_relationship_record_envelope(
        &relationship,
        relationship_ref.clone(),
        StorageVersion::V1,
        RecordFormat::JsonLinesV1,
        Some(checksum("relationship-record")),
    )
    .expect("relationship envelope should be valid");
    let mut catalog = create_empty_graph_catalog().expect("empty catalog should be created");
    let relationship_type = relationship_type("USES");
    let node_index_metadata = LabelIndexNodeMetadata {
        node_id: source_id.clone(),
        latest_storage_ref: Some(node_ref.clone()),
        graph_record_version: node_envelope.graph_record_version.clone(),
    };
    let relationship_index_metadata = RelationshipTypeIndexRelationshipMetadata {
        relationship_id: relationship_id.clone(),
        latest_storage_ref: Some(relationship_ref.clone()),
        graph_record_version: relationship_envelope.graph_record_version.clone(),
    };

    index_appended_node_record(&mut catalog, &node_envelope, node_ref.clone())
        .expect("latest node catalog indexing should succeed");
    index_appended_relationship_record(
        &mut catalog,
        &relationship_envelope,
        relationship_ref.clone(),
    )
    .expect("latest relationship catalog indexing should succeed");
    index_node_labels(
        &mut catalog,
        &vec!["Campaign".to_owned(), "FIMI".to_owned()],
        node_index_metadata.clone(),
    )
    .expect("label metadata indexing should succeed");
    index_relationship_type(
        &mut catalog,
        &relationship_type,
        relationship_index_metadata.clone(),
    )
    .expect("relationship-type metadata indexing should succeed");

    assert_eq!(
        resolve_latest_node_storage_ref(&catalog, &source_id),
        Ok(node_ref.clone())
    );
    assert_eq!(
        resolve_latest_relationship_storage_ref(&catalog, &relationship_id),
        Ok(relationship_ref.clone())
    );
    assert_eq!(
        resolve_node_ids_by_label(&catalog, "Campaign", CatalogIndexLookupMode::Strict)
            .expect("known label should resolve to node IDs"),
        vec![source_id.clone()]
    );
    assert_eq!(
        resolve_node_ids_by_label(&catalog, "FIMI", CatalogIndexLookupMode::Strict)
            .expect("second known label should resolve to node IDs"),
        vec![source_id]
    );
    assert_eq!(
        resolve_label_index_entries(&catalog, "Campaign", CatalogIndexLookupMode::Strict)
            .expect("known label should resolve to lightweight metadata"),
        vec![node_index_metadata]
    );
    assert_eq!(
        resolve_relationship_ids_by_type(
            &catalog,
            &relationship_type,
            CatalogIndexLookupMode::Strict,
        )
        .expect("known relationship type should resolve to relationship IDs"),
        vec![relationship_id]
    );
    assert_eq!(
        resolve_relationship_type_index_entries(
            &catalog,
            &relationship_type,
            CatalogIndexLookupMode::Strict,
        )
        .expect("known relationship type should resolve to lightweight metadata"),
        vec![relationship_index_metadata]
    );
}

//
// Validate deterministic unknown-key behavior required by for both
// strict planners and exploratory loading-profile callers.
//
// Given an empty catalog with no label or relationship-type metadata,
// when unknown keys are resolved in strict and empty-when-unknown modes,
// then strict mode returns explicit typed errors and exploratory mode returns
// empty result sets.
#[test]
fn public_api_resolves_unknown_metadata_keys_deterministically() {
    let catalog = create_empty_graph_catalog().expect("empty catalog should be created");
    let missing_type = relationship_type("UNKNOWN_TYPE");

    let label_error =
        resolve_node_ids_by_label(&catalog, "UnknownLabel", CatalogIndexLookupMode::Strict)
            .expect_err("strict unknown label lookup should fail explicitly");
    let relationship_error =
        resolve_relationship_ids_by_type(&catalog, &missing_type, CatalogIndexLookupMode::Strict)
            .expect_err("strict unknown relationship type lookup should fail explicitly");
    let exploratory_label_result = resolve_label_index_entries(
        &catalog,
        "UnknownLabel",
        CatalogIndexLookupMode::EmptyWhenUnknown,
    )
    .expect("exploratory unknown label lookup should succeed");
    let exploratory_relationship_result = resolve_relationship_type_index_entries(
        &catalog,
        &missing_type,
        CatalogIndexLookupMode::EmptyWhenUnknown,
    )
    .expect("exploratory unknown relationship type lookup should succeed");

    assert!(matches!(
    label_error,
    GraphStorageError::UnknownLabelCatalogEntry { label } if label == "UnknownLabel"
    ));
    assert!(matches!(
    relationship_error,
    GraphStorageError::UnknownRelationshipTypeCatalogEntry { relationship_type }
    if relationship_type == missing_type
    ));
    assert!(exploratory_label_result.is_empty());
    assert!(exploratory_relationship_result.is_empty());
}

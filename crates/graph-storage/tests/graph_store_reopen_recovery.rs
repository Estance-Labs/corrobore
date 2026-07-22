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
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use graph_core::{
    AdjacencyDirection, Graph, GraphPager, Node, NodeId, NodeInput, PropertyValue, Relationship,
    RelationshipId, RelationshipInput, RelationshipType,
};
use graph_storage::{
    CatalogRebuildOptions, GraphAdjacencyStorage, GraphCatalog, GraphId, GraphRecordVersion,
    GraphStorageError, GraphStoreCatalogRecoverySource, GraphStoreOpenMode, GraphStoreOpenOptions,
    JsonLinesRecordCodec, LabelIndexNodeMetadata, PersistedAdjacencyEntry, RecordChecksum,
    RecordFormat, RelationshipTypeIndexRelationshipMetadata, StorageManifest, StorageRef,
    StorageRoot, StorageSegment, StorageTimestamp, StorageVersion,
    build_recovered_file_backed_graph_store, calculate_encoded_record_checksum,
    create_file_backed_graph_pager, create_node_record_envelope,
    create_relationship_record_envelope, create_storage_root, index_appended_node_record,
    index_appended_relationship_record, open_existing_file_backed_graph_store,
    persist_graph_catalog_metadata, read_persisted_graph_catalog_metadata,
    recover_graph_store_adjacency_storage, recover_graph_store_catalog,
    validate_graph_store_reopen_manifest, validate_required_recovery_components,
    write_incoming_adjacency_by_node_id, write_outgoing_adjacency_by_node_id,
};

fn unique_temp_path(test_name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "intelligence_graph_engine_issue_60_reopen_{test_name}_{}_{}",
        std::process::id(),
        unique
    ))
}

fn manifest() -> StorageManifest {
    StorageManifest {
        storage_version: StorageVersion::V1,
        graph_id: GraphId {
            value: "graph--issue-60-reopen".to_owned(),
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

fn storage_root(test_name: &str) -> StorageRoot {
    let path = unique_temp_path(test_name);
    let _ = fs::remove_dir_all(&path);
    create_storage_root(path, manifest()).unwrap()
}

fn graph_fixture() -> (Graph, NodeId, NodeId, RelationshipId) {
    let mut graph = Graph::new();
    let campaign = graph
        .create_node(
            NodeInput::new(["Campaign", "FIMI"])
                .with_property("name", PropertyValue::String("campaign-alpha".to_owned())),
        )
        .unwrap();
    let infrastructure = graph
        .create_node(NodeInput::new(["Infrastructure"]))
        .unwrap();
    let relationship = graph
        .create_relationship(
            RelationshipInput::new(campaign.clone(), "USES", infrastructure.clone())
                .unwrap()
                .with_property("confidence", PropertyValue::Integer(80)),
        )
        .unwrap();
    (graph, campaign, infrastructure, relationship)
}

fn payload_path(root: &StorageRoot, segment: StorageSegment) -> PathBuf {
    match segment {
        StorageSegment::NodeRecords => root.path().join("nodes").join("node_records.log"),
        StorageSegment::RelationshipRecords => root
            .path()
            .join("relationships")
            .join("relationship_records.log"),
        _ => panic!("test payload segment must be node or relationship records"),
    }
}

fn write_payload(
    root: &StorageRoot,
    segment: StorageSegment,
    bytes: &[u8],
    checksum: Option<RecordChecksum>,
) -> StorageRef {
    let path = payload_path(root, segment.clone());
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let offset = fs::metadata(&path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .unwrap();
    file.write_all(bytes).unwrap();
    let checksum = checksum
        .or_else(|| Some(calculate_encoded_record_checksum(&JsonLinesRecordCodec, bytes).unwrap()));
    StorageRef {
        segment,
        offset,
        length: bytes.len() as u64,
        checksum,
    }
}

fn index_node(
    catalog: &mut GraphCatalog,
    node: &Node,
    storage_ref: StorageRef,
    labels: Vec<String>,
) {
    let envelope = create_node_record_envelope(
        node,
        storage_ref.clone(),
        StorageVersion::V1,
        RecordFormat::JsonLinesV1,
        storage_ref.checksum.clone(),
    )
    .unwrap();
    index_appended_node_record(catalog, &envelope, storage_ref.clone()).unwrap();
    graph_storage::index_node_labels(
        catalog,
        &labels,
        LabelIndexNodeMetadata {
            node_id: node.id().clone(),
            latest_storage_ref: Some(storage_ref),
            graph_record_version: Some(GraphRecordVersion::Node {
                version_id: node.version_id().clone(),
                version: node.version(),
                current: node.is_current(),
                previous_version_id: node.previous_version_id().cloned(),
            }),
        },
    )
    .unwrap();
}

fn index_relationship(
    catalog: &mut GraphCatalog,
    relationship: &Relationship,
    storage_ref: StorageRef,
) {
    let envelope = create_relationship_record_envelope(
        relationship,
        storage_ref.clone(),
        StorageVersion::V1,
        RecordFormat::JsonLinesV1,
        storage_ref.checksum.clone(),
    )
    .unwrap();
    index_appended_relationship_record(catalog, &envelope, storage_ref.clone()).unwrap();
    graph_storage::index_relationship_type(
        catalog,
        relationship.rel_type(),
        RelationshipTypeIndexRelationshipMetadata {
            relationship_id: relationship.id().clone(),
            latest_storage_ref: Some(storage_ref),
            graph_record_version: Some(GraphRecordVersion::Relationship {
                version_id: relationship.version_id().clone(),
                version: relationship.version(),
                current: relationship.is_current(),
                previous_version_id: relationship.previous_version_id().cloned(),
            }),
        },
    )
    .unwrap();
}

fn recovered_store_fixture(
    test_name: &str,
) -> (
    StorageRoot,
    GraphCatalog,
    GraphAdjacencyStorage,
    NodeId,
    NodeId,
    RelationshipId,
) {
    let root = storage_root(test_name);
    let (graph, campaign_id, infrastructure_id, relationship_id) = graph_fixture();
    let campaign = graph.get_node(&campaign_id).unwrap().unwrap();
    let infrastructure = graph.get_node(&infrastructure_id).unwrap().unwrap();
    let relationship = graph.get_relationship(&relationship_id).unwrap().unwrap();

    let mut catalog = GraphCatalog::default();
    let campaign_ref = write_payload(
        &root,
        StorageSegment::NodeRecords,
        &serde_json::to_vec(&campaign).unwrap(),
        None,
    );
    let infrastructure_ref = write_payload(
        &root,
        StorageSegment::NodeRecords,
        &serde_json::to_vec(&infrastructure).unwrap(),
        None,
    );
    let relationship_ref = write_payload(
        &root,
        StorageSegment::RelationshipRecords,
        &serde_json::to_vec(&relationship).unwrap(),
        None,
    );

    index_node(
        &mut catalog,
        &campaign,
        campaign_ref.clone(),
        vec!["Campaign".to_owned(), "FIMI".to_owned()],
    );
    index_node(
        &mut catalog,
        &infrastructure,
        infrastructure_ref.clone(),
        vec!["Infrastructure".to_owned()],
    );
    index_relationship(&mut catalog, &relationship, relationship_ref.clone());

    let mut adjacency_storage = GraphAdjacencyStorage::default();
    write_outgoing_adjacency_by_node_id(
        &mut adjacency_storage,
        &mut catalog,
        &campaign_id,
        vec![PersistedAdjacencyEntry {
            relationship_id: relationship_id.clone(),
            source_node_id: campaign_id.clone(),
            target_node_id: infrastructure_id.clone(),
            relationship_type: RelationshipType::new("USES").unwrap(),
            direction: AdjacencyDirection::Outgoing,
            relationship_storage_ref: Some(relationship_ref.clone()),
            source_node_storage_ref: Some(campaign_ref.clone()),
            target_node_storage_ref: Some(infrastructure_ref.clone()),
        }],
    )
    .unwrap();
    write_incoming_adjacency_by_node_id(
        &mut adjacency_storage,
        &mut catalog,
        &infrastructure_id,
        vec![PersistedAdjacencyEntry {
            relationship_id: relationship_id.clone(),
            source_node_id: campaign_id.clone(),
            target_node_id: infrastructure_id.clone(),
            relationship_type: RelationshipType::new("USES").unwrap(),
            direction: AdjacencyDirection::Incoming,
            relationship_storage_ref: Some(relationship_ref),
            source_node_storage_ref: Some(campaign_ref),
            target_node_storage_ref: Some(infrastructure_ref),
        }],
    )
    .unwrap();

    (
        root,
        catalog,
        adjacency_storage,
        campaign_id,
        infrastructure_id,
        relationship_id,
    )
}

fn write_unsupported_manifest(path: &PathBuf) {
    fs::create_dir_all(path).unwrap();
    fs::write(
        path.join("manifest.json"),
        r#"{
 "storage_version": "V999",
 "graph_id": { "value": "graph--unsupported" },
 "created_at": { "value": "2026-07-05T00:00:00Z" },
 "updated_at": { "value": "2026-07-05T00:00:00Z" },
 "record_format": "JsonLinesV1"
}
"#,
    )
    .unwrap();
}

fn options_requiring_only_node_log() -> GraphStoreOpenOptions {
    GraphStoreOpenOptions {
        mode: GraphStoreOpenMode::RebuildCatalogFromAppendLogs,
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
    }
}

fn load_catalog_mode_options() -> GraphStoreOpenOptions {
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
    }
}

//
// Validate that reopen manifest validation is an explicit first-class phase before
// catalog recovery or pager construction.
// Given: an existing storage root with a valid V1 manifest.
// When: the reopen manifest validation boundary is called.
// Then: the trusted manifest metadata is returned unchanged.
#[test]
fn validate_reopen_manifest_returns_valid_manifest_metadata() {
    let root = storage_root("validate_manifest_success");

    let reopened_manifest = validate_graph_store_reopen_manifest(&root)
        .expect("phase 3 should validate and return the manifest");

    assert_eq!(reopened_manifest, manifest());
    let _ = fs::remove_dir_all(root.path());
}

//
// Validate that unsupported storage versions are rejected before any catalog or
// record-log recovery work starts.
// Given: an existing storage root whose manifest declares an unsupported version.
// When: the reopen manifest validation boundary is called.
// Then: the error is `UnsupportedStorageVersion` with the declared version.
#[test]
fn validate_reopen_manifest_rejects_unsupported_storage_version() {
    let path = unique_temp_path("unsupported_storage_version");
    let _ = fs::remove_dir_all(&path);
    write_unsupported_manifest(&path);
    let root = StorageRoot { path: path.clone() };

    let error = validate_graph_store_reopen_manifest(&root).unwrap_err();

    assert!(matches!(
    error,
    GraphStorageError::UnsupportedStorageVersion { version } if version == "V999"
    ));
    let _ = fs::remove_dir_all(path);
}

//
// Validate that required recovery components are checked explicitly instead of
// being treated as an empty but valid catalog.
// Given: a valid storage root with no node append log on disk.
// When: reopen component validation requires the node record log.
// Then: the missing node log is surfaced as a typed missing rebuild source.
#[test]
fn validate_required_recovery_components_reports_missing_node_log() {
    let root = storage_root("missing_node_log");
    let options = options_requiring_only_node_log();

    let error = validate_required_recovery_components(&root, &options).unwrap_err();

    assert!(matches!(
    error,
    GraphStorageError::CatalogRebuildSourceMissing { segment, .. }
    if segment == StorageSegment::NodeRecords
    ));
    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn validate_required_recovery_components_reports_missing_relationship_log() {
    let root = storage_root("missing_relationship_log");
    let options = GraphStoreOpenOptions {
        mode: GraphStoreOpenMode::RebuildCatalogFromAppendLogs,
        catalog_rebuild_options: CatalogRebuildOptions {
            include_node_records: false,
            include_relationship_records: true,
            include_outgoing_adjacency: false,
            include_incoming_adjacency: false,
            fail_fast: true,
        },
        require_node_record_log: false,
        require_relationship_record_log: true,
        require_outgoing_adjacency_log: false,
        require_incoming_adjacency_log: false,
    };

    let error = validate_required_recovery_components(&root, &options)
        .expect_err("missing relationship log should be reported");

    assert!(matches!(
    error,
    GraphStorageError::CatalogRebuildSourceMissing { segment, .. }
    if segment == StorageSegment::RelationshipRecords
    ));
    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn validate_required_recovery_components_reports_missing_outgoing_adjacency_log() {
    let root = storage_root("missing_outgoing_adjacency_log");
    let options = GraphStoreOpenOptions {
        mode: GraphStoreOpenMode::RebuildCatalogFromAppendLogs,
        catalog_rebuild_options: CatalogRebuildOptions {
            include_node_records: false,
            include_relationship_records: false,
            include_outgoing_adjacency: true,
            include_incoming_adjacency: false,
            fail_fast: true,
        },
        require_node_record_log: false,
        require_relationship_record_log: false,
        require_outgoing_adjacency_log: true,
        require_incoming_adjacency_log: false,
    };

    let error = validate_required_recovery_components(&root, &options)
        .expect_err("missing outgoing adjacency log should be reported");

    assert!(matches!(
    error,
    GraphStorageError::CatalogRebuildSourceMissing { segment, .. }
    if segment == StorageSegment::OutgoingAdjacency
    ));
    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn validate_required_recovery_components_reports_missing_incoming_adjacency_log() {
    let root = storage_root("missing_incoming_adjacency_log");
    let options = GraphStoreOpenOptions {
        mode: GraphStoreOpenMode::RebuildCatalogFromAppendLogs,
        catalog_rebuild_options: CatalogRebuildOptions {
            include_node_records: false,
            include_relationship_records: false,
            include_outgoing_adjacency: false,
            include_incoming_adjacency: true,
            fail_fast: true,
        },
        require_node_record_log: false,
        require_relationship_record_log: false,
        require_outgoing_adjacency_log: false,
        require_incoming_adjacency_log: true,
    };

    let error = validate_required_recovery_components(&root, &options)
        .expect_err("missing incoming adjacency log should be reported");

    assert!(matches!(
    error,
    GraphStorageError::CatalogRebuildSourceMissing { segment, .. }
    if segment == StorageSegment::IncomingAdjacency
    ));
    let _ = fs::remove_dir_all(root.path());
}

//
// Validate that top-level reopen fails deterministically on missing manifests
// before attempting catalog rebuild, adjacency recovery, snapshot restore, or WAL
// replay.
// Given: a directory that exists but does not contain `manifest.json`.
// When: the public file-backed graph store reopen entry point is called.
// Then: the error is `ManifestNotFound`.
#[test]
fn open_existing_store_reports_missing_manifest() {
    let path = unique_temp_path("missing_manifest");
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();

    let error =
        open_existing_file_backed_graph_store(path.clone(), GraphStoreOpenOptions::default())
            .unwrap_err();

    assert!(matches!(error, GraphStorageError::ManifestNotFound { .. }));
    let _ = fs::remove_dir_all(path);
}

#[test]
fn open_existing_store_reports_missing_storage_root() {
    let path = unique_temp_path("missing_storage_root");
    let _ = fs::remove_dir_all(&path);

    let error = open_existing_file_backed_graph_store(path, GraphStoreOpenOptions::default())
        .expect_err("missing root path should fail before manifest checks");

    assert!(matches!(
        error,
        GraphStorageError::StorageRootNotFound { .. }
    ));
}

//
// Validate that once catalog and adjacency metadata have been recovered, assembling
// a file-backed store preserves lazy pager behavior without full graph
// deserialization.
// Given: a recovered catalog, recovered adjacency storage, and node/relationship
// payload bytes already present in their storage segments.
// When: the recovered file-backed graph store is assembled and used by the pager.
// Then: node lookup, relationship lookup, outgoing adjacency, and incoming
// adjacency can all be loaded lazily from the reopened store handle.
#[test]
fn build_recovered_store_preserves_pager_lookup_and_adjacency_behavior() {
    let (root, catalog, adjacency_storage, campaign_id, infrastructure_id, relationship_id) =
        recovered_store_fixture("recovered_store_pager_behavior");

    let store = build_recovered_file_backed_graph_store(root.clone(), catalog, adjacency_storage)
        .expect("phase 3 should assemble a recovered file-backed store");
    let pager = create_file_backed_graph_pager(store).unwrap();

    let campaign_payload = pager.load_node_payload(&campaign_id).unwrap();
    assert_eq!(campaign_payload.node.id(), &campaign_id);

    let relationship_payload = pager.load_relationship_payload(&relationship_id).unwrap();
    assert_eq!(relationship_payload.relationship.id(), &relationship_id);

    let outgoing = pager.load_outgoing_adjacency(&campaign_id).unwrap();
    assert_eq!(outgoing.owner_node_id, campaign_id);
    assert_eq!(outgoing.direction, AdjacencyDirection::Outgoing);
    assert_eq!(outgoing.entries[0].neighbor_node_id, infrastructure_id);

    let incoming = pager.load_incoming_adjacency(&infrastructure_id).unwrap();
    assert_eq!(incoming.owner_node_id, infrastructure_id);
    assert_eq!(incoming.direction, AdjacencyDirection::Incoming);
    assert_eq!(incoming.entries[0].neighbor_node_id, campaign_id);

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn validate_required_recovery_components_reports_storage_root_not_found() {
    let root = StorageRoot {
        path: unique_temp_path("missing_root_component_validation"),
    };
    let options = options_requiring_only_node_log();

    let error = validate_required_recovery_components(&root, &options)
        .expect_err("missing root should be surfaced before component checks");

    assert!(matches!(
        error,
        GraphStorageError::StorageRootNotFound { .. }
    ));
}

#[test]
fn recover_catalog_validate_only_returns_empty_catalog_with_persisted_source_marker() {
    let root = storage_root("recover_catalog_validate_only");
    let options = GraphStoreOpenOptions {
        mode: GraphStoreOpenMode::ValidateOnly,
        ..GraphStoreOpenOptions::default()
    };

    let outcome = recover_graph_store_catalog(&root, &options)
        .expect("validate-only mode should not require append logs");

    assert_eq!(outcome.catalog, GraphCatalog::default());
    assert_eq!(
        outcome.source,
        GraphStoreCatalogRecoverySource::PersistedCatalog
    );
    assert_eq!(
        outcome.report,
        graph_storage::CatalogRebuildReport::default()
    );

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn recover_catalog_rebuild_mode_reports_missing_required_sources() {
    let root = storage_root("recover_catalog_rebuild_missing_sources");
    let options = GraphStoreOpenOptions {
        mode: GraphStoreOpenMode::RebuildCatalogFromAppendLogs,
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
    };

    let error = recover_graph_store_catalog(&root, &options)
        .expect_err("rebuild mode should report missing required node record source");

    assert!(matches!(
    error,
    GraphStorageError::CatalogRebuildSourceMissing { segment, .. }
    if segment == StorageSegment::NodeRecords
    ));

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn recover_adjacency_storage_returns_empty_state_without_adjacency_logs() {
    let root = storage_root("recover_adjacency_without_logs");

    let storage = recover_graph_store_adjacency_storage(&root, &GraphCatalog::default())
        .expect("adjacency recovery should succeed when adjacency logs are absent");

    assert_eq!(storage, GraphAdjacencyStorage::default());
    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn open_existing_validate_only_skips_component_checks_and_catalog_rebuild() {
    let root = storage_root("open_existing_validate_only");

    let outcome = open_existing_file_backed_graph_store(
        root.path().to_path_buf(),
        GraphStoreOpenOptions {
            mode: GraphStoreOpenMode::ValidateOnly,
            ..GraphStoreOpenOptions::default()
        },
    )
    .expect("validate-only open should succeed with a valid manifest only");

    assert!(outcome.recovery_report.manifest_validated);
    assert!(!outcome.recovery_report.required_components_validated);
    assert!(!outcome.recovery_report.catalog_recovered);
    assert!(outcome.recovery_report.adjacency_storage_recovered);
    assert!(outcome.recovery_report.catalog_rebuild_report.is_none());
    assert!(outcome.recovery_report.warnings.is_empty());

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn open_existing_load_catalog_mode_emits_fallback_warning_when_rebuild_is_used() {
    let root = storage_root("open_existing_load_catalog_fallback_warning");
    let options = GraphStoreOpenOptions {
        mode: GraphStoreOpenMode::LoadCatalogWhenAvailable,
        catalog_rebuild_options: CatalogRebuildOptions {
            include_node_records: false,
            include_relationship_records: false,
            include_outgoing_adjacency: false,
            include_incoming_adjacency: false,
            fail_fast: true,
        },
        require_node_record_log: false,
        require_relationship_record_log: false,
        require_outgoing_adjacency_log: false,
        require_incoming_adjacency_log: false,
    };

    let outcome = open_existing_file_backed_graph_store(root.path().to_path_buf(), options)
        .expect("load-catalog mode should fall back to append-log rebuild when enabled");

    assert!(outcome.recovery_report.manifest_validated);
    assert!(outcome.recovery_report.required_components_validated);
    assert!(outcome.recovery_report.catalog_recovered);
    assert!(outcome.recovery_report.adjacency_storage_recovered);
    assert_eq!(outcome.recovery_report.warnings.len(), 1);
    assert!(outcome.recovery_report.warnings[0].contains("persisted catalog metadata missing"));

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn recover_catalog_load_mode_prefers_persisted_metadata_without_rebuild_sources() {
    let (root, catalog, _, _, _, _) = recovered_store_fixture("recover_catalog_load_persisted");
    persist_graph_catalog_metadata(&root, &catalog)
        .expect("persisted catalog metadata should be written");

    let persisted = read_persisted_graph_catalog_metadata(&root)
        .expect("persisted catalog metadata should be readable")
        .expect("persisted catalog metadata should exist");
    assert_eq!(persisted, catalog);

    let outcome = recover_graph_store_catalog(&root, &load_catalog_mode_options())
        .expect("load mode should use persisted metadata before append-log rebuild");

    assert_eq!(
        outcome.source,
        GraphStoreCatalogRecoverySource::PersistedCatalog
    );
    assert_eq!(outcome.catalog, catalog);
    assert_eq!(
        outcome.report,
        graph_storage::CatalogRebuildReport::default()
    );

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn recover_catalog_load_mode_rebuilds_when_persisted_metadata_is_deleted() {
    let root = storage_root("recover_catalog_load_rebuild_after_delete");
    let (graph, source_id, _, _) = graph_fixture();
    let source_node = graph
        .get_node(&source_id)
        .expect("node lookup should succeed")
        .expect("node should exist");
    let source_ref = StorageRef {
        segment: StorageSegment::NodeRecords,
        offset: 0,
        length: 16,
        checksum: None,
    };
    let source_envelope = create_node_record_envelope(
        &source_node,
        source_ref.clone(),
        StorageVersion::V1,
        RecordFormat::JsonLinesV1,
        None,
    )
    .expect("node envelope should be created");

    let node_log_path = root.path().join("nodes").join("node_records.log");
    if let Some(parent) = node_log_path.parent() {
        fs::create_dir_all(parent).expect("node log parent should be created");
    }
    fs::write(
        &node_log_path,
        format!(
            "{}\n",
            serde_json::to_string(&source_envelope).expect("node envelope should serialize")
        ),
    )
    .expect("node log should be written");

    let rebuild_outcome = recover_graph_store_catalog(
        &root,
        &GraphStoreOpenOptions {
            mode: GraphStoreOpenMode::RebuildCatalogFromAppendLogs,
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
    .expect("rebuild mode should construct expected catalog");

    persist_graph_catalog_metadata(&root, &rebuild_outcome.catalog)
        .expect("persisted catalog metadata should be written");
    fs::remove_file(root.path().join("catalog").join("catalog_metadata.json"))
        .expect("derived metadata file should be removable for repair-mode test");

    let load_outcome = recover_graph_store_catalog(&root, &load_catalog_mode_options())
        .expect("load mode should deterministically rebuild when metadata is missing");

    assert_eq!(
        load_outcome.source,
        GraphStoreCatalogRecoverySource::RebuiltFromAppendLogs
    );
    assert_eq!(load_outcome.catalog, rebuild_outcome.catalog);

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn open_existing_rebuild_mode_succeeds_when_required_component_logs_exist() {
    let root = storage_root("open_existing_rebuild_mode_success");
    let node_log = root.path().join("nodes").join("node_records.log");
    let relationship_log = root
        .path()
        .join("relationships")
        .join("relationship_records.log");
    let outgoing_log = root.path().join("adjacency").join("outgoing_adjacency.log");
    let incoming_log = root.path().join("adjacency").join("incoming_adjacency.log");

    for path in [node_log, relationship_log, outgoing_log, incoming_log] {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("component parent directories should exist");
        }
        fs::write(path, b"").expect("required component fixture should be created");
    }

    let outcome = open_existing_file_backed_graph_store(
        root.path().to_path_buf(),
        GraphStoreOpenOptions::default(),
    )
    .expect("default rebuild mode should succeed when all required logs exist");

    assert!(outcome.recovery_report.manifest_validated);
    assert!(outcome.recovery_report.required_components_validated);
    assert!(outcome.recovery_report.catalog_recovered);
    assert!(outcome.recovery_report.adjacency_storage_recovered);
    assert!(outcome.recovery_report.warnings.is_empty());

    let rebuild_report = outcome
        .recovery_report
        .catalog_rebuild_report
        .expect("default rebuild mode should include a catalog rebuild report");
    assert_eq!(rebuild_report.records_read.node_records, 0);
    assert_eq!(rebuild_report.records_read.relationship_records, 0);
    assert_eq!(rebuild_report.records_read.outgoing_adjacency_records, 0);
    assert_eq!(rebuild_report.records_read.incoming_adjacency_records, 0);

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn open_existing_rebuild_mode_recovers_adjacency_from_persisted_logs() {
    let root = storage_root("open_existing_rebuild_mode_recovers_adjacency");
    let (graph, source_id, target_id, relationship_id) = graph_fixture();
    let source_node = graph
        .get_node(&source_id)
        .expect("source lookup should succeed")
        .expect("source node should exist");
    let target_node = graph
        .get_node(&target_id)
        .expect("target lookup should succeed")
        .expect("target node should exist");

    let source_ref = StorageRef {
        segment: StorageSegment::NodeRecords,
        offset: 0,
        length: 16,
        checksum: None,
    };
    let target_ref = StorageRef {
        segment: StorageSegment::NodeRecords,
        offset: 16,
        length: 16,
        checksum: None,
    };

    let source_envelope = create_node_record_envelope(
        &source_node,
        source_ref.clone(),
        StorageVersion::V1,
        RecordFormat::JsonLinesV1,
        None,
    )
    .expect("source node envelope should be created");
    let target_envelope = create_node_record_envelope(
        &target_node,
        target_ref.clone(),
        StorageVersion::V1,
        RecordFormat::JsonLinesV1,
        None,
    )
    .expect("target node envelope should be created");

    let node_log_path = root.path().join("nodes").join("node_records.log");
    let relationship_log_path = root
        .path()
        .join("relationships")
        .join("relationship_records.log");
    let outgoing_log_path = root.path().join("adjacency").join("outgoing_adjacency.log");
    let incoming_log_path = root.path().join("adjacency").join("incoming_adjacency.log");

    for path in [
        node_log_path.clone(),
        relationship_log_path.clone(),
        outgoing_log_path.clone(),
        incoming_log_path.clone(),
    ] {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("component parent directories should exist");
        }
    }

    fs::write(
        &node_log_path,
        format!(
            "{}\n{}\n",
            serde_json::to_string(&source_envelope).expect("source envelope should serialize"),
            serde_json::to_string(&target_envelope).expect("target envelope should serialize")
        ),
    )
    .expect("node rebuild log should be written");
    fs::write(&relationship_log_path, b"").expect("relationship log should exist");

    let relationship_storage_ref = StorageRef {
        segment: StorageSegment::RelationshipRecords,
        offset: 0,
        length: 8,
        checksum: None,
    };
    let outgoing_record = graph_storage::PersistedAdjacencyRecord {
        owner_node_id: source_id.clone(),
        direction: AdjacencyDirection::Outgoing,
        entries: vec![PersistedAdjacencyEntry {
            relationship_id: relationship_id.clone(),
            source_node_id: source_id.clone(),
            target_node_id: target_id.clone(),
            relationship_type: RelationshipType::new("USES")
                .expect("relationship type should be valid"),
            direction: AdjacencyDirection::Outgoing,
            relationship_storage_ref: Some(relationship_storage_ref.clone()),
            source_node_storage_ref: Some(source_ref.clone()),
            target_node_storage_ref: Some(target_ref.clone()),
        }],
        storage_ref: Some(StorageRef {
            segment: StorageSegment::OutgoingAdjacency,
            offset: 0,
            length: 1,
            checksum: None,
        }),
    };
    let incoming_record = graph_storage::PersistedAdjacencyRecord {
        owner_node_id: target_id.clone(),
        direction: AdjacencyDirection::Incoming,
        entries: vec![PersistedAdjacencyEntry {
            relationship_id: relationship_id.clone(),
            source_node_id: source_id.clone(),
            target_node_id: target_id.clone(),
            relationship_type: RelationshipType::new("USES")
                .expect("relationship type should be valid"),
            direction: AdjacencyDirection::Incoming,
            relationship_storage_ref: Some(relationship_storage_ref),
            source_node_storage_ref: Some(source_ref),
            target_node_storage_ref: Some(target_ref),
        }],
        storage_ref: Some(StorageRef {
            segment: StorageSegment::IncomingAdjacency,
            offset: 0,
            length: 1,
            checksum: None,
        }),
    };

    fs::write(
        &outgoing_log_path,
        format!(
            "{}\n",
            serde_json::to_string(&outgoing_record).expect("outgoing record should serialize")
        ),
    )
    .expect("outgoing adjacency log should be written");
    fs::write(
        &incoming_log_path,
        format!(
            "{}\n",
            serde_json::to_string(&incoming_record).expect("incoming record should serialize")
        ),
    )
    .expect("incoming adjacency log should be written");

    let outcome = open_existing_file_backed_graph_store(
        root.path().to_path_buf(),
        GraphStoreOpenOptions::default(),
    )
    .expect("default rebuild mode should recover adjacency from persisted logs");

    assert!(outcome.recovery_report.manifest_validated);
    assert!(outcome.recovery_report.required_components_validated);
    assert!(outcome.recovery_report.catalog_recovered);
    assert!(outcome.recovery_report.adjacency_storage_recovered);

    let pager =
        create_file_backed_graph_pager(outcome.store).expect("reopened store should create pager");
    let outgoing = pager
        .load_outgoing_adjacency(&source_id)
        .expect("outgoing adjacency should be recoverable");
    let incoming = pager
        .load_incoming_adjacency(&target_id)
        .expect("incoming adjacency should be recoverable");
    assert_eq!(outgoing.entries.len(), 1);
    assert_eq!(incoming.entries.len(), 1);
    assert_eq!(outgoing.entries[0].neighbor_node_id, target_id);
    assert_eq!(incoming.entries[0].neighbor_node_id, source_id);

    let _ = fs::remove_dir_all(root.path());
}

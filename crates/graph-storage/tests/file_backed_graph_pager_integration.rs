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
    AdjacencyDirection, ExpansionBudget, ExpansionDirection, ExpansionFilters, Graph, GraphPager,
    GraphPagerError, GraphRecordRef, GraphWorkingSetCreateRequest, GraphWorkingSetManager,
    LoadingState, Node, NodeId, NodeInput, PagerBackedRuntime, PagerBackedRuntimeQuery,
    PropertyValue, Relationship, RelationshipId, RelationshipInput, RelationshipType,
    WarmAdjacencyEntry, WarmAdjacencyEntryInput, WorkingSetHotBudget, WorkingSetId,
    default_generic_loading_profile,
};
use graph_storage::{
    GraphAdjacencyStorage, GraphCatalog, GraphId, GraphRecordVersion, JsonLinesRecordCodec,
    LabelIndexNodeMetadata, PersistedAdjacencyEntry, RecordChecksum, RecordFormat,
    RelationshipTypeIndexRelationshipMetadata, StorageManifest, StorageRef, StorageSegment,
    StorageTimestamp, StorageVersion, calculate_encoded_record_checksum,
    create_file_backed_graph_pager, create_file_backed_graph_store, create_node_record_envelope,
    create_relationship_record_envelope, create_storage_root, index_appended_node_record,
    index_appended_relationship_record, index_node_labels, index_relationship_type,
    write_incoming_adjacency_by_node_id, write_outgoing_adjacency_by_node_id,
};

fn unique_temp_path(test_name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "intelligence_graph_engine_issue_59_integration_{test_name}_{}_{}",
        std::process::id(),
        unique
    ))
}

fn manifest() -> StorageManifest {
    StorageManifest {
        storage_version: StorageVersion::V1,
        graph_id: GraphId {
            value: "graph--issue-59-integration".to_owned(),
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

fn storage_root(test_name: &str) -> graph_storage::StorageRoot {
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

fn payload_path(root: &graph_storage::StorageRoot, segment: StorageSegment) -> PathBuf {
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
    root: &graph_storage::StorageRoot,
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
    index_node_labels(
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
    index_relationship_type(
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

fn pager_fixture(
    test_name: &str,
) -> (
    graph_storage::StorageRoot,
    graph_storage::FileBackedGraphPager,
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

    let store = create_file_backed_graph_store(root.clone(), catalog, adjacency_storage).unwrap();
    let pager = create_file_backed_graph_pager(store).unwrap();
    (root, pager, campaign_id, infrastructure_id, relationship_id)
}

//
// Validate the end-to-end acceptance path for using only public crate
// facades. This represents the / bridge: a working-set flow
// can use the file-backed pager to load a seed payload, attach warm adjacency, and
// page in the relationship payload without knowing the file layout.
#[test]
fn file_backed_pager_feeds_working_set_acceptance_flow() {
    let (root, pager, campaign_id, infrastructure_id, relationship_id) =
        pager_fixture("working_set_acceptance_flow");
    let working_set_id = WorkingSetId::new("working-set--issue-59").unwrap();
    let mut manager = GraphWorkingSetManager::new();
    manager
        .create_working_set(GraphWorkingSetCreateRequest::new(working_set_id.clone()))
        .unwrap();

    let seed_metadata = pager
        .load_indexed_metadata(&GraphRecordRef::Node(campaign_id.clone()))
        .unwrap();
    assert_eq!(seed_metadata.loading_state, LoadingState::Indexed);
    assert_eq!(
        seed_metadata.labels,
        vec!["Campaign".to_owned(), "FIMI".to_owned()]
    );

    let campaign_payload = pager.load_node_payload(&campaign_id).unwrap();
    assert_eq!(campaign_payload.node.id(), &campaign_id);
    manager
        .load_seed_node_ids(&working_set_id, [campaign_id.clone()], true)
        .unwrap();

    let outgoing = pager.load_outgoing_adjacency(&campaign_id).unwrap();
    let outgoing_entry = outgoing.entries.first().expect("one warm frontier edge");
    assert_eq!(outgoing_entry.relationship_id, relationship_id);
    assert_eq!(outgoing_entry.neighbor_node_id, infrastructure_id);

    let target_metadata = pager
        .load_indexed_metadata(&GraphRecordRef::Node(infrastructure_id.clone()))
        .unwrap();
    assert_eq!(target_metadata.labels, vec!["Infrastructure".to_owned()]);

    let warm_entry = WarmAdjacencyEntry::new(
        WarmAdjacencyEntryInput::new(
            outgoing_entry.relationship_id.clone(),
            outgoing_entry
                .relationship_type
                .clone()
                .expect("adjacency should carry relationship type"),
            campaign_id.clone(),
            outgoing_entry.neighbor_node_id.clone(),
            target_metadata.labels.clone(),
            AdjacencyDirection::Outgoing,
        )
        .with_storage_refs(
            outgoing_entry.relationship_storage_ref.clone(),
            outgoing_entry.neighbor_storage_ref.clone(),
        ),
    )
    .unwrap();
    manager
        .add_warm_adjacency(&working_set_id, campaign_id.clone(), warm_entry)
        .unwrap();

    let working_set = manager.get_working_set(&working_set_id).unwrap();
    let warm_entries = working_set
        .warm_adjacency_for_source(&campaign_id)
        .expect("warm frontier should be attached");
    assert_eq!(warm_entries.len(), 1);
    assert_eq!(warm_entries[0].target_node_id(), &infrastructure_id);
    assert_eq!(
        warm_entries[0].target_labels(),
        &vec!["Infrastructure".to_owned()]
    );
    assert!(warm_entries[0].relationship_storage_ref().is_some());
    assert!(warm_entries[0].target_storage_ref().is_some());

    let warm_stats = manager.stats(&working_set_id).unwrap();
    assert_eq!(warm_stats.hot_node_count(), 1);
    assert_eq!(warm_stats.warm_node_count(), 1);
    assert_eq!(warm_stats.warm_relationship_count(), 1);

    let relationship_payload = pager.load_relationship_payload(&relationship_id).unwrap();
    assert_eq!(relationship_payload.relationship.id(), &relationship_id);
    manager
        .add_hot_relationship(&working_set_id, relationship_id.clone())
        .unwrap();

    let final_stats = manager.stats(&working_set_id).unwrap();
    assert_eq!(final_stats.hot_node_count(), 1);
    assert_eq!(final_stats.hot_relationship_count(), 1);

    let incoming = pager.load_incoming_adjacency(&infrastructure_id).unwrap();
    assert_eq!(incoming.owner_node_id, infrastructure_id);
    assert_eq!(incoming.direction, AdjacencyDirection::Incoming);
    assert_eq!(incoming.entries[0].neighbor_node_id, campaign_id);

    let _ = fs::remove_dir_all(root.path());
}

//
// Validate acceptance-level deterministic error mapping across the public pager
// contract rather than through storage internals.
#[test]
fn file_backed_pager_acceptance_errors_are_deterministic() {
    let root = storage_root("acceptance_errors_are_deterministic");
    let store = create_file_backed_graph_store(
        root.clone(),
        GraphCatalog::default(),
        GraphAdjacencyStorage::default(),
    )
    .unwrap();
    let pager = create_file_backed_graph_pager(store).unwrap();
    let missing = NodeId::new("node--missing").unwrap();

    let error = pager.load_node_payload(&missing).unwrap_err();

    assert!(matches!(
    error,
    GraphPagerError::UnavailableRecord { record_ref }
    if record_ref == GraphRecordRef::Node(missing)
    ));
    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn file_backed_pager_runtime_query_is_budgeted_and_deterministic() {
    let (root, pager, campaign_id, infrastructure_id, _) = pager_fixture("runtime_query_budgeted");
    let working_set_id = WorkingSetId::new("working-set--issue-393").unwrap();
    let mut runtime = PagerBackedRuntime::new(WorkingSetHotBudget::new(1, 1));
    let query = PagerBackedRuntimeQuery::new(
        working_set_id.clone(),
        vec![campaign_id.clone()],
        ExpansionDirection::Outgoing,
        ExpansionFilters::empty(),
        1,
        default_generic_loading_profile(),
        ExpansionBudget {
            max_loaded_node_count: 100,
            max_loaded_relationship_count: 100,
            max_hot_node_count: 100,
            max_hot_relationship_count: 100,
            max_warm_adjacency_entry_count: 100,
            max_hop_count: 3,
            max_supernode_expansion_count: 10,
            max_payload_byte_count: 1_048_576,
            max_execution_time_ms: 10_000,
        },
    );

    let result = runtime.execute_query(&pager, query).unwrap();
    assert_eq!(
        result.expansion.status(),
        graph_core::ExpansionResultStatus::Complete
    );
    assert!(result.stats.hot_node_count() <= 1);
    assert!(result.stats.hot_relationship_count() <= 1);
    assert_eq!(
        result.eviction.evicted_hot_node_ids,
        vec![infrastructure_id]
    );
    assert_eq!(result.eviction.evicted_hot_relationship_ids.len(), 0);

    let working_set = runtime.manager().get_working_set(&working_set_id).unwrap();
    assert_eq!(
        working_set.node_loading_state(&campaign_id),
        Some(LoadingState::Hot)
    );

    let _ = fs::remove_dir_all(root.path());
}

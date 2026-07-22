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
use graph_core::{AdjacencyDirection, NodeId, NodeVersionId, RelationshipId, RelationshipType};
use graph_storage::{
    AdjacencyStorageLookupMode, GraphAdjacencyStorage, GraphCatalog, GraphRecordVersion,
    LatestRecordCatalogEntry, PersistedAdjacencyEntry, PersistedRecordId, PersistedRecordKind,
    RecordChecksum, StorageRef, StorageSegment, read_incoming_adjacency_by_node_id,
    read_outgoing_adjacency_by_node_id, resolve_incoming_adjacency_storage_ref,
    resolve_outgoing_adjacency_storage_ref, write_incoming_adjacency_by_node_id,
    write_outgoing_adjacency_by_node_id,
};

fn node_id(value: &str) -> NodeId {
    NodeId::new(value).expect("test node id should be valid")
}

fn relationship_id(value: &str) -> RelationshipId {
    RelationshipId::new(value).expect("test relationship id should be valid")
}

fn relationship_type(value: &str) -> RelationshipType {
    RelationshipType::new(value).expect("test relationship type should be valid")
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
        length: 64,
        checksum: Some(checksum(format!("checksum-{offset}"))),
    }
}

fn latest_node_entry(node_id: &NodeId, offset: u64) -> LatestRecordCatalogEntry {
    LatestRecordCatalogEntry {
        record_id: PersistedRecordId::Node(node_id.clone()),
        kind: PersistedRecordKind::Node,
        graph_record_version: Some(GraphRecordVersion::Node {
            version_id: NodeVersionId::new(format!("{}--version-1", node_id.as_str()))
                .expect("test node version id should be valid"),
            version: 1,
            current: true,
            previous_version_id: None,
        }),
        storage_ref: storage_ref(StorageSegment::NodeRecords, offset),
    }
}

fn catalog_with_nodes(nodes: &[NodeId]) -> GraphCatalog {
    let mut catalog = GraphCatalog::default();
    for (index, node_id) in nodes.iter().enumerate() {
        catalog.latest_node_records.insert(
            node_id.clone(),
            latest_node_entry(node_id, 100 + index as u64 * 100),
        );
    }
    catalog
}

fn adjacency_entry(
    relationship_id_value: &str,
    source: &NodeId,
    target: &NodeId,
    direction: AdjacencyDirection,
) -> PersistedAdjacencyEntry {
    PersistedAdjacencyEntry {
        relationship_id: relationship_id(relationship_id_value),
        source_node_id: source.clone(),
        target_node_id: target.clone(),
        relationship_type: relationship_type("AMPLIFIES"),
        direction,
        relationship_storage_ref: Some(storage_ref(StorageSegment::RelationshipRecords, 500)),
        source_node_storage_ref: Some(storage_ref(StorageSegment::NodeRecords, 600)),
        target_node_storage_ref: Some(storage_ref(StorageSegment::NodeRecords, 700)),
    }
}

#[test]
fn acceptance_outgoing_and_incoming_adjacency_roundtrip_without_payload_loading() {
    let source = node_id("node--campaign-1");
    let target = node_id("node--channel-1");
    let mut catalog = catalog_with_nodes(&[source.clone(), target.clone()]);
    let mut storage = GraphAdjacencyStorage::default();

    let outgoing_entry = adjacency_entry(
        "relationship--amplifies-1",
        &source,
        &target,
        AdjacencyDirection::Outgoing,
    );
    let incoming_entry = adjacency_entry(
        "relationship--amplifies-1",
        &source,
        &target,
        AdjacencyDirection::Incoming,
    );

    let outgoing_ref = write_outgoing_adjacency_by_node_id(
        &mut storage,
        &mut catalog,
        &source,
        vec![outgoing_entry.clone()],
    )
    .expect("outgoing adjacency should persist independently");
    let incoming_ref = write_incoming_adjacency_by_node_id(
        &mut storage,
        &mut catalog,
        &target,
        vec![incoming_entry.clone()],
    )
    .expect("incoming adjacency should persist independently");

    assert_eq!(outgoing_ref.segment, StorageSegment::OutgoingAdjacency);
    assert_eq!(incoming_ref.segment, StorageSegment::IncomingAdjacency);
    assert_eq!(
        resolve_outgoing_adjacency_storage_ref(
            &catalog,
            &source,
            AdjacencyStorageLookupMode::Strict,
        ),
        Ok(Some(outgoing_ref.clone()))
    );
    assert_eq!(
        resolve_incoming_adjacency_storage_ref(
            &catalog,
            &target,
            AdjacencyStorageLookupMode::Strict,
        ),
        Ok(Some(incoming_ref.clone()))
    );

    let outgoing_record = read_outgoing_adjacency_by_node_id(
        &storage,
        &catalog,
        &source,
        AdjacencyStorageLookupMode::Strict,
    )
    .expect("outgoing adjacency should be readable by owner node id");
    let incoming_record = read_incoming_adjacency_by_node_id(
        &storage,
        &catalog,
        &target,
        AdjacencyStorageLookupMode::Strict,
    )
    .expect("incoming adjacency should be readable by owner node id");

    assert_eq!(outgoing_record.owner_node_id, source);
    assert_eq!(outgoing_record.direction, AdjacencyDirection::Outgoing);
    assert_eq!(outgoing_record.storage_ref, Some(outgoing_ref));
    assert_eq!(outgoing_record.entries, vec![outgoing_entry]);

    assert_eq!(incoming_record.owner_node_id, target);
    assert_eq!(incoming_record.direction, AdjacencyDirection::Incoming);
    assert_eq!(incoming_record.storage_ref, Some(incoming_ref));
    assert_eq!(incoming_record.entries, vec![incoming_entry]);
}

#[test]
fn acceptance_known_empty_adjacency_is_deterministic_and_unknown_is_explicit() {
    let known = node_id("node--known-empty");
    let unknown = node_id("node--unknown");
    let catalog = catalog_with_nodes(std::slice::from_ref(&known));
    let storage = GraphAdjacencyStorage::default();

    let outgoing_empty = read_outgoing_adjacency_by_node_id(
        &storage,
        &catalog,
        &known,
        AdjacencyStorageLookupMode::EmptyWhenKnownNodeHasNoEdges,
    )
    .expect("known node with no outgoing edges should return deterministic empty adjacency");
    let incoming_empty = read_incoming_adjacency_by_node_id(
        &storage,
        &catalog,
        &known,
        AdjacencyStorageLookupMode::EmptyWhenKnownNodeHasNoEdges,
    )
    .expect("known node with no incoming edges should return deterministic empty adjacency");

    assert!(outgoing_empty.entries.is_empty());
    assert_eq!(outgoing_empty.storage_ref, None);
    assert_eq!(incoming_empty.direction, AdjacencyDirection::Incoming);
    assert!(incoming_empty.entries.is_empty());
    assert_eq!(incoming_empty.storage_ref, None);

    let strict_error = resolve_outgoing_adjacency_storage_ref(
        &catalog,
        &unknown,
        AdjacencyStorageLookupMode::Strict,
    )
    .expect_err("strict lookup for unknown adjacency should be explicit");

    assert!(matches!(
    strict_error,
    graph_storage::GraphStorageError::UnknownNodeAdjacencyCatalogEntry { node_id, direction }
    if node_id == unknown && direction == AdjacencyDirection::Outgoing
    ));
}

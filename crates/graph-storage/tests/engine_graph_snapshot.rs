// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use graph_core::{Graph, NodeInput, PropertyValue, RelationshipInput};
use graph_storage::{
    GraphId, GraphStorageError, RecordFormat, StorageManifest, StorageTimestamp, StorageVersion,
    create_storage_root, load_engine_graph_snapshot, persist_engine_graph_snapshot,
};

fn storage_root(name: &str) -> graph_storage::StorageRoot {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should follow Unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("corrobore-engine-snapshot-{name}-{suffix}"));
    create_storage_root(
        path,
        StorageManifest {
            storage_version: StorageVersion::V1,
            graph_id: GraphId {
                value: format!("graph--snapshot-{name}"),
            },
            created_at: StorageTimestamp {
                value: "2026-07-24T00:00:00Z".to_owned(),
            },
            updated_at: StorageTimestamp {
                value: "2026-07-24T00:00:00Z".to_owned(),
            },
            record_format: RecordFormat::JsonLinesV1,
        },
    )
    .expect("snapshot storage root should initialize")
}

fn populated_graph() -> Graph {
    let mut graph = Graph::new();
    let source = graph
        .create_node(
            NodeInput::new(["Indicator"])
                .with_property("name", PropertyValue::String("durable.example".to_owned())),
        )
        .expect("source node should be created");
    let target = graph
        .create_node(NodeInput::new(["Infrastructure"]))
        .expect("target node should be created");
    graph
        .create_relationship(
            RelationshipInput::new(source, "USES", target)
                .expect("relationship input should be valid"),
        )
        .expect("relationship should be created");
    graph
}

fn snapshot_path(root: &graph_storage::StorageRoot) -> PathBuf {
    root.path().join("runtime").join("engine-graph.json")
}

#[test]
fn engine_graph_snapshot_round_trip_preserves_nodes_relationships_and_sequences() {
    let root = storage_root("roundtrip");
    let graph = populated_graph();
    persist_engine_graph_snapshot(&root, &graph, true)
        .expect("engine graph snapshot should persist");

    let mut recovered =
        load_engine_graph_snapshot(&root).expect("engine graph snapshot should recover");
    assert_eq!(
        recovered
            .list_nodes()
            .expect("recovered nodes should list")
            .len(),
        2
    );
    assert_eq!(
        recovered
            .list_relationships()
            .expect("recovered relationships should list")
            .len(),
        1
    );
    let next = recovered
        .create_node(NodeInput::new(["Indicator"]))
        .expect("recovered sequence should allocate");
    assert_eq!(next.as_str(), "node--3");
}

#[test]
fn engine_graph_snapshot_rejects_corruption_explicitly() {
    let root = storage_root("corruption");
    persist_engine_graph_snapshot(&root, &populated_graph(), false)
        .expect("engine graph snapshot should persist");
    fs::write(snapshot_path(&root), b"{not-json")
        .expect("engine graph snapshot should be corruptible");

    let error = load_engine_graph_snapshot(&root).expect_err("corrupted engine snapshot must fail");
    assert!(matches!(error, GraphStorageError::DecodeFailed { .. }));
}

#[test]
fn engine_graph_snapshot_recovers_previous_file_after_interrupted_promotion() {
    let root = storage_root("previous");
    persist_engine_graph_snapshot(&root, &populated_graph(), false)
        .expect("engine graph snapshot should persist");
    let current = snapshot_path(&root);
    let previous = root
        .path()
        .join("runtime")
        .join("engine-graph.previous.json");
    fs::rename(&current, &previous).expect("interrupted promotion fixture should move current");

    let recovered =
        load_engine_graph_snapshot(&root).expect("previous complete snapshot should recover");
    assert_eq!(
        recovered
            .list_nodes()
            .expect("recovered nodes should list")
            .len(),
        2
    );
}

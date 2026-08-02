// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
#![allow(clippy::unwrap_used)]

use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use graph_core::{
    EvidenceId, EvidenceInput, EvidenceLocator, Graph, GraphPager, NodeInput, NodePatch,
    PropertyValue, RelationshipInput,
};
use graph_storage::{
    CanonicalEngineStore, CanonicalProjectionRequest, CanonicalStoreOptions, DurableTransactionId,
    GraphId, RecordFormat, StorageManifest, StorageTimestamp, StorageVersion,
    create_file_backed_graph_pager, create_storage_root, persist_engine_graph_snapshot,
};

fn unique_temp_path(test_name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "corrobore-issue-41-{test_name}-{}-{unique}",
        std::process::id()
    ))
}

fn manifest() -> StorageManifest {
    StorageManifest {
        storage_version: StorageVersion::V1,
        graph_id: GraphId {
            value: "graph--issue-41".to_owned(),
        },
        created_at: StorageTimestamp {
            value: "2026-07-25T00:00:00Z".to_owned(),
        },
        updated_at: StorageTimestamp {
            value: "2026-07-25T00:00:00Z".to_owned(),
        },
        record_format: RecordFormat::JsonLinesV1,
    }
}

fn empty_store(test_name: &str) -> graph_storage::StorageRoot {
    let path = unique_temp_path(test_name);
    let _ = fs::remove_dir_all(&path);
    create_storage_root(path, manifest()).unwrap()
}

fn graph_with_indicator(name: &str) -> Graph {
    let mut graph = Graph::new();
    graph
        .create_node(
            NodeInput::new(["Indicator"])
                .with_property("name", PropertyValue::String(name.to_owned())),
        )
        .unwrap();
    graph
}

fn directory_size(path: &std::path::Path) -> u64 {
    fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| {
            let path = entry.path();
            if path.is_dir() {
                directory_size(&path)
            } else {
                entry.metadata().map(|metadata| metadata.len()).unwrap_or(0)
            }
        })
        .sum()
}

#[test]
fn startup_recovers_metadata_without_hydrating_payloads() {
    let root = empty_store("cold-start");
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    let graph = graph_with_indicator("cold.example");

    store
        .commit_transition(
            &Graph::new(),
            &graph,
            DurableTransactionId::new("tx--cold-start").unwrap(),
            None,
        )
        .unwrap();
    drop(store);

    let reopened =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    assert_eq!(reopened.startup_report().payloads_hydrated, 0);
    assert_eq!(reopened.stats().page_ins, 0);
    assert_eq!(reopened.stats().resident_hot_nodes, 0);
    assert_eq!(reopened.catalog().latest_node_records.len(), 1);

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn canonical_store_recovers_first_class_evidence_with_attached_records() {
    let root = empty_store("evidence-recovery");
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    let mut current = Graph::new();
    let evidence_id = EvidenceId::new("evidence--canonical-store-1").unwrap();
    current
        .create_evidence(
            EvidenceInput::new(evidence_id.clone(), "document--canonical-store", "excerpt")
                .with_content_sha256(
                    "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
                )
                .with_locator(EvidenceLocator::Page { page: 5 }),
        )
        .unwrap();
    current
        .create_node(
            NodeInput::new(["OpenCtiObject"])
                .with_property(
                    "opencti.canonical_id",
                    PropertyValue::String("indicator--canonical-store".to_owned()),
                )
                .with_evidence_ref(evidence_id.clone()),
        )
        .unwrap();
    store
        .commit_transition(
            &Graph::new(),
            &current,
            DurableTransactionId::new("tx--evidence-recovery").unwrap(),
            None,
        )
        .unwrap();
    drop(store);

    let mut reopened =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    let projection = reopened
        .load_projection(CanonicalProjectionRequest::all())
        .unwrap();
    assert_eq!(projection.evidence_count(), 1);
    assert_eq!(
        projection
            .evidence_by_id(&evidence_id)
            .unwrap()
            .source_ref(),
        "document--canonical-store"
    );
    assert_eq!(
        projection.list_nodes().unwrap()[0].evidence_refs(),
        &[evidence_id]
    );

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn record_level_commit_pages_selected_records_and_never_writes_whole_snapshot() {
    let root = empty_store("record-level");
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    let graph = graph_with_indicator("paged.example");

    store
        .commit_transition(
            &Graph::new(),
            &graph,
            DurableTransactionId::new("tx--record-level").unwrap(),
            None,
        )
        .unwrap();

    assert!(
        !root
            .path()
            .join("runtime")
            .join("engine-graph.json")
            .exists()
    );
    assert!(
        root.path()
            .join("transactions")
            .join("transaction_wal.log")
            .metadata()
            .unwrap()
            .len()
            > 0
    );

    let empty_projection = store
        .load_projection(CanonicalProjectionRequest::default())
        .unwrap();
    assert!(empty_projection.list_nodes().unwrap().is_empty());
    assert_eq!(store.stats().page_ins, 0);

    let projection = store
        .load_projection(CanonicalProjectionRequest::for_label("Indicator"))
        .unwrap();
    assert_eq!(projection.list_nodes().unwrap().len(), 1);
    assert_eq!(store.stats().page_ins, 1);
    assert_eq!(store.stats().resident_hot_nodes, 1);
    assert!(store.stats().resident_hot_nodes <= store.options().max_hot_nodes);
    store
        .load_projection(CanonicalProjectionRequest::for_label("Indicator"))
        .unwrap();
    assert_eq!(store.stats().page_ins, 1);
    assert_eq!(store.stats().cache_hits, 1);

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn projection_rejects_working_sets_that_exceed_the_configured_hot_budget() {
    let root = empty_store("bounded-projection");
    let mut store = CanonicalEngineStore::open(
        root.clone(),
        CanonicalStoreOptions {
            max_hot_nodes: 1,
            max_hot_relationships: 1,
            max_warm_adjacency_entries: 1,
        },
    )
    .unwrap();
    let mut graph = Graph::new();
    graph.create_node(NodeInput::new(["Indicator"])).unwrap();
    graph.create_node(NodeInput::new(["Indicator"])).unwrap();
    store
        .commit_transition(
            &Graph::new(),
            &graph,
            DurableTransactionId::new("tx--bounded-projection").unwrap(),
            None,
        )
        .unwrap();

    let error = store
        .load_projection(CanonicalProjectionRequest::for_label("Indicator"))
        .expect_err("projection over budget must fail explicitly");
    assert!(error.to_string().contains("budget is 1"));
    assert_eq!(store.stats().resident_hot_nodes, 0);

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn projection_rejects_relationship_frontier_over_the_warm_adjacency_budget() {
    let root = empty_store("bounded-adjacency");
    let mut store = CanonicalEngineStore::open(
        root.clone(),
        CanonicalStoreOptions {
            max_hot_nodes: 10,
            max_hot_relationships: 10,
            max_warm_adjacency_entries: 1,
        },
    )
    .unwrap();
    let mut graph = Graph::new();
    let source = graph.create_node(NodeInput::new(["Source"])).unwrap();
    let target = graph.create_node(NodeInput::new(["Target"])).unwrap();
    graph
        .create_relationship(RelationshipInput::new(source, "LINKS", target).unwrap())
        .unwrap();
    store
        .commit_transition(
            &Graph::new(),
            &graph,
            DurableTransactionId::new("tx--bounded-adjacency").unwrap(),
            None,
        )
        .unwrap();

    let error = store
        .load_projection(CanonicalProjectionRequest::all())
        .expect_err("two directional adjacency entries must exceed a budget of one");
    assert!(error.to_string().contains("warm adjacency entries"));
    assert_eq!(store.stats().resident_warm_adjacency_entries, 0);

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn deleting_derived_catalog_rebuilds_from_canonical_records() {
    let root = empty_store("catalog-rebuild");
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    let graph = graph_with_indicator("rebuild.example");
    store
        .commit_transition(
            &Graph::new(),
            &graph,
            DurableTransactionId::new("tx--catalog-rebuild").unwrap(),
            None,
        )
        .unwrap();
    drop(store);

    fs::remove_file(root.path().join("catalog").join("catalog_metadata.json")).unwrap();
    let mut reopened =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    let projection = reopened
        .load_projection(CanonicalProjectionRequest::for_label("Indicator"))
        .unwrap();

    assert_eq!(projection.list_nodes().unwrap().len(), 1);
    assert!(reopened.startup_report().derived_indexes_rebuilt);

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn legacy_snapshot_migrates_once_and_preserves_an_explicit_rollback_boundary() {
    let root = empty_store("migration");
    let mut graph = graph_with_indicator("legacy.example");
    let node_id = graph.list_nodes().unwrap()[0].id().clone();
    graph
        .update_node(
            &node_id,
            NodePatch::default().set_property("score", PropertyValue::Integer(7)),
        )
        .unwrap();
    persist_engine_graph_snapshot(&root, &graph, true).unwrap();
    let original = fs::read(root.path().join("runtime").join("engine-graph.json")).unwrap();

    let mut migrated =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    assert!(migrated.startup_report().legacy_snapshot_migrated);
    assert_eq!(migrated.catalog().historical_records.len(), 1);
    assert!(
        !root
            .path()
            .join("runtime")
            .join("engine-graph.json")
            .exists()
    );
    assert_eq!(
        fs::read(
            root.path()
                .join("runtime")
                .join("engine-graph.rollback.json")
        )
        .unwrap(),
        original
    );
    assert!(
        root.path()
            .join("runtime")
            .join("engine-graph-migration.json")
            .is_file()
    );

    let projection = migrated
        .load_projection(CanonicalProjectionRequest::for_label("Indicator"))
        .unwrap();
    assert_eq!(projection.list_nodes().unwrap().len(), 1);
    drop(migrated);

    fs::remove_file(
        root.path()
            .join("runtime")
            .join("engine-graph-migration.json"),
    )
    .unwrap();
    let resumed =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    assert!(resumed.startup_report().legacy_snapshot_migrated);
    assert_eq!(resumed.catalog().latest_node_records.len(), 1);
    drop(resumed);

    let reopened =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    assert!(!reopened.startup_report().legacy_snapshot_migrated);
    assert_eq!(reopened.catalog().latest_node_records.len(), 1);

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn legacy_migration_rejects_same_count_data_with_a_different_payload() {
    let root = empty_store("migration-integrity");
    let canonical = graph_with_indicator("canonical.example");
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    store
        .commit_transition(
            &Graph::new(),
            &canonical,
            DurableTransactionId::new("tx--canonical-before-migration").unwrap(),
            None,
        )
        .unwrap();
    drop(store);
    persist_engine_graph_snapshot(&root, &graph_with_indicator("legacy.example"), true).unwrap();

    let error = CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default())
        .expect_err("same record counts must not bypass migration integrity");
    assert!(error.to_string().contains("payloads"));
    assert!(
        root.path()
            .join("runtime")
            .join("engine-graph.json")
            .is_file()
    );
    assert!(
        !root
            .path()
            .join("runtime")
            .join("engine-graph-migration.json")
            .exists()
    );

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn node_only_update_preserves_canonical_relationship_adjacency() {
    let root = empty_store("partial-projection-adjacency");
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    let mut baseline = Graph::new();
    let source = baseline.create_node(NodeInput::new(["Source"])).unwrap();
    let target = baseline.create_node(NodeInput::new(["Target"])).unwrap();
    baseline
        .create_relationship(RelationshipInput::new(source.clone(), "LINKS", target).unwrap())
        .unwrap();
    store
        .commit_transition(
            &Graph::new(),
            &baseline,
            DurableTransactionId::new("tx--adjacency-baseline").unwrap(),
            None,
        )
        .unwrap();

    let previous = store
        .load_projection(CanonicalProjectionRequest::for_label("Source"))
        .unwrap();
    let mut current = previous.clone();
    current
        .update_node(
            &source,
            NodePatch::default().set_property("score", PropertyValue::Integer(1)),
        )
        .unwrap();
    store
        .commit_transition(
            &previous,
            &current,
            DurableTransactionId::new("tx--node-only-update").unwrap(),
            None,
        )
        .unwrap();

    let pager = create_file_backed_graph_pager(store.file_backed_store().unwrap()).unwrap();
    assert_eq!(
        pager
            .load_outgoing_adjacency(&source)
            .unwrap()
            .entries
            .len(),
        1
    );

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn partial_relationship_update_preserves_unloaded_relationship_types_in_adjacency() {
    let root = empty_store("partial-relationship-adjacency");
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    let mut baseline = Graph::new();
    let source = baseline.create_node(NodeInput::new(["Source"])).unwrap();
    let linked_target = baseline
        .create_node(NodeInput::new(["LinkedTarget"]))
        .unwrap();
    let other_target = baseline
        .create_node(NodeInput::new(["OtherTarget"]))
        .unwrap();
    let links = baseline
        .create_relationship(
            RelationshipInput::new(source.clone(), "LINKS", linked_target).unwrap(),
        )
        .unwrap();
    let other = baseline
        .create_relationship(RelationshipInput::new(source.clone(), "OTHER", other_target).unwrap())
        .unwrap();
    store
        .commit_transition(
            &Graph::new(),
            &baseline,
            DurableTransactionId::new("tx--relationship-adjacency-baseline").unwrap(),
            None,
        )
        .unwrap();

    let previous = store
        .load_projection(
            CanonicalProjectionRequest::for_label("Source")
                .with_relationships(Some("LINKS".to_owned())),
        )
        .unwrap();
    let mut current = previous.clone();
    current.tombstone_relationship(&links).unwrap();
    store
        .commit_transition(
            &previous,
            &current,
            DurableTransactionId::new("tx--partial-relationship-update").unwrap(),
            None,
        )
        .unwrap();

    let pager = create_file_backed_graph_pager(store.file_backed_store().unwrap()).unwrap();
    let outgoing = pager.load_outgoing_adjacency(&source).unwrap();
    assert_eq!(outgoing.entries.len(), 1);
    assert_eq!(outgoing.entries[0].relationship_id, other);

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn small_profile_single_record_update_has_lower_write_amplification_than_snapshot_rewrite() {
    let root = empty_store("write-amplification");
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    let mut baseline = Graph::new();
    let mut first = None;
    for index in 0..256 {
        let id = baseline
            .create_node(NodeInput::new(["Indicator"]).with_property(
                "name",
                PropertyValue::String(format!("indicator-{index}.example")),
            ))
            .unwrap();
        first.get_or_insert(id);
    }
    store
        .commit_transition(
            &Graph::new(),
            &baseline,
            DurableTransactionId::new("tx--write-amplification-baseline").unwrap(),
            None,
        )
        .unwrap();

    let snapshot_bytes = serde_json::to_vec(&baseline.persistence_snapshot())
        .unwrap()
        .len() as u64;
    let before = directory_size(root.path());
    let mut updated = baseline.clone();
    updated
        .update_node(
            &first.unwrap(),
            NodePatch::default().set_property("score", PropertyValue::Integer(99)),
        )
        .unwrap();
    store
        .commit_transition(
            &baseline,
            &updated,
            DurableTransactionId::new("tx--write-amplification-update").unwrap(),
            None,
        )
        .unwrap();
    let record_level_bytes = directory_size(root.path()).saturating_sub(before);

    assert!(
        record_level_bytes * 4 < snapshot_bytes,
        "record-level update wrote {record_level_bytes} bytes versus a {snapshot_bytes}-byte graph snapshot"
    );

    let _ = fs::remove_dir_all(root.path());
}

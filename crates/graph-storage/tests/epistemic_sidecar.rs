// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
#![allow(clippy::unwrap_used)]

use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use graph_core::{Graph, NodeInput, PropertyValue};
use graph_storage::{
    CanonicalEngineStore, CanonicalProjectionRequest, CanonicalStoreOptions, DurableTransactionId,
    GraphId, RecordFormat, StorageManifest, StorageTimestamp, StorageVersion, create_storage_root,
    persist_engine_graph_snapshot,
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

use graph_core::{EpistemicStores, EvidenceSourceType, SourceId, SourceInput};
use graph_storage::load_engine_graph_snapshot;

fn governed_graph() -> Graph {
    let mut graph = graph_with_indicator("governed.example");
    graph
        .epistemic_stores_mut()
        .sources
        .register_source(SourceInput::new(
            SourceId::new("source--durable").unwrap(),
            "https://vendor.example/report.pdf",
            EvidenceSourceType::Document,
        ))
        .unwrap();
    graph
}

//
// Epic 0029 WS-A item 7 (issue #153): the canonical store persists the
// epistemic stores in a dedicated sidecar beside the evidence sidecar, recovers
// them on reopen, and serves them in every projection.
#[test]
fn canonical_store_recovers_epistemic_stores_from_the_sidecar() {
    let root = empty_store("epistemic-recovery");
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    let current = governed_graph();
    store
        .commit_transition(
            &Graph::new(),
            &current,
            DurableTransactionId::new("tx--epistemic-recovery").unwrap(),
            None,
        )
        .unwrap();
    drop(store);

    assert!(
        root.path()
            .join("runtime")
            .join("epistemic-records-v1.json")
            .is_file(),
        "the epistemic sidecar is written beside the evidence sidecar"
    );

    let mut reopened =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    let projection = reopened
        .load_projection(CanonicalProjectionRequest::all())
        .unwrap();
    assert_eq!(projection.epistemic_stores(), current.epistemic_stores());
    assert!(
        projection
            .epistemic_stores()
            .sources
            .current_source(&SourceId::new("source--durable").unwrap())
            .is_some()
    );

    let _ = fs::remove_dir_all(root.path());
}

//
// An epistemic-only change (no node or relationship touched) still commits
// and is visible after reopen.
#[test]
fn epistemic_only_transition_commits_the_sidecar() {
    let root = empty_store("epistemic-only");
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    let base = graph_with_indicator("base.example");
    store
        .commit_transition(
            &Graph::new(),
            &base,
            DurableTransactionId::new("tx--base").unwrap(),
            None,
        )
        .unwrap();

    let mut next = base.clone();
    next.epistemic_stores_mut()
        .sources
        .register_source(SourceInput::new(
            SourceId::new("source--later").unwrap(),
            "https://vendor.example/later.pdf",
            EvidenceSourceType::Document,
        ))
        .unwrap();
    let outcome = store
        .commit_transition(
            &base,
            &next,
            DurableTransactionId::new("tx--later").unwrap(),
            None,
        )
        .unwrap();
    assert!(outcome.applied);
    drop(store);

    let mut reopened =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    let projection = reopened
        .load_projection(CanonicalProjectionRequest::all())
        .unwrap();
    assert_eq!(projection.epistemic_stores().sources.len(), 1);

    let _ = fs::remove_dir_all(root.path());
}

//
// The engine snapshot file carries the epistemic stores with the graph.
#[test]
fn engine_graph_snapshot_round_trips_epistemic_stores() {
    let root = empty_store("epistemic-snapshot");
    let graph = governed_graph();
    persist_engine_graph_snapshot(&root, &graph, true).unwrap();
    let loaded = load_engine_graph_snapshot(&root).unwrap();
    assert_eq!(loaded.epistemic_stores(), graph.epistemic_stores());
    assert_ne!(loaded.epistemic_stores(), &EpistemicStores::default());

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn neutral_collections_survive_reopen_and_bounded_projections_without_loading_members() {
    use graph_core::{
        BitemporalStamp, CampaignId, CampaignInput, ContextMembership, NarrativeId, NarrativeInput,
        TemporalTimestamp,
    };
    let root = empty_store("neutral-collections");
    let mut graph = governed_graph();
    let actor = graph.list_nodes().unwrap()[0].id().clone();
    let members = ContextMembership {
        actors: vec![actor],
        content: vec![SourceId::new("source--durable").unwrap()],
        ..Default::default()
    };
    let time = TemporalTimestamp::new("2026-01-01T00:00:00Z").unwrap();
    let stamp = BitemporalStamp::new(time.clone(), time).unwrap();
    let narrative = graph
        .create_narrative(NarrativeInput::new(
            NarrativeId::new("narrative--durable").unwrap(),
            members.clone(),
            stamp.clone(),
        ))
        .unwrap();
    graph
        .create_campaign(CampaignInput::new(
            CampaignId::new("campaign--durable").unwrap(),
            vec![narrative],
            members,
            stamp,
        ))
        .unwrap();
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    store
        .commit_transition(
            &Graph::new(),
            &graph,
            DurableTransactionId::new("tx--collections").unwrap(),
            None,
        )
        .unwrap();
    drop(store);
    let mut reopened =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    let bounded = reopened
        .load_projection(CanonicalProjectionRequest::for_label("absent-label"))
        .unwrap();
    assert!(bounded.list_nodes().unwrap().is_empty());
    assert_eq!(
        bounded.epistemic_stores().narrative_campaigns,
        graph.epistemic_stores().narrative_campaigns
    );
    let view = bounded.epistemic_projection().unwrap();
    assert!(
        graph_core::validate_graph_structure(&view, &[])
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        graph_core::epistemic_nodes_of_kind(&view, graph_core::EpistemicNodeKind::RecordReference)
            .unwrap()
            .len(),
        1
    );
    let json = bounded.export_memory_json().unwrap();
    assert_eq!(
        Graph::from_memory_json(&json)
            .unwrap()
            .export_memory_json()
            .unwrap(),
        json
    );
    drop(reopened);
    fs::remove_dir_all(root.path()).unwrap();
}

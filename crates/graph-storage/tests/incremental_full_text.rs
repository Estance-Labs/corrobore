// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! Issue #325: the full-text index follows commits incrementally.
#![allow(clippy::unwrap_used)]

use std::{
    collections::BTreeMap,
    time::{SystemTime, UNIX_EPOCH},
};

use graph_core::{Graph, NodeInput, NodePatch, PropertyValue};
use graph_storage::{
    CanonicalAccessContext, CanonicalEngineStore, CanonicalProjectionRequest,
    CanonicalStoreOptions, DurableTransactionId, GraphId, MutationCrashStage, RecordFormat,
    StorageManifest, StorageTimestamp, StorageVersion, create_storage_root,
};
use opencti_search::{FullTextMatchMode, FullTextQuery, FullTextSearchReadiness};
use serde_json::json;

/// Tests in one process may start within the clock's resolution (about a
/// microsecond on macOS), so a counter keeps their directories apart.
static ROOTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn root() -> graph_storage::StorageRoot {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "corrobore-issue-325-{}-{unique}-{}",
        std::process::id(),
        ROOTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    create_storage_root(
        path,
        StorageManifest {
            storage_version: StorageVersion::V1,
            graph_id: GraphId {
                value: "graph--issue-325".to_owned(),
            },
            created_at: StorageTimestamp {
                value: "2026-07-25T00:00:00Z".to_owned(),
            },
            updated_at: StorageTimestamp {
                value: "2026-07-25T00:00:00Z".to_owned(),
            },
            record_format: RecordFormat::JsonLinesV1,
        },
    )
    .unwrap()
}

fn node(id: &str, name: &str, marking: &str) -> NodeInput {
    NodeInput::new(vec!["OpenCtiType_indicator".to_owned()])
        .with_property("opencti.canonical_id", PropertyValue::String(id.to_owned()))
        .with_property(
            "opencti.entity_type",
            PropertyValue::String("indicator".to_owned()),
        )
        .with_property("opencti.field.name", PropertyValue::String(name.to_owned()))
        .with_property(
            "opencti.access",
            PropertyValue::Json(json!({"marking_ids": [marking]})),
        )
}

fn access() -> CanonicalAccessContext {
    CanonicalAccessContext {
        subject_id: "user--clear".to_owned(),
        marking_ids: vec!["marking--clear".to_owned()],
        attributes: BTreeMap::from([("policy_version".to_owned(), "policy--v1".to_owned())]),
        ..CanonicalAccessContext::default()
    }
}

fn query(text: &str) -> FullTextQuery {
    FullTextQuery {
        text: text.to_owned(),
        mode: FullTextMatchMode::Term,
        fields: vec!["name".to_owned()],
        kinds: vec!["indicator".to_owned()],
        filters: Vec::new(),
        limit: 20,
        cursor: None,
    }
}

fn write(store: &mut CanonicalEngineStore, id: &str, change: impl FnOnce(&mut Graph)) {
    let previous = store
        .load_projection(CanonicalProjectionRequest::all())
        .unwrap();
    let mut current = previous.clone();
    change(&mut current);
    store
        .commit_transition(
            &previous,
            &current,
            DurableTransactionId::new(id).unwrap(),
            None,
        )
        .unwrap();
}

fn ids(store: &mut CanonicalEngineStore, text: &str) -> Vec<String> {
    let mut ids: Vec<String> = store
        .search_full_text(&query(text), &access())
        .unwrap()
        .hits
        .into_iter()
        .map(|hit| hit.id)
        .collect();
    ids.sort();
    ids
}

#[test]
fn commits_update_a_ready_index_without_a_rebuild() {
    let root = root();
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    write(&mut store, "tx--325-seed", |graph| {
        graph
            .create_node(node("indicator--one", "beacon one", "marking--clear"))
            .unwrap();
    });
    assert_eq!(
        ids(&mut store, "beacon").len(),
        1,
        "first search builds the index"
    );
    assert_eq!(
        store.full_text_readiness().unwrap(),
        FullTextSearchReadiness::Ready
    );

    write(&mut store, "tx--325-add", |graph| {
        graph
            .create_node(node("indicator--two", "beacon two", "marking--clear"))
            .unwrap();
    });
    assert_eq!(
        store.full_text_readiness().unwrap(),
        FullTextSearchReadiness::Ready,
        "the commit extended the index in place"
    );
    assert_eq!(ids(&mut store, "beacon").len(), 2);

    let first = store
        .load_projection(CanonicalProjectionRequest::all())
        .unwrap()
        .current_node_records()
        .unwrap()
        .into_iter()
        .find(|node| {
            node.properties().get("opencti.canonical_id")
                == Some(&PropertyValue::String("indicator--one".to_owned()))
        })
        .unwrap();
    write(&mut store, "tx--325-rename", |graph| {
        graph
            .update_node(
                first.id(),
                NodePatch::default().set_property(
                    "opencti.field.name",
                    PropertyValue::String("lighthouse one".to_owned()),
                ),
            )
            .unwrap();
    });
    assert_eq!(
        store.full_text_readiness().unwrap(),
        FullTextSearchReadiness::Ready
    );
    assert_eq!(ids(&mut store, "beacon").len(), 1, "the old text is gone");
    assert_eq!(ids(&mut store, "lighthouse").len(), 1);

    write(&mut store, "tx--325-delete", |graph| {
        graph.tombstone_node(first.id()).unwrap();
    });
    assert_eq!(
        store.full_text_readiness().unwrap(),
        FullTextSearchReadiness::Ready
    );
    assert!(ids(&mut store, "lighthouse").is_empty());

    let _ = std::fs::remove_dir_all(root.path());
}

#[test]
fn a_new_field_or_a_crash_falls_back_to_a_rebuild() {
    let root = root();
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    write(&mut store, "tx--325-b-seed", |graph| {
        graph
            .create_node(node("indicator--one", "beacon one", "marking--clear"))
            .unwrap();
    });
    assert_eq!(ids(&mut store, "beacon").len(), 1);

    write(&mut store, "tx--325-b-field", |graph| {
        graph
            .create_node(
                node("indicator--two", "beacon two", "marking--clear").with_property(
                    "opencti.field.description",
                    PropertyValue::String("a field the index has never seen".to_owned()),
                ),
            )
            .unwrap();
    });
    assert_eq!(
        store.full_text_readiness().unwrap(),
        FullTextSearchReadiness::RebuildRequired,
        "an unknown field needs a new schema"
    );
    assert_eq!(
        ids(&mut store, "beacon").len(),
        2,
        "the next search rebuilds"
    );

    let previous = store
        .load_projection(CanonicalProjectionRequest::all())
        .unwrap();
    let mut current = previous.clone();
    current
        .create_node(node("indicator--three", "beacon three", "marking--clear"))
        .unwrap();
    let crashed = store.commit_transition(
        &previous,
        &current,
        DurableTransactionId::new("tx--325-b-crash").unwrap(),
        Some(MutationCrashStage::AfterPayloadRecords),
    );
    assert!(crashed.is_err());
    assert_eq!(
        store.full_text_readiness().unwrap(),
        FullTextSearchReadiness::RebuildRequired,
        "an interrupted transaction never leaves a ready index behind"
    );

    let _ = std::fs::remove_dir_all(root.path());
}

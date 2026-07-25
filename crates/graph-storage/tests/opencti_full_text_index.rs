// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

#![allow(clippy::unwrap_used)]

use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use graph_core::{Graph, NodeInput, NodePatch, PropertyValue};
use graph_storage::{
    CanonicalAccessContext, CanonicalEngineStore, CanonicalStoreOptions, DurableTransactionId,
    GraphId, RecordFormat, StorageManifest, StorageTimestamp, StorageVersion, create_storage_root,
};
use opencti_search::{FullTextMatchMode, FullTextQuery, FullTextSearchReadiness};
use serde_json::json;

fn root() -> graph_storage::StorageRoot {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "corrobore-issue-46-canonical-{}-{unique}",
        std::process::id()
    ));
    create_storage_root(
        path,
        StorageManifest {
            storage_version: StorageVersion::V1,
            graph_id: GraphId {
                value: "graph--issue-46".to_owned(),
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

#[test]
fn canonical_commits_publish_access_aware_updates_and_rebuild_after_corruption() {
    let root = root();
    let mut current = Graph::new();
    let visible = current
        .create_node(node(
            "indicator--visible",
            "Documentation beacon",
            "marking--clear",
        ))
        .unwrap();
    current
        .create_node(node(
            "indicator--hidden",
            "Documentation hidden",
            "marking--amber",
        ))
        .unwrap();
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    store
        .commit_transition(
            &Graph::new(),
            &current,
            DurableTransactionId::new("tx--search-create").unwrap(),
            None,
        )
        .unwrap();
    let cursor_key_path: PathBuf = root.path().join("search/full-text-v1/cursor.key");
    let cursor_key = fs::read(&cursor_key_path).unwrap();
    assert_eq!(cursor_key.len(), 32);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        assert_eq!(
            fs::metadata(&cursor_key_path).unwrap().permissions().mode() & 0o077,
            0
        );
    }

    let first = store
        .search_full_text(&query("documentation"), &access())
        .unwrap();
    assert_eq!(first.total, 1);
    assert_eq!(first.hits[0].id, "indicator--visible");
    assert_eq!(first.authorization_denials, 1);

    let previous = current.clone();
    current
        .update_node(
            &visible,
            NodePatch::default().set_property(
                "opencti.field.name",
                PropertyValue::String("Updated quasar".to_owned()),
            ),
        )
        .unwrap();
    store
        .commit_transition(
            &previous,
            &current,
            DurableTransactionId::new("tx--search-update").unwrap(),
            None,
        )
        .unwrap();
    assert_eq!(
        store
            .search_full_text(&query("quasar"), &access())
            .unwrap()
            .total,
        1
    );
    assert_eq!(
        store
            .search_full_text(&query("beacon"), &access())
            .unwrap()
            .total,
        0
    );

    let index_path: PathBuf = root.path().join("search/full-text-v1/published");
    fs::remove_file(index_path.join("meta.json")).unwrap();
    assert_eq!(
        store.rebuild_full_text_index().unwrap().readiness,
        FullTextSearchReadiness::Ready
    );
    assert_eq!(
        store
            .search_full_text(&query("quasar"), &access())
            .unwrap()
            .total,
        1
    );

    drop(store);
    let mut reopened =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    assert_eq!(fs::read(&cursor_key_path).unwrap(), cursor_key);
    assert_eq!(
        reopened
            .search_full_text(&query("quasar"), &access())
            .unwrap()
            .total,
        1
    );

    let _ = fs::remove_dir_all(root.path());
}

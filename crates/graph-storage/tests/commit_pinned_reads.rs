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

//! Issue #318: reads pinned to a commit keep returning the versions visible
//! there, whatever lands afterwards.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use graph_core::{NodeId, NodeInput, NodePatch, PropertyValue, RelationshipInput};
use graph_storage::{
    CanonicalEngineStore, CanonicalProjectionRequest, CanonicalStoreOptions, DurableTransactionId,
    GraphCommitId, GraphId, GraphStorageError, HistoryRecordKind, MAIN_BRANCH, NeighbourDirection,
    RecordFormat, StorageManifest, StorageRoot, StorageTimestamp, StorageVersion,
    create_storage_root,
};

fn empty_store(test_name: &str) -> StorageRoot {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path: PathBuf = std::env::temp_dir().join(format!(
        "corrobore-issue-318-{test_name}-{}-{unique}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&path);
    create_storage_root(
        path,
        StorageManifest {
            storage_version: StorageVersion::V1,
            graph_id: GraphId {
                value: "graph--issue-318".to_owned(),
            },
            created_at: StorageTimestamp {
                value: "2026-11-15T00:00:00Z".to_owned(),
            },
            updated_at: StorageTimestamp {
                value: "2026-11-15T00:00:00Z".to_owned(),
            },
            record_format: RecordFormat::JsonLinesV1,
        },
    )
    .unwrap()
}

fn open(root: &StorageRoot) -> CanonicalEngineStore {
    CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap()
}

/// Apply `change` to the full current graph and commit it as `id`.
fn write(
    store: &mut CanonicalEngineStore,
    id: &str,
    change: impl FnOnce(&mut graph_core::Graph),
) -> GraphCommitId {
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
    store.graph_head(MAIN_BRANCH).unwrap()
}

fn name_of(node: &graph_core::Node) -> String {
    match node.properties().get("name") {
        Some(PropertyValue::String(value)) => value.clone(),
        other => panic!("unexpected name {other:?}"),
    }
}

fn seed(store: &mut CanonicalEngineStore) -> (GraphCommitId, NodeId, NodeId) {
    let mut ids = (None, None);
    let commit = write(store, "tx--issue-318-seed", |graph| {
        let indicator = graph
            .create_node(
                NodeInput::new(["Indicator"])
                    .with_property("name", PropertyValue::String("old.example".to_owned())),
            )
            .unwrap();
        let malware = graph
            .create_node(
                NodeInput::new(["Malware"])
                    .with_property("name", PropertyValue::String("emotet".to_owned())),
            )
            .unwrap();
        graph
            .create_relationship(
                RelationshipInput::new(indicator.clone(), "INDICATES", malware.clone()).unwrap(),
            )
            .unwrap();
        ids = (Some(indicator), Some(malware));
    });
    (commit, ids.0.unwrap(), ids.1.unwrap())
}

#[test]
fn read_at_an_older_commit_returns_the_old_version_after_later_writes() {
    let root = empty_store("old-version");
    let mut store = open(&root);
    let (seeded, indicator, _) = seed(&mut store);
    let updated = write(&mut store, "tx--issue-318-rename", |graph| {
        graph
            .update_node(
                &indicator,
                NodePatch::default()
                    .set_property("name", PropertyValue::String("new.example".to_owned())),
            )
            .unwrap();
    });

    let old = store
        .load_commit_projection(&seeded, &CanonicalProjectionRequest::for_label("Indicator"))
        .unwrap();
    let new = store
        .load_commit_projection(
            &updated,
            &CanonicalProjectionRequest::for_label("Indicator"),
        )
        .unwrap();
    let old_node = old.get_node(&indicator).unwrap().unwrap();
    let new_node = new.get_node(&indicator).unwrap().unwrap();
    assert_eq!(name_of(&old_node), "old.example");
    assert_eq!(name_of(&new_node), "new.example");
    assert_ne!(old_node.version_id(), new_node.version_id());

    // The same holds after reopening the store.
    drop(store);
    let mut reopened = open(&root);
    let old = reopened
        .load_commit_projection(&seeded, &CanonicalProjectionRequest::all())
        .unwrap();
    assert_eq!(
        name_of(&old.get_node(&indicator).unwrap().unwrap()),
        "old.example"
    );

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn projection_with_relationships_includes_their_endpoints() {
    let root = empty_store("projection");
    let mut store = open(&root);
    let (seeded, indicator, malware) = seed(&mut store);

    let graph = store
        .load_commit_projection(
            &seeded,
            &CanonicalProjectionRequest::for_label("Indicator").with_relationships(None),
        )
        .unwrap();
    assert!(graph.get_node(&indicator).unwrap().is_some());
    assert!(graph.get_node(&malware).unwrap().is_some());
    assert_eq!(graph.list_relationships().unwrap().len(), 1);

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn record_pages_are_stable_while_new_commits_land() {
    let root = empty_store("paging");
    let mut store = open(&root);
    let pinned = write(&mut store, "tx--issue-318-many", |graph| {
        for index in 0..5 {
            graph
                .create_node(
                    NodeInput::new(["Indicator"])
                        .with_property("name", PropertyValue::String(format!("n{index}"))),
                )
                .unwrap();
        }
    });
    let first = store
        .list_commit_records(
            &pinned,
            HistoryRecordKind::Node,
            Some("Indicator"),
            None,
            2,
            false,
        )
        .unwrap();
    assert_eq!(first.total, 5);
    assert_eq!(first.nodes.len(), 2);

    write(&mut store, "tx--issue-318-more", |graph| {
        graph
            .create_node(
                NodeInput::new(["Indicator"])
                    .with_property("name", PropertyValue::String("late".to_owned())),
            )
            .unwrap();
    });

    let mut seen: Vec<String> = first
        .nodes
        .iter()
        .map(|node| node.id().as_str().to_owned())
        .collect();
    let mut cursor = first.next_cursor.clone();
    while let Some(next) = cursor {
        let page = store
            .list_commit_records(
                &pinned,
                HistoryRecordKind::Node,
                Some("Indicator"),
                Some(&next),
                2,
                false,
            )
            .unwrap();
        assert_eq!(
            page.total, 5,
            "the pinned commit does not see the late write"
        );
        seen.extend(page.nodes.iter().map(|node| node.id().as_str().to_owned()));
        cursor = page.next_cursor;
    }
    let mut sorted = seen.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), 5);
    assert_eq!(seen, sorted, "keyset order is record-id order");

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn tombstoned_node_is_visible_before_and_hidden_after_its_deletion() {
    let root = empty_store("tombstone");
    let mut store = open(&root);
    let (seeded, indicator, _) = seed(&mut store);
    let deleted = write(&mut store, "tx--issue-318-delete", |graph| {
        graph.tombstone_node(&indicator).unwrap();
    });

    let before = store
        .list_commit_records(
            &seeded,
            HistoryRecordKind::Node,
            Some("Indicator"),
            None,
            10,
            false,
        )
        .unwrap();
    let after = store
        .list_commit_records(
            &deleted,
            HistoryRecordKind::Node,
            Some("Indicator"),
            None,
            10,
            false,
        )
        .unwrap();
    let after_with_deleted = store
        .list_commit_records(
            &deleted,
            HistoryRecordKind::Node,
            Some("Indicator"),
            None,
            10,
            true,
        )
        .unwrap();
    assert_eq!(before.total, 1);
    assert_eq!(after.total, 0);
    assert_eq!(after_with_deleted.total, 1);
    assert!(
        store
            .commit_node(&deleted, indicator.as_str(), 5)
            .unwrap()
            .is_none()
    );

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn node_neighbourhood_groups_relationships_by_type_and_direction() {
    let root = empty_store("neighbourhood");
    let mut store = open(&root);
    let (seeded, indicator, malware) = seed(&mut store);

    let around_malware = store
        .commit_node(&seeded, malware.as_str(), 5)
        .unwrap()
        .unwrap();
    assert_eq!(around_malware.groups.len(), 1);
    let group = &around_malware.groups[0];
    assert_eq!(group.relationship_type, "INDICATES");
    assert_eq!(group.direction, NeighbourDirection::Incoming);
    assert_eq!(group.count, 1);
    assert_eq!(group.items[0].1.id(), &indicator);

    let around_indicator = store
        .commit_node(&seeded, indicator.as_str(), 5)
        .unwrap()
        .unwrap();
    assert_eq!(
        around_indicator.groups[0].direction,
        NeighbourDirection::Outgoing
    );

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn transactions_and_times_resolve_to_commits() {
    let root = empty_store("resolve");
    let mut store = open(&root);
    let (seeded, _, _) = seed(&mut store);
    let created_at = store.graph_commit(&seeded).unwrap().created_at.clone();

    assert_eq!(
        store.commit_for_transaction("tx--issue-318-seed").unwrap(),
        Some(seeded.clone())
    );
    assert_eq!(store.commit_for_transaction("tx--unknown").unwrap(), None);
    assert_eq!(
        store.commit_at(MAIN_BRANCH, &created_at).unwrap(),
        Some(seeded.clone())
    );
    assert_eq!(
        store
            .commit_at(MAIN_BRANCH, "2000-01-01T00:00:00Z")
            .unwrap(),
        None
    );
    assert_eq!(store.resolve_graph_ref(None, None).unwrap(), seeded);

    let window = store.load_commit_window(&seeded, 1, 10).unwrap();
    assert_eq!(window.total_nodes, 2);
    assert_eq!(window.graph.list_nodes().unwrap().len(), 1);

    let unknown = GraphCommitId::new("gc0000000000000000000000000000000000000000").unwrap();
    assert!(matches!(
        store.resolve_graph_ref(None, Some(&unknown)),
        Err(GraphStorageError::UnknownGraphCommit { .. })
    ));

    let _ = fs::remove_dir_all(root.path());
}

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

//! Issue #317: every applied canonical transaction is an addressable graph
//! commit that survives reopen, crashes and torn writes.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use graph_core::{Graph, NodeInput, NodePatch, PropertyValue, RelationshipInput};
use graph_storage::{
    CanonicalEngineStore, CanonicalProjectionRequest, CanonicalStoreOptions, DurableTransactionId,
    GraphId, GraphStorageError, HistoryRecordKind, MAIN_BRANCH, RecordFormat, StorageManifest,
    StorageRoot, StorageTimestamp, StorageVersion, TransitionCommitContext, create_storage_root,
};

fn empty_store(test_name: &str) -> StorageRoot {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path: PathBuf = std::env::temp_dir().join(format!(
        "corrobore-issue-317-{test_name}-{}-{unique}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&path);
    create_storage_root(
        path,
        StorageManifest {
            storage_version: StorageVersion::V1,
            graph_id: GraphId {
                value: "graph--issue-317".to_owned(),
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

fn tx(value: &str) -> DurableTransactionId {
    DurableTransactionId::new(value).unwrap()
}

fn history_log(root: &StorageRoot) -> PathBuf {
    root.path().join("history").join("graph_history.log")
}

/// Create `count` indicators named `prefix-N` in one transition.
fn create_indicators(store: &mut CanonicalEngineStore, prefix: &str, count: usize, id: &str) {
    let previous = store
        .load_projection(CanonicalProjectionRequest::all())
        .unwrap();
    let mut current = previous.clone();
    for index in 0..count {
        current
            .create_node(
                NodeInput::new(["Indicator"])
                    .with_property("name", PropertyValue::String(format!("{prefix}-{index}"))),
            )
            .unwrap();
    }
    store
        .commit_transition(&previous, &current, tx(id), None)
        .unwrap();
}

#[test]
fn fresh_store_starts_history_with_an_empty_genesis_on_main() {
    let root = empty_store("genesis");
    let mut store = open(&root);

    let head = store.graph_head(MAIN_BRANCH).unwrap();
    let genesis = store.graph_commit(&head).unwrap();
    assert!(genesis.parents.is_empty());
    assert_eq!(genesis.generation, 0);
    assert!(genesis.changes.is_empty());
    assert_eq!(genesis.branch, MAIN_BRANCH);

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn applied_transition_publishes_one_main_commit_with_its_changes() {
    let root = empty_store("commit");
    let mut store = open(&root);
    let genesis = store.graph_head(MAIN_BRANCH).unwrap();

    let previous = Graph::new();
    let mut current = Graph::new();
    let indicator = current
        .create_node(
            NodeInput::new(["Indicator"])
                .with_property("name", PropertyValue::String("c2.example".to_owned())),
        )
        .unwrap();
    let malware = current.create_node(NodeInput::new(["Malware"])).unwrap();
    current
        .create_relationship(
            RelationshipInput::new(indicator.clone(), "indicates", malware.clone()).unwrap(),
        )
        .unwrap();
    store
        .commit_transition_with_context(
            &previous,
            &current,
            tx("tx--issue-317-commit"),
            TransitionCommitContext {
                audit_events: vec!["test".to_owned()],
                actor: Some("analyst@example".to_owned()),
                message: Some("seed indicator".to_owned()),
                ..TransitionCommitContext::default()
            },
            None,
        )
        .unwrap();

    let head = store.graph_head(MAIN_BRANCH).unwrap();
    assert_ne!(head, genesis);
    let commit = store.graph_commit(&head).unwrap();
    assert_eq!(commit.parents, vec![genesis]);
    assert_eq!(commit.generation, 1);
    assert_eq!(
        commit.transaction_id.as_deref(),
        Some("tx--issue-317-commit")
    );
    assert_eq!(commit.actor.as_deref(), Some("analyst@example"));
    assert_eq!(commit.message.as_deref(), Some("seed indicator"));
    assert_eq!(commit.node_change_count(), 2);
    assert_eq!(commit.relationship_change_count(), 1);
    let relationship = commit
        .changes
        .iter()
        .find(|change| change.kind == HistoryRecordKind::Relationship)
        .unwrap();
    assert_eq!(relationship.labels, vec!["indicates".to_owned()]);
    assert_eq!(relationship.source.as_deref(), Some(indicator.as_str()));
    assert_eq!(relationship.target.as_deref(), Some(malware.as_str()));

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn idempotent_transaction_replay_does_not_publish_a_second_commit() {
    let root = empty_store("idempotent");
    let mut store = open(&root);
    let previous = Graph::new();
    let mut current = Graph::new();
    current.create_node(NodeInput::new(["Indicator"])).unwrap();

    store
        .commit_transition(&previous, &current, tx("tx--issue-317-same"), None)
        .unwrap();
    let head = store.graph_head(MAIN_BRANCH).unwrap();
    let outcome = store
        .commit_transition(&previous, &current, tx("tx--issue-317-same"), None)
        .unwrap();

    assert!(!outcome.applied);
    assert_eq!(store.graph_head(MAIN_BRANCH).unwrap(), head);
    let page = store.list_graph_commits(MAIN_BRANCH, None, 10).unwrap();
    assert_eq!(page.commits.len(), 2);

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn commits_survive_reopen_with_identical_ids() {
    let root = empty_store("reopen");
    let mut store = open(&root);
    create_indicators(&mut store, "first", 2, "tx--issue-317-reopen-1");
    create_indicators(&mut store, "second", 1, "tx--issue-317-reopen-2");
    let before = store.list_graph_commits(MAIN_BRANCH, None, 10).unwrap();
    drop(store);

    let mut reopened = open(&root);
    let after = reopened.list_graph_commits(MAIN_BRANCH, None, 10).unwrap();
    let ids = |page: &graph_storage::GraphCommitPage| {
        page.commits
            .iter()
            .map(|commit| commit.commit_id.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&before), ids(&after));
    assert_eq!(after.commits.len(), 3);
    assert_eq!(
        after.commits[0].transaction_id.as_deref(),
        Some("tx--issue-317-reopen-2")
    );

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn listing_pages_newest_first_with_a_cursor() {
    let root = empty_store("paging");
    let mut store = open(&root);
    for index in 0..5 {
        create_indicators(
            &mut store,
            "page",
            1,
            &format!("tx--issue-317-page-{index}"),
        );
    }

    let first = store.list_graph_commits(MAIN_BRANCH, None, 2).unwrap();
    assert_eq!(first.commits.len(), 2);
    let cursor = first.next_cursor.clone().unwrap();
    let second = store
        .list_graph_commits(MAIN_BRANCH, Some(&cursor), 2)
        .unwrap();
    let third = store
        .list_graph_commits(MAIN_BRANCH, second.next_cursor.as_ref(), 2)
        .unwrap();
    assert_eq!(third.commits.len(), 2);
    assert!(third.next_cursor.is_none());
    assert!(
        third.commits[1].parents.is_empty(),
        "last page ends at genesis"
    );
    assert!(first.commits[0].generation > second.commits[0].generation);

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn head_view_matches_the_canonical_catalog() {
    let root = empty_store("view");
    let mut store = open(&root);
    create_indicators(&mut store, "view", 3, "tx--issue-317-view-1");
    let previous = store
        .load_projection(CanonicalProjectionRequest::all())
        .unwrap();
    let mut current = previous.clone();
    let first = previous.current_node_records().unwrap()[0].clone();
    current
        .update_node(
            first.id(),
            NodePatch::default().set_property("name", PropertyValue::String("renamed".to_owned())),
        )
        .unwrap();
    store
        .commit_transition(&previous, &current, tx("tx--issue-317-view-2"), None)
        .unwrap();

    let head = store.graph_head(MAIN_BRANCH).unwrap();
    let catalog = store.catalog().clone();
    let view = store.graph_history().unwrap().view(&head).unwrap();
    assert_eq!(view.nodes.len(), catalog.latest_node_records.len());
    for (node_id, entry) in &catalog.latest_node_records {
        let visible = &view.nodes[node_id.as_str()];
        match &entry.graph_record_version {
            Some(graph_storage::GraphRecordVersion::Node { version_id, .. }) => {
                assert_eq!(&visible.version_id, version_id.as_str());
            }
            other => panic!("unexpected catalog version {other:?}"),
        }
    }

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn store_without_history_gets_a_genesis_holding_its_current_records() {
    let root = empty_store("bootstrap");
    let mut store = open(&root);
    create_indicators(&mut store, "legacy", 2, "tx--issue-317-bootstrap");
    drop(store);
    fs::remove_dir_all(root.path().join("history")).unwrap();

    let mut reopened = open(&root);
    let page = reopened.list_graph_commits(MAIN_BRANCH, None, 10).unwrap();
    assert_eq!(page.commits.len(), 1, "only a genesis commit");
    let genesis = &page.commits[0];
    assert!(genesis.parents.is_empty());
    assert_eq!(genesis.node_change_count(), 2);
    assert!(genesis.mutation_sequence_number.is_some());

    // New writes continue from the genesis.
    create_indicators(&mut reopened, "after", 1, "tx--issue-317-bootstrap-2");
    let page = reopened.list_graph_commits(MAIN_BRANCH, None, 10).unwrap();
    assert_eq!(page.commits.len(), 2);

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn crash_between_wal_apply_and_history_append_is_reconciled_from_journals() {
    let root = empty_store("reconcile");
    let mut store = open(&root);
    create_indicators(&mut store, "kept", 1, "tx--issue-317-reconcile-1");
    let log = history_log(&root);
    let length_before_second = fs::metadata(&log).unwrap().len();
    create_indicators(&mut store, "lost", 2, "tx--issue-317-reconcile-2");
    drop(store);
    // Simulate a crash after the WAL apply but before the history append.
    OpenOptions::new()
        .write(true)
        .open(&log)
        .unwrap()
        .set_len(length_before_second)
        .unwrap();

    let mut reopened = open(&root);
    let page = reopened.list_graph_commits(MAIN_BRANCH, None, 10).unwrap();
    assert_eq!(page.commits.len(), 3);
    let reconciled = &page.commits[0];
    assert_eq!(
        reconciled.transaction_id.as_deref(),
        Some("tx--issue-317-reconcile-2")
    );
    assert_eq!(reconciled.node_change_count(), 2);
    assert!(reconciled.mutation_sequence_number.is_some());

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn torn_final_history_line_is_truncated_on_open() {
    let root = empty_store("torn");
    let mut store = open(&root);
    create_indicators(&mut store, "torn", 1, "tx--issue-317-torn");
    let head = store.graph_head(MAIN_BRANCH).unwrap();
    drop(store);
    let log = history_log(&root);
    let length = fs::metadata(&log).unwrap().len();
    OpenOptions::new()
        .append(true)
        .open(&log)
        .unwrap()
        .write_all(b"{\"entry\":{\"type\":\"commit\",\"commit_id\":\"gc")
        .unwrap();

    let mut reopened = open(&root);
    assert_eq!(reopened.graph_head(MAIN_BRANCH).unwrap(), head);
    assert_eq!(fs::metadata(&log).unwrap().len(), length);

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn corrupted_complete_history_line_refuses_to_open() {
    let root = empty_store("corrupt");
    let mut store = open(&root);
    create_indicators(&mut store, "a", 1, "tx--issue-317-corrupt-1");
    create_indicators(&mut store, "b", 1, "tx--issue-317-corrupt-2");
    drop(store);
    let log = history_log(&root);
    let text = fs::read_to_string(&log).unwrap();
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    lines[1] = lines[1].replacen("\"main\"", "\"evil\"", 1);
    fs::write(&log, format!("{}\n", lines.join("\n"))).unwrap();

    let error = CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default())
        .expect_err("a tampered complete line must not open");
    assert!(
        matches!(
            error,
            GraphStorageError::GraphHistoryCorrupted { line: 2, .. }
        ),
        "{error:?}"
    );

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn id_floor_covers_every_published_id() {
    let root = empty_store("floor");
    let mut store = open(&root);
    create_indicators(&mut store, "floor", 4, "tx--issue-317-floor");
    let floor = store.graph_history().unwrap().id_floor();
    assert!(floor.node >= 4, "{floor:?}");
    assert!(floor.node_version >= 4, "{floor:?}");

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn consistent_snapshot_and_restore_keep_the_commit_history() {
    let root = empty_store("snapshot");
    let mut store = open(&root);
    create_indicators(&mut store, "snap", 2, "tx--issue-317-snapshot");
    let before = store.list_graph_commits(MAIN_BRANCH, None, 10).unwrap();
    let snapshot_root = root.path().with_extension("snapshot");
    let restored_root = root.path().with_extension("restored");
    graph_storage::create_consistent_snapshot(
        store.root(),
        &snapshot_root,
        graph_storage::SnapshotRequest {
            created_at: "2026-11-15T00:00:00Z".to_owned(),
            encryption_key_id: None,
            retention_hook: None,
        },
    )
    .unwrap();
    drop(store);
    graph_storage::restore_consistent_snapshot(&snapshot_root, &restored_root, None).unwrap();

    let mut restored = open(&graph_storage::open_storage_root(&restored_root).unwrap());
    let after = restored.list_graph_commits(MAIN_BRANCH, None, 10).unwrap();
    assert_eq!(
        before
            .commits
            .iter()
            .map(|commit| commit.commit_id.clone())
            .collect::<Vec<_>>(),
        after
            .commits
            .iter()
            .map(|commit| commit.commit_id.clone())
            .collect::<Vec<_>>()
    );

    for path in [root.path().to_path_buf(), snapshot_root, restored_root] {
        let _ = fs::remove_dir_all(path);
    }
}

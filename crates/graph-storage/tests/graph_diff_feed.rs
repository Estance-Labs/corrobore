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

//! Issue #320: diffs, merge bases and the change feed between commits.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use graph_core::{Graph, NodeInput, NodePatch, PropertyValue};
use graph_storage::{
    CanonicalEngineStore, CanonicalProjectionRequest, CanonicalStoreOptions, DiffValues,
    DurableTransactionId, GraphCommitId, GraphId, GraphStorageError, HistoryRecordKind,
    MAIN_BRANCH, RecordChangeKind, RecordFormat, StorageManifest, StorageRoot, StorageTimestamp,
    StorageVersion, TransitionCommitContext, create_storage_root,
};

fn empty_store(test_name: &str) -> StorageRoot {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path: PathBuf = std::env::temp_dir().join(format!(
        "corrobore-issue-320-{test_name}-{}-{unique}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&path);
    create_storage_root(
        path,
        StorageManifest {
            storage_version: StorageVersion::V1,
            graph_id: GraphId {
                value: "graph--issue-320".to_owned(),
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

fn indicator(graph: &mut Graph, name: &str) -> graph_core::NodeId {
    graph
        .create_node(
            NodeInput::new(["Indicator"])
                .with_property("name", PropertyValue::String(name.to_owned())),
        )
        .unwrap()
}

fn write_main(store: &mut CanonicalEngineStore, id: &str, name: &str) -> graph_core::NodeId {
    let previous = store
        .load_projection(CanonicalProjectionRequest::all())
        .unwrap();
    let mut current = previous.clone();
    let node = indicator(&mut current, name);
    store
        .commit_transition(
            &previous,
            &current,
            DurableTransactionId::new(id).unwrap(),
            None,
        )
        .unwrap();
    node
}

fn write_branch(store: &mut CanonicalEngineStore, branch: &str, name: &str) -> graph_core::NodeId {
    let head = store.graph_head(branch).unwrap();
    let previous = store
        .load_commit_projection(&head, &CanonicalProjectionRequest::all())
        .unwrap();
    let mut current = previous.clone();
    let node = indicator(&mut current, name);
    store
        .commit_branch_transition(
            branch,
            &previous,
            &current,
            TransitionCommitContext {
                actor: Some("agent--ingest".to_owned()),
                message: Some(format!("add {name}")),
                ..TransitionCommitContext::default()
            },
            Some(&head),
        )
        .unwrap()
        .expect("a changed graph publishes a commit");
    node
}

fn update_main(store: &mut CanonicalEngineStore, id: &str, change: impl FnOnce(&mut Graph)) {
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

#[test]
fn diff_reports_added_changed_and_removed_records_with_values() {
    let root = empty_store("diff");
    let mut store = open(&root);
    let kept = write_main(&mut store, "tx--issue-320-a", "kept");
    let renamed = write_main(&mut store, "tx--issue-320-b", "before");
    let dropped = write_main(&mut store, "tx--issue-320-c", "dropped");
    let from = store.graph_head(MAIN_BRANCH).unwrap();
    update_main(&mut store, "tx--issue-320-d", |graph| {
        graph
            .update_node(
                &renamed,
                NodePatch::default()
                    .set_property("name", PropertyValue::String("after".to_owned())),
            )
            .unwrap();
        graph.tombstone_node(&dropped).unwrap();
    });
    let added = write_main(&mut store, "tx--issue-320-e", "added");
    let to = store.graph_head(MAIN_BRANCH).unwrap();

    let page = store.graph_diff(&from, &to, None, None, 100, true).unwrap();
    assert_eq!(page.totals.added, 1);
    assert_eq!(page.totals.changed, 1);
    assert_eq!(page.totals.removed, 1);
    let find = |id: &graph_core::NodeId| {
        page.entries
            .iter()
            .find(|entry| entry.record_id == id.as_str())
            .unwrap()
    };
    assert!(
        page.entries
            .iter()
            .all(|entry| entry.record_id != kept.as_str())
    );
    assert_eq!(find(&added).change, RecordChangeKind::Added);
    assert_eq!(find(&dropped).change, RecordChangeKind::Removed);
    let changed = find(&renamed);
    assert_eq!(changed.change, RecordChangeKind::Changed);
    match changed.values.as_ref().unwrap() {
        DiffValues::Node { before, after } => {
            assert_eq!(
                before.as_ref().unwrap().properties().get("name"),
                Some(&PropertyValue::String("before".to_owned()))
            );
            assert_eq!(
                after.as_ref().unwrap().properties().get("name"),
                Some(&PropertyValue::String("after".to_owned()))
            );
        }
        other => panic!("unexpected values {other:?}"),
    }

    // The reverse diff swaps added and removed.
    let reverse = store
        .graph_diff(&to, &from, None, None, 100, false)
        .unwrap();
    assert_eq!(reverse.totals.added, 1);
    assert_eq!(reverse.totals.removed, 1);
    assert!(reverse.entries.iter().all(|entry| entry.values.is_none()));

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn diff_pages_resume_from_their_cursor() {
    let root = empty_store("diff-paging");
    let mut store = open(&root);
    let from = store.graph_head(MAIN_BRANCH).unwrap();
    for index in 0..5 {
        write_main(
            &mut store,
            &format!("tx--issue-320-page-{index}"),
            &format!("n{index}"),
        );
    }
    let to = store.graph_head(MAIN_BRANCH).unwrap();

    let mut seen = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let page = store
            .graph_diff(
                &from,
                &to,
                Some(HistoryRecordKind::Node),
                cursor.as_deref(),
                2,
                false,
            )
            .unwrap();
        assert_eq!(page.totals.added, 5);
        seen.extend(page.entries.iter().map(|entry| entry.record_id.clone()));
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    let mut unique = seen.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 5);
    assert_eq!(seen, unique);

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn merge_base_of_a_branch_and_main_is_the_fork_point() {
    let root = empty_store("merge-base");
    let mut store = open(&root);
    write_main(&mut store, "tx--issue-320-mb", "base");
    let fork = store.graph_head(MAIN_BRANCH).unwrap();
    store
        .create_graph_branch("draft", None, None, None)
        .unwrap();
    write_branch(&mut store, "draft", "branch-side");
    write_main(&mut store, "tx--issue-320-mb-main", "main-side");

    let main_head = store.graph_head(MAIN_BRANCH).unwrap();
    let draft_head = store.graph_head("draft").unwrap();
    assert_eq!(
        store.graph_merge_base(&main_head, &draft_head).unwrap(),
        Some(fork.clone())
    );
    assert_eq!(
        store.graph_merge_base(&draft_head, &main_head).unwrap(),
        Some(fork)
    );
    let diff = store
        .graph_diff(&main_head, &draft_head, None, None, 10, false)
        .unwrap();
    assert_eq!(diff.totals.added, 1);
    assert_eq!(diff.totals.removed, 1);

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn change_feed_returns_commits_after_the_cursor_oldest_first() {
    let root = empty_store("feed");
    let mut store = open(&root);
    let genesis = store.graph_head(MAIN_BRANCH).unwrap();
    for index in 0..3 {
        write_main(
            &mut store,
            &format!("tx--issue-320-feed-{index}"),
            &format!("f{index}"),
        );
    }

    let all = store.graph_changes(MAIN_BRANCH, None, 10).unwrap();
    assert_eq!(all.commits.len(), 4, "genesis and three writes");
    assert_eq!(all.commits[0].commit_id, genesis);
    assert!(!all.has_more);

    let first = store.graph_changes(MAIN_BRANCH, Some(&genesis), 2).unwrap();
    assert_eq!(first.commits.len(), 2);
    assert!(first.has_more);
    let second = store
        .graph_changes(MAIN_BRANCH, first.next_cursor.as_ref(), 2)
        .unwrap();
    assert_eq!(second.commits.len(), 1);
    assert!(!second.has_more);
    assert_eq!(second.next_cursor.as_ref(), Some(&second.head));

    let caught_up = store
        .graph_changes(MAIN_BRANCH, second.next_cursor.as_ref(), 2)
        .unwrap();
    assert!(caught_up.commits.is_empty());
    assert_eq!(caught_up.next_cursor, second.next_cursor);

    let gone = GraphCommitId::new("gc0000000000000000000000000000000000000000").unwrap();
    assert!(matches!(
        store.graph_changes(MAIN_BRANCH, Some(&gone), 2),
        Err(GraphStorageError::ChangeFeedGap { .. })
    ));
    store
        .create_graph_branch("draft", None, None, None)
        .unwrap();
    let draft_commit = {
        write_branch(&mut store, "draft", "elsewhere");
        store.graph_head("draft").unwrap()
    };
    assert!(matches!(
        store.graph_changes(MAIN_BRANCH, Some(&draft_commit), 2),
        Err(GraphStorageError::GraphBranchConflict { .. })
    ));

    let _ = fs::remove_dir_all(root.path());
}

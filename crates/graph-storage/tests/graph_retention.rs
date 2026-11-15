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

//! Issue #324: history retention keeps exact views of retained commits.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use graph_core::{Graph, NodeInput, PropertyValue};
use graph_storage::{
    CanonicalEngineStore, CanonicalProjectionRequest, CanonicalStoreOptions, DurableTransactionId,
    GraphCommitId, GraphId, GraphMergeRequest, GraphStorageError, HistoryCleanupRequest,
    HistoryRecordKind, MAIN_BRANCH, RecordFormat, StorageManifest, StorageRoot, StorageTimestamp,
    StorageVersion, TransitionCommitContext, create_storage_root,
};

fn empty_store(test_name: &str) -> StorageRoot {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path: PathBuf = std::env::temp_dir().join(format!(
        "corrobore-issue-324-{test_name}-{}-{unique}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&path);
    create_storage_root(
        path,
        StorageManifest {
            storage_version: StorageVersion::V1,
            graph_id: GraphId {
                value: "graph--issue-324".to_owned(),
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

fn ids(store: &mut CanonicalEngineStore, branch: &str) -> Vec<GraphCommitId> {
    store
        .list_graph_commits(branch, None, 1_000)
        .unwrap()
        .commits
        .iter()
        .map(|commit| commit.commit_id.clone())
        .collect()
}

fn live_names(store: &mut CanonicalEngineStore, commit: &GraphCommitId) -> Vec<String> {
    let page = store
        .list_commit_records(commit, HistoryRecordKind::Node, None, None, 1_000, false)
        .unwrap();
    let mut names: Vec<String> = page
        .nodes
        .iter()
        .filter_map(|node| match node.properties().get("name") {
            Some(PropertyValue::String(value)) => Some(value.clone()),
            _ => None,
        })
        .collect();
    names.sort();
    names
}

fn cleanup(store: &mut CanonicalEngineStore, keep: usize) -> graph_storage::HistoryCleanupReport {
    store
        .cleanup_graph_history(&HistoryCleanupRequest {
            keep_commits: keep,
            keep_seconds: None,
            dry_run: false,
        })
        .unwrap()
}

#[test]
fn retention_keeps_the_window_and_a_baseline_with_exact_views() {
    let root = empty_store("window");
    let mut store = open(&root);
    for index in 0..8 {
        write_main(
            &mut store,
            &format!("tx--issue-324-w{index}"),
            &format!("n{index}"),
        );
    }
    let before = ids(&mut store, MAIN_BRANCH);
    assert_eq!(before.len(), 9);
    let head_names = live_names(&mut store, &before[0]);
    let boundary_names = live_names(&mut store, &before[3]);

    let preview = store
        .cleanup_graph_history(&HistoryCleanupRequest {
            keep_commits: 3,
            keep_seconds: None,
            dry_run: true,
        })
        .unwrap();
    assert_eq!(preview.pruned_commits, 5);
    assert_eq!(
        ids(&mut store, MAIN_BRANCH).len(),
        9,
        "a dry run writes nothing"
    );

    let report = cleanup(&mut store, 3);
    assert_eq!(report.pruned_commits, 5);
    assert_eq!(report.baselines, 1);
    assert!(report.compaction.history_bytes_after < report.compaction.history_bytes_before);

    let after = ids(&mut store, MAIN_BRANCH);
    assert_eq!(after, before[..4].to_vec());
    assert_eq!(live_names(&mut store, &after[0]), head_names);
    assert_eq!(live_names(&mut store, &after[3]), boundary_names);
    assert!(store.graph_commit(&after[3]).unwrap().baseline);
    assert!(matches!(
        store.graph_commit(&before[5]),
        Err(GraphStorageError::UnknownGraphCommit { .. })
    ));
    assert!(matches!(
        store.graph_changes(MAIN_BRANCH, Some(&before[6]), 10),
        Err(GraphStorageError::ChangeFeedGap { .. })
    ));

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn compacted_history_reopens_and_keeps_allocating_fresh_ids() {
    let root = empty_store("reopen");
    let mut store = open(&root);
    let mut created = Vec::new();
    for index in 0..5 {
        created.push(write_main(
            &mut store,
            &format!("tx--issue-324-r{index}"),
            &format!("r{index}"),
        ));
    }
    cleanup(&mut store, 1);
    let head = store.graph_head(MAIN_BRANCH).unwrap();
    drop(store);

    let mut reopened = open(&root);
    assert_eq!(reopened.graph_head(MAIN_BRANCH).unwrap(), head);
    let fresh = write_main(&mut reopened, "tx--issue-324-r-after", "after");
    assert!(!created.contains(&fresh));
    assert_eq!(
        ids(&mut reopened, MAIN_BRANCH).len(),
        3,
        "new commit, kept head, baseline"
    );

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn branch_payloads_of_deleted_branches_are_reclaimed() {
    let root = empty_store("payloads");
    let mut store = open(&root);
    write_main(&mut store, "tx--issue-324-p", "base");
    store.create_graph_branch("kept", None, None, None).unwrap();
    store
        .create_graph_branch("dropped", None, None, None)
        .unwrap();
    write_branch(&mut store, "kept", "kept-work");
    for index in 0..5 {
        write_branch(&mut store, "dropped", &format!("dropped-{index}"));
    }
    store.delete_graph_branch("dropped", true, None).unwrap();

    let report = cleanup(&mut store, 10);
    assert!(
        report.compaction.branch_payload_bytes_after
            < report.compaction.branch_payload_bytes_before,
        "{report:?}"
    );
    let kept = store.graph_head("kept").unwrap();
    assert_eq!(
        live_names(&mut store, &kept),
        vec!["base".to_owned(), "kept-work".to_owned()]
    );
    assert!(
        !root
            .path()
            .join("history")
            .join("branch_payloads.log")
            .exists()
    );

    write_branch(&mut store, "kept", "after-cleanup");
    drop(store);
    let mut reopened = open(&root);
    let kept = reopened.graph_head("kept").unwrap();
    assert_eq!(
        live_names(&mut reopened, &kept),
        vec![
            "after-cleanup".to_owned(),
            "base".to_owned(),
            "kept-work".to_owned()
        ]
    );

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn recent_commits_and_merges_survive_retention() {
    let root = empty_store("recent");
    let mut store = open(&root);
    write_main(&mut store, "tx--issue-324-m", "base");
    store
        .create_graph_branch("draft", None, None, None)
        .unwrap();
    write_branch(&mut store, "draft", "proposed");
    let merged = store
        .merge_graph_branch(GraphMergeRequest {
            source: "draft".to_owned(),
            delete_source: true,
            ..GraphMergeRequest::default()
        })
        .unwrap()
        .commit
        .unwrap();

    let report = store
        .cleanup_graph_history(&HistoryCleanupRequest {
            keep_commits: 1,
            keep_seconds: Some(3_600),
            dry_run: false,
        })
        .unwrap();
    assert_eq!(
        report.pruned_commits, 0,
        "everything is younger than an hour"
    );
    let commit = store.graph_commit(&merged).unwrap();
    assert_eq!(commit.parents.len(), 2);

    cleanup(&mut store, 1);
    let commit = store.graph_commit(&merged).unwrap();
    assert!(commit.parents.len() <= 2);
    assert_eq!(
        live_names(&mut store, &merged),
        vec!["base".to_owned(), "proposed".to_owned()]
    );

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn retention_keeps_the_merge_base_of_live_branches() {
    let root = empty_store("merge-base");
    let mut store = open(&root);
    write_main(&mut store, "tx--issue-324-mb-0", "base");
    store
        .create_graph_branch("long-lived", None, None, None)
        .unwrap();
    write_branch(&mut store, "long-lived", "branch-work");
    for index in 0..6 {
        write_main(
            &mut store,
            &format!("tx--issue-324-mb-{}", index + 1),
            &format!("main-{index}"),
        );
    }
    let main_head = store.graph_head(MAIN_BRANCH).unwrap();
    let branch_head = store.graph_head("long-lived").unwrap();
    let base = store.graph_merge_base(&main_head, &branch_head).unwrap();

    cleanup(&mut store, 1);
    assert_eq!(
        store.graph_merge_base(&main_head, &branch_head).unwrap(),
        base
    );
    let report = store
        .merge_graph_branch(GraphMergeRequest {
            source: "long-lived".to_owned(),
            ..GraphMergeRequest::default()
        })
        .unwrap();
    assert_eq!(
        report.outcome,
        graph_storage::MergeOutcomeKind::Merged,
        "{report:?}"
    );

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn the_commits_a_cleanup_prunes_are_listed_oldest_first() {
    // Issue #392: a cold tier archives exactly these before they go.
    let root = empty_store("pruned-list");
    let mut store = open(&root);
    for index in 0..8 {
        write_main(
            &mut store,
            &format!("tx--cold-w{index}"),
            &format!("n{index}"),
        );
    }
    store
        .create_graph_branch("drafts", None, None, None)
        .unwrap();
    write_branch(&mut store, "drafts", "draft");
    let before = ids(&mut store, MAIN_BRANCH);
    let request = HistoryCleanupRequest {
        keep_commits: 3,
        keep_seconds: None,
        dry_run: false,
    };
    let listed: Vec<GraphCommitId> = store
        .retention_pruned_commits(&request)
        .unwrap()
        .iter()
        .map(|commit| commit.commit_id.clone())
        .collect();
    // Still readable until the cleanup runs.
    for commit in &listed {
        assert!(store.graph_commit(commit).is_ok());
    }
    let report = store.cleanup_graph_history(&request).unwrap();
    assert_eq!(report.pruned_commits, listed.len());
    // Exactly the main commits that are gone after the cleanup, oldest first.
    let gone: Vec<GraphCommitId> = before
        .iter()
        .rev()
        .filter(|commit| store.graph_commit(commit).is_err())
        .cloned()
        .collect();
    assert_eq!(listed, gone);
    assert!(listed.len() >= 5, "{listed:?}");
    let _ = fs::remove_dir_all(root.path());
}

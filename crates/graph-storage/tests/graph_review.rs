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

//! Issue #348: revert one change of a branch to its merge-base version, and
//! record review decisions on exact branch heads.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use graph_core::{Graph, NodeId, NodeInput, NodePatch, PropertyValue, RelationshipInput};
use graph_storage::{
    CanonicalEngineStore, CanonicalProjectionRequest, CanonicalStoreOptions, DurableTransactionId,
    GraphCommitId, GraphId, GraphRevertRequest, GraphRevertRestoration, GraphReviewDecision,
    GraphStorageError, HistoryRecordKind, MAIN_BRANCH, RecordFormat, StorageManifest, StorageRoot,
    StorageTimestamp, StorageVersion, TransitionCommitContext, create_storage_root,
};

fn empty_store(test_name: &str) -> StorageRoot {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path: PathBuf = std::env::temp_dir().join(format!(
        "corrobore-issue-348-{test_name}-{}-{unique}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&path);
    create_storage_root(
        path,
        StorageManifest {
            storage_version: StorageVersion::V1,
            graph_id: GraphId {
                value: "graph--issue-348".to_owned(),
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

fn change_branch(store: &mut CanonicalEngineStore, branch: &str, change: impl FnOnce(&mut Graph)) {
    let head = store.graph_head(branch).unwrap();
    let previous = store
        .load_commit_projection(&head, &CanonicalProjectionRequest::all())
        .unwrap();
    let mut current = previous.clone();
    change(&mut current);
    store
        .commit_branch_transition(
            branch,
            &previous,
            &current,
            TransitionCommitContext::default(),
            Some(&head),
        )
        .unwrap()
        .expect("changed");
}

fn indicator(graph: &mut Graph, name: &str) -> NodeId {
    graph
        .create_node(
            NodeInput::new(["Indicator"])
                .with_property("name", PropertyValue::String(name.to_owned())),
        )
        .unwrap()
}

fn rename(graph: &mut Graph, id: &NodeId, name: &str) {
    graph
        .update_node(
            id,
            NodePatch::default().set_property("name", PropertyValue::String(name.to_owned())),
        )
        .unwrap();
}

/// Records that differ between the branch and the commit it split from main.
fn branch_changes(store: &mut CanonicalEngineStore, branch: &str) -> Vec<String> {
    let main = store.graph_head(MAIN_BRANCH).unwrap();
    let head = store.graph_head(branch).unwrap();
    let base = store.graph_merge_base(&main, &head).unwrap().unwrap();
    store
        .graph_diff(&base, &head, None, None, 100, false)
        .unwrap()
        .entries
        .into_iter()
        .map(|entry| entry.record_id)
        .collect()
}

fn revert(
    store: &mut CanonicalEngineStore,
    branch: &str,
    kind: HistoryRecordKind,
    record_id: &str,
    expected_head: Option<GraphCommitId>,
) -> Result<graph_storage::GraphRevertReport, GraphStorageError> {
    store.revert_graph_change(GraphRevertRequest {
        branch: branch.to_owned(),
        record_kind: kind,
        record_id: record_id.to_owned(),
        expected_head,
        actor: Some("reviewer".to_owned()),
    })
}

/// main holds `a` and `b`; `review` branches from it.
fn fixture(name: &str) -> (StorageRoot, CanonicalEngineStore, NodeId, NodeId) {
    let root = empty_store(name);
    let mut store = open(&root);
    let mut ids = Vec::new();
    update_main(&mut store, "tx--issue-348-seed", |graph| {
        ids.push(indicator(graph, "alpha"));
        ids.push(indicator(graph, "beta"));
    });
    store
        .create_graph_branch("review", None, None, Some("analyst".to_owned()))
        .unwrap();
    let b = ids.pop().unwrap();
    let a = ids.pop().unwrap();
    (root, store, a, b)
}

#[test]
fn reverting_a_modified_record_restores_its_base_version_on_the_branch() {
    let (_root, mut store, a, b) = fixture("modified");
    change_branch(&mut store, "review", |graph| {
        rename(graph, &a, "alpha-edited");
        rename(graph, &b, "beta-edited");
    });
    let before = store.graph_head("review").unwrap();
    let report = revert(
        &mut store,
        "review",
        HistoryRecordKind::Node,
        a.as_str(),
        Some(before.clone()),
    )
    .unwrap();
    assert_eq!(report.restored, GraphRevertRestoration::BaseVersion);
    assert_eq!(report.branch, "review");
    let head = store.graph_head("review").unwrap();
    assert_eq!(report.commit_id, head);
    assert_ne!(head, before);
    // Only the other change remains on the branch.
    assert_eq!(
        branch_changes(&mut store, "review"),
        [b.as_str().to_owned()]
    );
    let commit = store.graph_commit(&head).unwrap();
    assert_eq!(commit.actor.as_deref(), Some("reviewer"));
    assert!(commit.message.as_deref().unwrap().contains("revert"));
}

#[test]
fn reverting_an_added_record_removes_it_and_a_deleted_one_comes_back() {
    let (_root, mut store, a, _) = fixture("added-deleted");
    let mut added = None;
    change_branch(&mut store, "review", |graph| {
        added = Some(indicator(graph, "gamma"));
        graph.tombstone_node(&a).unwrap();
    });
    let added = added.unwrap();
    assert_eq!(branch_changes(&mut store, "review").len(), 2);

    let report = revert(
        &mut store,
        "review",
        HistoryRecordKind::Node,
        added.as_str(),
        None,
    )
    .unwrap();
    assert_eq!(report.restored, GraphRevertRestoration::Removed);
    let report = revert(
        &mut store,
        "review",
        HistoryRecordKind::Node,
        a.as_str(),
        None,
    )
    .unwrap();
    assert_eq!(report.restored, GraphRevertRestoration::Recreated);
    assert!(branch_changes(&mut store, "review").is_empty());

    // main never saw any of it.
    let main = store
        .load_projection(CanonicalProjectionRequest::all())
        .unwrap();
    assert_eq!(main.current_node_records().unwrap().len(), 2);
}

#[test]
fn reverts_are_refused_when_nothing_changed_or_the_head_moved() {
    let (_root, mut store, a, b) = fixture("refusals");
    change_branch(&mut store, "review", |graph| {
        rename(graph, &a, "alpha-edited")
    });

    let error = revert(
        &mut store,
        "review",
        HistoryRecordKind::Node,
        b.as_str(),
        None,
    )
    .unwrap_err();
    assert!(
        matches!(error, GraphStorageError::GraphRevertNoChange { .. }),
        "{error:?}"
    );

    let stale = store.graph_head(MAIN_BRANCH).unwrap();
    let error = revert(
        &mut store,
        "review",
        HistoryRecordKind::Node,
        a.as_str(),
        Some(stale),
    )
    .unwrap_err();
    assert!(
        matches!(error, GraphStorageError::GraphHeadMoved { .. }),
        "{error:?}"
    );

    let error = revert(
        &mut store,
        MAIN_BRANCH,
        HistoryRecordKind::Node,
        a.as_str(),
        None,
    )
    .unwrap_err();
    assert!(
        matches!(error, GraphStorageError::GraphBranchConflict { .. }),
        "{error:?}"
    );

    let error = revert(
        &mut store,
        "missing",
        HistoryRecordKind::Node,
        a.as_str(),
        None,
    )
    .unwrap_err();
    assert!(
        matches!(error, GraphStorageError::UnknownGraphBranch { .. }),
        "{error:?}"
    );
}

#[test]
fn a_revert_never_leaves_a_dangling_relationship() {
    let (_root, mut store, a, _) = fixture("dangling");
    let mut added = None;
    change_branch(&mut store, "review", |graph| {
        let gamma = indicator(graph, "gamma");
        graph
            .create_relationship(RelationshipInput::new(a.clone(), "LINKS", gamma.clone()).unwrap())
            .unwrap();
        added = Some(gamma);
    });
    let gamma = added.unwrap();
    let error = revert(
        &mut store,
        "review",
        HistoryRecordKind::Node,
        gamma.as_str(),
        None,
    )
    .unwrap_err();
    match error {
        GraphStorageError::GraphRevertBlocked { reason, .. } => {
            assert!(reason.contains("relationship"), "{reason}");
        }
        other => panic!("expected a blocked revert, got {other:?}"),
    }

    // Reverting the relationship first unblocks the node.
    let head = store.graph_head("review").unwrap();
    let view = store
        .load_commit_projection(&head, &CanonicalProjectionRequest::all())
        .unwrap();
    let relationship = view.current_relationship_records().unwrap().remove(0);
    let report = revert(
        &mut store,
        "review",
        HistoryRecordKind::Relationship,
        relationship.id().as_str(),
        None,
    )
    .unwrap();
    assert_eq!(report.restored, GraphRevertRestoration::Removed);
    revert(
        &mut store,
        "review",
        HistoryRecordKind::Node,
        gamma.as_str(),
        None,
    )
    .unwrap();
    assert!(branch_changes(&mut store, "review").is_empty());
}

#[test]
fn reviews_are_kept_per_head_and_survive_a_reopen() {
    let (root, mut store, a, _) = fixture("reviews");
    change_branch(&mut store, "review", |graph| {
        rename(graph, &a, "alpha-edited")
    });
    let first_head = store.graph_head("review").unwrap();
    store
        .record_graph_review(
            "review",
            &first_head,
            "alice",
            GraphReviewDecision::ChangesRequested,
            Some("rename it back".to_owned()),
            1_000,
        )
        .unwrap();
    revert(
        &mut store,
        "review",
        HistoryRecordKind::Node,
        a.as_str(),
        None,
    )
    .unwrap();
    change_branch(&mut store, "review", |graph| {
        indicator(graph, "gamma");
    });
    let second_head = store.graph_head("review").unwrap();
    let stale = store
        .record_graph_review(
            "review",
            &first_head,
            "bob",
            GraphReviewDecision::Approved,
            None,
            2_000,
        )
        .unwrap_err();
    assert!(
        matches!(stale, GraphStorageError::GraphHeadMoved { .. }),
        "{stale:?}"
    );
    store
        .record_graph_review(
            "review",
            &second_head,
            "bob",
            GraphReviewDecision::Approved,
            None,
            3_000,
        )
        .unwrap();
    let error = store
        .record_graph_review(
            MAIN_BRANCH,
            &second_head,
            "bob",
            GraphReviewDecision::Approved,
            None,
            4_000,
        )
        .unwrap_err();
    assert!(
        matches!(error, GraphStorageError::GraphBranchConflict { .. }),
        "{error:?}"
    );

    drop(store);
    let mut store = open(&root);
    let reviews = store.list_graph_reviews("review").unwrap();
    assert_eq!(reviews.len(), 2);
    assert_eq!(reviews[0].review.reviewer, "bob");
    assert!(reviews[0].current);
    assert_eq!(reviews[1].review.reviewer, "alice");
    assert_eq!(reviews[1].review.comment.as_deref(), Some("rename it back"));
    assert!(!reviews[1].current);
    assert!(store.list_graph_reviews("other").unwrap().is_empty());
}

#[test]
fn a_torn_review_line_is_dropped_on_open() {
    let (root, mut store, a, _) = fixture("torn-review");
    change_branch(&mut store, "review", |graph| {
        rename(graph, &a, "alpha-edited")
    });
    let head = store.graph_head("review").unwrap();
    store
        .record_graph_review(
            "review",
            &head,
            "alice",
            GraphReviewDecision::Approved,
            None,
            1,
        )
        .unwrap();
    drop(store);
    let log = root.path().join("history").join("reviews.log");
    let mut bytes = fs::read(&log).unwrap();
    bytes.extend_from_slice(b"{\"sha256\":\"tor");
    fs::write(&log, bytes).unwrap();
    let mut store = open(&root);
    assert_eq!(store.list_graph_reviews("review").unwrap().len(), 1);
    store
        .record_graph_review(
            "review",
            &head,
            "bob",
            GraphReviewDecision::Approved,
            None,
            2,
        )
        .unwrap();
    assert_eq!(store.list_graph_reviews("review").unwrap().len(), 2);
}

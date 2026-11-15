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

//! Issue #321: three-way branch merges with typed conflicts.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    fs::{self, OpenOptions},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use graph_core::{Graph, NodeId, NodeInput, NodePatch, PropertyValue, RelationshipInput};
use graph_storage::{
    CanonicalEngineStore, CanonicalProjectionRequest, CanonicalStoreOptions, DurableTransactionId,
    GraphId, GraphMergeRequest, GraphStorageError, HistoryRecordKind, MAIN_BRANCH,
    MergeConflictKind, MergeOutcomeKind, RecordFormat, StorageManifest, StorageRoot,
    StorageTimestamp, StorageVersion, TransitionCommitContext, create_storage_root,
};

fn empty_store(test_name: &str) -> StorageRoot {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path: PathBuf = std::env::temp_dir().join(format!(
        "corrobore-issue-321-{test_name}-{}-{unique}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&path);
    create_storage_root(
        path,
        StorageManifest {
            storage_version: StorageVersion::V1,
            graph_id: GraphId {
                value: "graph--issue-321".to_owned(),
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

fn rename(graph: &mut Graph, id: &NodeId, name: &str) {
    graph
        .update_node(
            id,
            NodePatch::default().set_property("name", PropertyValue::String(name.to_owned())),
        )
        .unwrap();
}

fn merge(
    store: &mut CanonicalEngineStore,
    source: &str,
    dry_run: bool,
) -> graph_storage::GraphMergeReport {
    store
        .merge_graph_branch(GraphMergeRequest {
            source: source.to_owned(),
            target: MAIN_BRANCH.to_owned(),
            dry_run,
            actor: Some("reviewer".to_owned()),
            ..GraphMergeRequest::default()
        })
        .unwrap()
}

fn main_names(store: &mut CanonicalEngineStore) -> Vec<String> {
    let graph = store
        .load_projection(CanonicalProjectionRequest::all())
        .unwrap();
    let mut names: Vec<String> = graph
        .current_node_records()
        .unwrap()
        .iter()
        .filter(|node| node.status() != graph_core::RecordStatus::Tombstoned)
        .filter_map(|node| match node.properties().get("name") {
            Some(PropertyValue::String(value)) => Some(value.clone()),
            _ => None,
        })
        .collect();
    names.sort();
    names
}

#[test]
fn fast_forward_merge_publishes_branch_work_on_main_with_two_parents() {
    let root = empty_store("fast-forward");
    let mut store = open(&root);
    write_main(&mut store, "tx--issue-321-ff", "base");
    let main_before = store.graph_head(MAIN_BRANCH).unwrap();
    store
        .create_graph_branch("draft", None, None, None)
        .unwrap();
    write_branch(&mut store, "draft", "proposed");
    let draft_head = store.graph_head("draft").unwrap();

    let preview = merge(&mut store, "draft", true);
    assert_eq!(preview.outcome, MergeOutcomeKind::FastForward);
    assert_eq!(preview.applied.len(), 1);
    assert!(preview.commit.is_none());
    assert_eq!(
        store.graph_head(MAIN_BRANCH).unwrap(),
        main_before,
        "dry run writes nothing"
    );

    let report = merge(&mut store, "draft", false);
    assert_eq!(report.outcome, MergeOutcomeKind::FastForward);
    let merged = store.graph_commit(report.commit.as_ref().unwrap()).unwrap();
    assert_eq!(merged.parents, vec![main_before, draft_head]);
    assert_eq!(merged.actor.as_deref(), Some("reviewer"));
    assert_eq!(
        main_names(&mut store),
        vec!["base".to_owned(), "proposed".to_owned()]
    );
    assert_eq!(store.catalog().latest_node_records.len(), 2);
    let draft = store
        .list_graph_branches()
        .unwrap()
        .into_iter()
        .find(|status| status.info.name == "draft")
        .unwrap();
    assert!(draft.merged);

    let again = merge(&mut store, "draft", false);
    assert_eq!(again.outcome, MergeOutcomeKind::AlreadyUpToDate);

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn divergent_changes_to_different_records_merge() {
    let root = empty_store("merged");
    let mut store = open(&root);
    let left = write_main(&mut store, "tx--issue-321-m-a", "left");
    let right = write_main(&mut store, "tx--issue-321-m-b", "right");
    store
        .create_graph_branch("draft", None, None, None)
        .unwrap();
    change_branch(&mut store, "draft", |graph| {
        rename(graph, &left, "left-on-branch")
    });
    update_main(&mut store, "tx--issue-321-m-c", |graph| {
        rename(graph, &right, "right-on-main")
    });

    let report = merge(&mut store, "draft", false);
    assert_eq!(report.outcome, MergeOutcomeKind::Merged);
    assert_eq!(
        main_names(&mut store),
        vec!["left-on-branch".to_owned(), "right-on-main".to_owned()]
    );

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn concurrent_updates_conflict_and_write_nothing() {
    let root = empty_store("concurrent");
    let mut store = open(&root);
    let shared = write_main(&mut store, "tx--issue-321-c-a", "shared");
    store
        .create_graph_branch("draft", None, None, None)
        .unwrap();
    change_branch(&mut store, "draft", |graph| {
        rename(graph, &shared, "branch")
    });
    update_main(&mut store, "tx--issue-321-c-b", |graph| {
        rename(graph, &shared, "main")
    });
    let main_before = store.graph_head(MAIN_BRANCH).unwrap();

    let report = merge(&mut store, "draft", false);
    assert_eq!(report.outcome, MergeOutcomeKind::Conflicted);
    assert_eq!(report.conflicts.len(), 1);
    let conflict = &report.conflicts[0];
    assert_eq!(conflict.kind, MergeConflictKind::ConcurrentUpdate);
    assert_eq!(conflict.record_id, shared.as_str());
    assert!(conflict.base_version.is_some() && conflict.source_version != conflict.target_version);
    assert_eq!(store.graph_head(MAIN_BRANCH).unwrap(), main_before);
    assert_eq!(main_names(&mut store), vec!["main".to_owned()]);

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn update_against_delete_conflicts() {
    let root = empty_store("update-delete");
    let mut store = open(&root);
    let shared = write_main(&mut store, "tx--issue-321-ud-a", "shared");
    store
        .create_graph_branch("draft", None, None, None)
        .unwrap();
    change_branch(&mut store, "draft", |graph| {
        rename(graph, &shared, "edited")
    });
    update_main(&mut store, "tx--issue-321-ud-b", |graph| {
        graph.tombstone_node(&shared).unwrap();
    });

    let report = merge(&mut store, "draft", false);
    assert_eq!(report.outcome, MergeOutcomeKind::Conflicted);
    assert_eq!(report.conflicts[0].kind, MergeConflictKind::UpdateDelete);

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn relationship_to_a_node_main_deleted_is_a_dangling_conflict() {
    let root = empty_store("dangling");
    let mut store = open(&root);
    let a = write_main(&mut store, "tx--issue-321-d-a", "a");
    let b = write_main(&mut store, "tx--issue-321-d-b", "b");
    store
        .create_graph_branch("draft", None, None, None)
        .unwrap();
    change_branch(&mut store, "draft", |graph| {
        graph
            .create_relationship(RelationshipInput::new(a.clone(), "LINKS", b.clone()).unwrap())
            .unwrap();
    });
    update_main(&mut store, "tx--issue-321-d-c", |graph| {
        graph.tombstone_node(&b).unwrap();
    });

    let report = merge(&mut store, "draft", false);
    assert_eq!(report.outcome, MergeOutcomeKind::Conflicted);
    assert_eq!(
        report.conflicts[0].kind,
        MergeConflictKind::DanglingRelationship
    );
    assert_eq!(
        report.conflicts[0].record_kind,
        HistoryRecordKind::Relationship
    );

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn merged_relationships_are_indexed_on_main() {
    let root = empty_store("relationships");
    let mut store = open(&root);
    let a = write_main(&mut store, "tx--issue-321-r-a", "a");
    store
        .create_graph_branch("draft", None, None, None)
        .unwrap();
    change_branch(&mut store, "draft", |graph| {
        let b = graph
            .create_node(
                NodeInput::new(["Malware"])
                    .with_property("name", PropertyValue::String("b".to_owned())),
            )
            .unwrap();
        graph
            .create_relationship(RelationshipInput::new(a.clone(), "INDICATES", b).unwrap())
            .unwrap();
    });
    let report = merge(&mut store, "draft", false);
    assert_eq!(report.outcome, MergeOutcomeKind::FastForward);

    let graph = store
        .load_projection(
            CanonicalProjectionRequest::for_label("Indicator")
                .with_relationships(Some("INDICATES".to_owned())),
        )
        .unwrap();
    assert_eq!(graph.list_relationships().unwrap().len(), 1);
    assert_eq!(store.catalog().latest_relationship_records.len(), 1);

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn merge_into_another_branch_reuses_the_source_payloads() {
    let root = empty_store("branch-target");
    let mut store = open(&root);
    write_main(&mut store, "tx--issue-321-bt", "base");
    store
        .create_graph_branch("integration", None, None, None)
        .unwrap();
    store
        .create_graph_branch("feature", None, None, None)
        .unwrap();
    write_branch(&mut store, "feature", "feature-work");

    let report = store
        .merge_graph_branch(GraphMergeRequest {
            source: "feature".to_owned(),
            target: "integration".to_owned(),
            delete_source: true,
            ..GraphMergeRequest::default()
        })
        .unwrap();
    assert_eq!(report.outcome, MergeOutcomeKind::FastForward);
    let integration = store.graph_head("integration").unwrap();
    let page = store
        .list_commit_records(&integration, HistoryRecordKind::Node, None, None, 10, false)
        .unwrap();
    assert_eq!(page.total, 2);
    assert!(matches!(
        store.graph_head("feature"),
        Err(GraphStorageError::UnknownGraphBranch { .. })
    ));
    assert_eq!(main_names(&mut store), vec!["base".to_owned()]);

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn reconciliation_keeps_the_merge_parent() {
    let root = empty_store("reconcile");
    let mut store = open(&root);
    write_main(&mut store, "tx--issue-321-rc", "base");
    store
        .create_graph_branch("draft", None, None, None)
        .unwrap();
    write_branch(&mut store, "draft", "proposed");
    let draft_head = store.graph_head("draft").unwrap();
    let log = root.path().join("history").join("graph_history.log");
    let before_merge = fs::metadata(&log).unwrap().len();
    merge(&mut store, "draft", false);
    drop(store);
    OpenOptions::new()
        .write(true)
        .open(&log)
        .unwrap()
        .set_len(before_merge)
        .unwrap();

    let mut reopened = open(&root);
    let head = reopened.graph_head(MAIN_BRANCH).unwrap();
    let commit = reopened.graph_commit(&head).unwrap();
    assert_eq!(commit.parents.len(), 2);
    assert_eq!(commit.parents[1], draft_head);

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn merge_refuses_a_moved_target_and_self_merges() {
    let root = empty_store("guards");
    let mut store = open(&root);
    write_main(&mut store, "tx--issue-321-g", "base");
    let stale = store.graph_head(MAIN_BRANCH).unwrap();
    store
        .create_graph_branch("draft", None, None, None)
        .unwrap();
    write_branch(&mut store, "draft", "proposed");
    write_main(&mut store, "tx--issue-321-g2", "moved");

    let moved = store.merge_graph_branch(GraphMergeRequest {
        source: "draft".to_owned(),
        expected_target_head: Some(stale),
        ..GraphMergeRequest::default()
    });
    assert!(
        matches!(moved, Err(GraphStorageError::GraphHeadMoved { .. })),
        "{moved:?}"
    );
    let itself = store.merge_graph_branch(GraphMergeRequest {
        source: MAIN_BRANCH.to_owned(),
        ..GraphMergeRequest::default()
    });
    assert!(matches!(
        itself,
        Err(GraphStorageError::GraphBranchConflict { .. })
    ));

    let _ = fs::remove_dir_all(root.path());
}

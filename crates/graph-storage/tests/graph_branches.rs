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

//! Issue #319: branches are named heads whose writes stay invisible on main
//! until merged, survive reopen and never leak into a catalog rebuild.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use graph_core::{Graph, NodeInput, PropertyValue};
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
        "corrobore-issue-319-{test_name}-{}-{unique}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&path);
    create_storage_root(
        path,
        StorageManifest {
            storage_version: StorageVersion::V1,
            graph_id: GraphId {
                value: "graph--issue-319".to_owned(),
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

fn names(store: &mut CanonicalEngineStore, branch: &str) -> Vec<String> {
    let head = store.graph_head(branch).unwrap();
    let page = store
        .list_commit_records(&head, HistoryRecordKind::Node, None, None, 100, false)
        .unwrap();
    let mut names: Vec<String> = page
        .nodes
        .iter()
        .map(|node| match node.properties().get("name") {
            Some(PropertyValue::String(value)) => value.clone(),
            _ => String::new(),
        })
        .collect();
    names.sort();
    names
}

#[test]
fn branch_writes_are_isolated_from_main() {
    let root = empty_store("isolation");
    let mut store = open(&root);
    write_main(&mut store, "tx--issue-319-base", "base");
    let main_head = store.graph_head(MAIN_BRANCH).unwrap();

    let branch = store
        .create_graph_branch("agent/ingest-42", None, None, Some("analyst".to_owned()))
        .unwrap();
    assert_eq!(branch.head, main_head);
    write_branch(&mut store, "agent/ingest-42", "proposed");

    assert_eq!(names(&mut store, MAIN_BRANCH), vec!["base".to_owned()]);
    assert_eq!(
        names(&mut store, "agent/ingest-42"),
        vec!["base".to_owned(), "proposed".to_owned()]
    );
    assert_eq!(store.graph_head(MAIN_BRANCH).unwrap(), main_head);
    assert_eq!(
        store.catalog().latest_node_records.len(),
        1,
        "catalog is main only"
    );

    let statuses = store.list_graph_branches().unwrap();
    assert_eq!(statuses[0].info.name, MAIN_BRANCH);
    let agent = statuses
        .iter()
        .find(|status| status.info.name == "agent/ingest-42")
        .unwrap();
    assert!(!agent.merged);
    assert_eq!(agent.commits_ahead, 1);
    let head = store.graph_head("agent/ingest-42").unwrap();
    let commit = store.graph_commit(&head).unwrap();
    assert_eq!(commit.actor.as_deref(), Some("agent--ingest"));
    assert_eq!(commit.parents, vec![main_head]);

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn branch_and_main_never_mint_the_same_id() {
    let root = empty_store("ids");
    let mut store = open(&root);
    write_main(&mut store, "tx--issue-319-ids-base", "base");
    store
        .create_graph_branch("draft", None, None, None)
        .unwrap();
    let on_branch = write_branch(&mut store, "draft", "branch-only");
    let on_main = write_main(&mut store, "tx--issue-319-ids-main", "main-only");
    assert_ne!(on_branch, on_main);

    drop(store);
    let mut reopened = open(&root);
    let again = write_main(&mut reopened, "tx--issue-319-ids-main-2", "main-again");
    assert_ne!(again, on_branch);

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn branches_and_their_payloads_survive_reopen() {
    let root = empty_store("reopen");
    let mut store = open(&root);
    write_main(&mut store, "tx--issue-319-reopen", "base");
    store
        .create_graph_branch("draft", None, None, None)
        .unwrap();
    write_branch(&mut store, "draft", "kept");
    let head = store.graph_head("draft").unwrap();
    drop(store);

    let mut reopened = open(&root);
    assert_eq!(reopened.graph_head("draft").unwrap(), head);
    assert_eq!(
        names(&mut reopened, "draft"),
        vec!["base".to_owned(), "kept".to_owned()]
    );

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn catalog_rebuild_ignores_branch_payloads() {
    let root = empty_store("rebuild");
    let mut store = open(&root);
    write_main(&mut store, "tx--issue-319-rebuild", "base");
    store
        .create_graph_branch("draft", None, None, None)
        .unwrap();
    write_branch(&mut store, "draft", "secret");
    drop(store);
    fs::remove_file(root.path().join("catalog").join("catalog_metadata.json")).unwrap();

    // Recovery rebuilds the catalog from the WAL and canonical journals.
    let mut reopened = open(&root);
    assert_eq!(reopened.catalog().latest_node_records.len(), 1);
    assert_eq!(names(&mut reopened, MAIN_BRANCH), vec!["base".to_owned()]);

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn branches_start_from_any_retained_commit() {
    let root = empty_store("from-commit");
    let mut store = open(&root);
    write_main(&mut store, "tx--issue-319-first", "first");
    let first = store.graph_head(MAIN_BRANCH).unwrap();
    write_main(&mut store, "tx--issue-319-second", "second");

    store
        .create_graph_branch("replay", None, Some(&first), None)
        .unwrap();
    assert_eq!(names(&mut store, "replay"), vec!["first".to_owned()]);
    store
        .create_graph_branch("nested", Some("replay"), None, None)
        .unwrap();
    assert_eq!(store.graph_head("nested").unwrap(), first);

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn deletion_guards_unmerged_work_and_main() {
    let root = empty_store("delete");
    let mut store = open(&root);
    write_main(&mut store, "tx--issue-319-delete", "base");
    store
        .create_graph_branch("empty", None, None, None)
        .unwrap();
    store.create_graph_branch("work", None, None, None).unwrap();
    write_branch(&mut store, "work", "unmerged");

    store.delete_graph_branch("empty", false, None).unwrap();
    assert!(matches!(
        store.delete_graph_branch("work", false, None),
        Err(GraphStorageError::GraphBranchConflict { .. })
    ));
    store.delete_graph_branch("work", true, None).unwrap();
    assert!(matches!(
        store.delete_graph_branch(MAIN_BRANCH, true, None),
        Err(GraphStorageError::GraphBranchConflict { .. })
    ));
    assert!(matches!(
        store.graph_head("work"),
        Err(GraphStorageError::UnknownGraphBranch { .. })
    ));
    for invalid in ["", "main", "a b", "../x", "/lead", "trail/"] {
        assert!(
            matches!(
                store.create_graph_branch(invalid, None, None, None),
                Err(GraphStorageError::InvalidGraphBranchName { .. })
            ),
            "{invalid:?} must be refused"
        );
    }
    store.create_graph_branch("work", None, None, None).unwrap();

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn branch_write_from_a_stale_head_is_refused() {
    let root = empty_store("stale");
    let mut store = open(&root);
    write_main(&mut store, "tx--issue-319-stale", "base");
    store
        .create_graph_branch("draft", None, None, None)
        .unwrap();
    let stale = store.graph_head("draft").unwrap();
    let previous = store
        .load_commit_projection(&stale, &CanonicalProjectionRequest::all())
        .unwrap();
    write_branch(&mut store, "draft", "first-writer");

    let mut current = previous.clone();
    indicator(&mut current, "second-writer");
    let refused = store.commit_branch_transition(
        "draft",
        &previous,
        &current,
        TransitionCommitContext::default(),
        Some(&stale),
    );
    assert!(
        matches!(refused, Err(GraphStorageError::GraphHeadMoved { .. })),
        "{refused:?}"
    );
    assert_eq!(
        names(&mut store, "draft"),
        vec!["base".to_owned(), "first-writer".to_owned()]
    );

    let _ = fs::remove_dir_all(root.path());
}

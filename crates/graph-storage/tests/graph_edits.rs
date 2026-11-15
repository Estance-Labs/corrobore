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

//! Issues #352 and #366: batches of node and relationship edits published as
//! one commit, on main through the canonical WAL and on branches through
//! branch payloads.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use std::collections::BTreeMap;

use graph_core::{Graph, NodeId, NodeInput, PropertyValue, RelationshipInput};
use graph_storage::{
    CanonicalEngineStore, CanonicalProjectionRequest, CanonicalStoreOptions, DurableTransactionId,
    GraphEdit, GraphEditRequest, GraphId, GraphStorageError, MAIN_BRANCH, RecordFormat,
    StorageManifest, StorageRoot, StorageTimestamp, StorageVersion, create_storage_root,
};

fn empty_store(test_name: &str) -> StorageRoot {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path: PathBuf = std::env::temp_dir().join(format!(
        "corrobore-issue-352-{test_name}-{}-{unique}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&path);
    create_storage_root(
        path,
        StorageManifest {
            storage_version: StorageVersion::V1,
            graph_id: GraphId {
                value: "graph--issue-352".to_owned(),
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

fn indicator(graph: &mut Graph, name: &str) -> NodeId {
    graph
        .create_node(
            NodeInput::new(["Indicator"])
                .with_property("name", PropertyValue::String(name.to_owned()))
                .with_property("alias", PropertyValue::String(format!("{name}-alias"))),
        )
        .unwrap()
}

fn text(value: &str) -> PropertyValue {
    PropertyValue::String(value.to_owned())
}

fn set(node: &NodeId, key: &str, value: PropertyValue) -> GraphEdit {
    GraphEdit::SetProperties {
        node_id: node.as_str().to_owned(),
        set: BTreeMap::from([(key.to_owned(), value)]),
        remove: Vec::new(),
    }
}

fn request(branch: &str, edits: Vec<GraphEdit>) -> GraphEditRequest {
    GraphEditRequest {
        branch: branch.to_owned(),
        expected_head: None,
        actor: Some("analyst".to_owned()),
        message: Some("tidy indicators".to_owned()),
        edits,
    }
}

/// Live nodes of a branch head by id, with their properties.
fn nodes_at(
    store: &mut CanonicalEngineStore,
    branch: &str,
) -> BTreeMap<String, BTreeMap<String, PropertyValue>> {
    let head = store.graph_head(branch).unwrap();
    store
        .load_commit_projection(&head, &CanonicalProjectionRequest::all())
        .unwrap()
        .current_node_records()
        .unwrap()
        .into_iter()
        .filter(|node| node.status() != graph_core::RecordStatus::Tombstoned)
        .map(|node| {
            (
                node.id().as_str().to_owned(),
                node.properties()
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
            )
        })
        .collect()
}

fn fixture(name: &str) -> (StorageRoot, CanonicalEngineStore, NodeId, NodeId) {
    let root = empty_store(name);
    let mut store = open(&root);
    let mut ids = Vec::new();
    update_main(&mut store, "tx--issue-352-seed", |graph| {
        ids.push(indicator(graph, "alpha"));
        ids.push(indicator(graph, "beta"));
    });
    let b = ids.pop().unwrap();
    let a = ids.pop().unwrap();
    (root, store, a, b)
}

#[test]
fn a_batch_on_main_publishes_one_commit_through_the_wal() {
    let (root, mut store, a, b) = fixture("main-batch");
    let before = store.graph_head(MAIN_BRANCH).unwrap();
    let mut req = request(
        MAIN_BRANCH,
        vec![
            set(&a, "name", text("alpha-renamed")),
            GraphEdit::SetProperties {
                node_id: a.as_str().to_owned(),
                set: BTreeMap::from([("score".to_owned(), PropertyValue::Integer(7))]),
                remove: vec!["alias".to_owned()],
            },
            GraphEdit::CreateNode {
                labels: vec!["Indicator".to_owned()],
                properties: BTreeMap::from([("name".to_owned(), text("gamma"))]),
            },
            GraphEdit::DeleteNode {
                node_id: b.as_str().to_owned(),
            },
        ],
    );
    req.expected_head = Some(before.clone());
    let report = store.apply_graph_edits(req).unwrap();
    let head = store.graph_head(MAIN_BRANCH).unwrap();
    assert_eq!(report.commit_id, head);
    assert_eq!(report.branch, MAIN_BRANCH);
    assert_eq!(report.created.len(), 1);
    assert_eq!(report.changed, 3);
    let commit = store.graph_commit(&head).unwrap();
    assert_eq!(commit.parents, vec![before]);
    assert_eq!(commit.actor.as_deref(), Some("analyst"));
    assert_eq!(commit.message.as_deref(), Some("tidy indicators"));
    assert!(
        commit.transaction_id.is_some(),
        "main edits go through the WAL"
    );

    let nodes = nodes_at(&mut store, MAIN_BRANCH);
    assert_eq!(nodes.len(), 2);
    let alpha = &nodes[a.as_str()];
    assert_eq!(alpha["name"], text("alpha-renamed"));
    assert_eq!(alpha["score"], PropertyValue::Integer(7));
    assert!(!alpha.contains_key("alias"), "removed keys are gone");
    assert_eq!(nodes[report.created[0].as_str()]["name"], text("gamma"));

    // The canonical store agrees after a reopen, and a later write never
    // reuses the created id.
    drop(store);
    let mut store = open(&root);
    assert_eq!(nodes_at(&mut store, MAIN_BRANCH), nodes);
    let mut later = None;
    update_main(&mut store, "tx--issue-352-later", |graph| {
        later = Some(indicator(graph, "delta"));
    });
    assert!(!report.created.contains(&later.unwrap().as_str().to_owned()));
}

#[test]
fn a_batch_on_a_branch_leaves_main_untouched() {
    let (_root, mut store, a, _) = fixture("branch-batch");
    store
        .create_graph_branch("edits", None, None, None)
        .unwrap();
    let main_before = nodes_at(&mut store, MAIN_BRANCH);
    let report = store
        .apply_graph_edits(request("edits", vec![set(&a, "name", text("on-branch"))]))
        .unwrap();
    assert_eq!(report.commit_id, store.graph_head("edits").unwrap());
    assert_eq!(
        nodes_at(&mut store, "edits")[a.as_str()]["name"],
        text("on-branch")
    );
    assert_eq!(nodes_at(&mut store, MAIN_BRANCH), main_before);
}

#[test]
fn a_failing_edit_writes_nothing() {
    let (_root, mut store, a, b) = fixture("atomic");
    let before = store.graph_head(MAIN_BRANCH).unwrap();
    let nodes = nodes_at(&mut store, MAIN_BRANCH);
    let error = store
        .apply_graph_edits(request(
            MAIN_BRANCH,
            vec![
                set(&a, "name", text("renamed")),
                set(&NodeId::new("node--missing").unwrap(), "name", text("x")),
            ],
        ))
        .unwrap_err();
    assert!(
        matches!(error, GraphStorageError::GraphEditNodeNotFound { .. }),
        "{error:?}"
    );
    assert_eq!(store.graph_head(MAIN_BRANCH).unwrap(), before);
    assert_eq!(nodes_at(&mut store, MAIN_BRANCH), nodes);

    for edits in [
        Vec::new(),
        vec![GraphEdit::CreateNode {
            labels: Vec::new(),
            properties: BTreeMap::new(),
        }],
        vec![
            set(&b, "name", text("x")),
            GraphEdit::DeleteNode {
                node_id: b.as_str().to_owned(),
            },
        ],
        vec![GraphEdit::SetProperties {
            node_id: a.as_str().to_owned(),
            set: BTreeMap::from([(" ".to_owned(), text("x"))]),
            remove: Vec::new(),
        }],
    ] {
        let error = store
            .apply_graph_edits(request(MAIN_BRANCH, edits.clone()))
            .unwrap_err();
        assert!(
            matches!(error, GraphStorageError::GraphEditInvalid { .. }),
            "{edits:?}: {error:?}"
        );
    }
    assert_eq!(store.graph_head(MAIN_BRANCH).unwrap(), before);
}

#[test]
fn edits_are_refused_on_a_moved_head_or_a_linked_node() {
    let (_root, mut store, a, b) = fixture("refusals");
    let stale = store.graph_head(MAIN_BRANCH).unwrap();
    update_main(&mut store, "tx--issue-352-link", |graph| {
        graph
            .create_relationship(RelationshipInput::new(a.clone(), "LINKS", b.clone()).unwrap())
            .unwrap();
    });
    let mut req = request(MAIN_BRANCH, vec![set(&a, "name", text("x"))]);
    req.expected_head = Some(stale);
    let error = store.apply_graph_edits(req).unwrap_err();
    assert!(
        matches!(error, GraphStorageError::GraphHeadMoved { .. }),
        "{error:?}"
    );

    let error = store
        .apply_graph_edits(request(
            MAIN_BRANCH,
            vec![GraphEdit::DeleteNode {
                node_id: b.as_str().to_owned(),
            }],
        ))
        .unwrap_err();
    match error {
        GraphStorageError::GraphEditBlocked { reason, .. } => {
            assert!(reason.contains("relationship"), "{reason}");
        }
        other => panic!("expected a blocked delete, got {other:?}"),
    }

    let error = store
        .apply_graph_edits(request("missing", vec![set(&a, "name", text("x"))]))
        .unwrap_err();
    assert!(
        matches!(error, GraphStorageError::UnknownGraphBranch { .. }),
        "{error:?}"
    );
}

fn relationships_at(
    store: &mut CanonicalEngineStore,
    branch: &str,
) -> BTreeMap<String, (String, String, String, BTreeMap<String, PropertyValue>)> {
    let head = store.graph_head(branch).unwrap();
    store
        .load_commit_projection(&head, &CanonicalProjectionRequest::all())
        .unwrap()
        .current_relationship_records()
        .unwrap()
        .into_iter()
        .filter(|relationship| relationship.status() != graph_core::RecordStatus::Tombstoned)
        .map(|relationship| {
            (
                relationship.id().as_str().to_owned(),
                (
                    relationship.source().as_str().to_owned(),
                    relationship.rel_type().as_str().to_owned(),
                    relationship.target().as_str().to_owned(),
                    relationship
                        .properties()
                        .iter()
                        .map(|(key, value)| (key.clone(), value.clone()))
                        .collect(),
                ),
            )
        })
        .collect()
}

#[test]
fn relationships_are_created_edited_and_deleted_in_one_batch() {
    let (_root, mut store, a, b) = fixture("relationships");
    // One batch: a new node, a relationship to it ($0), one between a and b.
    let report = store
        .apply_graph_edits(request(
            MAIN_BRANCH,
            vec![
                GraphEdit::CreateNode {
                    labels: vec!["Malware".to_owned()],
                    properties: BTreeMap::from([("name".to_owned(), text("m"))]),
                },
                GraphEdit::CreateRelationship {
                    source_id: a.as_str().to_owned(),
                    target_id: "$0".to_owned(),
                    rel_type: "INDICATES".to_owned(),
                    properties: BTreeMap::from([("weight".to_owned(), PropertyValue::Integer(2))]),
                },
                GraphEdit::CreateRelationship {
                    source_id: a.as_str().to_owned(),
                    target_id: b.as_str().to_owned(),
                    rel_type: "LINKS".to_owned(),
                    properties: BTreeMap::new(),
                },
            ],
        ))
        .unwrap();
    assert_eq!(report.created.len(), 1);
    assert_eq!(report.created_relationships.len(), 2);
    let relationships = relationships_at(&mut store, MAIN_BRANCH);
    let indicates = &relationships[&report.created_relationships[0]];
    assert_eq!(
        (
            indicates.0.as_str(),
            indicates.1.as_str(),
            indicates.2.as_str()
        ),
        (a.as_str(), "INDICATES", report.created[0].as_str())
    );
    assert_eq!(indicates.3["weight"], PropertyValue::Integer(2));

    // Edit one relationship and delete the other; then b can be deleted
    // together with its last relationship.
    let links = report.created_relationships[1].clone();
    store
        .apply_graph_edits(request(
            MAIN_BRANCH,
            vec![
                GraphEdit::SetRelationshipProperties {
                    relationship_id: report.created_relationships[0].clone(),
                    set: BTreeMap::from([("weight".to_owned(), PropertyValue::Integer(5))]),
                    remove: Vec::new(),
                },
                GraphEdit::DeleteRelationship {
                    relationship_id: links.clone(),
                },
                GraphEdit::DeleteNode {
                    node_id: b.as_str().to_owned(),
                },
            ],
        ))
        .unwrap();
    let relationships = relationships_at(&mut store, MAIN_BRANCH);
    assert_eq!(relationships.len(), 1);
    assert_eq!(
        relationships[&report.created_relationships[0]].3["weight"],
        PropertyValue::Integer(5)
    );
    assert!(!nodes_at(&mut store, MAIN_BRANCH).contains_key(b.as_str()));
}

#[test]
fn relationship_edits_are_validated_before_writing() {
    let (_root, mut store, a, b) = fixture("relationship-refusals");
    let before = store.graph_head(MAIN_BRANCH).unwrap();
    let link = GraphEdit::CreateRelationship {
        source_id: a.as_str().to_owned(),
        target_id: b.as_str().to_owned(),
        rel_type: "LINKS".to_owned(),
        properties: BTreeMap::new(),
    };
    let error = store
        .apply_graph_edits(request(
            MAIN_BRANCH,
            vec![GraphEdit::DeleteRelationship {
                relationship_id: "relationship--missing".to_owned(),
            }],
        ))
        .unwrap_err();
    assert!(
        matches!(
            error,
            GraphStorageError::GraphEditRelationshipNotFound { .. }
        ),
        "{error:?}"
    );
    for edits in [
        vec![GraphEdit::CreateRelationship {
            source_id: a.as_str().to_owned(),
            target_id: "$0".to_owned(),
            rel_type: "LINKS".to_owned(),
            properties: BTreeMap::new(),
        }],
        vec![GraphEdit::CreateRelationship {
            source_id: a.as_str().to_owned(),
            target_id: b.as_str().to_owned(),
            rel_type: " ".to_owned(),
            properties: BTreeMap::new(),
        }],
        vec![
            link.clone(),
            GraphEdit::DeleteNode {
                node_id: b.as_str().to_owned(),
            },
        ],
    ] {
        let error = store
            .apply_graph_edits(request(MAIN_BRANCH, edits.clone()))
            .unwrap_err();
        assert!(
            matches!(error, GraphStorageError::GraphEditInvalid { .. }),
            "{edits:?}: {error:?}"
        );
    }
    let error = store
        .apply_graph_edits(request(
            MAIN_BRANCH,
            vec![GraphEdit::CreateRelationship {
                source_id: a.as_str().to_owned(),
                target_id: "node--missing".to_owned(),
                rel_type: "LINKS".to_owned(),
                properties: BTreeMap::new(),
            }],
        ))
        .unwrap_err();
    assert!(
        matches!(error, GraphStorageError::GraphEditNodeNotFound { .. }),
        "{error:?}"
    );
    assert_eq!(store.graph_head(MAIN_BRANCH).unwrap(), before);
}

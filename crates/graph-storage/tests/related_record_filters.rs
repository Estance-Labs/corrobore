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

//! Issue #361: commit record listings filtered across relationships.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use graph_core::{Graph, NodeId, NodeInput, PropertyValue, RelationshipInput};
use graph_storage::{
    CanonicalEngineStore, CanonicalProjectionRequest, CanonicalStoreOptions, DurableTransactionId,
    GraphId, HistoryRecordKind, MAIN_BRANCH, RecordFormat, RelatedDirection, RelatedRecordFilter,
    StorageManifest, StorageRoot, StorageTimestamp, StorageVersion, create_storage_root,
};

fn empty_store(test_name: &str) -> StorageRoot {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path: PathBuf = std::env::temp_dir().join(format!(
        "corrobore-issue-361-{test_name}-{}-{unique}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&path);
    create_storage_root(
        path,
        StorageManifest {
            storage_version: StorageVersion::V1,
            graph_id: GraphId {
                value: "graph--issue-361".to_owned(),
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

fn node(graph: &mut Graph, label: &str, name: &str) -> NodeId {
    graph
        .create_node(
            NodeInput::new([label]).with_property("name", PropertyValue::String(name.to_owned())),
        )
        .unwrap()
}

fn link(graph: &mut Graph, source: &NodeId, kind: &str, target: &NodeId) {
    graph
        .create_relationship(RelationshipInput::new(source.clone(), kind, target.clone()).unwrap())
        .unwrap();
}

struct Fixture {
    store: CanonicalEngineStore,
    i1: NodeId,
    i2: NodeId,
    i3: NodeId,
    malware: NodeId,
}

/// i1 -INDICATES-> malware, i2 -INDICATES-> tool, i3 unlinked,
/// actor -USES-> malware.
fn fixture(name: &str) -> Fixture {
    let root = empty_store(name);
    let mut store = open(&root);
    let mut ids = Vec::new();
    update_main(&mut store, "tx--issue-361-seed", |graph| {
        let i1 = node(graph, "Indicator", "i1");
        let i2 = node(graph, "Indicator", "i2");
        let i3 = node(graph, "Indicator", "i3");
        let malware = node(graph, "Malware", "m");
        let tool = node(graph, "Tool", "t");
        let actor = node(graph, "ThreatActor", "a");
        link(graph, &i1, "INDICATES", &malware);
        link(graph, &i2, "INDICATES", &tool);
        link(graph, &actor, "USES", &malware);
        ids.extend([i1, i2, i3, malware]);
    });
    let malware = ids.pop().unwrap();
    let i3 = ids.pop().unwrap();
    let i2 = ids.pop().unwrap();
    let i1 = ids.pop().unwrap();
    Fixture {
        store,
        i1,
        i2,
        i3,
        malware,
    }
}

fn ids(
    store: &mut CanonicalEngineStore,
    label: Option<&str>,
    related: &RelatedRecordFilter,
) -> (Vec<String>, usize) {
    let head = store.graph_head(MAIN_BRANCH).unwrap();
    let page = store
        .list_commit_records_related(
            &head,
            HistoryRecordKind::Node,
            label,
            None,
            50,
            false,
            Some(related),
        )
        .unwrap();
    let mut ids: Vec<String> = page
        .nodes
        .iter()
        .map(|node| node.id().as_str().to_owned())
        .collect();
    ids.sort();
    (ids, page.total)
}

fn sorted(ids: &[&NodeId]) -> Vec<String> {
    let mut ids: Vec<String> = ids.iter().map(|id| id.as_str().to_owned()).collect();
    ids.sort();
    ids
}

#[test]
fn nodes_are_filtered_by_type_direction_and_neighbour_label() {
    let mut f = fixture("filters");
    let indicates_malware = RelatedRecordFilter {
        relationship_type: Some("INDICATES".to_owned()),
        direction: RelatedDirection::Out,
        neighbour_label: Some("Malware".to_owned()),
    };
    assert_eq!(
        ids(&mut f.store, Some("Indicator"), &indicates_malware),
        (sorted(&[&f.i1]), 1)
    );

    let indicates_anything = RelatedRecordFilter {
        relationship_type: Some("INDICATES".to_owned()),
        direction: RelatedDirection::Out,
        neighbour_label: None,
    };
    assert_eq!(
        ids(&mut f.store, Some("Indicator"), &indicates_anything),
        (sorted(&[&f.i1, &f.i2]), 2)
    );

    // Incoming: the malware is the target of INDICATES and USES.
    let indicated = RelatedRecordFilter {
        relationship_type: Some("INDICATES".to_owned()),
        direction: RelatedDirection::In,
        neighbour_label: None,
    };
    assert_eq!(
        ids(&mut f.store, Some("Malware"), &indicated),
        (sorted(&[&f.malware]), 1)
    );
    assert_eq!(ids(&mut f.store, Some("Indicator"), &indicated).1, 0);

    // Any direction, any type, no label filter: every linked node.
    let linked = RelatedRecordFilter::default();
    let (linked_ids, total) = ids(&mut f.store, None, &linked);
    assert_eq!(total, 5, "{linked_ids:?}");
    assert!(!linked_ids.contains(&f.i3.as_str().to_owned()));
}

#[test]
fn deleted_relationships_and_neighbours_do_not_match_and_paging_counts_matches() {
    let mut f = fixture("liveness");
    let malware = f.malware.clone();
    update_main(&mut f.store, "tx--issue-361-delete", |graph| {
        let attached: Vec<_> = graph
            .current_relationship_records()
            .unwrap()
            .into_iter()
            .filter(|relationship| relationship.target() == &malware)
            .map(|relationship| relationship.id().clone())
            .collect();
        for id in attached {
            graph.tombstone_relationship(&id).unwrap();
        }
        graph.tombstone_node(&malware).unwrap();
    });
    let indicates_malware = RelatedRecordFilter {
        relationship_type: Some("INDICATES".to_owned()),
        direction: RelatedDirection::Any,
        neighbour_label: Some("Malware".to_owned()),
    };
    let (found, total) = ids(&mut f.store, Some("Indicator"), &indicates_malware);
    assert!(found.is_empty() && total == 0, "{found:?}");

    // Keyset paging walks only matching nodes.
    let head = f.store.graph_head(MAIN_BRANCH).unwrap();
    let any_indicates = RelatedRecordFilter {
        relationship_type: Some("INDICATES".to_owned()),
        direction: RelatedDirection::Out,
        neighbour_label: None,
    };
    let first = f
        .store
        .list_commit_records_related(
            &head,
            HistoryRecordKind::Node,
            Some("Indicator"),
            None,
            1,
            false,
            Some(&any_indicates),
        )
        .unwrap();
    assert_eq!(first.nodes.len(), 1);
    assert_eq!(
        first.total,
        first.nodes.len() + usize::from(first.next_cursor.is_some())
    );
    if let Some(cursor) = first.next_cursor.as_deref() {
        let second = f
            .store
            .list_commit_records_related(
                &head,
                HistoryRecordKind::Node,
                Some("Indicator"),
                Some(cursor),
                1,
                false,
                Some(&any_indicates),
            )
            .unwrap();
        assert!(second.next_cursor.is_none());
        assert_ne!(first.nodes[0].id(), second.nodes[0].id());
    }
    let _ = (&f.i1, &f.i2, &f.i3);
}

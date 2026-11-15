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

//! Issue #323: the schema of a commit, with ids that stay stable.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

use graph_core::{Graph, NodeInput, PropertyValue, RelationshipInput};
use graph_storage::{
    CanonicalEngineStore, CanonicalProjectionRequest, CanonicalStoreOptions, DurableTransactionId,
    GraphId, MAIN_BRANCH, RecordFormat, StorageManifest, StorageRoot, StorageTimestamp,
    StorageVersion, create_storage_root,
};

fn empty_store(name: &str) -> StorageRoot {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "corrobore-issue-323-{name}-{}-{unique}",
        std::process::id()
    ));
    create_storage_root(
        path,
        StorageManifest {
            storage_version: StorageVersion::V1,
            graph_id: GraphId {
                value: "graph--issue-323".to_owned(),
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

fn write(store: &mut CanonicalEngineStore, id: &str, change: impl FnOnce(&mut Graph)) {
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

fn seed(store: &mut CanonicalEngineStore) {
    write(store, "tx--issue-323-seed", |graph| {
        let a = graph
            .create_node(
                NodeInput::new(["Indicator"])
                    .with_property("name", PropertyValue::String("a".to_owned()))
                    .with_property("score", PropertyValue::Integer(7)),
            )
            .unwrap();
        graph
            .create_node(
                NodeInput::new(["Indicator"])
                    .with_property("name", PropertyValue::String("b".to_owned()))
                    .with_property("score", PropertyValue::Float(0.5)),
            )
            .unwrap();
        let m = graph
            .create_node(
                NodeInput::new(["Malware"])
                    .with_property("name", PropertyValue::String("m".to_owned())),
            )
            .unwrap();
        graph
            .create_relationship(
                RelationshipInput::new(a, "INDICATES", m)
                    .unwrap()
                    .with_property("weight", PropertyValue::Integer(1)),
            )
            .unwrap();
    });
}

#[test]
fn schema_counts_labels_types_value_types_and_endpoints() {
    let root = empty_store("shape");
    let mut store = open(&root);
    seed(&mut store);
    let head = store.graph_head(MAIN_BRANCH).unwrap();
    let schema = store.graph_schema(&head).unwrap();

    let names: Vec<&str> = schema
        .labels
        .iter()
        .map(|label| label.name.as_str())
        .collect();
    assert_eq!(names, vec!["Indicator", "Malware"]);
    let indicator = &schema.labels[0];
    assert_eq!(indicator.count, 2);
    let score = indicator
        .properties
        .iter()
        .find(|property| property.key == "score")
        .unwrap();
    assert_eq!(
        score.value_types,
        vec!["float".to_owned(), "integer".to_owned()]
    );
    assert_eq!(score.count, 2);

    let indicates = &schema.relationship_types[0];
    assert_eq!(indicates.name, "INDICATES");
    assert_eq!(indicates.count, 1);
    assert_eq!(indicates.endpoints[0].source_label, "Indicator");
    assert_eq!(indicates.endpoints[0].target_label, "Malware");
    assert_eq!(indicates.properties[0].key, "weight");

    let ids: Vec<u64> = schema
        .labels
        .iter()
        .map(|label| label.id)
        .chain(schema.relationship_types.iter().map(|kind| kind.id))
        .collect();
    let mut unique = ids.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), ids.len());
    assert!(ids.iter().all(|id| *id > 0));

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn ids_survive_reopen_and_a_label_coming_back() {
    let root = empty_store("stable");
    let mut store = open(&root);
    seed(&mut store);
    let first = store.graph_head(MAIN_BRANCH).unwrap();
    let malware_id = store
        .graph_schema(&first)
        .unwrap()
        .labels
        .iter()
        .find(|label| label.name == "Malware")
        .unwrap()
        .id;

    let malware = store
        .load_projection(CanonicalProjectionRequest::for_label("Malware").with_relationships(None))
        .unwrap();
    let malware_node = malware
        .current_node_records()
        .unwrap()
        .into_iter()
        .find(|node| node.labels().contains(&"Malware".to_owned()))
        .unwrap();
    write(&mut store, "tx--issue-323-remove", |graph| {
        for relationship in graph.current_relationship_records().unwrap() {
            graph.tombstone_relationship(relationship.id()).unwrap();
        }
        graph.tombstone_node(malware_node.id()).unwrap();
    });
    let without = store.graph_head(MAIN_BRANCH).unwrap();
    assert!(
        store
            .graph_schema(&without)
            .unwrap()
            .labels
            .iter()
            .all(|label| label.name != "Malware")
    );
    drop(store);

    let mut reopened = open(&root);
    write(&mut reopened, "tx--issue-323-back", |graph| {
        graph.create_node(NodeInput::new(["Malware"])).unwrap();
    });
    let back = reopened.graph_head(MAIN_BRANCH).unwrap();
    let again = reopened
        .graph_schema(&back)
        .unwrap()
        .labels
        .iter()
        .find(|label| label.name == "Malware")
        .unwrap()
        .id;
    assert_eq!(again, malware_id);
    let old = reopened.graph_schema(&first).unwrap();
    assert_eq!(
        old.relationship_types.len(),
        1,
        "older commits keep their schema"
    );

    let _ = fs::remove_dir_all(root.path());
}

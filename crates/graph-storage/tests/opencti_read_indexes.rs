// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
#![allow(clippy::unwrap_used)]

use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use graph_core::{Graph, NodeInput, PropertyValue, RelationshipInput};
use graph_storage::{
    CanonicalAdjacencyProjection, CanonicalEngineStore, CanonicalProjectionRequest,
    CanonicalPropertyFilter, CanonicalPropertyOperator, CanonicalStoreOptions,
    DurableTransactionId, GraphId, RecordFormat, StorageManifest, StorageTimestamp, StorageVersion,
    create_storage_root,
};
use serde_json::json;

fn unique_temp_path(test_name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "corrobore-issue-44-{test_name}-{}-{unique}",
        std::process::id()
    ))
}

fn root(test_name: &str) -> graph_storage::StorageRoot {
    let path = unique_temp_path(test_name);
    let _ = fs::remove_dir_all(&path);
    create_storage_root(
        path,
        StorageManifest {
            storage_version: StorageVersion::V1,
            graph_id: GraphId {
                value: "graph--issue-44".to_owned(),
            },
            created_at: StorageTimestamp {
                value: "2026-07-25T00:00:00Z".to_owned(),
            },
            updated_at: StorageTimestamp {
                value: "2026-07-25T00:00:00Z".to_owned(),
            },
            record_format: RecordFormat::JsonLinesV1,
        },
    )
    .unwrap()
}

fn opencti_node(id: &str, kind: &str, name: &str, valid_from: &str) -> NodeInput {
    NodeInput::new(vec![
        "OpenCtiObject".to_owned(),
        format!("OpenCtiType_{kind}"),
    ])
    .with_property("opencti.canonical_id", PropertyValue::String(id.to_owned()))
    .with_property(
        "opencti.identifiers",
        PropertyValue::Json(json!([
            {"kind": "standard", "value": id},
            {"kind": "alias", "value": name}
        ])),
    )
    .with_property("opencti.field.type", PropertyValue::String(kind.to_owned()))
    .with_property("opencti.field.name", PropertyValue::String(name.to_owned()))
    .with_property(
        "opencti.field.valid_from",
        PropertyValue::String(valid_from.to_owned()),
    )
}

fn fixture_graph() -> Graph {
    let mut graph = Graph::new();
    let indicator = graph
        .create_node(opencti_node(
            "indicator--indexed",
            "indicator",
            "Indexed indicator",
            "2026-01-01T00:00:00Z",
        ))
        .unwrap();
    let malware = graph
        .create_node(opencti_node(
            "malware--indexed",
            "malware",
            "Indexed malware",
            "2026-01-02T00:00:00Z",
        ))
        .unwrap();
    graph
        .create_relationship(
            RelationshipInput::new(indicator, "indicates", malware)
                .unwrap()
                .with_property(
                    "opencti.canonical_id",
                    PropertyValue::String("relationship--indexed".to_owned()),
                ),
        )
        .unwrap();
    let unrelated_source = graph
        .create_node(opencti_node(
            "indicator--unrelated",
            "indicator",
            "Unrelated indicator",
            "2026-02-01T00:00:00Z",
        ))
        .unwrap();
    let unrelated_target = graph
        .create_node(opencti_node(
            "malware--unrelated",
            "malware",
            "Unrelated malware",
            "2026-02-02T00:00:00Z",
        ))
        .unwrap();
    graph
        .create_relationship(
            RelationshipInput::new(unrelated_source, "indicates", unrelated_target)
                .unwrap()
                .with_property(
                    "opencti.canonical_id",
                    PropertyValue::String("relationship--unrelated".to_owned()),
                ),
        )
        .unwrap();
    graph
}

#[test]
fn cold_point_and_filtered_reads_use_compact_identifier_property_and_temporal_indexes() {
    let root = root("indexed-read");
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    store
        .commit_transition(
            &Graph::new(),
            &fixture_graph(),
            DurableTransactionId::new("tx--indexed-read").unwrap(),
            None,
        )
        .unwrap();
    drop(store);
    fs::remove_file(root.path().join("catalog/catalog_metadata.json"))
        .expect("derived metadata removal should force committed-log recovery");

    let mut reopened =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    assert_eq!(reopened.startup_report().payloads_hydrated, 0);
    assert!(reopened.startup_report().derived_indexes_rebuilt);
    assert_eq!(reopened.stats().page_ins, 0);
    assert!(reopened.stats().identifier_index_entries >= 4);
    assert!(reopened.stats().property_index_entries >= 4);
    assert!(reopened.stats().temporal_index_entries >= 2);

    let point = reopened
        .load_projection(CanonicalProjectionRequest::for_identifier(
            "indicator--indexed",
        ))
        .unwrap();
    assert_eq!(point.list_nodes().unwrap().len(), 1);
    assert_eq!(reopened.stats().page_ins, 1);
    assert_eq!(
        reopened.last_projection_stats().access_paths,
        vec!["identifier_index"]
    );

    let filtered = reopened
        .load_projection(
            CanonicalProjectionRequest::for_label("OpenCtiType_indicator").with_property_filters([
                CanonicalPropertyFilter {
                    field: "opencti.field.name".to_owned(),
                    operator: CanonicalPropertyOperator::Equal,
                    value: Some(json!("Indexed indicator")),
                },
                CanonicalPropertyFilter {
                    field: "opencti.field.valid_from".to_owned(),
                    operator: CanonicalPropertyOperator::GreaterThanOrEqual,
                    value: Some(json!("2026-01-01T00:00:00Z")),
                },
            ]),
        )
        .unwrap();
    assert_eq!(filtered.list_nodes().unwrap().len(), 1);
    assert_eq!(
        reopened.last_projection_stats().access_paths,
        vec!["label_index", "property_index", "temporal_index"]
    );

    let membership = reopened
        .load_projection(
            CanonicalProjectionRequest::for_label("OpenCtiType_indicator").with_property_filters([
                CanonicalPropertyFilter {
                    field: "opencti.field.name".to_owned(),
                    operator: CanonicalPropertyOperator::In,
                    value: Some(json!(["Indexed indicator", "another value"])),
                },
            ]),
        )
        .unwrap();
    assert_eq!(membership.list_nodes().unwrap().len(), 1);

    let exclusion = reopened
        .load_projection(
            CanonicalProjectionRequest::for_label("OpenCtiType_indicator").with_property_filters([
                CanonicalPropertyFilter {
                    field: "opencti.field.name".to_owned(),
                    operator: CanonicalPropertyOperator::NotIn,
                    value: Some(json!(["Unrelated indicator"])),
                },
            ]),
        )
        .unwrap();
    assert_eq!(exclusion.list_nodes().unwrap().len(), 1);

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn persistent_adjacency_expansion_pages_only_bounded_typed_neighborhoods() {
    let root = root("adjacency-read");
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    store
        .commit_transition(
            &Graph::new(),
            &fixture_graph(),
            DurableTransactionId::new("tx--adjacency-read").unwrap(),
            None,
        )
        .unwrap();
    drop(store);

    let mut reopened =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    let projection = reopened
        .load_projection(
            CanonicalProjectionRequest::for_identifier("indicator--indexed").with_adjacency(
                CanonicalAdjacencyProjection {
                    incoming: false,
                    outgoing: true,
                    relationship_types: vec!["indicates".to_owned()],
                    max_depth: 1,
                    max_relationships: 1,
                    supernode_threshold: 10,
                },
            ),
        )
        .unwrap();
    assert_eq!(projection.list_nodes().unwrap().len(), 2);
    assert_eq!(projection.list_relationships().unwrap().len(), 1);
    assert_eq!(
        reopened.last_projection_stats().access_paths,
        vec!["identifier_index", "persistent_adjacency"]
    );
    assert_eq!(reopened.stats().resident_hot_nodes, 2);
    assert_eq!(reopened.stats().resident_hot_relationships, 1);

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn unknown_and_tombstoned_identifiers_return_empty_without_scanning_payloads() {
    let root = root("missing-deleted");
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    let mut graph = fixture_graph();
    store
        .commit_transition(
            &Graph::new(),
            &graph,
            DurableTransactionId::new("tx--before-delete").unwrap(),
            None,
        )
        .unwrap();
    let indicator = graph
        .list_nodes()
        .unwrap()
        .into_iter()
        .find(|node| {
            node.property("opencti.canonical_id")
                == Some(&PropertyValue::String("indicator--indexed".to_owned()))
        })
        .unwrap()
        .id()
        .clone();
    let before = graph.clone();
    graph.tombstone_node(&indicator).unwrap();
    store
        .commit_transition(
            &before,
            &graph,
            DurableTransactionId::new("tx--delete").unwrap(),
            None,
        )
        .unwrap();
    drop(store);

    let mut reopened =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    for identifier in ["unknown--identifier", "indicator--indexed"] {
        let projection = reopened
            .load_projection(CanonicalProjectionRequest::for_identifier(identifier))
            .unwrap();
        assert!(projection.list_nodes().unwrap().is_empty());
    }
    assert_eq!(reopened.stats().page_ins, 0);

    let _ = fs::remove_dir_all(root.path());
}

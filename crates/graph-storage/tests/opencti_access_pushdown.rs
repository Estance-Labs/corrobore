// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
#![allow(clippy::unwrap_used)]

use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use graph_core::{Graph, NodeInput, NodePatch, PropertyValue, RelationshipInput};
use graph_storage::{
    CanonicalAccessContext, CanonicalAdjacencyProjection, CanonicalEngineStore,
    CanonicalProjectionRequest, CanonicalStoreOptions, DurableTransactionId, GraphId, RecordFormat,
    StorageManifest, StorageTimestamp, StorageVersion, create_storage_root,
};
use serde_json::{Value, json};

fn unique_temp_path(test_name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "corrobore-issue-45-{test_name}-{}-{unique}",
        std::process::id()
    ))
}

fn root(test_name: &str) -> graph_storage::StorageRoot {
    create_storage_root(
        unique_temp_path(test_name),
        StorageManifest {
            storage_version: StorageVersion::V1,
            graph_id: GraphId {
                value: "graph--issue-45".to_owned(),
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

fn access(policy_version: &str) -> CanonicalAccessContext {
    CanonicalAccessContext {
        subject_id: "user--alpha".to_owned(),
        organization_ids: vec!["organization--alpha".to_owned()],
        marking_ids: vec!["marking--amber".to_owned()],
        tenant_id: Some("tenant--alpha".to_owned()),
        roles: vec!["analyst".to_owned()],
        attributes: BTreeMap::from([("policy_version".to_owned(), policy_version.to_owned())]),
    }
}

fn node(id: &str, access: Value) -> NodeInput {
    NodeInput::new(vec!["OpenCtiObject".to_owned()])
        .with_property("opencti.canonical_id", PropertyValue::String(id.to_owned()))
        .with_property(
            "opencti.identifiers",
            PropertyValue::Json(json!([{"value": id}])),
        )
        .with_property("opencti.access", PropertyValue::Json(access))
        .with_property(
            "opencti.raw",
            PropertyValue::Json(json!({"id": id, "type": "indicator"})),
        )
}

fn fixture_graph() -> Graph {
    let mut graph = Graph::new();
    let seed = graph
        .create_node(node(
            "indicator--seed",
            json!({
                "marking_ids": ["marking--amber"],
                "organization_ids": ["organization--alpha"],
                "tenant_ids": ["tenant--alpha"]
            }),
        ))
        .unwrap();
    let visible = graph
        .create_node(node(
            "indicator--visible",
            json!({
                "marking_ids": ["marking--amber"],
                "owner_ids": ["user--alpha"],
                "tenant_ids": ["tenant--alpha"]
            }),
        ))
        .unwrap();
    let hidden = graph
        .create_node(node(
            "indicator--hidden",
            json!({
                "marking_ids": ["marking--red"],
                "organization_ids": ["organization--beta"],
                "tenant_ids": ["tenant--beta"]
            }),
        ))
        .unwrap();
    graph
        .create_relationship(
            RelationshipInput::new(seed.clone(), "related-to", visible.clone())
                .unwrap()
                .with_property(
                    "opencti.canonical_id",
                    PropertyValue::String("relationship--visible".to_owned()),
                )
                .with_property(
                    "opencti.access",
                    PropertyValue::Json(json!({
                        "marking_ids": ["marking--amber"],
                        "organization_ids": ["organization--alpha"],
                        "tenant_ids": ["tenant--alpha"]
                    })),
                ),
        )
        .unwrap();
    graph
        .create_relationship(
            RelationshipInput::new(seed.clone(), "related-to", hidden)
                .unwrap()
                .with_property(
                    "opencti.canonical_id",
                    PropertyValue::String("relationship--hidden-endpoint".to_owned()),
                )
                .with_property(
                    "opencti.access",
                    PropertyValue::Json(json!({
                        "marking_ids": ["marking--amber"],
                        "organization_ids": ["organization--alpha"],
                        "tenant_ids": ["tenant--alpha"]
                    })),
                ),
        )
        .unwrap();
    graph
        .create_relationship(
            RelationshipInput::new(seed, "derived-from", visible)
                .unwrap()
                .with_property(
                    "opencti.canonical_id",
                    PropertyValue::String("relationship--hidden-policy".to_owned()),
                )
                .with_property(
                    "opencti.access",
                    PropertyValue::Json(json!({
                        "marking_ids": ["marking--red"],
                        "organization_ids": ["organization--beta"],
                        "tenant_ids": ["tenant--beta"]
                    })),
                ),
        )
        .unwrap();
    graph
}

#[test]
fn access_policy_filters_indexes_and_adjacency_before_payload_page_in() {
    let root = root("pushdown");
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    store
        .commit_transition(
            &Graph::new(),
            &fixture_graph(),
            DurableTransactionId::new("tx--access-pushdown").unwrap(),
            None,
        )
        .unwrap();
    drop(store);

    let mut reopened =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    let denied = reopened
        .load_projection(
            CanonicalProjectionRequest::for_identifier("indicator--hidden")
                .with_access_context(access("policy--v1")),
        )
        .unwrap();
    assert!(denied.list_nodes().unwrap().is_empty());
    assert_eq!(reopened.last_projection_stats().page_ins, 0);
    assert_eq!(
        reopened.last_projection_stats().access_paths,
        vec!["identifier_index", "access_policy_index"]
    );

    let graph = reopened
        .load_projection(
            CanonicalProjectionRequest::for_identifier("indicator--seed")
                .with_adjacency(CanonicalAdjacencyProjection {
                    incoming: false,
                    outgoing: true,
                    relationship_types: Vec::new(),
                    max_depth: 1,
                    max_relationships: 10,
                    supernode_threshold: 10,
                })
                .with_access_context(access("policy--v1")),
        )
        .unwrap();
    assert_eq!(graph.list_nodes().unwrap().len(), 2);
    assert_eq!(graph.list_relationships().unwrap().len(), 1);
    assert_eq!(reopened.last_projection_stats().page_ins, 3);
    assert_eq!(reopened.last_projection_stats().authorization_denials, 2);

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn generic_relationship_selection_requires_both_authorized_endpoints_before_page_in() {
    let root = root("relationship-endpoints");
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    store
        .commit_transition(
            &Graph::new(),
            &fixture_graph(),
            DurableTransactionId::new("tx--relationship-endpoints").unwrap(),
            None,
        )
        .unwrap();
    drop(store);

    let mut reopened =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    let graph = reopened
        .load_projection(
            CanonicalProjectionRequest::all().with_access_context(access("policy--v1")),
        )
        .unwrap();

    assert_eq!(graph.list_nodes().unwrap().len(), 2);
    assert_eq!(graph.list_relationships().unwrap().len(), 1);
    assert_eq!(reopened.last_projection_stats().page_ins, 3);
    assert_eq!(reopened.last_projection_stats().authorization_denials, 3);

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn policy_version_change_invalidates_hot_payloads_and_recovered_access_indexes() {
    let root = root("policy-invalidation");
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    store
        .commit_transition(
            &Graph::new(),
            &fixture_graph(),
            DurableTransactionId::new("tx--policy-invalidation").unwrap(),
            None,
        )
        .unwrap();
    drop(store);
    fs::remove_file(root.path().join("catalog/catalog_metadata.json")).unwrap();

    let mut reopened =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    assert!(reopened.startup_report().derived_indexes_rebuilt);
    let request = CanonicalProjectionRequest::for_identifier("indicator--seed")
        .with_access_context(access("policy--v1"));
    reopened.load_projection(request.clone()).unwrap();
    assert_eq!(reopened.last_projection_stats().page_ins, 1);
    reopened.load_projection(request).unwrap();
    assert_eq!(reopened.last_projection_stats().cache_hits, 1);

    reopened
        .load_projection(
            CanonicalProjectionRequest::for_identifier("indicator--seed")
                .with_access_context(access("policy--v2")),
        )
        .unwrap();
    assert_eq!(reopened.last_projection_stats().cache_hits, 0);
    assert_eq!(reopened.last_projection_stats().page_ins, 1);
    assert!(reopened.last_projection_stats().policy_cache_invalidated);

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn missing_and_denied_point_reads_share_the_same_non_materializing_timing_class() {
    let root = root("timing-class");
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    store
        .commit_transition(
            &Graph::new(),
            &fixture_graph(),
            DurableTransactionId::new("tx--timing-class").unwrap(),
            None,
        )
        .unwrap();
    drop(store);

    let mut reopened =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    for identifier in ["indicator--hidden", "indicator--absent"] {
        let result = reopened
            .load_projection(
                CanonicalProjectionRequest::for_identifier(identifier)
                    .with_access_context(access("policy--v1")),
            )
            .unwrap();
        assert!(result.list_nodes().unwrap().is_empty());
        assert_eq!(reopened.last_projection_stats().page_ins, 0);
        assert_eq!(
            reopened.last_projection_stats().timing_class,
            "visibility_miss"
        );
    }

    let mut measure = |identifier: &str| {
        let mut samples = (0..128)
            .map(|_| {
                let started = Instant::now();
                reopened
                    .load_projection(
                        CanonicalProjectionRequest::for_identifier(identifier)
                            .with_access_context(access("policy--v1")),
                    )
                    .unwrap();
                started.elapsed().as_nanos()
            })
            .collect::<Vec<_>>();
        samples.sort_unstable();
        samples[samples.len() * 95 / 100]
    };
    let denied_p95_ns = measure("indicator--hidden");
    let missing_p95_ns = measure("indicator--absent");
    let faster_p95_ns = denied_p95_ns.min(missing_p95_ns).max(1);
    let slower_p95_ns = denied_p95_ns.max(missing_p95_ns);
    assert!(
        slower_p95_ns <= faster_p95_ns.saturating_mul(6).saturating_add(2_000_000),
        "denied and missing p95 latency diverged: denied={denied_p95_ns}ns missing={missing_p95_ns}ns"
    );

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn access_metadata_mutation_replaces_the_policy_index_and_invalidates_residency() {
    let root = root("policy-mutation");
    let mut graph = fixture_graph();
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    store
        .commit_transition(
            &Graph::new(),
            &graph,
            DurableTransactionId::new("tx--policy-before").unwrap(),
            None,
        )
        .unwrap();
    let denied = store
        .load_projection(
            CanonicalProjectionRequest::for_identifier("indicator--hidden")
                .with_access_context(access("policy--v1")),
        )
        .unwrap();
    assert!(denied.list_nodes().unwrap().is_empty());

    let hidden_id = graph
        .list_nodes()
        .unwrap()
        .into_iter()
        .find(|node| {
            node.property("opencti.canonical_id")
                == Some(&PropertyValue::String("indicator--hidden".to_owned()))
        })
        .unwrap()
        .id()
        .clone();
    let before = graph.clone();
    graph
        .update_node(
            &hidden_id,
            NodePatch::default().set_property(
                "opencti.access",
                PropertyValue::Json(json!({
                    "marking_ids": ["marking--amber"],
                    "organization_ids": ["organization--alpha"],
                    "tenant_ids": ["tenant--alpha"]
                })),
            ),
        )
        .unwrap();
    store
        .commit_transition(
            &before,
            &graph,
            DurableTransactionId::new("tx--policy-after").unwrap(),
            None,
        )
        .unwrap();

    let allowed = store
        .load_projection(
            CanonicalProjectionRequest::for_identifier("indicator--hidden")
                .with_access_context(access("policy--v2")),
        )
        .unwrap();
    assert_eq!(allowed.list_nodes().unwrap().len(), 1);
    assert_eq!(store.last_projection_stats().page_ins, 1);
    assert_eq!(store.last_projection_stats().cache_hits, 0);

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn relationship_tombstone_removes_the_current_access_index() {
    let root = root("relationship-tombstone");
    let mut graph = fixture_graph();
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    store
        .commit_transition(
            &Graph::new(),
            &graph,
            DurableTransactionId::new("tx--relationship-before").unwrap(),
            None,
        )
        .unwrap();
    let relationship_id = graph
        .list_relationships()
        .unwrap()
        .into_iter()
        .find(|relationship| {
            relationship.property("opencti.canonical_id")
                == Some(&PropertyValue::String("relationship--visible".to_owned()))
        })
        .unwrap()
        .id()
        .clone();
    let before = graph.clone();
    graph.tombstone_relationship(&relationship_id).unwrap();
    store
        .commit_transition(
            &before,
            &graph,
            DurableTransactionId::new("tx--relationship-tombstone").unwrap(),
            None,
        )
        .unwrap();
    assert_eq!(store.stats().relationship_access_index_entries, 2);
    drop(store);

    let reopened =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    assert_eq!(reopened.stats().relationship_access_index_entries, 2);

    let _ = fs::remove_dir_all(root.path());
}

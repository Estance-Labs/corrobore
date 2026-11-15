// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! Issue #326: exact vector search kept per commit, and hybrid retrieval
//! fusing full text, vectors and graph proximity with RRF.
#![allow(clippy::unwrap_used)]

use std::{
    collections::BTreeMap,
    time::{SystemTime, UNIX_EPOCH},
};

use graph_core::{Graph, NodeId, NodeInput, PropertyValue, RelationshipInput};
use graph_storage::{
    CanonicalAccessContext, CanonicalEngineStore, CanonicalProjectionRequest,
    CanonicalStoreOptions, DurableTransactionId, GraphId, HybridSearchRequest, MAIN_BRANCH,
    RecordFormat, StorageManifest, StorageTimestamp, StorageVersion, VectorMetric,
    create_storage_root, reciprocal_rank_fusion,
};
use serde_json::json;

/// Tests in one process may start within the clock's resolution (about a
/// microsecond on macOS), so a counter keeps their directories apart.
static ROOTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn root() -> graph_storage::StorageRoot {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let sequence = ROOTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "corrobore-issue-326-{}-{unique}-{sequence}",
        std::process::id()
    ));
    create_storage_root(
        path,
        StorageManifest {
            storage_version: StorageVersion::V1,
            graph_id: GraphId {
                value: "graph--issue-326".to_owned(),
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

fn doc(name: &str, embedding: &[f64]) -> NodeInput {
    NodeInput::new(vec!["Document".to_owned()])
        .with_property("opencti.field.name", PropertyValue::String(name.to_owned()))
        .with_property(
            "opencti.access",
            PropertyValue::Json(json!({"marking_ids": ["marking--clear"]})),
        )
        .with_property("embedding", PropertyValue::FloatList(embedding.to_vec()))
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

fn access() -> CanonicalAccessContext {
    CanonicalAccessContext {
        subject_id: "user--clear".to_owned(),
        marking_ids: vec!["marking--clear".to_owned()],
        attributes: BTreeMap::from([("policy_version".to_owned(), "policy--v1".to_owned())]),
        ..CanonicalAccessContext::default()
    }
}

#[test]
fn reciprocal_rank_fusion_rewards_agreement() {
    let fused = reciprocal_rank_fusion(
        &[
            vec!["a".to_owned(), "b".to_owned(), "c".to_owned()],
            vec!["b".to_owned(), "c".to_owned(), "a".to_owned()],
            vec!["b".to_owned()],
        ],
        60.0,
    );
    assert_eq!(fused[0].0, "b");
    assert!((fused[0].1 - (1.0 / 62.0 + 1.0 / 61.0 + 1.0 / 61.0)).abs() < 1e-12);
    assert_eq!(fused.len(), 3);
}

#[test]
fn vector_search_ranks_by_similarity_and_follows_commits() {
    let root = root();
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    let mut ids: Vec<NodeId> = Vec::new();
    write(&mut store, "tx--326-seed", |graph| {
        ids.push(graph.create_node(doc("east", &[1.0, 0.0])).unwrap());
        ids.push(graph.create_node(doc("north", &[0.0, 1.0])).unwrap());
        ids.push(graph.create_node(doc("north-east", &[0.7, 0.7])).unwrap());
    });
    let first = store.graph_head(MAIN_BRANCH).unwrap();
    let hits = store
        .vector_search(
            &first,
            "embedding",
            &[1.0, 0.1],
            2,
            VectorMetric::Cosine,
            None,
        )
        .unwrap();
    assert_eq!(
        hits.iter()
            .map(|hit| hit.node_id.as_str())
            .collect::<Vec<_>>(),
        vec![ids[0].as_str(), ids[2].as_str()]
    );
    assert!(hits[0].score > hits[1].score);
    assert_eq!(hits[0].ranks["vector"], 1);

    write(&mut store, "tx--326-more", |graph| {
        ids.push(graph.create_node(doc("due-east", &[1.0, 0.05])).unwrap());
    });
    let second = store.graph_head(MAIN_BRANCH).unwrap();
    let moved = store
        .vector_search(
            &second,
            "embedding",
            &[1.0, 0.05],
            1,
            VectorMetric::L2,
            None,
        )
        .unwrap();
    assert_eq!(
        moved[0].node_id,
        ids[3].as_str(),
        "the index moved forward with the new commit"
    );
    let back = store
        .vector_search(
            &first,
            "embedding",
            &[1.0, 0.05],
            4,
            VectorMetric::Cosine,
            None,
        )
        .unwrap();
    assert_eq!(back.len(), 3, "an older commit does not see the new node");

    let error = store.vector_search(
        &second,
        "embedding",
        &[1.0, 0.0, 0.0],
        1,
        VectorMetric::Cosine,
        None,
    );
    assert!(error.is_err(), "dimension mismatch is refused");
    let labelled = store
        .vector_search(
            &second,
            "embedding",
            &[0.0, 1.0],
            5,
            VectorMetric::Cosine,
            Some("Missing"),
        )
        .unwrap();
    assert!(labelled.is_empty());

    let _ = std::fs::remove_dir_all(root.path());
}

#[test]
fn hybrid_search_fuses_text_vectors_and_graph_neighbours() {
    let root = root();
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    let mut ids: Vec<NodeId> = Vec::new();
    write(&mut store, "tx--326-h", |graph| {
        ids.push(
            graph
                .create_node(doc("ransomware campaign", &[0.0, 1.0]))
                .unwrap(),
        );
        ids.push(graph.create_node(doc("phishing kit", &[1.0, 0.0])).unwrap());
        ids.push(
            graph
                .create_node(doc("unrelated note", &[0.5, 0.5]))
                .unwrap(),
        );
        let actor = graph
            .create_node(
                NodeInput::new(["ThreatActor"])
                    .with_property("name", PropertyValue::String("actor".to_owned())),
            )
            .unwrap();
        graph
            .create_relationship(
                RelationshipInput::new(actor.clone(), "USES", ids[0].clone()).unwrap(),
            )
            .unwrap();
        graph
            .create_relationship(
                RelationshipInput::new(actor.clone(), "USES", ids[1].clone()).unwrap(),
            )
            .unwrap();
        ids.push(actor);
    });
    let head = store.graph_head(MAIN_BRANCH).unwrap();
    let hits = store
        .hybrid_search(
            &head,
            &HybridSearchRequest {
                text: Some("ransomware".to_owned()),
                vector: Some(vec![1.0, 0.0]),
                property: None,
                k: 4,
                graph_expansion: true,
                access: access(),
            },
        )
        .unwrap();
    let ranked: Vec<&str> = hits.iter().map(|hit| hit.node_id.as_str()).collect();
    assert!(
        ranked.contains(&ids[0].as_str()),
        "the text hit is fused in: {hits:?}"
    );
    assert!(
        ranked.contains(&ids[1].as_str()),
        "the vector hit is fused in: {hits:?}"
    );
    let actor = hits
        .iter()
        .find(|hit| hit.node_id == ids[3].as_str())
        .unwrap();
    assert_eq!(
        actor.ranks["graph"], 1,
        "the node linking both hits ranks first by graph proximity"
    );
    let text_hit = hits
        .iter()
        .find(|hit| hit.node_id == ids[0].as_str())
        .unwrap();
    assert_eq!(text_hit.ranks["full_text"], 1);

    assert!(
        store
            .hybrid_search(
                &head,
                &HybridSearchRequest {
                    k: 3,
                    ..HybridSearchRequest::default()
                }
            )
            .is_err()
    );

    let _ = std::fs::remove_dir_all(root.path());
}

/// Issue #369: text search works at commits the full-text index does not
/// cover (branch heads, older commits), with the same access rules.
#[test]
fn hybrid_text_search_scans_branches_and_older_commits_with_access_rules() {
    let root = root();
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    let mut ids: Vec<NodeId> = Vec::new();
    write(&mut store, "tx--369-seed", |graph| {
        ids.push(
            graph
                .create_node(doc("ransomware campaign", &[0.0, 1.0]))
                .unwrap(),
        );
        ids.push(graph.create_node(doc("phishing kit", &[1.0, 0.0])).unwrap());
        ids.push(
            graph
                .create_node(
                    NodeInput::new(["Document"])
                        .with_property(
                            "opencti.field.name",
                            PropertyValue::String("secret ransomware dossier".to_owned()),
                        )
                        .with_property(
                            "opencti.access",
                            PropertyValue::Json(json!({"marking_ids": ["marking--secret"]})),
                        ),
                )
                .unwrap(),
        );
    });
    let seeded = store.graph_head(MAIN_BRANCH).unwrap();
    write(&mut store, "tx--369-more", |graph| {
        ids.push(
            graph
                .create_node(doc("ransomware loader", &[0.3, 0.3]))
                .unwrap(),
        );
    });
    store
        .create_graph_branch("review", None, None, None)
        .unwrap();
    let branch_head = store.graph_head("review").unwrap();
    let text_only = |text: &str| HybridSearchRequest {
        text: Some(text.to_owned()),
        k: 10,
        access: access(),
        ..HybridSearchRequest::default()
    };

    // A branch head is not the main head once main moves on; force that.
    write(&mut store, "tx--369-main-moves", |graph| {
        graph
            .create_node(doc("ransomware on main only", &[0.1, 0.1]))
            .unwrap();
    });
    assert_ne!(store.graph_head(MAIN_BRANCH).unwrap(), branch_head);

    let hits = store
        .hybrid_search(&branch_head, &text_only("ransomware campaign"))
        .unwrap();
    let found: Vec<&str> = hits.iter().map(|hit| hit.node_id.as_str()).collect();
    assert_eq!(
        found.first(),
        Some(&ids[0].as_str()),
        "two matching terms rank first: {found:?}"
    );
    assert!(found.contains(&ids[3].as_str()), "{found:?}");
    assert!(
        !found.contains(&ids[2].as_str()),
        "a secret record stays hidden: {found:?}"
    );
    assert!(hits.iter().all(|hit| hit.ranks.contains_key("full_text")));

    // An older commit only sees what existed then.
    let hits = store
        .hybrid_search(&seeded, &text_only("ransomware"))
        .unwrap();
    let found: Vec<&str> = hits.iter().map(|hit| hit.node_id.as_str()).collect();
    assert_eq!(found, [ids[0].as_str()], "{found:?}");

    // No term matches: an empty result, not an error.
    assert!(
        store
            .hybrid_search(&seeded, &text_only("zebra"))
            .unwrap()
            .is_empty()
    );
}

/// Issue #369: on the main head, nodes outside the OpenCTI full-text fields
/// (a plain `name`) are found too.
#[test]
fn main_head_text_search_also_finds_plain_properties() {
    let root = root();
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    let mut ids: Vec<NodeId> = Vec::new();
    write(&mut store, "tx--369-plain", |graph| {
        ids.push(
            graph
                .create_node(doc("x-agent dossier", &[0.0, 1.0]))
                .unwrap(),
        );
        ids.push(
            graph
                .create_node(
                    NodeInput::new(["Malware"])
                        .with_property("name", PropertyValue::String("X-Agent".to_owned())),
                )
                .unwrap(),
        );
    });
    let head = store.graph_head(MAIN_BRANCH).unwrap();
    let hits = store
        .hybrid_search(
            &head,
            &HybridSearchRequest {
                text: Some("x-agent".to_owned()),
                k: 10,
                access: access(),
                ..HybridSearchRequest::default()
            },
        )
        .unwrap();
    let found: Vec<&str> = hits.iter().map(|hit| hit.node_id.as_str()).collect();
    assert!(
        found.contains(&ids[0].as_str()),
        "indexed document: {found:?}"
    );
    assert!(
        found.contains(&ids[1].as_str()),
        "plain property: {found:?}"
    );
}

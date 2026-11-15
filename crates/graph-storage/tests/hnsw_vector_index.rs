// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! Issue #342: approximate nearest-neighbour index (HNSW) behind
//! `VectorIndex`, chosen by policy or by index size, with recall measured
//! against the exact index.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::{SystemTime, UNIX_EPOCH};

use graph_core::{Graph, NodeId, NodeInput, PropertyValue};
use graph_storage::{
    CanonicalEngineStore, CanonicalProjectionRequest, CanonicalStoreOptions,
    DEFAULT_HNSW_MIN_VECTORS, DurableTransactionId, ExactVectorIndex, GraphId, HnswParams,
    HnswVectorIndex, MAIN_BRANCH, RecordFormat, StorageManifest, StorageTimestamp, StorageVersion,
    VectorIndex, VectorIndexKind, VectorIndexMode, VectorIndexPolicy, VectorMetric,
    create_storage_root,
};

/// SplitMix64: a small seeded generator so datasets replay exactly.
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [-1, 1).
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    fn unit(&mut self) -> f32 {
        ((self.next_u64() >> 11) as f64 / (1_u64 << 53) as f64 * 2.0 - 1.0) as f32
    }

    fn vector(&mut self, dimensions: usize) -> Vec<f32> {
        (0..dimensions).map(|_| self.unit()).collect()
    }
}

const DATASET: usize = 5_000;
const DIMENSIONS: usize = 64;
const QUERIES: usize = 200;
const K: usize = 10;

fn dataset(seed: u64) -> Vec<(String, Vec<f32>)> {
    let mut rng = Rng(seed);
    (0..DATASET)
        .map(|index| (format!("node--{index:05}"), rng.vector(DIMENSIONS)))
        .collect()
}

fn filled(index: &mut dyn VectorIndex, vectors: &[(String, Vec<f32>)]) {
    for (id, vector) in vectors {
        index.upsert(id.clone(), vector.clone());
    }
}

/// Mean recall@K of `candidate` against the exact index over seeded queries.
#[allow(clippy::cast_precision_loss)]
fn recall_at_k(metric: VectorMetric) -> f64 {
    let vectors = dataset(342);
    let mut exact = ExactVectorIndex::default();
    filled(&mut exact, &vectors);
    let mut hnsw = HnswVectorIndex::new(metric, HnswParams::default());
    filled(&mut hnsw, &vectors);
    assert_eq!(hnsw.len(), DATASET);
    assert_eq!(hnsw.kind(), VectorIndexKind::Hnsw);

    let mut rng = Rng(7_342);
    let mut found = 0_usize;
    for _ in 0..QUERIES {
        let query = rng.vector(DIMENSIONS);
        let truth: Vec<String> = exact
            .search(&query, K, metric)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        let approximate = hnsw.search(&query, K, metric);
        assert_eq!(approximate.len(), K);
        found += approximate
            .iter()
            .filter(|(id, _)| truth.contains(id))
            .count();
    }
    let recall = found as f64 / (QUERIES * K) as f64;
    eprintln!("hnsw recall@{K} {metric:?}: {recall:.4} ({DATASET} x {DIMENSIONS})");
    recall
}

#[test]
fn hnsw_recall_at_10_against_exact_is_at_least_95_percent_for_cosine() {
    let recall = recall_at_k(VectorMetric::Cosine);
    assert!(recall >= 0.95, "cosine recall@10 {recall}");
}

#[test]
fn hnsw_recall_at_10_against_exact_is_at_least_95_percent_for_l2() {
    let recall = recall_at_k(VectorMetric::L2);
    assert!(recall >= 0.95, "l2 recall@10 {recall}");
}

#[test]
fn hnsw_scores_match_the_exact_similarity() {
    let vectors = dataset(11);
    let mut exact = ExactVectorIndex::default();
    filled(&mut exact, &vectors[..500]);
    let mut hnsw = HnswVectorIndex::new(VectorMetric::Cosine, HnswParams::default());
    filled(&mut hnsw, &vectors[..500]);
    let query = vectors[42].1.clone();
    let approximate = hnsw.search(&query, 5, VectorMetric::Cosine);
    let truth = exact.search(&query, 5, VectorMetric::Cosine);
    assert_eq!(
        approximate[0].0, "node--00042",
        "a stored vector finds itself"
    );
    assert_eq!(approximate[0], truth[0], "scores use the exact similarity");
    for pair in approximate.windows(2) {
        assert!(pair[0].1 >= pair[1].1, "hits are ranked best first");
    }
}

#[test]
fn hnsw_search_with_another_metric_falls_back_to_exact() {
    let vectors = dataset(5);
    let mut exact = ExactVectorIndex::default();
    filled(&mut exact, &vectors[..800]);
    let mut hnsw = HnswVectorIndex::new(VectorMetric::Cosine, HnswParams::default());
    filled(&mut hnsw, &vectors[..800]);
    let mut rng = Rng(99);
    for _ in 0..10 {
        let query = rng.vector(DIMENSIONS);
        assert_eq!(
            hnsw.search(&query, K, VectorMetric::L2),
            exact.search(&query, K, VectorMetric::L2),
            "a metric the graph was not built for is answered exactly"
        );
    }
    assert_eq!(hnsw.metric(), VectorMetric::Cosine);
}

#[test]
fn hnsw_removed_vectors_are_excluded_and_upserts_replace() {
    let vectors = dataset(21);
    let mut hnsw = HnswVectorIndex::new(VectorMetric::L2, HnswParams::default());
    filled(&mut hnsw, &vectors[..1_000]);
    let target = vectors[123].1.clone();
    assert_eq!(
        hnsw.search(&target, 1, VectorMetric::L2)[0].0,
        "node--00123"
    );

    hnsw.remove("node--00123");
    assert_eq!(hnsw.len(), 999);
    let hits = hnsw.search(&target, K, VectorMetric::L2);
    assert_eq!(hits.len(), K);
    assert!(
        hits.iter().all(|(id, _)| id != "node--00123"),
        "a removed vector is never returned: {hits:?}"
    );
    let everything = hnsw.search(&target, 5_000, VectorMetric::L2);
    assert_eq!(
        everything.len(),
        999,
        "an exhaustive search skips tombstones"
    );
    assert!(everything.iter().all(|(id, _)| id != "node--00123"));

    // Moving a vector: the old position no longer matches, the new one does.
    let moved = vectors[900].1.clone();
    hnsw.upsert("node--00124".to_owned(), moved.clone());
    assert_eq!(hnsw.len(), 999, "an upsert of a known id replaces it");
    let hits = hnsw.search(&moved, 2, VectorMetric::L2);
    let ids: Vec<&str> = hits.iter().map(|(id, _)| id.as_str()).collect();
    assert!(ids.contains(&"node--00124") && ids.contains(&"node--00900"));
    assert!(
        hnsw.search(&vectors[124].1, 1, VectorMetric::L2)[0].0 != "node--00124",
        "the replaced vector is gone"
    );

    hnsw.upsert("node--00123".to_owned(), target.clone());
    assert_eq!(
        hnsw.search(&target, 1, VectorMetric::L2)[0].0,
        "node--00123"
    );
}

#[test]
fn hnsw_rebuilds_when_tombstones_dominate() {
    let vectors = dataset(31);
    let mut hnsw = HnswVectorIndex::new(VectorMetric::Cosine, HnswParams::default());
    filled(&mut hnsw, &vectors[..1_000]);
    for (id, _) in &vectors[..900] {
        hnsw.remove(id);
    }
    assert_eq!(hnsw.len(), 100);
    assert!(
        hnsw.tombstones() < hnsw.len(),
        "tombstones never outnumber live vectors: {} tombstones",
        hnsw.tombstones()
    );
    let mut exact = ExactVectorIndex::default();
    filled(&mut exact, &vectors[900..1_000]);
    let query = vectors[950].1.clone();
    assert_eq!(
        hnsw.search(&query, K, VectorMetric::Cosine)[0],
        exact.search(&query, K, VectorMetric::Cosine)[0]
    );
    for (id, _) in &vectors[900..1_000] {
        hnsw.remove(id);
    }
    assert!(hnsw.is_empty());
    assert!(hnsw.search(&query, K, VectorMetric::Cosine).is_empty());
}

#[test]
fn hnsw_results_are_deterministic_across_builds_and_clones() {
    let vectors = dataset(77);
    let build = || {
        let mut hnsw = HnswVectorIndex::new(VectorMetric::Cosine, HnswParams::default());
        filled(&mut hnsw, &vectors[..2_000]);
        for (id, _) in vectors[..2_000].iter().step_by(7) {
            hnsw.remove(id);
        }
        hnsw
    };
    let first = build();
    let second = build();
    let cloned = first.boxed_clone();
    let mut rng = Rng(1);
    for _ in 0..25 {
        let query = rng.vector(DIMENSIONS);
        let expected = first.search(&query, K, VectorMetric::Cosine);
        assert_eq!(second.search(&query, K, VectorMetric::Cosine), expected);
        assert_eq!(cloned.search(&query, K, VectorMetric::Cosine), expected);
    }
    assert_eq!(cloned.len(), first.len());
    assert_eq!(cloned.dimensions(), Some(DIMENSIONS));
    assert_eq!(cloned.kind(), VectorIndexKind::Hnsw);
}

#[test]
fn hnsw_seed_changes_the_graph_but_not_correctness() {
    let vectors = dataset(3);
    let mut seeded = HnswVectorIndex::new(
        VectorMetric::Cosine,
        HnswParams {
            seed: 12_345,
            ..HnswParams::default()
        },
    );
    filled(&mut seeded, &vectors[..300]);
    let query = vectors[10].1.clone();
    assert_eq!(
        seeded.search(&query, 1, VectorMetric::Cosine)[0].0,
        "node--00010"
    );
}

#[test]
fn index_policy_picks_the_implementation_by_mode_and_size() {
    let policy = VectorIndexPolicy::default();
    assert_eq!(policy.mode, VectorIndexMode::Auto);
    assert_eq!(policy.hnsw_min_vectors, DEFAULT_HNSW_MIN_VECTORS);
    assert_eq!(DEFAULT_HNSW_MIN_VECTORS, 10_000);
    assert_eq!(policy.kind_for(0), VectorIndexKind::Exact);
    assert_eq!(policy.kind_for(9_999), VectorIndexKind::Exact);
    assert_eq!(policy.kind_for(10_000), VectorIndexKind::Hnsw);

    let exact = VectorIndexPolicy {
        mode: VectorIndexMode::Exact,
        ..VectorIndexPolicy::default()
    };
    assert_eq!(exact.kind_for(1_000_000), VectorIndexKind::Exact);
    let hnsw = VectorIndexPolicy {
        mode: VectorIndexMode::Hnsw,
        ..VectorIndexPolicy::default()
    };
    assert_eq!(hnsw.kind_for(1), VectorIndexKind::Hnsw);

    for mode in [
        VectorIndexMode::Auto,
        VectorIndexMode::Exact,
        VectorIndexMode::Hnsw,
    ] {
        assert_eq!(VectorIndexMode::parse(mode.as_str()), Some(mode));
    }
    assert_eq!(
        VectorIndexMode::parse(" HNSW "),
        Some(VectorIndexMode::Hnsw)
    );
    assert_eq!(VectorIndexMode::parse("ivf"), None);
    assert_eq!(
        CanonicalStoreOptions::default().vector_index,
        VectorIndexPolicy::default()
    );
}

/// Tests in one process may start within the clock's resolution (about a
/// microsecond on macOS), so a counter keeps their directories apart.
static ROOTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn root() -> graph_storage::StorageRoot {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "corrobore-issue-342-{}-{unique}-{}",
        std::process::id(),
        ROOTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    create_storage_root(
        path,
        StorageManifest {
            storage_version: StorageVersion::V1,
            graph_id: GraphId {
                value: "graph--issue-342".to_owned(),
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

fn doc(label: &str, embedding: &[f32]) -> NodeInput {
    NodeInput::new(vec![label.to_owned()]).with_property(
        "embedding",
        PropertyValue::FloatList(embedding.iter().map(|value| f64::from(*value)).collect()),
    )
}

fn store_with(policy: VectorIndexPolicy) -> (graph_storage::StorageRoot, CanonicalEngineStore) {
    let root = root();
    let store = CanonicalEngineStore::open(
        root.clone(),
        CanonicalStoreOptions {
            vector_index: policy,
            ..CanonicalStoreOptions::default()
        },
    )
    .unwrap();
    (root, store)
}

#[test]
fn store_switches_from_exact_to_hnsw_when_the_index_grows() {
    let (root, mut store) = store_with(VectorIndexPolicy {
        mode: VectorIndexMode::Auto,
        hnsw_min_vectors: 40,
        ..VectorIndexPolicy::default()
    });
    let mut rng = Rng(342);
    let vectors: Vec<Vec<f32>> = (0..60).map(|_| rng.vector(8)).collect();
    let mut ids: Vec<NodeId> = Vec::new();
    write(&mut store, "tx--342-small", |graph| {
        for vector in &vectors[..30] {
            ids.push(graph.create_node(doc("Doc", vector)).unwrap());
        }
    });
    let small = store.graph_head(MAIN_BRANCH).unwrap();
    assert_eq!(
        store.vector_index_kind("embedding", VectorMetric::Cosine),
        None,
        "no index is built before the first search"
    );
    let hits = store
        .vector_search(
            &small,
            "embedding",
            &vectors[3],
            1,
            VectorMetric::Cosine,
            None,
        )
        .unwrap();
    assert_eq!(hits[0].node_id, ids[3].as_str());
    assert_eq!(
        store.vector_index_kind("embedding", VectorMetric::Cosine),
        Some(VectorIndexKind::Exact),
        "30 vectors stay below the threshold"
    );
    assert_eq!(
        store.vector_index_kind("embedding", VectorMetric::L2),
        Some(VectorIndexKind::Exact),
        "the exact index answers every metric"
    );

    write(&mut store, "tx--342-large", |graph| {
        for vector in &vectors[30..] {
            ids.push(graph.create_node(doc("Doc", vector)).unwrap());
        }
    });
    let large = store.graph_head(MAIN_BRANCH).unwrap();
    let hits = store
        .vector_search(
            &large,
            "embedding",
            &vectors[45],
            1,
            VectorMetric::Cosine,
            None,
        )
        .unwrap();
    assert_eq!(hits[0].node_id, ids[45].as_str());
    assert_eq!(
        store.vector_index_kind("embedding", VectorMetric::Cosine),
        Some(VectorIndexKind::Hnsw),
        "60 vectors cross the threshold"
    );
    assert_eq!(
        store.vector_index_kind("embedding", VectorMetric::L2),
        None,
        "an HNSW graph serves the metric it was built for"
    );
    let hits = store
        .vector_search(&large, "embedding", &vectors[50], 1, VectorMetric::L2, None)
        .unwrap();
    assert_eq!(hits[0].node_id, ids[50].as_str());
    assert_eq!(
        store.vector_index_kind("embedding", VectorMetric::L2),
        Some(VectorIndexKind::Hnsw),
        "a second metric gets its own graph"
    );
    assert_eq!(
        store.vector_index_kind("embedding", VectorMetric::Cosine),
        Some(VectorIndexKind::Hnsw),
        "the first graph is kept"
    );

    // Going back to the smaller commit rebuilds below the threshold.
    let back = store
        .vector_search(
            &small,
            "embedding",
            &vectors[45],
            60,
            VectorMetric::Cosine,
            None,
        )
        .unwrap();
    assert_eq!(back.len(), 30, "an older commit does not see the new nodes");
    assert_eq!(
        store.vector_index_kind("embedding", VectorMetric::Cosine),
        Some(VectorIndexKind::Exact)
    );

    let _ = std::fs::remove_dir_all(root.path());
}

#[test]
fn store_hnsw_follows_commits_removals_and_label_filters() {
    let (root, mut store) = store_with(VectorIndexPolicy {
        mode: VectorIndexMode::Hnsw,
        ..VectorIndexPolicy::default()
    });
    let mut ids: Vec<NodeId> = Vec::new();
    write(&mut store, "tx--342-seed", |graph| {
        ids.push(graph.create_node(doc("Doc", &[1.0, 0.0])).unwrap());
        ids.push(graph.create_node(doc("Doc", &[0.0, 1.0])).unwrap());
        ids.push(graph.create_node(doc("Note", &[0.9, 0.1])).unwrap());
    });
    let first = store.graph_head(MAIN_BRANCH).unwrap();
    let hits = store
        .vector_search(
            &first,
            "embedding",
            &[1.0, 0.05],
            2,
            VectorMetric::Cosine,
            None,
        )
        .unwrap();
    assert_eq!(
        store.vector_index_kind("embedding", VectorMetric::Cosine),
        Some(VectorIndexKind::Hnsw)
    );
    assert_eq!(hits[0].node_id, ids[0].as_str());
    assert_eq!(hits[1].node_id, ids[2].as_str());
    let notes = store
        .vector_search(
            &first,
            "embedding",
            &[0.0, 1.0],
            5,
            VectorMetric::Cosine,
            Some("Note"),
        )
        .unwrap();
    assert_eq!(notes.len(), 1, "the label filter applies to HNSW hits");
    assert_eq!(notes[0].node_id, ids[2].as_str());

    let removed = ids[0].clone();
    write(&mut store, "tx--342-delete", |graph| {
        graph.tombstone_node(&removed).unwrap();
    });
    let second = store.graph_head(MAIN_BRANCH).unwrap();
    let hits = store
        .vector_search(
            &second,
            "embedding",
            &[1.0, 0.0],
            3,
            VectorMetric::Cosine,
            None,
        )
        .unwrap();
    assert_eq!(hits.len(), 2);
    assert!(
        hits.iter().all(|hit| hit.node_id != removed.as_str()),
        "a deleted node leaves the index: {hits:?}"
    );
    assert_eq!(
        store.vector_index_kind("embedding", VectorMetric::Cosine),
        Some(VectorIndexKind::Hnsw)
    );

    let _ = std::fs::remove_dir_all(root.path());
}

#[test]
fn store_exact_mode_never_builds_hnsw() {
    let (root, mut store) = store_with(VectorIndexPolicy {
        mode: VectorIndexMode::Exact,
        hnsw_min_vectors: 1,
        ..VectorIndexPolicy::default()
    });
    write(&mut store, "tx--342-exact", |graph| {
        graph.create_node(doc("Doc", &[1.0, 0.0])).unwrap();
        graph.create_node(doc("Doc", &[0.0, 1.0])).unwrap();
    });
    let head = store.graph_head(MAIN_BRANCH).unwrap();
    store
        .vector_search(&head, "embedding", &[1.0, 0.0], 1, VectorMetric::L2, None)
        .unwrap();
    assert_eq!(
        store.vector_index_kind("embedding", VectorMetric::L2),
        Some(VectorIndexKind::Exact)
    );
    let _ = std::fs::remove_dir_all(root.path());
}

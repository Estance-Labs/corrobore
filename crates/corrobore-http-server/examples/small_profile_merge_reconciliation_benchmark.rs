// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! Reproducible issue #51 small-profile supernode merge and repair benchmark.

use std::{
    collections::{BTreeMap, HashMap},
    error::Error,
    fs,
    path::PathBuf,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use corrobore_http_server::opencti_reconciliation::OpenCtiReconciliationRuntime;
use graph_core::Graph;
use graph_storage::{
    CanonicalEngineStore, CanonicalProjectionRequest, CanonicalStoreOptions, DurableTransactionId,
    GraphId, RecordFormat, StorageManifest, StorageTimestamp, StorageVersion, create_storage_root,
};
use opencti_adapter::{
    MappedRecord, MergeLimits, OpenCtiAdapter, OpenCtiMergeExecutor, OpenCtiMergeRequest,
    OpenCtiReconciliationCommand, ReconciliationLimits, ReconciliationMode, ReconciliationScope,
};
use serde_json::{Value, json};

const SMALL_PROFILE_OBJECTS: usize = 100_000;
const SMALL_PROFILE_RELATIONSHIPS: usize = 500_000;
const BENCHMARK_OBJECTS: usize = 1_000;
const BENCHMARK_RELATIONSHIPS: usize = 4_000;
const REFERENCE_THROUGHPUT: f64 = 42_048.865;
const MINIMUM_MERGE_THROUGHPUT: f64 = 10_000.0;
const MINIMUM_REPAIR_THROUGHPUT: f64 = 1_500.0;

fn main() -> Result<(), Box<dyn Error>> {
    let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let path = std::env::temp_dir().join(format!(
        "corrobore-opencti-merge-reconcile-small-{}-{unique}",
        std::process::id()
    ));
    let _temporary_directory = TemporaryDirectory(path.clone());
    let root = create_benchmark_root(path)?;
    let mut store = CanonicalEngineStore::open(root, CanonicalStoreOptions::default())?;
    let seed = build_supernode_graph()?;
    store.commit_transition(
        &Graph::new(),
        &seed,
        DurableTransactionId::new("tx--issue-51-benchmark-seed")?,
        None,
    )?;

    let previous = store.load_projection(CanonicalProjectionRequest::all())?;
    let merge_started = Instant::now();
    let merge = OpenCtiMergeExecutor::new(MergeLimits {
        max_sources: 8,
        max_relationships: BENCHMARK_RELATIONSHIPS,
    })
    .apply(
        &previous,
        &OpenCtiMergeRequest::new(
            "merge--issue-51-benchmark",
            "indicator--benchmark-target",
            vec!["indicator--benchmark-source".to_owned()],
            BTreeMap::new(),
        )?,
    )?;
    let merge_planning_seconds = merge_started.elapsed().as_secs_f64();
    let merge_commit_started = Instant::now();
    store.commit_transition_with_audit(
        &previous,
        &merge.graph,
        DurableTransactionId::new("tx--issue-51-benchmark-merge")?,
        vec!["issue-51 payload-free merge benchmark receipt".to_owned()],
        None,
    )?;
    let merge_commit_seconds = merge_commit_started.elapsed().as_secs_f64();
    let merge_seconds = merge_planning_seconds + merge_commit_seconds;
    let scanned_records = BENCHMARK_OBJECTS + BENCHMARK_RELATIONSHIPS;
    let merge_throughput = scanned_records as f64 / merge_seconds;

    let merged = store.load_projection(CanonicalProjectionRequest::all())?;
    let mut reference_records = raw_records(&merged)?;
    for record in &mut reference_records {
        record["x_corrobore_benchmark_revision"] = json!(2);
    }
    let repair_record_count = reference_records.len();
    let command = OpenCtiReconciliationCommand::new(
        "reconcile--issue-51-benchmark",
        ReconciliationMode::Repair,
        ReconciliationScope::Full {
            max_records: repair_record_count,
        },
        reference_records,
        false,
    )?;
    let mut runtime = OpenCtiReconciliationRuntime::open(
        None,
        ReconciliationLimits {
            max_records: repair_record_count,
            max_payload_bytes: 64 * 1024 * 1024,
        },
        8,
    )?;
    let repair_started = Instant::now();
    let report = runtime.execute(&mut store, command)?;
    let repair_seconds = repair_started.elapsed().as_secs_f64();
    assert!(
        report.parity_verified,
        "benchmark repair must verify parity"
    );
    let repair_throughput = repair_record_count as f64 / repair_seconds;

    let output = json!({
        "schema_version": 1,
        "recorded_at_unix_seconds": SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        "engine": "corrobore-canonical-wal-v1",
        "environment": {
            "architecture": std::env::consts::ARCH,
            "operating_system": std::env::consts::OS,
            "build_profile": if cfg!(debug_assertions) { "debug" } else { "release" }
        },
        "profile": {
            "id": "small",
            "target_objects": SMALL_PROFILE_OBJECTS,
            "target_relationships": SMALL_PROFILE_RELATIONSHIPS,
            "measured_objects": BENCHMARK_OBJECTS,
            "supernode_relationships": BENCHMARK_RELATIONSHIPS,
            "merge_sources": 1,
            "repair_records": repair_record_count,
            "concurrency": 1
        },
        "merge": {
            "planning_seconds": merge_planning_seconds,
            "durable_commit_seconds": merge_commit_seconds,
            "end_to_end_seconds": merge_seconds,
            "throughput_scanned_records_per_second": merge_throughput,
            "redirected_relationships": merge.redirected_relationship_ids.len(),
            "deduplicated_relationships": merge.deduplicated_relationship_ids.len(),
            "wal_fsync_required": true
        },
        "repair": {
            "end_to_end_seconds": repair_seconds,
            "throughput_records_per_second": repair_throughput,
            "parity_verified": report.parity_verified,
            "projection_rebuilt": !report.projection_rebuild_ids.is_empty(),
            "wal_fsync_required": true
        },
        "reference_context": {
            "reference_engine": "opensearch-3.7.0",
            "reference_profile": "small",
            "reference_throughput_records_per_second": REFERENCE_THROUGHPUT,
            "comparison": "context_only_bulk_ingestion_is_not_the_merge_repair_workload"
        },
        "performance_gate": {
            "minimum_merge_scanned_records_per_second": MINIMUM_MERGE_THROUGHPUT,
            "minimum_repair_records_per_second": MINIMUM_REPAIR_THROUGHPUT,
            "merge_passed": merge_throughput >= MINIMUM_MERGE_THROUGHPUT,
            "repair_passed": repair_throughput >= MINIMUM_REPAIR_THROUGHPUT,
            "passed": merge_throughput >= MINIMUM_MERGE_THROUGHPUT && repair_throughput >= MINIMUM_REPAIR_THROUGHPUT
        },
        "reproduction": {
            "command": "cargo run --release -p corrobore-http-server --example small_profile_merge_reconciliation_benchmark --locked"
        }
    });
    println!("{}", serde_json::to_string_pretty(&output)?);
    if merge_throughput < MINIMUM_MERGE_THROUGHPUT || repair_throughput < MINIMUM_REPAIR_THROUGHPUT
    {
        return Err(format!(
            "merge {merge_throughput:.3} or repair {repair_throughput:.3} records/s is below its workload gate"
        )
        .into());
    }
    Ok(())
}

fn build_supernode_graph() -> Result<Graph, Box<dyn Error>> {
    let objects = vec![
        json!({"id":"indicator--benchmark-target","type":"indicator","name":"target"}),
        json!({"id":"indicator--benchmark-source","type":"indicator","name":"source"}),
    ]
    .into_iter()
    .chain((0..BENCHMARK_OBJECTS - 2).map(|index| {
        json!({
            "id": format!("malware--benchmark-{index:06}"),
            "type": "malware",
            "name": format!("malware {index:06}")
        })
    }));
    let adapter = OpenCtiAdapter::pinned();
    let mut graph = Graph::new();
    let mut node_ids = HashMap::new();
    for raw in objects {
        let canonical_id = raw["id"]
            .as_str()
            .ok_or("benchmark object ID is required")?
            .to_owned();
        let MappedRecord::Object(mapped) = adapter.map(raw)? else {
            return Err("benchmark object mapped as relationship".into());
        };
        node_ids.insert(canonical_id, graph.create_node(mapped.to_node_input())?);
    }
    for index in 0..BENCHMARK_RELATIONSHIPS {
        let raw = json!({
            "id": format!("relationship--benchmark-{index:06}"),
            "type": "relationship",
            "relationship_type": "indicates",
            "source_ref": "indicator--benchmark-source",
            "target_ref": format!("malware--benchmark-{:06}", index % (BENCHMARK_OBJECTS - 2)),
            "object_marking_refs": [format!("marking--{}", index % 4)]
        });
        let MappedRecord::Relationship(mapped) = adapter.map(raw)? else {
            return Err("benchmark relationship mapped as object".into());
        };
        let source = node_ids
            .get(mapped.source_ref())
            .ok_or("benchmark source is missing")?
            .clone();
        let target = node_ids
            .get(mapped.target_ref())
            .ok_or("benchmark target is missing")?
            .clone();
        graph.create_relationship(mapped.to_relationship_input(source, target)?)?;
    }
    Ok(graph)
}

fn raw_records(graph: &Graph) -> Result<Vec<Value>, Box<dyn Error>> {
    let adapter = OpenCtiAdapter::pinned();
    let mut records = graph
        .list_nodes()?
        .into_iter()
        .map(|node| {
            adapter
                .restore_node(&node)
                .map(|record| record.raw().clone())
        })
        .chain(graph.list_relationships()?.into_iter().map(|relationship| {
            adapter
                .restore_relationship(&relationship)
                .map(|record| record.raw().clone())
        }))
        .collect::<Result<Vec<_>, _>>()?;
    records.sort_by(|left, right| left["id"].as_str().cmp(&right["id"].as_str()));
    Ok(records)
}

fn create_benchmark_root(path: PathBuf) -> Result<graph_storage::StorageRoot, Box<dyn Error>> {
    let root = create_storage_root(
        path,
        StorageManifest {
            storage_version: StorageVersion::V1,
            graph_id: GraphId {
                value: "graph--issue-51-small-profile".to_owned(),
            },
            created_at: StorageTimestamp {
                value: "2026-07-26T00:00:00Z".to_owned(),
            },
            updated_at: StorageTimestamp {
                value: "2026-07-26T00:00:00Z".to_owned(),
            },
            record_format: RecordFormat::JsonLinesV1,
        },
    )?;
    for relative in [
        "nodes/node_records.log",
        "relationships/relationship_records.log",
        "adjacency/outgoing_adjacency.log",
        "adjacency/incoming_adjacency.log",
    ] {
        let path = root.path().join(relative);
        fs::create_dir_all(path.parent().ok_or("benchmark log path has no parent")?)?;
        fs::write(path, b"")?;
    }
    Ok(root)
}

struct TemporaryDirectory(PathBuf);

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! Reproducible issue #50 WAL-backed small-profile write benchmark.

use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use graph_core::{Graph, NodeId, NodeInput, PropertyValue, RelationshipInput};
use graph_storage::{
    CanonicalEngineStore, CanonicalProjectionRequest, CanonicalStoreOptions, DurableTransactionId,
    GraphId, RecordFormat, StorageManifest, StorageTimestamp, StorageVersion, create_storage_root,
};
use serde_json::json;

const SMALL_PROFILE_OBJECTS: usize = 100_000;
const SMALL_PROFILE_RELATIONSHIPS: usize = 500_000;
const BULK_OBJECTS: usize = 1_000;
const BULK_RELATIONSHIPS: usize = 4_000;
const REFERENCE_THROUGHPUT: f64 = 42_048.865;
const MAXIMUM_REGRESSION_PERCENT: f64 = 20.0;

fn main() -> Result<(), Box<dyn Error>> {
    let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let path = std::env::temp_dir().join(format!(
        "corrobore-opencti-write-small-profile-{}-{unique}",
        std::process::id()
    ));
    let _temporary_directory = TemporaryDirectory(path.clone());
    let root = create_benchmark_root(path)?;
    let mut store = CanonicalEngineStore::open(root, CanonicalStoreOptions::default())?;

    let mut warmup = Graph::new();
    warmup.create_node(NodeInput::new(["BenchmarkWarmup"]))?;
    store.commit_transition(
        &Graph::new(),
        &warmup,
        DurableTransactionId::new("tx--issue-50-benchmark-warmup")?,
        None,
    )?;
    let first_warmup = warmup.clone();
    warmup.create_node(NodeInput::new(["BenchmarkWarmup"]))?;
    store.commit_transition(
        &first_warmup,
        &warmup,
        DurableTransactionId::new("tx--issue-50-benchmark-checkpoint-warmup")?,
        None,
    )?;
    let previous = store.load_projection(CanonicalProjectionRequest::all())?;
    let planning_started = Instant::now();
    let graph = build_reference_bulk(previous.clone())?;
    let planning_seconds = planning_started.elapsed().as_secs_f64();
    let commit_started = Instant::now();
    let outcome = store.commit_transition_with_audit(
        &previous,
        &graph,
        DurableTransactionId::new("tx--issue-50-small-profile")?,
        vec!["issue-50 small-profile payload-free benchmark receipt".to_owned()],
        None,
    )?;
    let commit_seconds = commit_started.elapsed().as_secs_f64();
    assert!(outcome.applied, "benchmark transition must commit");

    let records = BULK_OBJECTS + BULK_RELATIONSHIPS;
    let end_to_end_seconds = planning_seconds + commit_seconds;
    let throughput = records as f64 / end_to_end_seconds;
    let minimum_throughput = REFERENCE_THROUGHPUT * (1.0 - MAXIMUM_REGRESSION_PERCENT / 100.0);
    let output = json!({
        "schema_version": 1,
        "recorded_at_unix_seconds": SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_secs(),
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
            "reference_bulk_documents": records,
            "bulk_objects": BULK_OBJECTS,
            "bulk_relationships": BULK_RELATIONSHIPS,
            "atomic_transactions": 1,
            "concurrency": 1
        },
        "planning": {
            "elapsed_seconds": planning_seconds,
            "throughput_records_per_second": records as f64 / planning_seconds
        },
        "durable_commit": {
            "elapsed_seconds": commit_seconds,
            "throughput_records_per_second": records as f64 / commit_seconds,
            "wal_fsync_required": true,
            "disk_bytes": directory_size(store.root().path())?
        },
        "end_to_end": {
            "elapsed_seconds": end_to_end_seconds,
            "throughput_records_per_second": throughput,
            "resident_memory_bytes": process_resident_bytes()
        },
        "parity_gate": {
            "reference_engine": "opensearch-3.7.0",
            "reference_profile": "small",
            "reference_throughput_records_per_second": REFERENCE_THROUGHPUT,
            "maximum_regression_percent": MAXIMUM_REGRESSION_PERCENT,
            "minimum_throughput_records_per_second": minimum_throughput,
            "passed": throughput >= minimum_throughput
        },
        "reproduction": {
            "command": "cargo run --release -p corrobore-http-server --example small_profile_transactional_write_benchmark --locked"
        }
    });
    println!("{}", serde_json::to_string_pretty(&output)?);
    if throughput < minimum_throughput {
        return Err(format!(
            "end-to-end throughput {throughput:.3} records/s is below gate {minimum_throughput:.3} records/s"
        )
        .into());
    }
    Ok(())
}

fn build_reference_bulk(mut graph: Graph) -> Result<Graph, Box<dyn Error>> {
    let mut ids = Vec::<NodeId>::with_capacity(BULK_OBJECTS);
    for index in 0..BULK_OBJECTS {
        ids.push(
            graph.create_node(NodeInput::new(["OpenCtiObject"]).with_property(
                "opencti.raw",
                PropertyValue::Json(json!({
                    "id": format!("indicator--{index:012}"),
                    "type": "indicator",
                    "name": format!("Synthetic observation {index:012}")
                })),
            ))?,
        );
    }
    for index in 0..BULK_RELATIONSHIPS {
        graph.create_relationship(
            RelationshipInput::new(
                ids[index % BULK_OBJECTS].clone(),
                "RELATED_TO",
                ids[(index + 1) % BULK_OBJECTS].clone(),
            )?
            .with_property(
                "opencti.raw",
                PropertyValue::Json(json!({
                    "id": format!("relationship--{index:012}"),
                    "type": "relationship",
                    "relationship_type": "related-to"
                })),
            ),
        )?;
    }
    Ok(graph)
}

fn create_benchmark_root(path: PathBuf) -> Result<graph_storage::StorageRoot, Box<dyn Error>> {
    let root = create_storage_root(
        path,
        StorageManifest {
            storage_version: StorageVersion::V1,
            graph_id: GraphId {
                value: "graph--issue-50-small-profile".to_owned(),
            },
            created_at: StorageTimestamp {
                value: "2026-07-25T00:00:00Z".to_owned(),
            },
            updated_at: StorageTimestamp {
                value: "2026-07-25T00:00:00Z".to_owned(),
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
        fs::create_dir_all(
            path.parent()
                .ok_or("benchmark log path must have a parent")?,
        )?;
        fs::write(path, b"")?;
    }
    Ok(root)
}

fn directory_size(path: &Path) -> Result<u64, Box<dyn Error>> {
    let mut total = 0_u64;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        total = total.saturating_add(if metadata.is_dir() {
            directory_size(&entry.path())?
        } else {
            metadata.len()
        });
    }
    Ok(total)
}

fn process_resident_bytes() -> Option<u64> {
    let output = Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(|kilobytes| kilobytes.saturating_mul(1_024))
}

struct TemporaryDirectory(PathBuf);

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

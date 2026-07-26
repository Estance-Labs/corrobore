// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

use std::{
    collections::BTreeMap,
    sync::Mutex,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use corrobore_engine::{
    AccessContext, AggregateRequest, Aggregation, AggregationPlan, ConsistencyLevel,
    ContractVersion, CorroboreEngine, CorroboreKnowledgeDataProvider, DateHistogramInterval,
    EnginePersistence, KnowledgeDataOperation, KnowledgeDataOutcome, KnowledgeDataRequest,
    RequestContext,
};
use graph_core::{Graph, NodeId, NodeInput, PropertyValue, RelationshipInput};
use serde_json::{Value, json};

const OBJECTS: usize = 100_000;
const RELATIONSHIPS: usize = 500_000;
const WARMUPS: usize = 20;
const ITERATIONS: usize = 60;
const KEY: &[u8] = b"issue-47-small-profile-benchmark-key";

#[derive(Debug)]
struct MoveOncePersistence {
    graph: Mutex<Option<Graph>>,
}

impl EnginePersistence for MoveOncePersistence {
    fn load_graph(&self) -> Result<Graph, String> {
        self.graph
            .lock()
            .map_err(|_| "benchmark graph lock is poisoned".to_owned())?
            .take()
            .ok_or_else(|| "benchmark graph was already loaded".to_owned())
    }

    fn persist_graph(&mut self, _graph: &Graph) -> Result<(), String> {
        Ok(())
    }
}

fn build_small_profile() -> Graph {
    let mut graph = Graph::new();
    let mut ids = Vec::<NodeId>::with_capacity(OBJECTS);
    for index in 0..OBJECTS {
        let day = 1 + (index % 28);
        let raw = json!({
            "id": format!("indicator--{index:012}"),
            "type": "indicator",
            "valid_from": format!("2026-01-{day:02}T00:00:00.000Z"),
        });
        ids.push(
            graph
                .create_node(
                    NodeInput::new(["Indicator"])
                        .with_property("opencti.raw", PropertyValue::Json(raw)),
                )
                .expect("benchmark node should be valid"),
        );
    }
    for index in 0..RELATIONSHIPS {
        let source = ids[index % OBJECTS].clone();
        let target = ids[(index + 1) % OBJECTS].clone();
        graph
            .create_relationship(
                RelationshipInput::new(source, "RELATED_TO", target)
                    .expect("benchmark relationship input should be valid")
                    .with_property(
                        "opencti.raw",
                        PropertyValue::Json(json!({
                            "id": format!("relationship--{index:012}"),
                            "type": "relationship",
                        })),
                    ),
            )
            .expect("benchmark relationship should be valid");
    }
    graph
}

fn access() -> AccessContext {
    AccessContext {
        subject_id: "system--advanced-benchmark".to_owned(),
        roles: vec!["system".to_owned()],
        attributes: BTreeMap::from([("policy_version".to_owned(), "benchmark-v1".to_owned())]),
        ..AccessContext::default()
    }
}

fn execute(engine: &mut CorroboreEngine, plan: AggregationPlan, sequence: usize) {
    let response = CorroboreKnowledgeDataProvider::new(engine, KEY)
        .expect("benchmark provider should initialize")
        .execute(KnowledgeDataRequest {
            contract_version: ContractVersion::CURRENT,
            context: RequestContext {
                request_id: format!("benchmark-request-{sequence}"),
                correlation_id: format!("benchmark-correlation-{sequence}"),
                access: access(),
                consistency: ConsistencyLevel::Snapshot,
                ..RequestContext::default()
            },
            operation: KnowledgeDataOperation::Aggregate(AggregateRequest { plan }),
        });
    assert!(matches!(
        response.outcome,
        KnowledgeDataOutcome::Success { .. }
    ));
}

fn percentile_ms(samples: &mut [f64], percentile: f64) -> f64 {
    samples.sort_by(f64::total_cmp);
    let index = ((samples.len() - 1) as f64 * percentile).ceil() as usize;
    samples[index]
}

fn measure(engine: &mut CorroboreEngine, plan: AggregationPlan, base: usize) -> Value {
    execute(engine, plan.clone(), base);
    for sequence in 0..WARMUPS {
        execute(engine, plan.clone(), base + sequence + 1);
    }
    let mut samples = Vec::with_capacity(ITERATIONS);
    for sequence in 0..ITERATIONS {
        let started = Instant::now();
        execute(engine, plan.clone(), base + WARMUPS + sequence + 1);
        samples.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    let p50 = percentile_ms(&mut samples.clone(), 0.50);
    let p95 = percentile_ms(&mut samples.clone(), 0.95);
    let p99 = percentile_ms(&mut samples, 0.99);
    json!({
        "latency_p50_ms": p50,
        "latency_p95_ms": p95,
        "latency_p99_ms": p99,
        "throughput_ops_per_second": 1000.0 / p50,
    })
}

fn main() {
    let build_started = Instant::now();
    let graph = build_small_profile();
    let build_seconds = build_started.elapsed().as_secs_f64();
    let mut engine = CorroboreEngine::builder()
        .persistence(Box::new(MoveOncePersistence {
            graph: Mutex::new(Some(graph)),
        }))
        .build()
        .expect("benchmark engine should initialize");
    let terms_plan = AggregationPlan {
        kinds: Vec::new(),
        predicate: None,
        aggregation: Aggregation::Terms {
            field: "type".to_owned(),
            limit: 20,
        },
        candidate_limit: 600_000,
        include_relationships: true,
    };
    let histogram_plan = AggregationPlan {
        kinds: vec!["indicator".to_owned()],
        predicate: None,
        aggregation: Aggregation::DateHistogram {
            field: "valid_from".to_owned(),
            interval: DateHistogramInterval::Day,
            time_zone_offset_minutes: 0,
            include_empty: false,
        },
        candidate_limit: 600_000,
        include_relationships: false,
    };
    let terms = measure(&mut engine, terms_plan, 0);
    let histogram = measure(&mut engine, histogram_plan, 1_000);
    let recorded_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema_version": 1,
            "recorded_at_unix_seconds": recorded_at,
            "engine": "corrobore-advanced-query-cache-v1",
            "profile": {
                "id": "small",
                "objects": OBJECTS,
                "relationships": RELATIONSHIPS,
                "records": OBJECTS + RELATIONSHIPS,
                "warmup_iterations": WARMUPS,
                "measurement_iterations": ITERATIONS,
                "concurrency": 1,
            },
            "ingestion": {
                "elapsed_seconds": build_seconds,
                "throughput_records_per_second": (OBJECTS + RELATIONSHIPS) as f64 / build_seconds,
            },
            "workload_metrics": {
                "terms-aggregation": terms,
                "date-histogram": histogram,
            },
            "parity_gate": {
                "reference_engine": "opensearch-3.7.0",
                "maximum_regression_percent": 20,
                "terms_reference_p95_ms": 4.845,
                "terms_maximum_p95_ms": 5.814,
                "histogram_reference_p95_ms": 4.857,
                "histogram_maximum_p95_ms": 5.8284,
            },
            "reproduction": {
                "command": "cargo run --release -p corrobore-engine --example small_profile_advanced_query_benchmark --locked",
            }
        }))
        .expect("benchmark result should serialize")
    );
}

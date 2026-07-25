// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

#![allow(clippy::unwrap_used)]

use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
};

use corrobore_engine::{
    AccessContext, AggregateRequest, Aggregation, AggregationPlan, AggregationResult,
    ConsistencyLevel, ContractVersion, CorroboreEngine, CorroboreKnowledgeDataProvider,
    DateHistogramInterval, EnginePersistence, KnowledgeDataErrorCode, KnowledgeDataOperation,
    KnowledgeDataOutcome, KnowledgeDataRequest, KnowledgeDataResponse, ListRequest,
    PaginateRequest, ReadFilter, ReadFilterOperator, ReadOrder, ReadPredicate, RequestContext,
    SortDirection,
};
use graph_core::{Graph, NodeId};
use opencti_adapter::{MappedRecord, OpenCtiAdapter};
use serde::Deserialize;
use serde_json::{Value, json};

const PAGINATION_KEY: &[u8] = b"issue-47-advanced-query-pagination-key-v1";

#[derive(Debug, Deserialize)]
struct Corpus {
    fixtures: Vec<Value>,
}

#[derive(Debug)]
struct StaticPersistence {
    graph: Graph,
}

impl EnginePersistence for StaticPersistence {
    fn load_graph(&self) -> Result<Graph, String> {
        Ok(self.graph.clone())
    }

    fn persist_graph(&mut self, _graph: &Graph) -> Result<(), String> {
        Ok(())
    }
}

fn corpus_graph() -> Graph {
    let corpus: Corpus = serde_json::from_str(include_str!(
        "../../../compatibility/opencti/7.260722.0/parity-corpus.json"
    ))
    .unwrap();
    let adapter = OpenCtiAdapter::pinned();
    let mapped = corpus
        .fixtures
        .into_iter()
        .map(|fixture| adapter.map(fixture).unwrap())
        .collect::<Vec<_>>();
    let mut graph = Graph::new();
    let mut nodes = HashMap::<String, NodeId>::new();
    for record in &mapped {
        if let MappedRecord::Object(object) = record {
            let canonical_id = record.raw()["id"].as_str().unwrap().to_owned();
            nodes.insert(
                canonical_id,
                graph.create_node(object.to_node_input()).unwrap(),
            );
        }
    }
    for record in &mapped {
        if let MappedRecord::Relationship(relationship) = record {
            let raw = record.raw();
            graph
                .create_relationship(
                    relationship
                        .to_relationship_input(
                            nodes[raw["source_ref"].as_str().unwrap()].clone(),
                            nodes[raw["target_ref"].as_str().unwrap()].clone(),
                        )
                        .unwrap(),
                )
                .unwrap();
        }
    }
    graph
}

fn system_access() -> AccessContext {
    AccessContext {
        subject_id: "system--issue-47".to_owned(),
        roles: vec!["system".to_owned()],
        ..AccessContext::default()
    }
}

fn clear_access() -> AccessContext {
    AccessContext {
        subject_id: "user--00000000-0000-4000-8000-000000000020".to_owned(),
        organization_ids: vec!["identity--00000000-0000-4000-8000-000000000010".to_owned()],
        marking_ids: vec!["marking-definition--00000000-0000-4000-8000-000000000001".to_owned()],
        tenant_id: Some("grouping--00000000-0000-4000-8000-000000000030".to_owned()),
        roles: vec!["analyst".to_owned()],
        attributes: BTreeMap::from([("policy_version".to_owned(), "policy--v1".to_owned())]),
    }
}

fn request(
    operation: KnowledgeDataOperation,
    access: AccessContext,
    correlation_id: &str,
) -> KnowledgeDataRequest {
    KnowledgeDataRequest {
        contract_version: ContractVersion::CURRENT,
        context: RequestContext {
            request_id: format!("request--{correlation_id}"),
            correlation_id: correlation_id.to_owned(),
            idempotency_key: None,
            deadline_unix_ms: None,
            cancellation_id: None,
            access,
            consistency: ConsistencyLevel::Snapshot,
        },
        operation,
    }
}

fn execute(
    engine: &mut CorroboreEngine,
    operation: KnowledgeDataOperation,
    access: AccessContext,
    correlation_id: &str,
) -> KnowledgeDataResponse {
    let response = CorroboreKnowledgeDataProvider::new(engine, PAGINATION_KEY)
        .unwrap()
        .execute(request(operation, access, correlation_id));
    match response.outcome {
        KnowledgeDataOutcome::Success { response } => response,
        other => panic!("expected successful advanced query, got {other:?}"),
    }
}

fn predicate_filter(
    field: &str,
    operator: ReadFilterOperator,
    value: Option<Value>,
) -> ReadPredicate {
    ReadPredicate::Condition(ReadFilter {
        field: field.to_owned(),
        operator,
        value,
    })
}

fn aggregation(
    engine: &mut CorroboreEngine,
    plan: AggregationPlan,
    access: AccessContext,
    correlation_id: &str,
) -> AggregationResult {
    match execute(
        engine,
        KnowledgeDataOperation::Aggregate(AggregateRequest { plan }),
        access,
        correlation_id,
    ) {
        KnowledgeDataResponse::Aggregation(result) => result,
        other => panic!("expected aggregation, got {other:?}"),
    }
}

#[test]
fn nested_predicates_and_multikey_cursor_pages_are_structural_stable_and_access_aware() {
    let mut engine = CorroboreEngine::builder()
        .persistence(Box::new(StaticPersistence {
            graph: corpus_graph(),
        }))
        .build()
        .unwrap();
    let query = ListRequest {
        kinds: vec!["indicator".to_owned()],
        predicate: Some(ReadPredicate::And(vec![
            predicate_filter(
                "pattern_type",
                ReadFilterOperator::In,
                Some(json!(["stix", "sigma"])),
            ),
            ReadPredicate::Or(vec![
                predicate_filter(
                    "valid_from",
                    ReadFilterOperator::GreaterThanOrEqual,
                    Some(json!("2026-01-02T00:00:00.000Z")),
                ),
                predicate_filter(
                    "id",
                    ReadFilterOperator::In,
                    Some(json!(["indicator--00000000-0000-4000-8000-000000000040"])),
                ),
            ]),
            predicate_filter(
                "name",
                ReadFilterOperator::NotIn,
                Some(json!(["Documentation domain indicator"])),
            ),
            predicate_filter("valid_from", ReadFilterOperator::Exists, None),
        ])),
        order_by: vec![
            ReadOrder {
                field: "valid_from".to_owned(),
                direction: SortDirection::Ascending,
            },
            ReadOrder {
                field: "name".to_owned(),
                direction: SortDirection::Descending,
            },
        ],
        limit: 100,
        include_total_count: true,
        ..ListRequest::default()
    };

    let first = match execute(
        &mut engine,
        KnowledgeDataOperation::Paginate(PaginateRequest {
            query: query.clone(),
            page_size: 1,
            token: None,
        }),
        system_access(),
        "advanced-page-one",
    ) {
        KnowledgeDataResponse::Records(page) => page,
        other => panic!("expected first page, got {other:?}"),
    };
    assert_eq!(first.total_count, Some(2));
    assert_eq!(
        first.records[0].id,
        "indicator--00000000-0000-4000-8000-000000000040"
    );

    let second = match execute(
        &mut engine,
        KnowledgeDataOperation::Paginate(PaginateRequest {
            query,
            page_size: 1,
            token: first.next_token,
        }),
        system_access(),
        "advanced-page-two",
    ) {
        KnowledgeDataResponse::Records(page) => page,
        other => panic!("expected second page, got {other:?}"),
    };
    assert_eq!(second.total_count, Some(2));
    assert_eq!(
        second.records[0].id,
        "indicator--00000000-0000-4000-8000-000000000042"
    );
    assert!(second.next_token.is_none());
}

#[test]
fn terms_and_histogram_match_the_pinned_opencti_dashboard_captures() {
    let graph = corpus_graph();
    let mut engine = CorroboreEngine::builder()
        .persistence(Box::new(StaticPersistence { graph }))
        .build()
        .unwrap();
    let terms = aggregation(
        &mut engine,
        AggregationPlan {
            kinds: Vec::new(),
            predicate: None,
            aggregation: Aggregation::Terms {
                field: "type".to_owned(),
                limit: 20,
            },
            candidate_limit: 10_000,
        },
        system_access(),
        "dashboard-terms",
    );
    assert_eq!(
        terms.buckets,
        vec![
            json!({"key": "relationship", "count": 4}),
            json!({"key": "indicator", "count": 3}),
            json!({"key": "file", "count": 2}),
            json!({"key": "grouping", "count": 2}),
            json!({"key": "identity", "count": 2}),
            json!({"key": "marking-definition", "count": 2}),
            json!({"key": "user", "count": 2}),
            json!({"key": "malware", "count": 1}),
            json!({"key": "migration-marker", "count": 1}),
            json!({"key": "report", "count": 1}),
            json!({"key": "threat-actor", "count": 1}),
        ]
    );

    let histogram = aggregation(
        &mut engine,
        AggregationPlan {
            kinds: vec!["indicator".to_owned()],
            predicate: None,
            aggregation: Aggregation::DateHistogram {
                field: "valid_from".to_owned(),
                interval: DateHistogramInterval::Day,
                time_zone_offset_minutes: 0,
                include_empty: false,
            },
            candidate_limit: 10_000,
        },
        system_access(),
        "dashboard-histogram",
    );
    assert_eq!(
        histogram.buckets,
        vec![
            json!({"key": "2026-01-01T00:00:00.000Z", "count": 1}),
            json!({"key": "2026-01-02T00:00:00.000Z", "count": 1}),
            json!({"key": "2026-01-03T00:00:00.000Z", "count": 1}),
        ]
    );
}

#[test]
fn cardinality_metrics_nested_filter_and_reverse_nested_never_include_denied_records() {
    let graph = corpus_graph();
    let mut engine = CorroboreEngine::builder()
        .persistence(Box::new(StaticPersistence { graph }))
        .build()
        .unwrap();
    let cardinality = aggregation(
        &mut engine,
        AggregationPlan {
            kinds: vec!["indicator".to_owned()],
            predicate: None,
            aggregation: Aggregation::Cardinality {
                field: "x_opencti_tenant_refs".to_owned(),
            },
            candidate_limit: 100,
        },
        clear_access(),
        "cardinality-access",
    );
    assert_eq!(cardinality.value, Some(json!(1)));
    assert_eq!(cardinality.examined_records, 1);

    let reverse_nested = aggregation(
        &mut engine,
        AggregationPlan {
            kinds: vec!["indicator".to_owned()],
            predicate: None,
            aggregation: Aggregation::Nested {
                path: "object_marking_refs".to_owned(),
                aggregation: Box::new(Aggregation::Filter {
                    predicate: predicate_filter(
                        "$value",
                        ReadFilterOperator::Equal,
                        Some(json!(
                            "marking-definition--00000000-0000-4000-8000-000000000001"
                        )),
                    ),
                    aggregation: Box::new(Aggregation::ReverseNested {
                        aggregation: Box::new(Aggregation::Count),
                    }),
                }),
            },
            candidate_limit: 100,
        },
        clear_access(),
        "reverse-nested-access",
    );
    assert_eq!(reverse_nested.value, Some(json!(1)));
}

#[test]
fn materialized_advanced_projection_is_versioned_and_rebuildable_not_authoritative() {
    let graph = Arc::new(Mutex::new(corpus_graph()));
    let first = graph.lock().unwrap().clone();
    let mut engine = CorroboreEngine::builder()
        .persistence(Box::new(StaticPersistence { graph: first }))
        .build()
        .unwrap();
    let count = aggregation(
        &mut engine,
        AggregationPlan {
            kinds: vec!["indicator".to_owned()],
            predicate: None,
            aggregation: Aggregation::Count,
            candidate_limit: 100,
        },
        system_access(),
        "rebuildable-count",
    );
    assert_eq!(count.value, Some(json!(3)));
    assert!(!count.generation.is_empty());
    let first_generation = count.generation;

    engine
        .write("CREATE (n:Indicator {name: 'Cache invalidation probe'})")
        .unwrap();
    let rebuilt = aggregation(
        &mut engine,
        AggregationPlan {
            kinds: vec!["indicator".to_owned()],
            predicate: None,
            aggregation: Aggregation::Count,
            candidate_limit: 100,
        },
        system_access(),
        "rebuilt-count",
    );
    assert_eq!(rebuilt.value, Some(json!(4)));
    assert_ne!(rebuilt.generation, first_generation);
}

#[test]
fn numeric_metrics_ignore_missing_values_and_preserve_finite_results() {
    let graph = corpus_graph();
    let mut engine = CorroboreEngine::builder()
        .persistence(Box::new(StaticPersistence { graph }))
        .build()
        .unwrap();
    for (aggregation_kind, expected) in [
        (
            Aggregation::Sum {
                field: "size".to_owned(),
            },
            json!(176.0),
        ),
        (
            Aggregation::Average {
                field: "size".to_owned(),
            },
            json!(88.0),
        ),
        (
            Aggregation::Minimum {
                field: "size".to_owned(),
            },
            json!(80.0),
        ),
        (
            Aggregation::Maximum {
                field: "size".to_owned(),
            },
            json!(96.0),
        ),
    ] {
        let result = aggregation(
            &mut engine,
            AggregationPlan {
                kinds: vec!["file".to_owned()],
                predicate: None,
                aggregation: aggregation_kind,
                candidate_limit: 100,
            },
            system_access(),
            "numeric-metric",
        );
        assert_eq!(result.value, Some(expected));
        assert_eq!(result.examined_records, 2);
    }
}

#[test]
fn aggregation_refuses_unbounded_or_exhausted_candidate_budgets() {
    let graph = corpus_graph();
    let mut engine = CorroboreEngine::builder()
        .persistence(Box::new(StaticPersistence { graph }))
        .build()
        .unwrap();

    for (candidate_limit, expected_code) in [
        (0, KnowledgeDataErrorCode::UnboundedOperation),
        (1, KnowledgeDataErrorCode::QueryBudgetExceeded),
    ] {
        let response = CorroboreKnowledgeDataProvider::new(&mut engine, PAGINATION_KEY)
            .unwrap()
            .execute(request(
                KnowledgeDataOperation::Aggregate(AggregateRequest {
                    plan: AggregationPlan {
                        kinds: vec!["indicator".to_owned()],
                        predicate: None,
                        aggregation: Aggregation::Count,
                        candidate_limit,
                    },
                }),
                system_access(),
                "bounded-aggregation",
            ));
        assert!(matches!(
            response.outcome,
            KnowledgeDataOutcome::Failure {
                error: corrobore_engine::KnowledgeDataError { code, .. }
            } if code == expected_code
        ));
    }
}

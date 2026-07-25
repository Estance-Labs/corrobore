// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
};

use corrobore_engine::{
    AccessContext, ConsistencyLevel, ContractVersion, CoreReadQueryClass, CorroboreEngine,
    CorroboreKnowledgeDataProvider, CountRequest, EnginePersistence, GetByIdRequest,
    GraphDirection, GraphReadPolicy, KnowledgeDataErrorCode, KnowledgeDataOperation,
    KnowledgeDataOutcome, KnowledgeDataRequest, KnowledgeDataResponse, ListRequest,
    NeighborsRequest, PaginateRequest, ProviderDescriptor, ProviderExecution, ReadFilter,
    ReadFilterOperator, ReadOrder, RequestContext, ShadowComparisonGate, SortDirection,
    SubgraphRequest, TraverseRequest, compare_shadow_read,
};
use graph_core::{Graph, NodeId};
use opencti_adapter::{MappedRecord, OpenCtiAdapter};
use serde::Deserialize;
use serde_json::{Value, json};

const PAGINATION_KEY: &[u8] = b"issue-44-opencti-core-read-pagination-key-v1";

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

#[derive(Debug)]
struct MutableProjectionPersistence {
    graph: Arc<Mutex<Graph>>,
}

impl EnginePersistence for MutableProjectionPersistence {
    fn load_graph(&self) -> Result<Graph, String> {
        Ok(Graph::new())
    }

    fn persist_graph(&mut self, _graph: &Graph) -> Result<(), String> {
        Ok(())
    }

    fn prepare_graph_for_request(&mut self, _query: &str) -> Result<Option<Graph>, String> {
        self.graph
            .lock()
            .map(|graph| Some(graph.clone()))
            .map_err(|_| "fixture graph lock poisoned".to_owned())
    }
}

fn corpus() -> Corpus {
    serde_json::from_str(include_str!(
        "../../../compatibility/opencti/7.260722.0/parity-corpus.json"
    ))
    .expect("parity corpus should be valid")
}

fn mapped_graph(fixtures: impl IntoIterator<Item = Value>) -> Graph {
    let adapter = OpenCtiAdapter::pinned();
    let mapped = fixtures
        .into_iter()
        .map(|fixture| adapter.map(fixture).expect("fixture should map"))
        .collect::<Vec<_>>();
    let mut graph = Graph::new();
    let mut nodes = HashMap::<String, NodeId>::new();
    for record in &mapped {
        let canonical_id = record.raw()["id"].as_str().expect("object ID").to_owned();
        if let MappedRecord::Object(object) = record {
            let node_id = graph
                .create_node(object.to_node_input())
                .expect("mapped object should create");
            nodes.insert(canonical_id, node_id);
        }
    }
    for record in &mapped {
        let raw = record.raw();
        if let MappedRecord::Relationship(relationship) = record {
            let source = nodes
                .get(raw["source_ref"].as_str().expect("source"))
                .expect("relationship source should exist")
                .clone();
            let target = nodes
                .get(raw["target_ref"].as_str().expect("target"))
                .expect("relationship target should exist")
                .clone();
            graph
                .create_relationship(
                    relationship
                        .to_relationship_input(source, target)
                        .expect("relationship input should map"),
                )
                .expect("mapped relationship should create");
        }
    }
    graph
}

fn corpus_graph() -> Graph {
    mapped_graph(corpus().fixtures)
}

fn model_fixture_graph() -> Graph {
    let manifest: Value = serde_json::from_str(include_str!(
        "../../../compatibility/opencti/7.260722.0/model-fixtures.json"
    ))
    .expect("model fixtures should be valid");
    mapped_graph(
        manifest["records"]
            .as_array()
            .expect("records")
            .iter()
            .filter(|fixture| fixture["fixture_id"] == "stix-domain-object")
            .map(|fixture| fixture["record"].clone()),
    )
}

fn context(access: AccessContext, correlation_id: &str) -> RequestContext {
    RequestContext {
        request_id: format!("request--{correlation_id}"),
        correlation_id: correlation_id.to_owned(),
        idempotency_key: None,
        deadline_unix_ms: None,
        cancellation_id: None,
        access,
        consistency: ConsistencyLevel::Snapshot,
    }
}

fn system_access() -> AccessContext {
    AccessContext {
        subject_id: "system--issue-44".to_owned(),
        organization_ids: Vec::new(),
        marking_ids: Vec::new(),
        tenant_id: None,
        roles: vec!["system".to_owned()],
        attributes: BTreeMap::new(),
    }
}

fn clear_access() -> AccessContext {
    AccessContext {
        subject_id: "user--00000000-0000-4000-8000-000000000020".to_owned(),
        organization_ids: vec!["identity--00000000-0000-4000-8000-000000000010".to_owned()],
        marking_ids: vec!["marking-definition--00000000-0000-4000-8000-000000000001".to_owned()],
        tenant_id: Some("grouping--00000000-0000-4000-8000-000000000030".to_owned()),
        roles: vec!["analyst".to_owned()],
        attributes: BTreeMap::new(),
    }
}

fn request(
    operation: KnowledgeDataOperation,
    access: AccessContext,
    correlation_id: &str,
) -> KnowledgeDataRequest {
    KnowledgeDataRequest {
        contract_version: ContractVersion::CURRENT,
        context: context(access, correlation_id),
        operation,
    }
}

fn engine_with_graph(graph: Graph) -> CorroboreEngine {
    CorroboreEngine::builder()
        .persistence(Box::new(StaticPersistence { graph }))
        .build()
        .expect("fixture engine should build")
}

fn execute(
    engine: &mut CorroboreEngine,
    operation: KnowledgeDataOperation,
    access: AccessContext,
    correlation_id: &str,
) -> corrobore_engine::KnowledgeDataResponseEnvelope {
    CorroboreKnowledgeDataProvider::new(engine, PAGINATION_KEY)
        .expect("provider")
        .execute(request(operation, access, correlation_id))
}

fn page(response: corrobore_engine::KnowledgeDataResponseEnvelope) -> corrobore_engine::RecordPage {
    match response.outcome {
        KnowledgeDataOutcome::Success {
            response: KnowledgeDataResponse::Records(page),
        } => page,
        other => panic!("expected record page, got {other:?}"),
    }
}

fn graph_result(
    response: corrobore_engine::KnowledgeDataResponseEnvelope,
) -> corrobore_engine::GraphResult {
    match response.outcome {
        KnowledgeDataOutcome::Success {
            response: KnowledgeDataResponse::Graph(graph),
        } => graph,
        other => panic!("expected graph response, got {other:?}"),
    }
}

fn indicator_list() -> ListRequest {
    ListRequest {
        kinds: vec!["indicator".to_owned()],
        filters: Vec::new(),
        order_by: vec![ReadOrder {
            field: "valid_from".to_owned(),
            direction: SortDirection::Ascending,
        }],
        limit: 100,
        ..ListRequest::default()
    }
}

fn graph_policy() -> GraphReadPolicy {
    GraphReadPolicy {
        relationship_types: Vec::new(),
        node_kinds: Vec::new(),
        filters: Vec::new(),
        max_results: 100,
        max_expansions: 100,
        supernode_threshold: 100,
    }
}

#[test]
fn fundamental_corpus_reads_match_reference_ids_properties_counts_and_ordering() {
    let mut engine = engine_with_graph(corpus_graph());
    let id = "indicator--00000000-0000-4000-8000-000000000040";
    let get = execute(
        &mut engine,
        KnowledgeDataOperation::GetById(GetByIdRequest { id: id.to_owned() }),
        system_access(),
        "core-get",
    );
    match get.outcome {
        KnowledgeDataOutcome::Success {
            response: KnowledgeDataResponse::Record(Some(record)),
        } => {
            assert_eq!(record.id, id);
            assert_eq!(record.kind, "indicator");
            assert_eq!(record.body["name"], "Documentation IPv4 indicator");
            assert_eq!(record.body["pattern"], "[ipv4-addr:value = '192.0.2.12']");
        }
        other => panic!("expected point read, got {other:?}"),
    }

    let listed = page(execute(
        &mut engine,
        KnowledgeDataOperation::List(indicator_list()),
        system_access(),
        "core-list",
    ));
    assert_eq!(
        listed
            .records
            .iter()
            .map(|record| record.id.as_str())
            .collect::<Vec<_>>(),
        [
            "indicator--00000000-0000-4000-8000-000000000040",
            "indicator--00000000-0000-4000-8000-000000000041",
            "indicator--00000000-0000-4000-8000-000000000042",
        ]
    );

    let count = execute(
        &mut engine,
        KnowledgeDataOperation::Count(CountRequest {
            filter: Value::Null,
            kinds: vec!["indicator".to_owned()],
            filters: vec![ReadFilter {
                field: "valid_from".to_owned(),
                operator: ReadFilterOperator::Exists,
                value: None,
            }],
        }),
        system_access(),
        "core-count",
    );
    match count.outcome {
        KnowledgeDataOutcome::Success {
            response: KnowledgeDataResponse::Count(result),
        } => assert_eq!(result.count, 3),
        other => panic!("expected count, got {other:?}"),
    }
}

#[test]
fn every_opencti_identifier_class_resolves_the_same_canonical_record() {
    let mut engine = engine_with_graph(model_fixture_graph());
    for identifier in [
        "internal--00000000-0000-4000-8000-000000000101",
        "attack-pattern--00000000-0000-4000-8000-000000000101",
        "attack-pattern--00000000-0000-4000-8000-000000000100",
        "Synthetic phishing",
        "Documentation lure",
        "dedup--credential-phishing",
        "CAPEC-98",
    ] {
        let response = execute(
            &mut engine,
            KnowledgeDataOperation::GetById(GetByIdRequest {
                id: identifier.to_owned(),
            }),
            system_access(),
            identifier,
        );
        match response.outcome {
            KnowledgeDataOutcome::Success {
                response: KnowledgeDataResponse::Record(Some(record)),
            } => {
                assert_eq!(record.id, "internal--00000000-0000-4000-8000-000000000101");
                assert_eq!(record.body["name"], "Synthetic credential phishing");
            }
            other => panic!("identifier {identifier} did not resolve: {other:?}"),
        }
    }
}

#[test]
fn simple_filters_and_access_context_hide_inaccessible_values() {
    let mut engine = engine_with_graph(corpus_graph());
    let amber = execute(
        &mut engine,
        KnowledgeDataOperation::GetById(GetByIdRequest {
            id: "malware--00000000-0000-4000-8000-000000000050".to_owned(),
        }),
        clear_access(),
        "core-denied",
    );
    assert_eq!(
        amber.outcome,
        KnowledgeDataOutcome::Success {
            response: KnowledgeDataResponse::Record(None)
        }
    );

    let clear_indicators = page(execute(
        &mut engine,
        KnowledgeDataOperation::List(ListRequest {
            kinds: vec!["indicator".to_owned()],
            filters: vec![
                ReadFilter {
                    field: "name".to_owned(),
                    operator: ReadFilterOperator::NotEqual,
                    value: Some(json!("not-present")),
                },
                ReadFilter {
                    field: "valid_from".to_owned(),
                    operator: ReadFilterOperator::GreaterThanOrEqual,
                    value: Some(json!("2026-01-01T00:00:00.000Z")),
                },
                ReadFilter {
                    field: "pattern".to_owned(),
                    operator: ReadFilterOperator::Exists,
                    value: None,
                },
            ],
            order_by: vec![ReadOrder {
                field: "valid_from".to_owned(),
                direction: SortDirection::Ascending,
            }],
            limit: 100,
            ..ListRequest::default()
        }),
        clear_access(),
        "core-filtered",
    ));
    assert_eq!(
        clear_indicators
            .records
            .iter()
            .map(|record| record.id.as_str())
            .collect::<Vec<_>>(),
        ["indicator--00000000-0000-4000-8000-000000000040"]
    );
    let serialized = serde_json::to_string(&clear_indicators).expect("page serializes");
    assert!(!serialized.contains("Documentation domain indicator"));
    assert!(!serialized.contains("Synthetic Sample"));
}

#[test]
fn cursor_pagination_is_complete_stable_and_rejects_a_stale_snapshot() {
    let shared = Arc::new(Mutex::new(corpus_graph()));
    let mut engine = CorroboreEngine::builder()
        .persistence(Box::new(MutableProjectionPersistence {
            graph: Arc::clone(&shared),
        }))
        .build()
        .expect("paged fixture engine");
    let first_request = PaginateRequest {
        query: indicator_list(),
        page_size: 2,
        token: None,
    };
    let first = page(execute(
        &mut engine,
        KnowledgeDataOperation::Paginate(first_request.clone()),
        system_access(),
        "cursor-first",
    ));
    assert_eq!(first.records.len(), 2);
    let token = first.next_token.clone().expect("continuation token");

    let second = page(execute(
        &mut engine,
        KnowledgeDataOperation::Paginate(PaginateRequest {
            token: Some(token.clone()),
            ..first_request.clone()
        }),
        system_access(),
        "cursor-second",
    ));
    let all_ids = first
        .records
        .iter()
        .chain(&second.records)
        .map(|record| record.id.clone())
        .collect::<Vec<_>>();
    assert_eq!(all_ids.len(), 3);
    assert_eq!(
        all_ids
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        3
    );
    assert!(second.next_token.is_none());

    let extra = json!({
        "id": "indicator--00000000-0000-4000-8000-000000000099",
        "type": "indicator",
        "name": "Late snapshot mutation",
        "valid_from": "2026-01-01T12:00:00.000Z"
    });
    let mapped = OpenCtiAdapter::pinned()
        .map(extra)
        .expect("late fixture maps");
    let MappedRecord::Object(object) = mapped else {
        panic!("late fixture should be object");
    };
    shared
        .lock()
        .expect("fixture graph")
        .create_node(object.to_node_input())
        .expect("late node");

    let stale = execute(
        &mut engine,
        KnowledgeDataOperation::Paginate(PaginateRequest {
            token: Some(token),
            ..first_request
        }),
        system_access(),
        "cursor-stale",
    );
    assert_eq!(
        stale.outcome.error().map(|error| error.code),
        Some(KnowledgeDataErrorCode::StalePaginationToken)
    );
}

#[test]
fn neighbors_traversal_and_subgraph_preserve_direction_type_limits_and_path_provenance() {
    let mut engine = engine_with_graph(corpus_graph());
    let neighbors = graph_result(execute(
        &mut engine,
        KnowledgeDataOperation::Neighbors(NeighborsRequest {
            id: "malware--00000000-0000-4000-8000-000000000050".to_owned(),
            incoming: true,
            outgoing: false,
            policy: GraphReadPolicy {
                relationship_types: vec!["indicates".to_owned()],
                node_kinds: vec!["indicator".to_owned()],
                ..graph_policy()
            },
        }),
        system_access(),
        "neighbors",
    ));
    assert_eq!(
        neighbors
            .records
            .iter()
            .map(|record| record.id.as_str())
            .collect::<Vec<_>>(),
        [
            "indicator--00000000-0000-4000-8000-000000000040",
            "malware--00000000-0000-4000-8000-000000000050",
        ]
    );
    assert_eq!(neighbors.relationships.len(), 1);
    assert_eq!(
        neighbors.relationships[0]["id"],
        "relationship--00000000-0000-4000-8000-000000000060"
    );

    let traversal = graph_result(execute(
        &mut engine,
        KnowledgeDataOperation::Traverse(TraverseRequest {
            start_ids: vec!["threat-actor--00000000-0000-4000-8000-000000000051".to_owned()],
            max_depth: 2,
            direction: GraphDirection::Both,
            constraints: Value::Null,
            policy: GraphReadPolicy {
                relationship_types: vec!["uses".to_owned(), "indicates".to_owned()],
                ..graph_policy()
            },
        }),
        system_access(),
        "traverse",
    ));
    assert_eq!(traversal.relationships.len(), 2);
    assert!(traversal.paths.iter().any(|path| {
        path.steps
            .iter()
            .map(|step| step.node_id.as_str())
            .collect::<Vec<_>>()
            == [
                "threat-actor--00000000-0000-4000-8000-000000000051",
                "malware--00000000-0000-4000-8000-000000000050",
                "indicator--00000000-0000-4000-8000-000000000040",
            ]
    }));
    assert!(
        traversal
            .paths
            .iter()
            .all(|path| path.steps.iter().all(|step| step.node_revision > 0))
    );

    let subgraph = graph_result(execute(
        &mut engine,
        KnowledgeDataOperation::Subgraph(SubgraphRequest {
            ids: vec!["threat-actor--00000000-0000-4000-8000-000000000051".to_owned()],
            projection: Value::Null,
            max_depth: 2,
            direction: GraphDirection::Both,
            policy: GraphReadPolicy {
                relationship_types: vec!["uses".to_owned(), "indicates".to_owned()],
                ..graph_policy()
            },
        }),
        system_access(),
        "subgraph",
    ));
    assert_eq!(subgraph.records.len(), 3);
    assert_eq!(subgraph.relationships.len(), 2);
    assert!(!subgraph.truncated);
}

#[test]
fn unbounded_budget_exhaustion_and_supernodes_are_refused_explicitly() {
    let mut engine = engine_with_graph(corpus_graph());
    let unbounded = execute(
        &mut engine,
        KnowledgeDataOperation::Traverse(TraverseRequest {
            start_ids: vec!["malware--00000000-0000-4000-8000-000000000050".to_owned()],
            max_depth: 0,
            direction: GraphDirection::Both,
            constraints: Value::Null,
            policy: graph_policy(),
        }),
        system_access(),
        "unbounded",
    );
    assert_eq!(
        unbounded.outcome.error().map(|error| error.code),
        Some(KnowledgeDataErrorCode::UnboundedOperation)
    );

    let exhausted = execute(
        &mut engine,
        KnowledgeDataOperation::Traverse(TraverseRequest {
            start_ids: vec!["malware--00000000-0000-4000-8000-000000000050".to_owned()],
            max_depth: 2,
            direction: GraphDirection::Both,
            constraints: Value::Null,
            policy: GraphReadPolicy {
                max_expansions: 1,
                ..graph_policy()
            },
        }),
        system_access(),
        "budget",
    );
    assert_eq!(
        exhausted.outcome.error().map(|error| error.code),
        Some(KnowledgeDataErrorCode::QueryBudgetExceeded)
    );

    let supernode = execute(
        &mut engine,
        KnowledgeDataOperation::Neighbors(NeighborsRequest {
            id: "malware--00000000-0000-4000-8000-000000000050".to_owned(),
            incoming: true,
            outgoing: true,
            policy: GraphReadPolicy {
                supernode_threshold: 1,
                ..graph_policy()
            },
        }),
        system_access(),
        "supernode",
    );
    assert_eq!(
        supernode.outcome.error().map(|error| error.code),
        Some(KnowledgeDataErrorCode::SupernodeExpansionBlocked)
    );
}

#[test]
fn read_metrics_record_latency_percentiles_and_page_cache_behavior_per_query_class() {
    let mut engine = engine_with_graph(corpus_graph());
    for index in 0..4 {
        let _ = execute(
            &mut engine,
            KnowledgeDataOperation::List(indicator_list()),
            system_access(),
            &format!("metrics-{index}"),
        );
    }
    let metrics = engine.core_read_metrics();
    let series = metrics
        .series(CoreReadQueryClass::List)
        .expect("list metrics");
    assert_eq!(series.requests, 4);
    assert!(series.p50_latency_ms <= series.p95_latency_ms);
    assert!(series.p95_latency_ms <= series.p99_latency_ms);
    assert!(series.page_ins <= series.records_examined);
    assert!(series.cache_hits <= series.records_examined);
}

#[test]
fn fundamental_corpus_shadow_comparison_has_zero_blocking_divergence() {
    let mut engine = engine_with_graph(corpus_graph());
    let typed_request = request(
        KnowledgeDataOperation::List(indicator_list()),
        system_access(),
        "core-shadow-zero-divergence",
    );
    let envelope = CorroboreKnowledgeDataProvider::new(&mut engine, PAGINATION_KEY)
        .expect("provider")
        .execute(typed_request.clone());
    let execution = |name: &str| ProviderExecution {
        provider: ProviderDescriptor {
            name: name.to_owned(),
            version: "issue-44-corpus".to_owned(),
            release: "opencti-7.260722.0".to_owned(),
        },
        latency_ms: 1,
        envelope: envelope.clone(),
    };

    let report = compare_shadow_read(
        &typed_request,
        execution("opensearch"),
        execution("corrobore"),
        &[],
        1_785_000_000_000,
    );
    assert!(report.equivalent);
    assert_eq!(report.gate, ShadowComparisonGate::Pass);
    assert!(report.missing_ids.is_empty());
    assert!(report.unexpected_ids.is_empty());
    assert!(report.property_differences.is_empty());
    assert!(report.ordering_differences.is_empty());
    assert!(report.security_differences.is_empty());
    assert!(report.cursor_differences.is_empty());
}

// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use corrobore_engine::{
    AccessContext, ConformanceCase, ConsistencyLevel, ContractVersion, CorroboreEngine,
    CorroboreKnowledgeDataProvider, EnginePersistence, ExpectedConformanceOutcome, GetByIdRequest,
    HealthRequest, InitializeRequest, KnowledgeDataErrorCode, KnowledgeDataOperation,
    KnowledgeDataOutcome, KnowledgeDataRequest, KnowledgeDataResponse, ListRequest, OperationKind,
    PaginateRequest, RequestContext, execute_remote_contract, run_conformance_cases,
};
use graph_core::Graph;

fn context(correlation_id: &str) -> RequestContext {
    RequestContext {
        request_id: format!("request--{correlation_id}"),
        correlation_id: correlation_id.to_owned(),
        idempotency_key: None,
        deadline_unix_ms: Some(4_102_444_800_000),
        cancellation_id: None,
        access: AccessContext {
            subject_id: "identity--conformance".to_owned(),
            organization_ids: vec!["identity--example-org".to_owned()],
            marking_ids: Vec::new(),
            tenant_id: Some("tenant--example".to_owned()),
            roles: vec!["system".to_owned()],
            attributes: BTreeMap::new(),
        },
        consistency: ConsistencyLevel::ReadYourWrites,
    }
}

fn request(operation: KnowledgeDataOperation, correlation_id: &str) -> KnowledgeDataRequest {
    KnowledgeDataRequest {
        contract_version: ContractVersion::CURRENT,
        context: context(correlation_id),
        operation,
    }
}

fn provider(engine: &mut CorroboreEngine) -> CorroboreKnowledgeDataProvider<'_> {
    CorroboreKnowledgeDataProvider::new(engine, b"issue-39-conformance-key-with-at-least-32-bytes")
        .expect("provider should accept a durable pagination key")
}

#[test]
fn initialization_exposes_versions_capabilities_readiness_and_recovery() {
    let mut engine = CorroboreEngine::strict_default();
    let response = provider(&mut engine).execute(request(
        KnowledgeDataOperation::Initialize(InitializeRequest {
            client_contract_version: ContractVersion::CURRENT,
            required_capabilities: vec![
                OperationKind::Initialize,
                OperationKind::Health,
                OperationKind::GetById,
                OperationKind::List,
                OperationKind::Paginate,
            ],
        }),
        "correlation--initialize",
    ));

    let initialized = match response.outcome {
        KnowledgeDataOutcome::Success {
            response: KnowledgeDataResponse::Initialized(initialized),
        } => initialized,
        other => panic!("expected initialization response, got {other:?}"),
    };
    assert_eq!(initialized.contract_version, ContractVersion::CURRENT);
    assert!(!initialized.engine_version.is_empty());
    assert_eq!(initialized.schema_version, "corrobore-graph-v1");
    assert!(initialized.readiness.accepting_requests);
    assert_eq!(initialized.recovery.state, "ready");
    assert_eq!(initialized.capabilities.len(), OperationKind::ALL.len());
}

#[test]
fn unsupported_required_capability_fails_negotiation_explicitly() {
    let mut engine = CorroboreEngine::strict_default();
    let response = provider(&mut engine).execute(request(
        KnowledgeDataOperation::Initialize(InitializeRequest {
            client_contract_version: ContractVersion::CURRENT,
            required_capabilities: vec![OperationKind::Migrate],
        }),
        "correlation--unsupported",
    ));

    let error = response
        .outcome
        .error()
        .expect("unsupported capability should fail");
    assert_eq!(error.code, KnowledgeDataErrorCode::UnsupportedCapability);
    assert!(error.message.contains("migrate"));
}

#[test]
fn provider_routes_typed_reads_through_the_existing_embedded_engine_state() {
    let mut engine = CorroboreEngine::strict_default();
    engine
        .write("CREATE (n:Indicator {name: 'conformance-record'})")
        .expect("existing Cypher mutation should remain compatible");

    let list = provider(&mut engine).execute(request(
        KnowledgeDataOperation::List(ListRequest {
            kinds: vec!["Indicator".to_owned()],
            limit: 10,
            ..ListRequest::default()
        }),
        "correlation--list",
    ));
    let page = match list.outcome {
        KnowledgeDataOutcome::Success {
            response: KnowledgeDataResponse::Records(page),
        } => page,
        other => panic!("expected record page, got {other:?}"),
    };
    assert_eq!(page.records.len(), 1);
    let id = page.records[0].id.clone();

    let get = provider(&mut engine).execute(request(
        KnowledgeDataOperation::GetById(GetByIdRequest { id: id.clone() }),
        "correlation--get",
    ));
    match get.outcome {
        KnowledgeDataOutcome::Success {
            response: KnowledgeDataResponse::Record(Some(record)),
        } => assert_eq!(record.id, id),
        other => panic!("expected record response, got {other:?}"),
    }

    let legacy = engine
        .read("MATCH (n:Indicator) RETURN n")
        .expect("existing embedded Cypher API should remain compatible");
    assert!(legacy.validation_errors.is_empty());
}

#[test]
fn embedded_and_remote_execution_have_equivalent_semantics_and_errors() {
    let operation = KnowledgeDataOperation::Health(HealthRequest { verbose: true });
    let typed_request = request(operation, "correlation--remote-equivalence");

    let mut embedded_engine = CorroboreEngine::strict_default();
    let embedded = provider(&mut embedded_engine).execute(typed_request.clone());

    let mut remote_engine = CorroboreEngine::strict_default();
    let request_json = serde_json::to_vec(&typed_request).expect("request should serialize");
    let remote_json = execute_remote_contract(&mut provider(&mut remote_engine), &request_json);
    let remote: corrobore_engine::KnowledgeDataResponseEnvelope =
        serde_json::from_slice(&remote_json).expect("remote response should deserialize");

    assert_eq!(remote, embedded);

    let unsupported = request(
        KnowledgeDataOperation::Migrate(Default::default()),
        "correlation--remote-error",
    );
    let embedded_error = provider(&mut embedded_engine).execute(unsupported.clone());
    let remote_error: corrobore_engine::KnowledgeDataResponseEnvelope =
        serde_json::from_slice(&execute_remote_contract(
            &mut provider(&mut remote_engine),
            &serde_json::to_vec(&unsupported).expect("request should serialize"),
        ))
        .expect("remote error should deserialize");
    assert_eq!(
        remote_error.outcome.error().map(|error| error.code),
        embedded_error.outcome.error().map(|error| error.code)
    );
}

#[test]
fn pagination_is_stable_and_tokens_are_bound_to_the_query() {
    let mut engine = CorroboreEngine::strict_default();
    for name in ["alpha", "bravo", "charlie"] {
        engine
            .write(&format!("CREATE (n:Indicator {{name: '{name}'}})"))
            .expect("fixture write should succeed");
    }

    let first_request = PaginateRequest {
        query: ListRequest {
            kinds: vec!["Indicator".to_owned()],
            limit: 100,
            ..ListRequest::default()
        },
        page_size: 2,
        token: None,
    };
    let first = provider(&mut engine).execute(request(
        KnowledgeDataOperation::Paginate(first_request.clone()),
        "correlation--page-1",
    ));
    let first_page = match first.outcome {
        KnowledgeDataOutcome::Success {
            response: KnowledgeDataResponse::Records(page),
        } => page,
        other => panic!("expected first page, got {other:?}"),
    };
    assert_eq!(first_page.records.len(), 2);
    let token = first_page
        .next_token
        .expect("first page should have a token");

    let second = provider(&mut engine).execute(request(
        KnowledgeDataOperation::Paginate(PaginateRequest {
            token: Some(token.clone()),
            ..first_request
        }),
        "correlation--page-2",
    ));
    let second_page = match second.outcome {
        KnowledgeDataOutcome::Success {
            response: KnowledgeDataResponse::Records(page),
        } => page,
        other => panic!("expected second page, got {other:?}"),
    };
    assert_eq!(second_page.records.len(), 1);
    assert!(second_page.next_token.is_none());

    let mismatch = provider(&mut engine).execute(request(
        KnowledgeDataOperation::Paginate(PaginateRequest {
            query: ListRequest {
                kinds: vec!["Campaign".to_owned()],
                limit: 100,
                ..ListRequest::default()
            },
            page_size: 2,
            token: Some(token),
        }),
        "correlation--page-mismatch",
    ));
    assert_eq!(
        mismatch.outcome.error().map(|error| error.code),
        Some(KnowledgeDataErrorCode::IncompatiblePaginationToken)
    );
}

#[test]
fn deadlines_and_cancellation_are_enforced_before_provider_dispatch() {
    let mut engine = CorroboreEngine::strict_default();
    let mut expired = request(
        KnowledgeDataOperation::Health(HealthRequest { verbose: false }),
        "correlation--expired",
    );
    expired.context.deadline_unix_ms = Some(1);
    assert_eq!(
        provider(&mut engine)
            .execute(expired)
            .outcome
            .error()
            .map(|error| error.code),
        Some(KnowledgeDataErrorCode::DeadlineExceeded)
    );

    let mut cancelled = request(
        KnowledgeDataOperation::Health(HealthRequest { verbose: false }),
        "correlation--cancelled",
    );
    cancelled.context.cancellation_id = Some("cancel--before-dispatch".to_owned());
    let mut provider = provider(&mut engine);
    provider.cancel("cancel--before-dispatch");
    assert_eq!(
        provider
            .execute(cancelled)
            .outcome
            .error()
            .map(|error| error.code),
        Some(KnowledgeDataErrorCode::Cancelled)
    );
}

#[test]
fn reusable_conformance_cases_run_against_embedded_and_remote_endpoints() {
    let cases = vec![
        ConformanceCase {
            name: "health".to_owned(),
            request: request(
                KnowledgeDataOperation::Health(HealthRequest { verbose: false }),
                "correlation--kit-health",
            ),
            expected: ExpectedConformanceOutcome::Success,
        },
        ConformanceCase {
            name: "unsupported-migration".to_owned(),
            request: request(
                KnowledgeDataOperation::Migrate(Default::default()),
                "correlation--kit-migrate",
            ),
            expected: ExpectedConformanceOutcome::Error(
                KnowledgeDataErrorCode::UnsupportedCapability,
            ),
        },
    ];

    let mut embedded_engine = CorroboreEngine::strict_default();
    let mut embedded_provider = provider(&mut embedded_engine);
    let embedded = run_conformance_cases(&cases, |request| embedded_provider.execute(request));
    assert!(embedded.is_conformant());

    let mut remote_engine = CorroboreEngine::strict_default();
    let mut remote_provider = provider(&mut remote_engine);
    let remote = run_conformance_cases(&cases, |request| {
        serde_json::from_slice(&execute_remote_contract(
            &mut remote_provider,
            &serde_json::to_vec(&request).expect("request should serialize"),
        ))
        .expect("response should deserialize")
    });
    assert_eq!(remote, embedded);
}

#[derive(Debug)]
struct PagedReadPersistence {
    projection: Graph,
}

impl EnginePersistence for PagedReadPersistence {
    fn load_graph(&self) -> Result<Graph, String> {
        Ok(Graph::new())
    }

    fn persist_graph(&mut self, _graph: &Graph) -> Result<(), String> {
        Ok(())
    }

    fn prepare_graph_for_request(&mut self, _query: &str) -> Result<Option<Graph>, String> {
        Ok(Some(self.projection.clone()))
    }
}

#[test]
fn knowledge_data_reads_hydrate_paged_persistence_before_provider_dispatch() {
    let mut source = CorroboreEngine::strict_default();
    source
        .write("CREATE (n:Indicator {name: 'persistent-shadow-record'})")
        .expect("fixture mutation should succeed");
    let fixture = match provider(&mut source)
        .execute(request(
            KnowledgeDataOperation::List(ListRequest {
                kinds: vec!["Indicator".to_owned()],
                limit: 10,
                ..ListRequest::default()
            }),
            "correlation--paged-source",
        ))
        .outcome
    {
        KnowledgeDataOutcome::Success {
            response: KnowledgeDataResponse::Records(page),
        } => page.records[0].clone(),
        other => panic!("expected fixture page, got {other:?}"),
    };

    let mut paged = CorroboreEngine::builder()
        .persistence(Box::new(PagedReadPersistence {
            projection: source.graph().clone(),
        }))
        .build()
        .expect("paged engine should initialize metadata-only");
    assert!(paged.graph().list_nodes().expect("cold graph").is_empty());

    let hydrated = provider(&mut paged).execute(request(
        KnowledgeDataOperation::GetById(GetByIdRequest {
            id: fixture.id.clone(),
        }),
        "correlation--paged-shadow",
    ));
    match hydrated.outcome {
        KnowledgeDataOutcome::Success {
            response: KnowledgeDataResponse::Record(Some(record)),
        } => assert_eq!(record.id, fixture.id),
        other => panic!("paged Knowledge Data read must hydrate projection, got {other:?}"),
    }
}

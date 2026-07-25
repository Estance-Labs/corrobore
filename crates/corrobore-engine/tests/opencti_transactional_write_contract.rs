// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
#![allow(clippy::unwrap_used)]

use std::sync::{Arc, Mutex};

use corrobore_engine::{
    AccessContext, ConsistencyLevel, ContractVersion, CorroboreEngine,
    CorroboreKnowledgeDataProvider, CreateRequest, EnginePersistence, KnowledgeDataEngine,
    KnowledgeDataError, KnowledgeDataErrorCode, KnowledgeDataOperation, KnowledgeDataOutcome,
    KnowledgeDataRequest, KnowledgeDataResponse, OperationKind, ProviderCapabilityStatus,
    RequestContext, WriteResult,
};
use graph_core::Graph;
use serde_json::json;

#[derive(Debug)]
struct WritePersistence {
    calls: Arc<Mutex<Vec<String>>>,
}

impl EnginePersistence for WritePersistence {
    fn load_graph(&self) -> Result<Graph, String> {
        Ok(Graph::new())
    }

    fn persist_graph(&mut self, _graph: &Graph) -> Result<(), String> {
        Err("snapshot persistence is not used by typed writes".to_owned())
    }

    fn execute_knowledge_data_mutation(
        &mut self,
        operation: &KnowledgeDataOperation,
        context: &RequestContext,
    ) -> Result<Option<KnowledgeDataResponse>, KnowledgeDataError> {
        self.calls.lock().unwrap().push(format!(
            "{}:{}",
            operation.kind(),
            context.idempotency_key.as_deref().unwrap_or_default()
        ));
        Ok(Some(KnowledgeDataResponse::Write(WriteResult {
            id: "indicator--1".to_owned(),
            revision: 1,
        })))
    }
}

fn request(idempotency_key: Option<&str>) -> KnowledgeDataRequest {
    KnowledgeDataRequest {
        contract_version: ContractVersion::CURRENT,
        context: RequestContext {
            request_id: "request--write".to_owned(),
            correlation_id: "correlation--write".to_owned(),
            idempotency_key: idempotency_key.map(str::to_owned),
            deadline_unix_ms: Some(4_102_444_800_000),
            cancellation_id: None,
            access: AccessContext {
                subject_id: "identity--writer".to_owned(),
                roles: vec!["system".to_owned()],
                ..AccessContext::default()
            },
            consistency: ConsistencyLevel::ReadYourWrites,
        },
        operation: KnowledgeDataOperation::Create(CreateRequest {
            record: json!({"id": "indicator--1", "type": "indicator", "name": "one"}),
        }),
    }
}

#[test]
fn provider_declares_and_dispatches_transactional_mutations_through_persistence() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut engine = CorroboreEngine::builder()
        .persistence(Box::new(WritePersistence {
            calls: Arc::clone(&calls),
        }))
        .build()
        .unwrap();
    let mut provider = CorroboreKnowledgeDataProvider::new(
        &mut engine,
        b"issue-50-transactional-write-contract-key",
    )
    .unwrap();

    for kind in [
        OperationKind::Create,
        OperationKind::Update,
        OperationKind::Delete,
        OperationKind::Bulk,
    ] {
        assert!(matches!(
            provider
                .capabilities()
                .into_iter()
                .find(|capability| capability.operation == kind)
                .unwrap()
                .status,
            ProviderCapabilityStatus::Supported
        ));
    }

    let response = provider.execute(request(Some("idempotency--1")));
    assert!(matches!(
        response.outcome,
        KnowledgeDataOutcome::Success {
            response: KnowledgeDataResponse::Write(WriteResult { revision: 1, .. })
        }
    ));
    assert_eq!(calls.lock().unwrap().as_slice(), ["create:idempotency--1"]);
}

#[test]
fn mutation_without_idempotency_key_fails_before_persistence_dispatch() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut engine = CorroboreEngine::builder()
        .persistence(Box::new(WritePersistence {
            calls: Arc::clone(&calls),
        }))
        .build()
        .unwrap();
    let response = CorroboreKnowledgeDataProvider::new(
        &mut engine,
        b"issue-50-transactional-write-contract-key",
    )
    .unwrap()
    .execute(request(None));

    assert_eq!(
        response.outcome.error().map(|error| error.code),
        Some(KnowledgeDataErrorCode::InvalidRequest)
    );
    assert!(calls.lock().unwrap().is_empty());
}

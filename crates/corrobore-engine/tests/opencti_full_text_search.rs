// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;

use corrobore_engine::{
    AccessContext, ConsistencyLevel, ContractVersion, CorroboreEngine,
    CorroboreKnowledgeDataProvider, EnginePersistence, KnowledgeDataEngine, KnowledgeDataErrorCode,
    KnowledgeDataOperation, KnowledgeDataOutcome, KnowledgeDataRequest, KnowledgeDataResponse,
    OperationKind, ProviderCapabilityStatus, RequestContext, SearchRequest,
};
use graph_core::{Graph, NodeInput, PropertyValue};
use serde_json::json;

const CURSOR_KEY: &[u8] = b"issue-46-provider-full-text-key-32-bytes";

#[derive(Debug)]
struct StaticPersistence(Graph);

impl EnginePersistence for StaticPersistence {
    fn load_graph(&self) -> Result<Graph, String> {
        Ok(self.0.clone())
    }

    fn persist_graph(&mut self, _graph: &Graph) -> Result<(), String> {
        Ok(())
    }
}

fn node(id: &str, name: &str, marking: &str) -> NodeInput {
    NodeInput::new(vec![
        "OpenCtiObject".to_owned(),
        "OpenCtiType_indicator".to_owned(),
    ])
    .with_property("opencti.canonical_id", PropertyValue::String(id.to_owned()))
    .with_property(
        "opencti.entity_type",
        PropertyValue::String("indicator".to_owned()),
    )
    .with_property("opencti.field.name", PropertyValue::String(name.to_owned()))
    .with_property(
        "opencti.access",
        PropertyValue::Json(json!({"marking_ids": [marking]})),
    )
}

fn graph() -> Graph {
    let mut graph = Graph::new();
    graph
        .create_node(node(
            "indicator--clear",
            "Documentation network indicator",
            "marking--clear",
        ))
        .unwrap();
    graph
        .create_node(node(
            "indicator--amber",
            "Documentation domain indicator",
            "marking--amber",
        ))
        .unwrap();
    graph
}

fn access(markings: &[&str], version: &str) -> AccessContext {
    AccessContext {
        subject_id: "user--search".to_owned(),
        marking_ids: markings.iter().map(|value| (*value).to_owned()).collect(),
        attributes: BTreeMap::from([("policy_version".to_owned(), version.to_owned())]),
        ..AccessContext::default()
    }
}

fn execute(
    engine: &mut CorroboreEngine,
    expression: serde_json::Value,
    access: AccessContext,
) -> corrobore_engine::KnowledgeDataResponseEnvelope {
    CorroboreKnowledgeDataProvider::new(engine, CURSOR_KEY)
        .unwrap()
        .execute(KnowledgeDataRequest {
            contract_version: ContractVersion::CURRENT,
            context: RequestContext {
                request_id: "request--full-text".to_owned(),
                correlation_id: "correlation--full-text".to_owned(),
                access,
                consistency: ConsistencyLevel::Snapshot,
                ..RequestContext::default()
            },
            operation: KnowledgeDataOperation::Search(SearchRequest {
                expression,
                limit: 20,
            }),
        })
}

#[test]
fn provider_negotiates_full_text_without_claiming_advanced_search() {
    let mut engine = CorroboreEngine::builder()
        .persistence(Box::new(StaticPersistence(graph())))
        .build()
        .unwrap();
    let provider = CorroboreKnowledgeDataProvider::new(&mut engine, CURSOR_KEY).unwrap();
    let search = provider
        .capabilities()
        .into_iter()
        .find(|capability| capability.operation == OperationKind::Search)
        .unwrap();
    assert_eq!(search.status, ProviderCapabilityStatus::Supported);
}

#[test]
fn provider_returns_only_authorized_ranked_hits_and_counts() {
    let mut engine = CorroboreEngine::builder()
        .persistence(Box::new(StaticPersistence(graph())))
        .build()
        .unwrap();
    let response = execute(
        &mut engine,
        json!({
            "text": "documentation indicator",
            "mode": "term",
            "fields": ["name"],
            "types": ["indicator"]
        }),
        access(&["marking--clear"], "policy--v1"),
    );

    match response.outcome {
        KnowledgeDataOutcome::Success {
            response: KnowledgeDataResponse::Search(page),
        } => {
            assert_eq!(page.total, 1);
            assert_eq!(page.hits[0].id, "indicator--clear");
            assert_eq!(page.authorization_denials, 1);
        }
        other => panic!("expected full-text page, got {other:?}"),
    }
}

#[test]
fn provider_rejects_backend_dsl_and_arbitrary_analyzers() {
    let mut engine = CorroboreEngine::builder()
        .persistence(Box::new(StaticPersistence(graph())))
        .build()
        .unwrap();
    for expression in [
        json!({"query": {"match_all": {}}}),
        json!({"text": "documentation", "analyzer": "english"}),
    ] {
        let response = execute(
            &mut engine,
            expression,
            access(&["marking--clear"], "policy--v1"),
        );
        assert!(matches!(
            response.outcome,
            KnowledgeDataOutcome::Failure {
                error: corrobore_engine::KnowledgeDataError {
                    code: KnowledgeDataErrorCode::InvalidRequest,
                    ..
                }
            }
        ));
    }
}

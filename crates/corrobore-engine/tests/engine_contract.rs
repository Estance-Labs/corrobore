// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.
use std::collections::HashMap;

use corrobore_engine::{
    CorroboreEngine, CypherResponseData, CypherResponseStatus, EngineError, EnginePersistence,
    EngineRequest, EngineRequestMode, ExportMode, StixExportOptions,
};
use graph_core::Graph;

#[derive(Debug)]
struct FailingPersistence;

impl EnginePersistence for FailingPersistence {
    fn load_graph(&self) -> Result<Graph, String> {
        Ok(Graph::new())
    }

    fn persist_graph(&mut self, _graph: &Graph) -> Result<(), String> {
        Err("injected durable commit failure".to_owned())
    }
}

#[test]
fn engine_contract_strict_default_executes_read_query() {
    let mut engine = CorroboreEngine::strict_default();

    let response = engine
        .read("MATCH (n) RETURN n LIMIT 1")
        .expect("read query should execute on a default engine");

    assert_eq!(response.status, CypherResponseStatus::Success);
    assert!(matches!(response.data, CypherResponseData::Records(_)));
}

#[test]
fn engine_contract_auto_mode_routes_leading_create_to_mutation() {
    let mut engine = CorroboreEngine::strict_default();

    let response = engine
        .execute("CREATE (n:Indicator {name: 'auto-created'})")
        .expect("auto-mode mutation should execute");

    assert_eq!(response.status, CypherResponseStatus::Success);
    match response.data {
        CypherResponseData::MutationSummary(summary) => {
            assert_eq!(summary.created_nodes, 1);
            assert_eq!(summary.matched_rows, 0);
        }
        other => panic!("expected typed mutation summary, got {other:?}"),
    }

    let nodes = engine
        .graph()
        .list_nodes()
        .expect("graph listing should succeed");
    assert_eq!(
        nodes.len(),
        1,
        "auto-mode must route the CREATE to mutation"
    );
}

#[test]
fn engine_contract_auto_mode_routes_plain_match_to_read() {
    let mut engine = CorroboreEngine::strict_default();

    let response = engine
        .execute("MATCH (n) RETURN n LIMIT 1")
        .expect("auto-mode read should execute");

    assert_eq!(response.status, CypherResponseStatus::Success);
    assert!(matches!(response.data, CypherResponseData::Records(_)));
}

#[test]
fn engine_contract_write_then_read_back_returns_records() {
    let mut engine = CorroboreEngine::strict_default();

    engine
        .write("CREATE (n:Indicator {name: 'written'})")
        .expect("write should execute");

    let response = engine
        .read("MATCH (n:Indicator) RETURN n")
        .expect("read-back should execute");

    assert_eq!(response.status, CypherResponseStatus::Success);
    match response.data {
        CypherResponseData::Records(records) => {
            assert!(
                !records.is_empty(),
                "read-back should return the written node"
            );
        }
        other => panic!("expected records, got {other:?}"),
    }
}

#[test]
fn engine_contract_rolls_back_memory_when_durable_commit_fails() {
    let mut engine = CorroboreEngine::builder()
        .persistence(Box::new(FailingPersistence))
        .build()
        .expect("persistent engine should initialize");

    let error = engine
        .write("CREATE (n:Indicator {name: 'must-rollback'})")
        .expect_err("failed durable commit must reject the mutation");
    assert!(matches!(
        error,
        EngineError::Persistence(reason) if reason.contains("injected durable commit failure")
    ));
    assert!(
        engine
            .graph()
            .list_nodes()
            .expect("rolled-back graph should be readable")
            .is_empty(),
        "in-memory graph must roll back when persistence fails"
    );
}

#[test]
fn engine_contract_read_mode_rejects_leading_mutation_clause() {
    let mut engine = CorroboreEngine::strict_default();

    let response = engine
        .read("CREATE (n:Indicator {name: 'blocked'})")
        .expect("read-mode mutation should produce a structured rejection");

    assert_eq!(response.status, CypherResponseStatus::Rejected);
    assert_eq!(response.validation_errors.len(), 1);
    assert_eq!(
        response.validation_errors[0].code,
        "WRITE_PERMISSION_REQUIRED"
    );

    let nodes = engine
        .graph()
        .list_nodes()
        .expect("graph listing should succeed");
    assert!(
        nodes.is_empty(),
        "rejected mutation must not change the graph"
    );
}

#[test]
fn engine_contract_read_only_engine_rejects_write_mode() {
    let mut engine = CorroboreEngine::builder()
        .read_only(true)
        .build()
        .expect("read-only engine should build");

    let response = engine
        .write("CREATE (n:Indicator {name: 'blocked'})")
        .expect("write on read-only engine should produce a structured rejection");

    assert_eq!(response.status, CypherResponseStatus::Rejected);
    assert_eq!(response.validation_errors.len(), 1);
    assert_eq!(
        response.validation_errors[0].code,
        "WRITE_PERMISSION_REQUIRED"
    );

    let nodes = engine
        .graph()
        .list_nodes()
        .expect("graph listing should succeed");
    assert!(
        nodes.is_empty(),
        "read-only engine must not mutate the graph"
    );
}

#[test]
fn engine_contract_builder_rejects_blank_workspace_id() {
    let error = CorroboreEngine::builder()
        .workspace_id("   ")
        .build()
        .expect_err("blank workspace id should be rejected at build time");

    assert!(matches!(
        error,
        EngineError::InvalidConfiguration {
            field: "workspace_id",
            ..
        }
    ));
}

#[test]
fn engine_contract_builder_applies_custom_identifiers() {
    let engine = CorroboreEngine::builder()
        .workspace_id("workspace--embedded-tests")
        .session_id("session--embedded-tests")
        .build()
        .expect("custom identifiers should be accepted");

    assert_eq!(engine.workspace_id(), "workspace--embedded-tests");
    assert_eq!(engine.session_id(), "session--embedded-tests");
}

#[test]
fn engine_contract_accepts_string_parameters() {
    let mut engine = CorroboreEngine::strict_default();

    let mut params = HashMap::new();
    params.insert("name".to_owned(), "param-value".to_owned());

    let response = engine
        .read_with_params("MATCH (n) RETURN n LIMIT 1", params)
        .expect("parameterized read should execute");

    assert_eq!(response.status, CypherResponseStatus::Success);
}

#[test]
fn engine_contract_contextual_request_uses_the_public_execution_boundary() {
    let mut engine = CorroboreEngine::strict_default();
    let request = EngineRequest::new(
        "CREATE (n:Indicator {name: 'contextual-boundary'})",
        EngineRequestMode::Auto,
    )
    .with_workspace_id("workspace--contextual-boundary")
    .with_session_id("session--contextual-boundary")
    .with_budget_ref("budget--contextual-boundary");

    let response = engine
        .execute_request(request)
        .expect("contextual request should execute through the public engine");

    assert_eq!(response.status, CypherResponseStatus::Success);
    let nodes = engine
        .graph()
        .list_nodes()
        .expect("contextual mutation should update the engine graph");
    assert_eq!(nodes.len(), 1);
}

#[test]
fn engine_contract_rejects_query_over_policy_limit() {
    let mut engine = CorroboreEngine::strict_default();
    let query = format!("MATCH (n) RETURN '{}'", "x".repeat(8_300));

    let response = engine
        .read(&query)
        .expect("over-limit query should produce a structured rejection");

    assert_eq!(response.status, CypherResponseStatus::Rejected);
    assert_eq!(response.validation_errors.len(), 1);
    assert_eq!(response.validation_errors[0].code, "REQUEST_LIMIT_EXCEEDED");
}

#[test]
fn engine_contract_graph_accessor_reflects_mutations() {
    let mut engine = CorroboreEngine::strict_default();

    engine
        .write("CREATE (n:Indicator {name: 'graph-visible'})")
        .expect("write should execute");

    let nodes = engine
        .graph()
        .list_nodes()
        .expect("graph listing should succeed");
    assert_eq!(nodes.len(), 1);
}

#[test]
fn engine_contract_exports_stix_bundle_from_embedded_graph() {
    let mut engine = CorroboreEngine::strict_default();

    assert!(
        !StixExportOptions::default().force,
        "forced validation override must remain opt-in"
    );

    engine
        .write("CREATE (n:Indicator {name: 'export-me'})")
        .expect("write should execute");

    let options = StixExportOptions {
        mode: ExportMode::Permissive,
        ..StixExportOptions::default()
    };
    let bundle = engine
        .export_stix_bundle(&options)
        .expect("permissive export should succeed on an embedded engine");

    let json = serde_json::to_value(&bundle).expect("bundle should serialize");
    assert_eq!(json["type"], "bundle");
    assert_eq!(json["export_metadata"]["mode"], "permissive");
    assert!(
        json["export_metadata"]["determinism_key"]
            .as_str()
            .is_some_and(|key| !key.is_empty()),
        "export metadata should carry a determinism key"
    );
}

#[test]
fn engine_contract_seed_search_returns_ranked_candidates() {
    let mut engine = CorroboreEngine::strict_default();

    engine
        .write("CREATE (n:Campaign {name: 'acme phishing campaign'})")
        .expect("write should execute");

    let response = engine
        .seed_search("phishing campaign", 5)
        .expect("seed search should resolve");

    let candidates = response.seed_candidates();
    assert_eq!(candidates.len(), 1);
    assert!(candidates[0].score() > 0.0);
    assert!(
        candidates[0]
            .explanation()
            .rationale()
            .contains("matched terms")
    );
}

#[test]
fn engine_contract_seed_search_surfaces_typed_resolution_errors() {
    let engine = CorroboreEngine::strict_default();

    let error = engine
        .seed_search("phishing campaign", 5)
        .expect_err("an empty graph must yield a typed NO_SEED error");

    match error {
        EngineError::Graph(graph_core::GraphError::SemanticSeedResolutionFailed(details)) => {
            assert_eq!(
                details.code,
                graph_core::SemanticSeedResolutionErrorCode::NoSeed
            );
        }
        other => panic!("expected typed seed resolution failure, got {other:?}"),
    }
}

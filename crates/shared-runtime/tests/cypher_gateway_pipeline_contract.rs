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
use graph_core::{SessionId, WorkspaceId};
use shared_runtime::{
    CypherBudgetRef, CypherGateway, CypherParameters, CypherRequest, CypherResponseData,
    CypherResponseStatus,
};

fn workspace_id() -> WorkspaceId {
    WorkspaceId::new("workspace--gateway-contract").expect("workspace id should be valid")
}

fn session_id() -> SessionId {
    SessionId::new("session--gateway-contract").expect("session id should be valid")
}

fn budget_ref() -> CypherBudgetRef {
    CypherBudgetRef::new("budget--gateway-contract").expect("budget ref should be valid")
}

#[test]
fn gateway_contract_executes_read_request_through_parser_planner_executor() {
    let mut gateway = CypherGateway::strict_default();
    let request = CypherRequest::build_read_only_request(
        "MATCH (n) RETURN n LIMIT 1",
        CypherParameters::default(),
        workspace_id(),
        session_id(),
        budget_ref(),
    )
    .expect("request should be valid");

    let response = gateway
        .execute(&request)
        .expect("read request should execute through gateway pipeline");

    assert_eq!(response.status, CypherResponseStatus::Success);
    assert!(matches!(response.data, CypherResponseData::Records(_)));
}

#[test]
fn gateway_contract_rejects_unsupported_query_with_actionable_error() {
    let mut gateway = CypherGateway::strict_default();
    let request = CypherRequest::build_read_only_request(
        "LOAD CSV FROM 'file:///tmp/input.csv' AS row RETURN row",
        CypherParameters::default(),
        workspace_id(),
        session_id(),
        budget_ref(),
    )
    .expect("request should be valid");

    let response = gateway
        .execute(&request)
        .expect("unsupported query should produce a structured rejection response");

    assert_eq!(response.status, CypherResponseStatus::Rejected);
    assert_eq!(response.validation_errors.len(), 1);
    assert_eq!(
        response.validation_errors[0].code,
        "UNSUPPORTED_CYPHER_FEATURE"
    );
    assert!(!response.fix_hints.is_empty());
}

#[test]
fn gateway_contract_rejects_read_only_request_with_leading_mutation_clause() {
    let mut gateway = CypherGateway::strict_default();
    let request = CypherRequest::build_read_only_request(
        "CREATE (n:Indicator {name: 'leading-clause'})",
        CypherParameters::default(),
        workspace_id(),
        session_id(),
        budget_ref(),
    )
    .expect("request should be valid");

    let response = gateway
        .execute(&request)
        .expect("read-only mutation should produce a structured rejection response");

    assert_eq!(response.status, CypherResponseStatus::Rejected);
    assert_eq!(response.validation_errors.len(), 1);
    assert_eq!(
        response.validation_errors[0].code,
        "WRITE_PERMISSION_REQUIRED"
    );
}

#[test]
fn gateway_contract_accepts_mutation_mode_without_explicit_write_clause() {
    let mut gateway = CypherGateway::strict_default();
    let request = CypherRequest::build_mutation_request(
        "MATCH (n) RETURN n",
        CypherParameters::default(),
        workspace_id(),
        session_id(),
        budget_ref(),
    )
    .expect("request should be valid");

    let response = gateway
        .execute(&request)
        .expect("mutation-mode request without write clause should execute");

    assert_eq!(response.status, CypherResponseStatus::Success);
    assert!(matches!(response.data, CypherResponseData::Records(_)));
}

#[test]
fn gateway_contract_validate_only_mutation_returns_success_without_touching_graph() {
    let mut gateway = CypherGateway::strict_default();
    let request = CypherRequest::build_validate_only_request(
        "CREATE (n:Indicator {name: 'should-not-exist'})",
        CypherParameters::default(),
        workspace_id(),
        session_id(),
        budget_ref(),
    )
    .expect("validate-only mutation should be structurally valid");

    let response = gateway
        .execute(&request)
        .expect("valid query should return a validation response");

    assert_eq!(response.status, CypherResponseStatus::Success);
    assert_eq!(response.data, CypherResponseData::Empty);
    assert!(response.validation_errors.is_empty());
    assert!(
        gateway
            .graph()
            .list_nodes()
            .expect("graph should remain readable")
            .is_empty()
    );
}

#[test]
fn gateway_contract_validate_only_unsupported_query_returns_validation_response() {
    let mut gateway = CypherGateway::strict_default();
    let request = CypherRequest::build_validate_only_request(
        "LOAD CSV FROM 'file:///tmp/input.csv' AS row RETURN row",
        CypherParameters::default(),
        workspace_id(),
        session_id(),
        budget_ref(),
    )
    .expect("unsupported query should reach validation");

    let response = gateway
        .execute(&request)
        .expect("unsupported query should produce a validation response");

    assert_eq!(response.status, CypherResponseStatus::Rejected);
    assert_eq!(response.data, CypherResponseData::Empty);
    assert_eq!(response.validation_errors.len(), 1);
    assert!(
        gateway
            .graph()
            .list_nodes()
            .expect("graph should remain readable")
            .is_empty()
    );
}

#[test]
fn gateway_contract_returns_error_for_unsupported_explain_execution_mode() {
    let mut gateway = CypherGateway::strict_default();
    let request = CypherRequest::new(
        "EXPLAIN MATCH (n) RETURN n",
        CypherParameters::default(),
        shared_runtime::CypherRequestMode::Explain,
        workspace_id(),
        session_id(),
        budget_ref(),
    )
    .expect("request should be structurally valid before execution-mode validation");

    let error = gateway
        .execute(&request)
        .expect_err("explain mode should be rejected before gateway execution");

    assert!(matches!(
        error,
        shared_runtime::RuntimeError::UnsupportedCypherRequestMode(
            shared_runtime::CypherRequestMode::Explain
        )
    ));
}

#[test]
fn gateway_contract_rejects_request_over_policy_query_length_limit() {
    let mut gateway = CypherGateway::strict_default();
    let query = format!("MATCH (n) RETURN '{}'", "x".repeat(8_300));
    let request = CypherRequest::build_read_only_request(
        query,
        CypherParameters::default(),
        workspace_id(),
        session_id(),
        budget_ref(),
    )
    .expect("request should still be constructible");

    let response = gateway
        .execute(&request)
        .expect("over-limit query should produce a structured rejection response");

    assert_eq!(response.status, CypherResponseStatus::Rejected);
    assert_eq!(response.validation_errors.len(), 1);
    assert_eq!(response.validation_errors[0].code, "REQUEST_LIMIT_EXCEEDED");
    assert!(!response.fix_hints.is_empty());
}

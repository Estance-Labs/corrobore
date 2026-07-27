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

use graph_core::{SessionId, WorkspaceId};
use shared_runtime::{
    CypherAuditReference, CypherBudgetRef, CypherFixHint, CypherMutationSummary, CypherParameters,
    CypherRequest, CypherRequestMode, CypherResponse, CypherResponseData, CypherResponseStatus,
    CypherValidationError, CypherValue, RuntimeError,
};

fn workspace_id() -> WorkspaceId {
    WorkspaceId::new("workspace--cypher").expect("workspace id should be valid")
}

fn session_id() -> SessionId {
    SessionId::new("session--cypher").expect("session id should be valid")
}

fn budget_ref() -> CypherBudgetRef {
    CypherBudgetRef::new("budget-profile--default").expect("budget ref should be valid")
}

#[test]
fn build_read_only_request_sets_expected_mode_and_context() {
    // `LIMIT` takes a row count, so the binding carries an integer rather than
    // text: the type survives all the way to the executor.
    let mut params = HashMap::new();
    params.insert("limit".to_owned(), CypherValue::Integer(25));

    let request = CypherRequest::build_read_only_request(
        "MATCH (n) RETURN n LIMIT $limit",
        CypherParameters::typed(params.clone()),
        workspace_id(),
        session_id(),
        budget_ref(),
    )
    .expect("read-only request should be valid");

    assert_eq!(request.mode, CypherRequestMode::ReadOnly);
    assert_eq!(request.parameters.values(), &params);
}

#[test]
fn build_mutation_request_sets_expected_mode() {
    let request = CypherRequest::build_mutation_request(
        "CREATE (n:Indicator)",
        CypherParameters::default(),
        workspace_id(),
        session_id(),
        budget_ref(),
    )
    .expect("mutation request should be valid");

    assert_eq!(request.mode, CypherRequestMode::Mutation);
}

#[test]
fn build_validate_only_request_sets_expected_mode() {
    let request = CypherRequest::build_validate_only_request(
        "MATCH (n RETURN n",
        CypherParameters::default(),
        workspace_id(),
        session_id(),
        budget_ref(),
    )
    .expect("validate-only request should be valid");

    assert_eq!(request.mode, CypherRequestMode::ValidateOnly);
}

#[test]
fn request_validation_rejects_empty_query_text() {
    let error = CypherRequest::build_read_only_request(
        " ",
        CypherParameters::default(),
        workspace_id(),
        session_id(),
        budget_ref(),
    )
    .expect_err("empty query text should be rejected");

    assert!(matches!(error, RuntimeError::MalformedCypherRequest(field) if field == "query_text"));
}

#[test]
fn request_validation_rejects_unsupported_explain_mode_for_execution() {
    let request = CypherRequest::new(
        "EXPLAIN MATCH (n) RETURN n",
        CypherParameters::default(),
        CypherRequestMode::Explain,
        workspace_id(),
        session_id(),
        budget_ref(),
    )
    .expect("request shape should be valid even when execution mode is unsupported");

    let error = request
        .validate_for_gateway_execution()
        .expect_err("unsupported mode should be explicit");

    assert!(matches!(
        error,
        RuntimeError::UnsupportedCypherRequestMode(CypherRequestMode::Explain)
    ));
}

#[test]
fn response_model_carries_mutation_summary_warnings_and_fix_hints() {
    let response = CypherResponse {
        status: CypherResponseStatus::ValidationFailed,
        data: CypherResponseData::MutationSummary(CypherMutationSummary {
            created_nodes: 0,
            updated_nodes: 0,
            deleted_nodes: 0,
            created_relationships: 0,
            deleted_relationships: 0,
            properties_set: 0,
        }),
        warnings: vec!["planner_fallback".to_owned()],
        validation_errors: vec![CypherValidationError {
            code: "SYNTAX_ERROR".to_owned(),
            message: "Unexpected token RETURN".to_owned(),
            field: Some("query_text".to_owned()),
        }],
        budget_usage: None,
        audit_references: vec![CypherAuditReference {
            transaction_id: None,
            request_id: Some("request--cypher-001".to_owned()),
        }],
        fix_hints: vec![CypherFixHint {
            code: "ADD_MISSING_PAREN".to_owned(),
            message: "Close the MATCH pattern before RETURN".to_owned(),
        }],
    };

    assert_eq!(response.status, CypherResponseStatus::ValidationFailed);
    assert_eq!(response.warnings.len(), 1);
    assert_eq!(response.validation_errors.len(), 1);
    assert_eq!(response.fix_hints.len(), 1);
}

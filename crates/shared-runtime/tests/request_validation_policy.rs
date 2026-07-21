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
    CypherBudgetRef, CypherParameters, CypherRequest, CypherRequestMode, RequestValidator,
    RuntimeBudget, RuntimeBudgetUsage, RuntimeError, RuntimePolicy,
};

fn workspace_id() -> WorkspaceId {
    WorkspaceId::new("workspace--policy").expect("workspace id should be valid")
}

fn session_id() -> SessionId {
    SessionId::new("session--policy").expect("session id should be valid")
}

fn budget_ref() -> CypherBudgetRef {
    CypherBudgetRef::new("budget-policy--default").expect("budget ref should be valid")
}

fn request(
    mode: CypherRequestMode,
    query_text: &str,
    parameters: CypherParameters,
) -> CypherRequest {
    CypherRequest::new(
        query_text,
        parameters,
        mode,
        workspace_id(),
        session_id(),
        budget_ref(),
    )
    .expect("request should be valid")
}

#[test]
fn validate_read_only_request_accepts_request_inside_policy_limits() {
    let validator = RequestValidator::new(RuntimePolicy::strict_default());
    let mut values = HashMap::new();
    values.insert("limit".to_owned(), "50".to_owned());

    let read_request = request(
        CypherRequestMode::ReadOnly,
        "MATCH (n) RETURN n LIMIT $limit",
        CypherParameters::new(values),
    );

    let validation = validator.validate_read_only_request(&read_request);

    assert!(validation.is_ok());
}

#[test]
fn validate_mutation_request_rejects_when_policy_disallows_mutations() {
    let validator = RequestValidator::new(RuntimePolicy {
        mutation_permissions: false,
        ..RuntimePolicy::strict_default()
    });

    let mutation_request = request(
        CypherRequestMode::Mutation,
        "CREATE (n:Indicator {name: 'x'})",
        CypherParameters::default(),
    );

    let error = validator
        .validate_mutation_request(&mutation_request)
        .expect_err("unsafe mutation should be rejected when policy disallows mutations");

    assert!(matches!(error, RuntimeError::UnsafeMutationAttempt { .. }));
}

#[test]
fn validate_explain_request_rejects_when_mode_not_allowed() {
    let validator = RequestValidator::new(RuntimePolicy {
        allowed_request_modes: vec![CypherRequestMode::ReadOnly, CypherRequestMode::Mutation],
        ..RuntimePolicy::strict_default()
    });

    let explain_request = request(
        CypherRequestMode::Explain,
        "EXPLAIN MATCH (n) RETURN n",
        CypherParameters::default(),
    );

    let error = validator
        .validate_explain_request(&explain_request)
        .expect_err("disallowed mode should be rejected");

    assert!(matches!(error, RuntimeError::DisallowedRequestMode { .. }));
}

#[test]
fn validate_validate_only_request_rejects_unsupported_cypher_feature() {
    let validator = RequestValidator::new(RuntimePolicy::strict_default());

    let validate_request = request(
        CypherRequestMode::ValidateOnly,
        "LOAD CSV FROM 'file:///tmp/data.csv' AS row RETURN row",
        CypherParameters::default(),
    );

    let error = validator
        .validate_validate_only_request(&validate_request)
        .expect_err("unsupported feature should be rejected explicitly");

    assert!(matches!(
        error,
        RuntimeError::UnsupportedCypherFeature { .. }
    ));
}

#[test]
fn validate_validate_only_request_rejects_other_unsupported_cypher_features() {
    let validator = RequestValidator::new(RuntimePolicy::strict_default());

    let call_dbms_request = request(
        CypherRequestMode::ValidateOnly,
        "CALL DBMS.PROCEDURES()",
        CypherParameters::default(),
    );
    let error = validator
        .validate_validate_only_request(&call_dbms_request)
        .expect_err("CALL DBMS should be rejected as unsupported");
    assert!(matches!(
    error,
    RuntimeError::UnsupportedCypherFeature { feature, .. } if feature == "CALL DBMS"
    ));

    let periodic_commit_request = request(
        CypherRequestMode::ValidateOnly,
        "USING PERIODIC COMMIT MATCH (n) RETURN n",
        CypherParameters::default(),
    );
    let error = validator
        .validate_validate_only_request(&periodic_commit_request)
        .expect_err("USING PERIODIC COMMIT should be rejected as unsupported");
    assert!(matches!(
    error,
    RuntimeError::UnsupportedCypherFeature { feature, .. }
    if feature == "USING PERIODIC COMMIT"
    ));
}

#[test]
fn validate_request_rejects_query_over_policy_limit() {
    let validator = RequestValidator::new(RuntimePolicy {
        max_query_length: 12,
        ..RuntimePolicy::strict_default()
    });

    let read_request = request(
        CypherRequestMode::ReadOnly,
        "MATCH (n) RETURN n",
        CypherParameters::default(),
    );

    let error = validator
        .validate_read_only_request(&read_request)
        .expect_err("query length over policy should be rejected");

    assert!(matches!(
    error,
    RuntimeError::RequestLimitExceeded { field, .. } if field == "query_length"
    ));
}

#[test]
fn validate_request_rejects_parameter_count_over_policy_limit() {
    let validator = RequestValidator::new(RuntimePolicy {
        max_parameter_count: 1,
        ..RuntimePolicy::strict_default()
    });

    let mut params = HashMap::new();
    params.insert("a".to_owned(), "1".to_owned());
    params.insert("b".to_owned(), "2".to_owned());

    let read_request = request(
        CypherRequestMode::ReadOnly,
        "MATCH (n) RETURN n",
        CypherParameters::new(params),
    );

    let error = validator
        .validate_read_only_request(&read_request)
        .expect_err("parameter count over policy should be rejected");

    assert!(matches!(
    error,
    RuntimeError::RequestLimitExceeded { field, .. } if field == "parameter_count"
    ));
}

#[test]
fn validate_read_only_request_rejects_mutation_clauses() {
    let validator = RequestValidator::new(RuntimePolicy::strict_default());
    let read_request = request(
        CypherRequestMode::ReadOnly,
        "MATCH (n) SET n.flag = true RETURN n",
        CypherParameters::default(),
    );

    let error = validator
        .validate_read_only_request(&read_request)
        .expect_err("read-only mode should reject mutation keywords");

    assert!(matches!(error, RuntimeError::UnsafeMutationAttempt { .. }));
}

#[test]
fn validate_mutation_request_accepts_without_explicit_write_clause() {
    let validator = RequestValidator::new(RuntimePolicy::strict_default());
    let mutation_request = request(
        CypherRequestMode::Mutation,
        "MATCH (n) RETURN n",
        CypherParameters::default(),
    );

    validator
        .validate_mutation_request(&mutation_request)
        .expect("mutation mode no longer requires explicit write keywords");
}

#[test]
fn validate_mutation_request_rejects_when_mode_does_not_match_validator_entrypoint() {
    let validator = RequestValidator::new(RuntimePolicy::strict_default());
    let read_request = request(
        CypherRequestMode::ReadOnly,
        "MATCH (n) RETURN n",
        CypherParameters::default(),
    );

    let error = validator
        .validate_mutation_request(&read_request)
        .expect_err("mode mismatch should be rejected before policy checks");

    assert!(matches!(
        error,
        RuntimeError::MalformedCypherRequest("mode")
    ));
}

#[test]
fn validate_request_budget_limits_rejects_query_length_over_budget() {
    let validator = RequestValidator::new(RuntimePolicy::strict_default());
    let budget = RuntimeBudget {
        max_query_length: 5,
        ..RuntimeBudget::strict_default()
    };
    let read_request = request(
        CypherRequestMode::ReadOnly,
        "MATCH (n) RETURN n",
        CypherParameters::default(),
    );

    let error = validator
        .validate_request_budget_limits(&read_request, &budget)
        .expect_err("query over budget should be rejected");

    assert!(matches!(
    error,
    RuntimeError::QueryBudgetExceeded { details }
    if details.dimension == "query_length"
    ));
}

#[test]
fn validate_request_budget_limits_rejects_parameter_count_over_budget() {
    let validator = RequestValidator::new(RuntimePolicy::strict_default());
    let budget = RuntimeBudget {
        max_parameter_count: 1,
        ..RuntimeBudget::strict_default()
    };
    let mut params = HashMap::new();
    params.insert("a".to_owned(), "1".to_owned());
    params.insert("b".to_owned(), "2".to_owned());
    let read_request = request(
        CypherRequestMode::ReadOnly,
        "MATCH (n) RETURN n",
        CypherParameters::new(params),
    );

    let error = validator
        .validate_request_budget_limits(&read_request, &budget)
        .expect_err("parameter count over budget should be rejected");

    assert!(matches!(
    error,
    RuntimeError::QueryBudgetExceeded { details }
    if details.dimension == "parameter_count"
    ));
}

#[test]
fn record_budget_usage_rejects_dimension_overages_and_reports_usage_when_valid() {
    let validator = RequestValidator::new(RuntimePolicy::strict_default());
    let budget_ref = budget_ref();
    let budget = RuntimeBudget {
        max_mutation_count: 2,
        max_loaded_records: 3,
        max_returned_records: 4,
        max_payload_bytes: 5,
        max_execution_time_ms: 6,
        ..RuntimeBudget::strict_default()
    };

    let error = validator
        .record_budget_usage(
            &budget_ref,
            &budget,
            RuntimeBudgetUsage {
                query_length: 1,
                parameter_count: 1,
                loaded_record_count: 1,
                returned_record_count: 1,
                mutation_count: 3,
                payload_bytes: 1,
                execution_time_ms: 1,
            },
        )
        .expect_err("mutation over budget should be rejected");
    assert!(matches!(
    error,
    RuntimeError::MutationBudgetExceeded { details }
    if details.dimension == "mutation_count"
    ));

    let error = validator
        .record_budget_usage(
            &budget_ref,
            &budget,
            RuntimeBudgetUsage {
                query_length: 1,
                parameter_count: 1,
                loaded_record_count: 4,
                returned_record_count: 1,
                mutation_count: 1,
                payload_bytes: 1,
                execution_time_ms: 1,
            },
        )
        .expect_err("loaded records over budget should be rejected");
    assert!(matches!(
    error,
    RuntimeError::QueryBudgetExceeded { details }
    if details.dimension == "loaded_record_count"
    ));

    let error = validator
        .record_budget_usage(
            &budget_ref,
            &budget,
            RuntimeBudgetUsage {
                query_length: 1,
                parameter_count: 1,
                loaded_record_count: 1,
                returned_record_count: 5,
                mutation_count: 1,
                payload_bytes: 1,
                execution_time_ms: 1,
            },
        )
        .expect_err("returned records over budget should be rejected");
    assert!(matches!(
    error,
    RuntimeError::QueryBudgetExceeded { details }
    if details.dimension == "returned_record_count"
    ));

    let error = validator
        .record_budget_usage(
            &budget_ref,
            &budget,
            RuntimeBudgetUsage {
                query_length: 1,
                parameter_count: 1,
                loaded_record_count: 1,
                returned_record_count: 1,
                mutation_count: 1,
                payload_bytes: 6,
                execution_time_ms: 1,
            },
        )
        .expect_err("payload bytes over budget should be rejected");
    assert!(matches!(
    error,
    RuntimeError::QueryBudgetExceeded { details }
    if details.dimension == "payload_bytes"
    ));

    let error = validator
        .record_budget_usage(
            &budget_ref,
            &budget,
            RuntimeBudgetUsage {
                query_length: 1,
                parameter_count: 1,
                loaded_record_count: 1,
                returned_record_count: 1,
                mutation_count: 1,
                payload_bytes: 1,
                execution_time_ms: 7,
            },
        )
        .expect_err("execution time over budget should be rejected");
    assert!(matches!(
    error,
    RuntimeError::QueryBudgetExceeded { details }
    if details.dimension == "execution_time_ms"
    ));

    let usage = validator
        .record_budget_usage(
            &budget_ref,
            &budget,
            RuntimeBudgetUsage {
                query_length: 1,
                parameter_count: 1,
                loaded_record_count: 1,
                returned_record_count: 1,
                mutation_count: 1,
                payload_bytes: 1,
                execution_time_ms: 1,
            },
        )
        .expect("in-budget usage should succeed");

    assert_eq!(usage.consumed_units, 7);
    assert!(usage.remaining_units.is_some());
}

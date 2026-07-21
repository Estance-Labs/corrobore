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
    WorkspaceId::new("workspace--budget").expect("workspace id should be valid")
}

fn session_id() -> SessionId {
    SessionId::new("session--budget").expect("session id should be valid")
}

fn budget_ref() -> CypherBudgetRef {
    CypherBudgetRef::new("budget-profile--runtime").expect("budget ref should be valid")
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
fn validate_request_budget_limits_rejects_query_length_over_budget() {
    let validator = RequestValidator::new(RuntimePolicy::strict_default());
    let budget = RuntimeBudget {
        max_query_length: 10,
        ..RuntimeBudget::strict_default()
    };

    let read_request = request(
        CypherRequestMode::ReadOnly,
        "MATCH (n) RETURN n",
        CypherParameters::default(),
    );

    let error = validator
        .validate_request_budget_limits(&read_request, &budget)
        .expect_err("query length over budget should be rejected");

    assert!(matches!(error, RuntimeError::QueryBudgetExceeded { .. }));
}

#[test]
fn validate_request_budget_limits_rejects_parameter_count_over_budget() {
    let validator = RequestValidator::new(RuntimePolicy::strict_default());
    let budget = RuntimeBudget {
        max_parameter_count: 1,
        ..RuntimeBudget::strict_default()
    };

    let mut values = HashMap::new();
    values.insert("a".to_owned(), "1".to_owned());
    values.insert("b".to_owned(), "2".to_owned());

    let read_request = request(
        CypherRequestMode::ReadOnly,
        "MATCH (n) RETURN n",
        CypherParameters::new(values),
    );

    let error = validator
        .validate_request_budget_limits(&read_request, &budget)
        .expect_err("parameter count over budget should be rejected");

    assert!(matches!(error, RuntimeError::QueryBudgetExceeded { .. }));
}

#[test]
fn record_budget_usage_rejects_mutation_count_over_budget() {
    let validator = RequestValidator::new(RuntimePolicy::strict_default());
    let budget = RuntimeBudget {
        max_mutation_count: 3,
        ..RuntimeBudget::strict_default()
    };

    let usage = RuntimeBudgetUsage {
        query_length: 20,
        parameter_count: 2,
        loaded_record_count: 0,
        returned_record_count: 0,
        mutation_count: 4,
        payload_bytes: 40,
        execution_time_ms: 5,
    };

    let error = validator
        .record_budget_usage(&budget_ref(), &budget, usage)
        .expect_err("mutation budget overflow should be explicit");

    assert!(matches!(error, RuntimeError::MutationBudgetExceeded { .. }));
}

#[test]
fn record_budget_usage_rejects_payload_bytes_over_budget() {
    let validator = RequestValidator::new(RuntimePolicy::strict_default());
    let budget = RuntimeBudget {
        max_payload_bytes: 32,
        ..RuntimeBudget::strict_default()
    };

    let usage = RuntimeBudgetUsage {
        query_length: 8,
        parameter_count: 1,
        loaded_record_count: 1,
        returned_record_count: 1,
        mutation_count: 0,
        payload_bytes: 64,
        execution_time_ms: 5,
    };

    let error = validator
        .record_budget_usage(&budget_ref(), &budget, usage)
        .expect_err("payload over budget should be explicit");

    assert!(matches!(error, RuntimeError::QueryBudgetExceeded { .. }));
}

#[test]
fn record_budget_usage_returns_budget_usage_snapshot_when_within_limits() {
    let validator = RequestValidator::new(RuntimePolicy::strict_default());
    let budget = RuntimeBudget::strict_default();
    let usage = RuntimeBudgetUsage {
        query_length: 32,
        parameter_count: 2,
        loaded_record_count: 10,
        returned_record_count: 5,
        mutation_count: 1,
        payload_bytes: 128,
        execution_time_ms: 20,
    };

    let response_usage = validator
        .record_budget_usage(&budget_ref(), &budget, usage)
        .expect("budget usage within limits should be accepted");

    assert_eq!(response_usage.consumed_units, 198);
    assert!(response_usage.remaining_units.is_some());
}

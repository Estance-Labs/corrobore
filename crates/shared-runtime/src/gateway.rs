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
use std::time::Instant;

use cypher_parser::{LiteralValue, ParameterBindings};

use crate::*;

/// Converts runtime parameter values into typed parser bindings.
///
/// This replaces the former textual binding step. Values are never spliced into
/// the query, so there is no stage at which a value could close a string literal
/// or introduce a clause.
fn to_parser_bindings(parameters: &CypherParameters) -> ParameterBindings {
    parameters
        .values()
        .iter()
        .map(|(name, value)| {
            let literal = match value {
                CypherValue::String(text) => LiteralValue::String(text.clone()),
                CypherValue::Integer(number) => LiteralValue::Integer(*number),
                CypherValue::Float(text) => LiteralValue::Float(text.clone()),
                CypherValue::Boolean(flag) => LiteralValue::Boolean(*flag),
                CypherValue::Null => LiteralValue::Null,
                CypherValue::List(values) => {
                    LiteralValue::List(values.iter().map(runtime_value_to_literal).collect())
                }
            };
            (name.clone(), literal)
        })
        .collect()
}

fn runtime_value_to_literal(value: &CypherValue) -> LiteralValue {
    match value {
        CypherValue::String(text) => LiteralValue::String(text.clone()),
        CypherValue::Integer(number) => LiteralValue::Integer(*number),
        CypherValue::Float(text) => LiteralValue::Float(text.clone()),
        CypherValue::Boolean(flag) => LiteralValue::Boolean(*flag),
        CypherValue::Null => LiteralValue::Null,
        CypherValue::List(values) => {
            LiteralValue::List(values.iter().map(runtime_value_to_literal).collect())
        }
    }
}

/// Projects the runtime budget onto the bounds the executor enforces while a
/// query runs.
///
/// `max_query_length`, `max_parameter_count` and `max_payload_bytes` are absent
/// deliberately: the first two are validated before execution starts, and payload
/// size is only knowable once records exist, so it stays a post-execution check.
fn execution_limits(budget: &RuntimeBudget) -> ExecutionLimits {
    ExecutionLimits {
        max_loaded_records: budget.max_loaded_records,
        max_returned_records: budget.max_returned_records,
        max_mutation_count: budget.max_mutation_count,
        max_execution_time_ms: budget.max_execution_time_ms,
    }
}

/// Derives the budget dimensions consumed by one completed execution.
///
/// `loaded_record_count` mirrors the returned count because the executor does not
/// yet report records it touched but did not project. That under-reports the
/// dimension rather than inventing a value, so the limit can only fire on work
/// that is genuinely observable.
fn measure_budget_usage(
    request: &CypherRequest,
    result: &ExecutionResult,
    execution_time_ms: u64,
) -> RuntimeBudgetUsage {
    let (returned_record_count, mutation_count, payload_bytes) = match &result.data {
        ExecutionResultData::Records(records) => {
            let payload_bytes = records
                .iter()
                .flat_map(|record| record.fields.iter())
                .map(|(field, value)| field.len() + value.len())
                .sum();
            (records.len(), 0, payload_bytes)
        }
        ExecutionResultData::MutationSummary {
            nodes_created,
            relationships_created,
            properties_set,
            nodes_deleted,
            relationships_deleted,
            native_fields_changed,
            ..
        } => (
            0,
            nodes_created
                + relationships_created
                + properties_set
                + native_fields_changed
                + nodes_deleted
                + relationships_deleted,
            0,
        ),
        ExecutionResultData::Empty => (0, 0, 0),
    };

    RuntimeBudgetUsage {
        query_length: request.query_text.chars().count(),
        parameter_count: request.parameters.values().len(),
        loaded_record_count: returned_record_count,
        returned_record_count,
        mutation_count,
        payload_bytes,
        execution_time_ms,
    }
}

#[derive(Clone, Debug)]
/// Cypher gateway.
pub struct CypherGateway {
    validator: RequestValidator,
    budget: RuntimeBudget,
    executor: CypherPipelineExecutor,
}

impl CypherGateway {
    /// Creates a gateway from explicit runtime and execution policies.
    pub fn with_policies(
        runtime_policy: RuntimePolicy,
        budget: RuntimeBudget,
        execution_policy: ExecutionPolicy,
    ) -> Self {
        Self {
            validator: RequestValidator::new(runtime_policy),
            budget,
            executor: CypherPipelineExecutor::new(execution_policy),
        }
    }

    /// Creates a gateway from explicit policies and an existing graph.
    pub fn with_graph(
        runtime_policy: RuntimePolicy,
        budget: RuntimeBudget,
        execution_policy: ExecutionPolicy,
        graph: Graph,
    ) -> Self {
        Self {
            validator: RequestValidator::new(runtime_policy),
            budget,
            executor: CypherPipelineExecutor::with_graph(execution_policy, graph),
        }
    }

    /// Strict default.
    pub fn strict_default() -> Self {
        Self::with_policies(
            RuntimePolicy::strict_default(),
            RuntimeBudget::strict_default(),
            ExecutionPolicy {
                // Expose all parser/executor capabilities; host tools decide
                // which mode (read/write) is allowed per use case.
                read_only_by_default: false,
            },
        )
    }

    //
    // Keep an explicit gateway pipeline so runtime callers always pass through
    // policy validation, budget checks, and deterministic executor responses.
    /// Execute.
    pub fn execute(&mut self, request: &CypherRequest) -> Result<CypherResponse, RuntimeError> {
        request.validate_for_gateway_execution()?;

        let validation = match request.mode {
            CypherRequestMode::ReadOnly => self.validator.validate_read_only_request(request),
            CypherRequestMode::Mutation => self.validator.validate_mutation_request(request),
            CypherRequestMode::ValidateOnly => {
                self.validator.validate_validate_only_request(request)
            }
            CypherRequestMode::Explain => self.validator.validate_explain_request(request),
        };

        if let Err(error) = validation {
            return Ok(runtime_error_to_rejected_response(error));
        }

        if let Err(error) = self
            .validator
            .validate_request_budget_limits(request, &self.budget)
        {
            return Ok(runtime_error_to_rejected_response(error));
        }

        // Values travel as typed bindings, so the query the executor parses is the
        // query the caller wrote. Mode classification above inspected that same
        // text, which is why no post-binding re-check is needed any more.
        let bindings = to_parser_bindings(&request.parameters);

        if request.mode == CypherRequestMode::ValidateOnly {
            return Ok(
                match self
                    .executor
                    .validate_with_parameters(&request.query_text, &bindings)
                {
                    Ok(validation_result) => map_execution_result_to_response(validation_result),
                    Err(error) => {
                        runtime_error_to_rejected_response(execution_error_to_runtime_error(error))
                    }
                },
            );
        }

        let started = Instant::now();
        let execution_result = match self.executor.execute_with_limits(
            &request.query_text,
            &bindings,
            execution_limits(&self.budget),
        ) {
            Ok(result) => result,
            // A bound reached mid-execution stopped the query before it could
            // finish, and for a mutation before it wrote anything.
            Err(error) => {
                return Ok(runtime_error_to_rejected_response(
                    execution_error_to_runtime_error(error),
                ));
            }
        };
        let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

        let usage = measure_budget_usage(request, &execution_result, elapsed_ms);
        let recorded = self
            .validator
            .record_budget_usage(&request.budget_ref, &self.budget, usage);

        match recorded {
            Ok(budget_usage) => {
                let mut response = map_execution_result_to_response(execution_result);
                response.budget_usage = Some(budget_usage);
                Ok(response)
            }
            // The executor enforces the scan, mutation and deadline bounds while a
            // query runs, so reaching this point means a dimension it does not
            // track (payload bytes) went over. A read can be rejected outright; a
            // mutation has already been applied and is reported instead, because
            // claiming failure would leave the caller unable to reconcile a write
            // the durable layer then skips.
            Err(error) if request.mode != CypherRequestMode::Mutation => {
                Ok(runtime_error_to_rejected_response(error))
            }
            Err(error) => {
                let mut response = map_execution_result_to_response(execution_result);
                response.warnings.push(format!(
                    "mutation exceeded its runtime budget and was still applied: {error}"
                ));
                Ok(response)
            }
        }
    }

    /// Returns an immutable reference to the runtime graph.
    pub fn graph(&self) -> &Graph {
        self.executor.graph()
    }

    /// Replaces the runtime graph after a failed durable commit.
    pub fn replace_graph(&mut self, graph: Graph) {
        *self.executor.graph_mut() = graph;
    }
}

pub(crate) fn execution_error_to_runtime_error(error: ExecutionError) -> RuntimeError {
    match error {
        // Surfaced as a budget overage so callers get the same code and fix hint
        // whether the bound was reached mid-execution or measured afterwards.
        ExecutionError::LimitExceeded {
            dimension,
            limit,
            reached,
        } => RuntimeError::QueryBudgetExceeded {
            details: RuntimeBudgetExceeded {
                dimension,
                limit,
                actual: reached,
                fix_hint:
                    "Narrow the pattern, add a LIMIT, or split the work into smaller requests."
                        .to_owned(),
            },
        },
 ExecutionError::InvalidQuery(_) => RuntimeError::MalformedCypherRequest("query_text"),
 ExecutionError::FunctionInvocation(registry_error) => RuntimeError::UnsupportedCypherFeature {
 feature: format!("function invocation: {registry_error}"),
 fix_hint:
 "Validate function registration, argument types, and model adapter configuration."
 .to_owned(),
 },
 }
}

pub(crate) fn runtime_error_to_rejected_response(error: RuntimeError) -> CypherResponse {
    let (status, code, message, fix_hint) = match error {
        RuntimeError::UnsupportedCypherFeature { feature, fix_hint } => (
            CypherResponseStatus::Rejected,
            "UNSUPPORTED_CYPHER_FEATURE",
            format!("unsupported cypher feature: {feature}"),
            Some(fix_hint),
        ),
        RuntimeError::UnsafeMutationAttempt { reason, fix_hint } => (
            CypherResponseStatus::Rejected,
            "WRITE_PERMISSION_REQUIRED",
            reason,
            Some(fix_hint),
        ),
        RuntimeError::DisallowedRequestMode { mode, fix_hint } => (
            CypherResponseStatus::Rejected,
            "REQUEST_MODE_DISALLOWED",
            format!("request mode is disallowed by runtime policy: {mode:?}"),
            Some(fix_hint),
        ),
        RuntimeError::RequestLimitExceeded {
            field,
            limit,
            actual,
            fix_hint,
        } => (
            CypherResponseStatus::Rejected,
            "REQUEST_LIMIT_EXCEEDED",
            format!("request limit exceeded for {field}: actual {actual}, limit {limit}"),
            Some(fix_hint),
        ),
        RuntimeError::QueryBudgetExceeded { details }
        | RuntimeError::MutationBudgetExceeded { details } => (
            CypherResponseStatus::Rejected,
            "QUERY_BUDGET_EXCEEDED",
            format!(
                "query budget exceeded for {}: actual {}, limit {}",
                details.dimension, details.actual, details.limit
            ),
            Some(details.fix_hint),
        ),
        _ => (
            CypherResponseStatus::ValidationFailed,
            "REQUEST_VALIDATION_FAILED",
            error.to_string(),
            None,
        ),
    };

    let mut response = CypherResponse {
        status,
        data: CypherResponseData::Empty,
        warnings: vec![],
        validation_errors: vec![CypherValidationError {
            code: code.to_owned(),
            message,
            field: Some("query_text".to_owned()),
        }],
        budget_usage: None,
        audit_references: vec![],
        fix_hints: vec![],
    };

    if let Some(message) = fix_hint {
        response.fix_hints.push(CypherFixHint {
            code: "ACTIONABLE_HINT".to_owned(),
            message,
        });
    }

    response
}

pub(crate) fn map_execution_result_to_response(result: ExecutionResult) -> CypherResponse {
    CypherResponse {
        // Status.
        status: match result.status {
            ExecutionStatus::Success => CypherResponseStatus::Success,
            ExecutionStatus::Rejected => CypherResponseStatus::Rejected,
            ExecutionStatus::ValidationFailed => CypherResponseStatus::ValidationFailed,
        },
        // Data.
        data: map_execution_data(result.data),
        // Warnings.
        warnings: result.warnings,
        // Validation errors.
        validation_errors: result
            .validation_errors
            .into_iter()
            .map(|error| CypherValidationError {
                // Code.
                code: error.code,
                // Message.
                message: error.message,
                // Field.
                field: Some("query_text".to_owned()),
            })
            .collect(),
        // Budget usage.
        budget_usage: None,
        // Audit references.
        audit_references: vec![],
        // Fix hints.
        fix_hints: result
            .fix_hints
            .into_iter()
            .map(|hint| CypherFixHint {
                // Code.
                code: hint.code,
                // Message.
                message: hint.message,
            })
            .collect(),
    }
}

pub(crate) fn map_execution_data(data: ExecutionResultData) -> CypherResponseData {
    match data {
        ExecutionResultData::Records(records) => {
            CypherResponseData::Records(records_to_cypher_records(records))
        }
        ExecutionResultData::MutationSummary {
            nodes_created,
            relationships_created,
            properties_set,
            nodes_deleted,
            relationships_deleted,
            matched_rows,
            native_fields_changed,
            property_fields_changed,
            nodes_updated,
            relationships_updated,
        } => CypherResponseData::MutationSummary(CypherMutationSummary {
            matched_rows: matched_rows as u64,
            created_nodes: nodes_created as u64,
            updated_nodes: nodes_updated as u64,
            deleted_nodes: nodes_deleted as u64,
            created_relationships: relationships_created as u64,
            updated_relationships: relationships_updated as u64,
            deleted_relationships: relationships_deleted as u64,
            properties_set: properties_set as u64,
            native_fields_changed: native_fields_changed as u64,
            property_fields_changed: property_fields_changed as u64,
        }),
        ExecutionResultData::Empty => CypherResponseData::Empty,
    }
}

fn records_to_cypher_records(records: Vec<ExecutionRecord>) -> Vec<CypherRecord> {
    records
        .into_iter()
        .map(|record| CypherRecord {
            fields: record.fields,
        })
        .collect()
}

/// Returns true when the query text contains a Cypher mutation clause keyword.
///
/// Detection is token-based so leading clauses (for example a query that
/// starts with `CREATE`) are recognized, unlike substring checks that require
/// surrounding whitespace.
pub fn contains_mutation_keywords(query_text: &str) -> bool {
    query_text
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .any(|token| {
            matches!(
                token.to_ascii_uppercase().as_str(),
                "CREATE" | "MERGE" | "DELETE" | "SET" | "REMOVE" | "DROP"
            )
        })
}

pub(crate) fn first_unsupported_feature(query_text: &str) -> Option<&'static str> {
    let uppercase = query_text.to_ascii_uppercase();

    if uppercase.contains("LOAD CSV") {
        return Some("LOAD CSV");
    }

    if uppercase.contains("CALL DBMS") {
        return Some("CALL DBMS");
    }

    if uppercase.contains("USING PERIODIC COMMIT") {
        return Some("USING PERIODIC COMMIT");
    }

    None
}

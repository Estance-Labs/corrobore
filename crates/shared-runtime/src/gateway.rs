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
            };
            (name.clone(), literal)
        })
        .collect()
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

        let execution_result = self
            .executor
            .execute_with_parameters(&request.query_text, &bindings)
            .map_err(execution_error_to_runtime_error)?;

        Ok(map_execution_result_to_response(execution_result))
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
        } => {
            // Map mutation summary into a single record with summary fields.
            let mut fields = std::collections::HashMap::new();
            fields.insert("nodes_created".to_owned(), nodes_created.to_string());
            fields.insert(
                "relationships_created".to_owned(),
                relationships_created.to_string(),
            );
            fields.insert("properties_set".to_owned(), properties_set.to_string());
            fields.insert("nodes_deleted".to_owned(), nodes_deleted.to_string());
            fields.insert(
                "relationships_deleted".to_owned(),
                relationships_deleted.to_string(),
            );
            CypherResponseData::Records(vec![CypherRecord { fields }])
        }
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

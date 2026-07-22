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
use crate::*;

#[derive(Clone, Debug)]
/// Request validator.
pub struct RequestValidator {
    policy: RuntimePolicy,
}

impl RequestValidator {
    /// Creates a new instance.
    pub fn new(policy: RuntimePolicy) -> Self {
        Self { policy }
    }

    /// Validates the read only request.
    pub fn validate_read_only_request(&self, request: &CypherRequest) -> Result<(), RuntimeError> {
        self.ensure_mode_matches_request(request, CypherRequestMode::ReadOnly)?;
        self.validate_common_request_policy(request)?;

        if contains_mutation_keywords(&request.query_text) {
            return Err(RuntimeError::UnsafeMutationAttempt {
                reason: "mutation clauses are not allowed in read-only mode".to_owned(),
                fix_hint: "Use mutation mode for write clauses or remove write operations."
                    .to_owned(),
            });
        }

        Ok(())
    }

    // Validation happens before any mutation boundary so unsafe or disallowed
    // requests are rejected before graph state can change.
    /// Validates the mutation request.
    pub fn validate_mutation_request(&self, request: &CypherRequest) -> Result<(), RuntimeError> {
        self.ensure_mode_matches_request(request, CypherRequestMode::Mutation)?;
        self.validate_common_request_policy(request)?;

        if !self.policy.mutation_permissions {
            return Err(RuntimeError::UnsafeMutationAttempt {
                reason: "runtime policy disallows mutations for this validator".to_owned(),
                fix_hint:
                    "Use a read-only mode or apply a policy that enables mutation permissions."
                        .to_owned(),
            });
        }

        Ok(())
    }

    /// Validates the explain request.
    pub fn validate_explain_request(&self, request: &CypherRequest) -> Result<(), RuntimeError> {
        self.ensure_mode_matches_request(request, CypherRequestMode::Explain)?;
        self.validate_common_request_policy(request)
    }

    /// Validates the only request.
    pub fn validate_validate_only_request(
        &self,
        request: &CypherRequest,
    ) -> Result<(), RuntimeError> {
        self.ensure_mode_matches_request(request, CypherRequestMode::ValidateOnly)?;
        self.validate_common_request_policy(request)
    }

    /// Validates the request budget limits.
    pub fn validate_request_budget_limits(
        &self,
        request: &CypherRequest,
        budget: &RuntimeBudget,
    ) -> Result<(), RuntimeError> {
        let query_length = request.query_text.chars().count();
        if query_length > budget.max_query_length {
            return Err(RuntimeError::QueryBudgetExceeded {
                details: RuntimeBudgetExceeded {
                    dimension: "query_length",
                    limit: budget.max_query_length,
                    actual: query_length,
                    fix_hint: "Shorten the query text or split request logic across smaller calls."
                        .to_owned(),
                },
            });
        }

        let parameter_count = request.parameters.values().len();
        if parameter_count > budget.max_parameter_count {
            return Err(RuntimeError::QueryBudgetExceeded {
                details: RuntimeBudgetExceeded {
                    dimension: "parameter_count",
                    limit: budget.max_parameter_count,
                    actual: parameter_count,
                    fix_hint: "Reduce parameter fan-out or batch requests into smaller sets."
                        .to_owned(),
                },
            });
        }

        Ok(())
    }

    /// Record budget usage.
    pub fn record_budget_usage(
        &self,
        budget_ref: &CypherBudgetRef,
        budget: &RuntimeBudget,
        usage: RuntimeBudgetUsage,
    ) -> Result<CypherBudgetUsage, RuntimeError> {
        if usage.mutation_count > budget.max_mutation_count {
            return Err(RuntimeError::MutationBudgetExceeded {
                details: RuntimeBudgetExceeded {
                    dimension: "mutation_count",
                    limit: budget.max_mutation_count,
                    actual: usage.mutation_count,
                    fix_hint: "Split mutation batches or reduce write set size per request."
                        .to_owned(),
                },
            });
        }

        if usage.loaded_record_count > budget.max_loaded_records {
            return Err(RuntimeError::QueryBudgetExceeded {
                details: RuntimeBudgetExceeded {
                    dimension: "loaded_record_count",
                    limit: budget.max_loaded_records,
                    actual: usage.loaded_record_count,
                    fix_hint:
                        "Narrow traversal scope or add stronger filters to reduce loaded records."
                            .to_owned(),
                },
            });
        }

        if usage.returned_record_count > budget.max_returned_records {
            return Err(RuntimeError::QueryBudgetExceeded {
                details: RuntimeBudgetExceeded {
                    dimension: "returned_record_count",
                    limit: budget.max_returned_records,
                    actual: usage.returned_record_count,
                    fix_hint: "Use LIMIT or page through results to reduce response cardinality."
                        .to_owned(),
                },
            });
        }

        if usage.payload_bytes > budget.max_payload_bytes {
            return Err(RuntimeError::QueryBudgetExceeded {
                details: RuntimeBudgetExceeded {
                    dimension: "payload_bytes",
                    limit: budget.max_payload_bytes,
                    actual: usage.payload_bytes,
                    fix_hint: "Return fewer fields or reduce payload size per request.".to_owned(),
                },
            });
        }

        if usage.execution_time_ms > budget.max_execution_time_ms {
            return Err(RuntimeError::QueryBudgetExceeded {
                details: RuntimeBudgetExceeded {
                    dimension: "execution_time_ms",
                    limit: budget.max_execution_time_ms as usize,
                    actual: usage.execution_time_ms as usize,
                    fix_hint: "Reduce query complexity or split work into shorter operations."
                        .to_owned(),
                },
            });
        }

        let consumed_units = usage.query_length as u64
            + usage.parameter_count as u64
            + usage.loaded_record_count as u64
            + usage.returned_record_count as u64
            + usage.mutation_count as u64
            + usage.payload_bytes as u64
            + usage.execution_time_ms;

        let budget_ceiling = budget.max_query_length as u64
            + budget.max_parameter_count as u64
            + budget.max_loaded_records as u64
            + budget.max_returned_records as u64
            + budget.max_mutation_count as u64
            + budget.max_payload_bytes as u64
            + budget.max_execution_time_ms;

        Ok(CypherBudgetUsage {
            budget_ref: budget_ref.clone(),
            consumed_units,
            remaining_units: Some(budget_ceiling.saturating_sub(consumed_units)),
        })
    }

    fn ensure_mode_matches_request(
        &self,
        request: &CypherRequest,
        expected_mode: CypherRequestMode,
    ) -> Result<(), RuntimeError> {
        if request.mode != expected_mode {
            return Err(RuntimeError::MalformedCypherRequest("mode"));
        }

        Ok(())
    }

    fn validate_common_request_policy(&self, request: &CypherRequest) -> Result<(), RuntimeError> {
        if !self.policy.allowed_request_modes.contains(&request.mode) {
            return Err(RuntimeError::DisallowedRequestMode {
 mode: request.mode.clone(),
 fix_hint: "Select a request mode allowed by runtime policy or update policy configuration."
 .to_owned(),
 });
        }

        let query_length = request.query_text.chars().count();
        if query_length > self.policy.max_query_length {
            return Err(RuntimeError::RequestLimitExceeded {
                field: "query_length",
                limit: self.policy.max_query_length,
                actual: query_length,
                fix_hint: "Shorten the query or split the operation into smaller requests."
                    .to_owned(),
            });
        }

        let parameter_count = request.parameters.values().len();
        if parameter_count > self.policy.max_parameter_count {
            return Err(RuntimeError::RequestLimitExceeded {
                field: "parameter_count",
                limit: self.policy.max_parameter_count,
                actual: parameter_count,
                fix_hint: "Reduce parameter fan-out or batch requests with smaller parameter sets."
                    .to_owned(),
            });
        }

        if let Some(feature) = first_unsupported_feature(&request.query_text) {
            return Err(RuntimeError::UnsupportedCypherFeature {
                feature: feature.to_owned(),
                fix_hint: "Rewrite the query using the supported runtime Cypher subset.".to_owned(),
            });
        }

        Ok(())
    }
}

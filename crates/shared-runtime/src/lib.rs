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
#![warn(missing_docs)]

//! Shared durable runtime for the intelligence graph engine.
//!
//! Provides the runtime lifecycle, workspace and session registries, Cypher
//! gateway pipeline, request validation, audit event model, transaction
//! metadata, budget enforcement, and benchmark harness contracts.

pub use cypher_executor::ExecutionPolicy;
pub(crate) use cypher_executor::{
    CypherPipelineExecutor, ExecutionError, ExecutionRecord, ExecutionResult, ExecutionResultData,
    ExecutionStatus,
};
pub(crate) use graph_core::{ActorId, Graph, SessionId, TransactionId, WorkspaceId};
pub(crate) use serde::{Deserialize, Serialize};
pub(crate) use std::collections::{HashMap, HashSet};
pub(crate) use thiserror::Error;

mod actor;
mod audit;
mod benchmark;
mod budget;
mod cypher;
mod error;
mod gateway;
mod policy;
mod registries;
mod session;
mod validation;
mod workspace;

pub use actor::*;
pub use audit::*;
pub use benchmark::*;
pub use budget::*;
pub use cypher::*;
pub use error::*;
pub use gateway::*;
pub use policy::*;
pub use registries::*;
pub use session::*;
pub use validation::*;
pub use workspace::*;

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace_id() -> WorkspaceId {
        WorkspaceId::new("workspace--runtime-tests").expect("workspace id should be valid")
    }

    fn session_id() -> SessionId {
        SessionId::new("session--runtime-tests").expect("session id should be valid")
    }

    fn budget_ref() -> CypherBudgetRef {
        CypherBudgetRef::new("budget--runtime-tests").expect("budget ref should be valid")
    }

    #[test]
    fn percentage_handles_zero_denominator_and_caps_large_values() {
        assert_eq!(percentage(1, 0), 0);
        assert_eq!(percentage(1, 2), 50);
        assert_eq!(percentage(1_000, 1), 100);
    }

    #[test]
    fn mutation_keyword_and_unsupported_feature_detection_work() {
        assert!(contains_mutation_keywords("MATCH (n) CREATE (m) RETURN m"));
        assert!(!contains_mutation_keywords("MATCH (n) RETURN n"));

        assert_eq!(
            first_unsupported_feature("USING PERIODIC COMMIT LOAD CSV FROM 'x' AS row"),
            Some("LOAD CSV")
        );
        assert_eq!(
            first_unsupported_feature("CALL DBMS.PROCEDURES()"),
            Some("CALL DBMS")
        );
        assert_eq!(first_unsupported_feature("MATCH (n) RETURN n"), None);
    }

    #[test]
    fn cypher_request_validate_for_gateway_execution_rejects_explain_mode() {
        let request = CypherRequest::new(
            "MATCH (n) RETURN n",
            CypherParameters::default(),
            CypherRequestMode::Explain,
            workspace_id(),
            session_id(),
            budget_ref(),
        )
        .expect("request should be created");

        let error = request
            .validate_for_gateway_execution()
            .expect_err("explain mode should be rejected for gateway execution");

        assert!(matches!(
            error,
            RuntimeError::UnsupportedCypherRequestMode(CypherRequestMode::Explain)
        ));
    }

    #[test]
    fn validate_audit_reason_rejects_blank_fields() {
        let blank_code = validate_audit_reason(&RuntimeAuditReasonMetadata {
            code: " ".to_owned(),
            message: "reason".to_owned(),
            fix_hint: None,
        })
        .expect_err("blank code should be rejected");
        let blank_message = validate_audit_reason(&RuntimeAuditReasonMetadata {
            code: "RUNTIME_CODE".to_owned(),
            message: " ".to_owned(),
            fix_hint: None,
        })
        .expect_err("blank message should be rejected");

        assert!(matches!(
            blank_code,
            RuntimeError::AuditMetadataCreationFailed("reason.code")
        ));
        assert!(matches!(
            blank_message,
            RuntimeError::AuditMetadataCreationFailed("reason.message")
        ));
    }

    #[test]
    fn runtime_error_to_rejected_response_maps_fix_hint_and_status() {
        let rejected = runtime_error_to_rejected_response(RuntimeError::UnsupportedCypherFeature {
            feature: "LOAD CSV".to_owned(),
            fix_hint: "Use supported subset".to_owned(),
        });
        assert_eq!(rejected.status, CypherResponseStatus::Rejected);
        assert_eq!(rejected.validation_errors.len(), 1);
        assert_eq!(rejected.fix_hints.len(), 1);

        let validation_failed =
            runtime_error_to_rejected_response(RuntimeError::MalformedCypherRequest("query_text"));
        assert_eq!(
            validation_failed.status,
            CypherResponseStatus::ValidationFailed
        );
        assert!(validation_failed.fix_hints.is_empty());
    }

    #[test]
    fn benchmark_summary_aggregates_rates() {
        let empty = BenchmarkComparisonSummary::from_comparisons(&[]);
        assert_eq!(empty.fixture_count, 0);

        let comparisons = vec![
            BenchmarkFixtureComparison {
                // Fixture id.
                fixture_id: "f1".to_owned(),
                // Token usage delta.
                token_usage_delta: 1,
                // Context size delta.
                context_size_delta: 1,
                // Correction iterations delta.
                correction_iterations_delta: 0,
                // Graph export valid.
                graph_export_valid: true,
                // Graph audit coverage.
                graph_audit_coverage: false,
                // Graph snapshot reproducible.
                graph_snapshot_reproducible: true,
            },
            BenchmarkFixtureComparison {
                // Fixture id.
                fixture_id: "f2".to_owned(),
                // Token usage delta.
                token_usage_delta: -1,
                // Context size delta.
                context_size_delta: 2,
                // Correction iterations delta.
                correction_iterations_delta: 1,
                // Graph export valid.
                graph_export_valid: false,
                // Graph audit coverage.
                graph_audit_coverage: true,
                // Graph snapshot reproducible.
                graph_snapshot_reproducible: true,
            },
        ];

        let summary = BenchmarkComparisonSummary::from_comparisons(&comparisons);
        assert_eq!(summary.fixture_count, 2);
        assert_eq!(summary.graph_export_valid_rate_percent, 50);
        assert_eq!(summary.graph_audit_coverage_rate_percent, 50);
        assert_eq!(summary.graph_snapshot_reproducibility_rate_percent, 100);
    }

    #[test]
    fn request_builders_set_mode_and_validate_allowed_gateway_modes() {
        let workspace = workspace_id();
        let session = session_id();
        let budget = budget_ref();

        let read_only = CypherRequest::build_read_only_request(
            "MATCH (n) RETURN n",
            CypherParameters::default(),
            workspace.clone(),
            session.clone(),
            budget.clone(),
        )
        .expect("read-only request should be built");
        let mutation = CypherRequest::build_mutation_request(
            "MATCH (n) CREATE (m)",
            CypherParameters::default(),
            workspace.clone(),
            session.clone(),
            budget.clone(),
        )
        .expect("mutation request should be built");
        let validate_only = CypherRequest::build_validate_only_request(
            "MATCH (n) RETURN n",
            CypherParameters::default(),
            workspace,
            session,
            budget,
        )
        .expect("validate-only request should be built");

        assert_eq!(read_only.mode, CypherRequestMode::ReadOnly);
        assert_eq!(mutation.mode, CypherRequestMode::Mutation);
        assert_eq!(validate_only.mode, CypherRequestMode::ValidateOnly);

        read_only
            .validate_for_gateway_execution()
            .expect("read-only mode should be accepted");
        mutation
            .validate_for_gateway_execution()
            .expect("mutation mode should be accepted");
        validate_only
            .validate_for_gateway_execution()
            .expect("validate-only mode should be accepted");
    }

    #[test]
    fn request_validator_reports_disallowed_mode_and_mutation_permissions_errors() {
        let policy = RuntimePolicy {
            allowed_request_modes: vec![CypherRequestMode::ReadOnly],
            max_query_length: 256,
            max_parameter_count: 8,
            mutation_permissions: false,
            audit_policy_references: vec![],
        };
        let validator = RequestValidator::new(policy);

        let mutation_request = CypherRequest::build_mutation_request(
            "MATCH (n) CREATE (m)",
            CypherParameters::default(),
            workspace_id(),
            session_id(),
            budget_ref(),
        )
        .expect("mutation request should be built");

        let disallowed = validator
            .validate_mutation_request(&mutation_request)
            .expect_err("mutation mode should be disallowed by policy");
        assert!(matches!(
            disallowed,
            RuntimeError::DisallowedRequestMode {
                mode: CypherRequestMode::Mutation,
                ..
            }
        ));

        let mutation_policy = RuntimePolicy {
            allowed_request_modes: vec![CypherRequestMode::Mutation],
            max_query_length: 256,
            max_parameter_count: 8,
            mutation_permissions: true,
            audit_policy_references: vec![],
        };
        let mutation_validator = RequestValidator::new(mutation_policy);
        let read_shape_in_mutation_mode = CypherRequest::build_mutation_request(
            "MATCH (n) RETURN n",
            CypherParameters::default(),
            workspace_id(),
            session_id(),
            budget_ref(),
        )
        .expect("mutation request should be built");

        mutation_validator
            .validate_mutation_request(&read_shape_in_mutation_mode)
            .expect("mutation mode no longer requires explicit write keywords");

        let disabled_mutation_policy = RuntimePolicy {
            allowed_request_modes: vec![CypherRequestMode::Mutation],
            max_query_length: 256,
            max_parameter_count: 8,
            mutation_permissions: false,
            audit_policy_references: vec![],
        };
        let disabled_validator = RequestValidator::new(disabled_mutation_policy);

        let unsafe_error = disabled_validator
            .validate_mutation_request(&read_shape_in_mutation_mode)
            .expect_err("mutation mode should be rejected when policy disables writes");
        assert!(matches!(
            unsafe_error,
            RuntimeError::UnsafeMutationAttempt { .. }
        ));
    }

    #[test]
    fn stable_query_hash_is_deterministic_and_changes_with_text() {
        let query_a = "MATCH (n) RETURN n";
        let query_b = "MATCH (n) RETURN n LIMIT 1";

        let hash_a1 = stable_query_text_hash(query_a);
        let hash_a2 = stable_query_text_hash(query_a);
        let hash_b = stable_query_text_hash(query_b);

        assert_eq!(hash_a1, hash_a2);
        assert_ne!(hash_a1, hash_b);
    }

    #[test]
    fn execution_mapping_helpers_cover_error_and_data_paths() {
        let invalid_query_error =
            execution_error_to_runtime_error(ExecutionError::InvalidQuery("bad query".to_owned()));
        assert!(matches!(
            invalid_query_error,
            RuntimeError::MalformedCypherRequest("query_text")
        ));

        let mut fields = HashMap::new();
        fields.insert("name".to_owned(), "alpha".to_owned());
        let data = map_execution_data(ExecutionResultData::Records(vec![ExecutionRecord {
            fields,
        }]));

        assert!(matches!(
        data,
        CypherResponseData::Records(records)
        if records.len() == 1
        && records[0].fields.get("name") == Some(&"alpha".to_owned())
        ));
        assert!(matches!(
            map_execution_data(ExecutionResultData::Empty),
            CypherResponseData::Empty
        ));
    }

    #[test]
    fn simple_identity_helpers_preserve_values() {
        let actor = ActorRef::new(
            ActorId::new("actor--runtime-test").expect("actor id should be valid"),
            ActorKind::Agent,
        );
        let timestamp = RuntimeTimestamp::from_millis(42);

        assert_eq!(actor.kind, ActorKind::Agent);
        assert_eq!(timestamp.as_millis(), 42);
    }

    #[test]
    fn benchmark_runners_validate_required_fixture_fields() {
        let fixture = BenchmarkFixture {
            fixture_id: "fixture--1".to_owned(),
            description: "benchmark fixture".to_owned(),
            baseline: BenchmarkWorkflowInput {
                workflow_name: "baseline-workflow".to_owned(),
                steps: vec!["step-a".to_owned()],
            },
            graph: BenchmarkWorkflowInput {
                workflow_name: "graph-workflow".to_owned(),
                steps: vec!["step-b".to_owned()],
            },
            expected: BenchmarkExpectation {
                requires_export_validation: true,
                requires_audit_coverage: true,
                requires_snapshot_reproducibility: true,
            },
        };
        let metrics = BenchmarkObservedMetrics {
            token_usage: 10,
            context_size: 20,
            correction_iterations: 1,
            export_valid: true,
            snapshot_reproducible: true,
            audit_coverage: true,
            invalid_write_rejections: 0,
        };

        let baseline_run = FlatJsonBaselineRunner
            .run_fixture(&fixture, metrics.clone())
            .expect("baseline fixture should be accepted");
        let graph_run = GraphWorkflowRunner
            .run_fixture(&fixture, metrics)
            .expect("graph fixture should be accepted");

        assert_eq!(baseline_run.workflow_name, "baseline-workflow");
        assert_eq!(graph_run.workflow_name, "graph-workflow");

        let invalid_fixture = BenchmarkFixture {
            fixture_id: " ".to_owned(),
            ..fixture
        };
        assert!(matches!(
            FlatJsonBaselineRunner.run_fixture(&invalid_fixture, baseline_run.metrics.clone()),
            Err(RuntimeError::InvalidBenchmarkRun("fixture_id"))
        ));
        assert!(matches!(
            GraphWorkflowRunner.run_fixture(&invalid_fixture, graph_run.metrics),
            Err(RuntimeError::InvalidBenchmarkRun("fixture_id"))
        ));
    }

    #[test]
    fn benchmark_harness_comparison_and_multi_agent_evaluation_cover_validation_paths() {
        let harness = BenchmarkHarness;

        let baseline = BenchmarkRun {
            fixture_id: "fixture--a".to_owned(),
            workflow_name: "baseline".to_owned(),
            metrics: BenchmarkObservedMetrics {
                token_usage: 10,
                context_size: 10,
                correction_iterations: 1,
                export_valid: false,
                snapshot_reproducible: false,
                audit_coverage: false,
                invalid_write_rejections: 0,
            },
        };
        let graph = BenchmarkRun {
            fixture_id: "fixture--b".to_owned(),
            workflow_name: "graph".to_owned(),
            metrics: BenchmarkObservedMetrics {
                token_usage: 5,
                context_size: 8,
                correction_iterations: 0,
                export_valid: true,
                snapshot_reproducible: true,
                audit_coverage: true,
                invalid_write_rejections: 0,
            },
        };

        let mismatch = harness
            .compare_fixture_runs(&baseline, &graph)
            .expect_err("fixture mismatch should be rejected");
        assert!(matches!(
            mismatch,
            RuntimeError::BenchmarkFixtureMismatch { .. }
        ));

        let scenario = MultiAgentBenchmarkScenario {
            scenario_id: "scenario--1".to_owned(),
            workspace_id: workspace_id(),
            agent_ids: vec![
                ActorId::new("actor--a").expect("actor id should be valid"),
                ActorId::new("actor--b").expect("actor id should be valid"),
            ],
            expected_handoffs: 2,
        };
        let result = harness
            .evaluate_multi_agent_scenario(
                &scenario,
                &BenchmarkScenarioObservation {
                    completed_handoffs: 3,
                    persisted_state_reads: 1,
                },
            )
            .expect("scenario should be evaluated");
        assert_eq!(result.handoff_completion_rate_percent, 100);
        assert!(result.shared_state_persistence_confirmed);

        let invalid_scenario = MultiAgentBenchmarkScenario {
            scenario_id: " ".to_owned(),
            ..scenario
        };
        let invalid = harness
            .evaluate_multi_agent_scenario(
                &invalid_scenario,
                &BenchmarkScenarioObservation {
                    completed_handoffs: 0,
                    persisted_state_reads: 0,
                },
            )
            .expect_err("blank scenario id should be rejected");
        assert!(matches!(
            invalid,
            RuntimeError::InvalidMultiAgentBenchmarkScenario("scenario_id")
        ));
    }

    #[test]
    fn execution_result_statuses_map_to_response_statuses() {
        let rejected = map_execution_result_to_response(ExecutionResult {
            status: ExecutionStatus::Rejected,
            data: ExecutionResultData::Empty,
            warnings: vec!["w".to_owned()],
            validation_errors: vec![],
            fix_hints: vec![],
        });
        assert_eq!(rejected.status, CypherResponseStatus::Rejected);
        assert_eq!(rejected.warnings, vec!["w".to_owned()]);

        let validation_failed = map_execution_result_to_response(ExecutionResult {
            status: ExecutionStatus::ValidationFailed,
            data: ExecutionResultData::Empty,
            warnings: vec![],
            validation_errors: vec![cypher_executor::ExecutionValidationError {
                code: "V".to_owned(),
                message: "invalid".to_owned(),
            }],
            fix_hints: vec![cypher_executor::ExecutionFixHint {
                code: "H".to_owned(),
                message: "fix".to_owned(),
            }],
        });
        assert_eq!(
            validation_failed.status,
            CypherResponseStatus::ValidationFailed
        );
        assert_eq!(validation_failed.validation_errors.len(), 1);
        assert_eq!(validation_failed.fix_hints.len(), 1);
    }
}

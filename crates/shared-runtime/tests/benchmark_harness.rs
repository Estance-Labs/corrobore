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
use graph_core::{ActorId, WorkspaceId};
use shared_runtime::{
    BenchmarkComparisonSummary, BenchmarkCorpus, BenchmarkExpectation, BenchmarkFixture,
    BenchmarkHarness, BenchmarkObservedMetrics, BenchmarkScenarioObservation,
    BenchmarkWorkflowInput, FlatJsonBaselineRunner, GraphWorkflowRunner, MultiAgentBenchmarkResult,
    MultiAgentBenchmarkScenario, RuntimeError,
};

fn fixture(id: &str) -> BenchmarkFixture {
    BenchmarkFixture {
        fixture_id: id.to_owned(),
        description: "benchmark fixture".to_owned(),
        baseline: BenchmarkWorkflowInput {
            workflow_name: "flat-json-baseline".to_owned(),
            steps: vec!["load".to_owned(), "correct".to_owned()],
        },
        graph: BenchmarkWorkflowInput {
            workflow_name: "graph-workflow".to_owned(),
            steps: vec!["query".to_owned(), "update".to_owned()],
        },
        expected: BenchmarkExpectation {
            requires_export_validation: true,
            requires_audit_coverage: true,
            requires_snapshot_reproducibility: true,
        },
    }
}

#[test]
fn benchmark_runners_capture_expected_workflow_names_and_metrics() {
    let fixture = fixture("fixture-001");
    let baseline_runner = FlatJsonBaselineRunner::default();
    let graph_runner = GraphWorkflowRunner::default();

    let baseline_metrics = BenchmarkObservedMetrics {
        token_usage: 1000,
        context_size: 200,
        correction_iterations: 4,
        export_valid: true,
        snapshot_reproducible: false,
        audit_coverage: true,
        invalid_write_rejections: 0,
    };

    let graph_metrics = BenchmarkObservedMetrics {
        token_usage: 400,
        context_size: 80,
        correction_iterations: 2,
        export_valid: true,
        snapshot_reproducible: true,
        audit_coverage: true,
        invalid_write_rejections: 1,
    };

    let baseline_run = baseline_runner
        .run_fixture(&fixture, baseline_metrics.clone())
        .expect("baseline run should succeed");
    let graph_run = graph_runner
        .run_fixture(&fixture, graph_metrics.clone())
        .expect("graph run should succeed");

    assert_eq!(baseline_run.workflow_name, "flat-json-baseline");
    assert_eq!(graph_run.workflow_name, "graph-workflow");
    assert_eq!(baseline_run.metrics, baseline_metrics);
    assert_eq!(graph_run.metrics, graph_metrics);
}

#[test]
fn benchmark_harness_comparison_tracks_token_context_and_iteration_deltas() {
    let harness = BenchmarkHarness::default();
    let corpus = BenchmarkCorpus {
        schema_version: "1.0".to_owned(),
        fixtures: vec![fixture("fixture-001")],
    };

    let baseline_runner = FlatJsonBaselineRunner::default();
    let graph_runner = GraphWorkflowRunner::default();

    let baseline_run = baseline_runner
        .run_fixture(
            &corpus.fixtures[0],
            BenchmarkObservedMetrics {
                token_usage: 1000,
                context_size: 200,
                correction_iterations: 4,
                export_valid: true,
                snapshot_reproducible: false,
                audit_coverage: true,
                invalid_write_rejections: 0,
            },
        )
        .expect("baseline run should succeed");

    let graph_run = graph_runner
        .run_fixture(
            &corpus.fixtures[0],
            BenchmarkObservedMetrics {
                token_usage: 400,
                context_size: 80,
                correction_iterations: 2,
                export_valid: true,
                snapshot_reproducible: true,
                audit_coverage: true,
                invalid_write_rejections: 1,
            },
        )
        .expect("graph run should succeed");

    let comparison = harness
        .compare_fixture_runs(&baseline_run, &graph_run)
        .expect("comparison should succeed");

    assert_eq!(comparison.token_usage_delta, -600);
    assert_eq!(comparison.context_size_delta, -120);
    assert_eq!(comparison.correction_iterations_delta, -2);
    assert!(comparison.graph_snapshot_reproducible);

    let summary = BenchmarkComparisonSummary::from_comparisons(&[comparison]);
    assert_eq!(summary.fixture_count, 1);
    assert_eq!(summary.graph_export_valid_rate_percent, 100);
    assert_eq!(summary.graph_audit_coverage_rate_percent, 100);
    assert_eq!(summary.graph_snapshot_reproducibility_rate_percent, 100);
}

#[test]
fn benchmark_harness_rejects_fixture_mismatch_between_runs() {
    let harness = BenchmarkHarness::default();
    let baseline_runner = FlatJsonBaselineRunner::default();
    let graph_runner = GraphWorkflowRunner::default();

    let baseline_run = baseline_runner
        .run_fixture(
            &fixture("fixture-001"),
            BenchmarkObservedMetrics {
                token_usage: 100,
                context_size: 50,
                correction_iterations: 1,
                export_valid: true,
                snapshot_reproducible: true,
                audit_coverage: true,
                invalid_write_rejections: 0,
            },
        )
        .expect("baseline run should succeed");

    let graph_run = graph_runner
        .run_fixture(
            &fixture("fixture-002"),
            BenchmarkObservedMetrics {
                token_usage: 90,
                context_size: 40,
                correction_iterations: 1,
                export_valid: true,
                snapshot_reproducible: true,
                audit_coverage: true,
                invalid_write_rejections: 0,
            },
        )
        .expect("graph run should succeed");

    let error = harness
        .compare_fixture_runs(&baseline_run, &graph_run)
        .expect_err("fixture mismatch should be rejected");

    assert!(matches!(
        error,
        RuntimeError::BenchmarkFixtureMismatch { .. }
    ));
}

#[test]
fn multi_agent_benchmark_scenario_evaluates_shared_runtime_persistence() {
    let harness = BenchmarkHarness::default();
    let scenario = MultiAgentBenchmarkScenario {
        scenario_id: "scenario-shared-runtime-001".to_owned(),
        workspace_id: WorkspaceId::new("workspace--benchmark")
            .expect("workspace id should be valid"),
        agent_ids: vec![
            ActorId::new("agent--planner").expect("actor id should be valid"),
            ActorId::new("agent--validator").expect("actor id should be valid"),
        ],
        expected_handoffs: 2,
    };

    let observation = BenchmarkScenarioObservation {
        completed_handoffs: 2,
        persisted_state_reads: 3,
    };

    let result = harness
        .evaluate_multi_agent_scenario(&scenario, &observation)
        .expect("scenario evaluation should succeed");

    assert_eq!(
        result,
        MultiAgentBenchmarkResult {
            scenario_id: "scenario-shared-runtime-001".to_owned(),
            handoff_completion_rate_percent: 100,
            shared_state_persistence_confirmed: true,
        }
    );
}

#[test]
fn benchmark_summary_defaults_to_zero_for_empty_comparisons() {
    let summary = BenchmarkComparisonSummary::from_comparisons(&[]);

    assert_eq!(summary.fixture_count, 0);
    assert_eq!(summary.graph_export_valid_rate_percent, 0);
    assert_eq!(summary.graph_audit_coverage_rate_percent, 0);
    assert_eq!(summary.graph_snapshot_reproducibility_rate_percent, 0);
}

#[test]
fn evaluate_multi_agent_scenario_rejects_invalid_inputs() {
    let harness = BenchmarkHarness::default();
    let invalid_scenario_id = MultiAgentBenchmarkScenario {
        scenario_id: " ".to_owned(),
        workspace_id: WorkspaceId::new("workspace--benchmark")
            .expect("workspace id should be valid"),
        agent_ids: vec![ActorId::new("agent--planner").expect("actor id should be valid")],
        expected_handoffs: 1,
    };

    let error = harness
        .evaluate_multi_agent_scenario(
            &invalid_scenario_id,
            &BenchmarkScenarioObservation {
                completed_handoffs: 1,
                persisted_state_reads: 1,
            },
        )
        .expect_err("blank scenario_id should be rejected");
    assert_eq!(
        error,
        RuntimeError::InvalidMultiAgentBenchmarkScenario("scenario_id")
    );

    let invalid_agent_ids = MultiAgentBenchmarkScenario {
        scenario_id: "scenario-2".to_owned(),
        workspace_id: WorkspaceId::new("workspace--benchmark-2")
            .expect("workspace id should be valid"),
        agent_ids: vec![],
        expected_handoffs: 1,
    };
    let error = harness
        .evaluate_multi_agent_scenario(
            &invalid_agent_ids,
            &BenchmarkScenarioObservation {
                completed_handoffs: 1,
                persisted_state_reads: 1,
            },
        )
        .expect_err("empty agent list should be rejected");
    assert_eq!(
        error,
        RuntimeError::InvalidMultiAgentBenchmarkScenario("agent_ids")
    );

    let invalid_expected_handoffs = MultiAgentBenchmarkScenario {
        scenario_id: "scenario-3".to_owned(),
        workspace_id: WorkspaceId::new("workspace--benchmark-4")
            .expect("workspace id should be valid"),
        agent_ids: vec![ActorId::new("agent--planner").expect("actor id should be valid")],
        expected_handoffs: 0,
    };
    let error = harness
        .evaluate_multi_agent_scenario(
            &invalid_expected_handoffs,
            &BenchmarkScenarioObservation {
                completed_handoffs: 0,
                persisted_state_reads: 0,
            },
        )
        .expect_err("zero expected handoffs should be rejected");
    assert_eq!(
        error,
        RuntimeError::InvalidMultiAgentBenchmarkScenario("expected_handoffs")
    );
}

#[test]
fn evaluate_multi_agent_scenario_clamps_completion_and_requires_persistence_reads() {
    let harness = BenchmarkHarness::default();
    let scenario = MultiAgentBenchmarkScenario {
        scenario_id: "scenario-over-complete".to_owned(),
        workspace_id: WorkspaceId::new("workspace--benchmark-3")
            .expect("workspace id should be valid"),
        agent_ids: vec![ActorId::new("agent--planner").expect("actor id should be valid")],
        expected_handoffs: 2,
    };

    let result = harness
        .evaluate_multi_agent_scenario(
            &scenario,
            &BenchmarkScenarioObservation {
                completed_handoffs: 9,
                persisted_state_reads: 0,
            },
        )
        .expect("over-complete observation should still be handled deterministically");

    assert_eq!(result.handoff_completion_rate_percent, 100);
    assert!(!result.shared_state_persistence_confirmed);
}

#[test]
fn benchmark_runners_reject_missing_fixture_and_workflow_inputs() {
    let baseline_runner = FlatJsonBaselineRunner::default();
    let graph_runner = GraphWorkflowRunner::default();
    let metrics = BenchmarkObservedMetrics {
        token_usage: 1,
        context_size: 1,
        correction_iterations: 0,
        export_valid: true,
        snapshot_reproducible: true,
        audit_coverage: true,
        invalid_write_rejections: 0,
    };

    let mut invalid_fixture_id = fixture("fixture-invalid-id");
    invalid_fixture_id.fixture_id = " ".to_owned();
    let error = baseline_runner
        .run_fixture(&invalid_fixture_id, metrics.clone())
        .expect_err("blank fixture id should be rejected");
    assert_eq!(error, RuntimeError::InvalidBenchmarkRun("fixture_id"));

    let mut invalid_baseline_workflow = fixture("fixture-invalid-baseline-workflow");
    invalid_baseline_workflow.baseline.workflow_name = " ".to_owned();
    let error = baseline_runner
        .run_fixture(&invalid_baseline_workflow, metrics.clone())
        .expect_err("blank baseline workflow name should be rejected");
    assert_eq!(
        error,
        RuntimeError::InvalidBenchmarkRun("baseline.workflow_name")
    );

    let mut invalid_baseline_steps = fixture("fixture-invalid-baseline-steps");
    invalid_baseline_steps.baseline.steps.clear();
    let error = baseline_runner
        .run_fixture(&invalid_baseline_steps, metrics.clone())
        .expect_err("empty baseline steps should be rejected");
    assert_eq!(error, RuntimeError::InvalidBenchmarkRun("baseline.steps"));

    let mut invalid_graph_workflow = fixture("fixture-invalid-graph-workflow");
    invalid_graph_workflow.graph.workflow_name = " ".to_owned();
    let error = graph_runner
        .run_fixture(&invalid_graph_workflow, metrics.clone())
        .expect_err("blank graph workflow name should be rejected");
    assert_eq!(
        error,
        RuntimeError::InvalidBenchmarkRun("graph.workflow_name")
    );

    let mut invalid_graph_steps = fixture("fixture-invalid-graph-steps");
    invalid_graph_steps.graph.steps.clear();
    let error = graph_runner
        .run_fixture(&invalid_graph_steps, metrics)
        .expect_err("empty graph steps should be rejected");
    assert_eq!(error, RuntimeError::InvalidBenchmarkRun("graph.steps"));
}

#[test]
fn benchmark_harness_reports_delta_overflow_for_extreme_metric_values() {
    let harness = BenchmarkHarness::default();
    let fixture_id = "fixture-overflow".to_owned();

    let baseline = shared_runtime::BenchmarkRun {
        fixture_id: fixture_id.clone(),
        workflow_name: "baseline".to_owned(),
        metrics: BenchmarkObservedMetrics {
            token_usage: 0,
            context_size: 0,
            correction_iterations: 0,
            export_valid: true,
            snapshot_reproducible: true,
            audit_coverage: true,
            invalid_write_rejections: 0,
        },
    };

    let mut graph = baseline.clone();
    graph.workflow_name = "graph".to_owned();

    graph.metrics.token_usage = u64::MAX;
    let error = harness
        .compare_fixture_runs(&baseline, &graph)
        .expect_err("token usage delta overflow should be rejected");
    assert_eq!(
        error,
        RuntimeError::InvalidBenchmarkRun("token_usage_delta")
    );

    graph.metrics.token_usage = 0;
    graph.metrics.context_size = u64::MAX;
    let error = harness
        .compare_fixture_runs(&baseline, &graph)
        .expect_err("context size delta overflow should be rejected");
    assert_eq!(
        error,
        RuntimeError::InvalidBenchmarkRun("context_size_delta")
    );

    graph.metrics.context_size = 0;
    graph.metrics.correction_iterations = u32::MAX;
    let error = harness
        .compare_fixture_runs(&baseline, &graph)
        .expect_err("correction iteration delta overflow should be rejected");
    assert_eq!(
        error,
        RuntimeError::InvalidBenchmarkRun("correction_iterations_delta")
    );
}

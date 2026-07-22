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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Benchmark workflow input.
pub struct BenchmarkWorkflowInput {
    /// Workflow name.
    pub workflow_name: String,
    /// Steps.
    pub steps: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Benchmark expectation.
pub struct BenchmarkExpectation {
    /// Requires export validation.
    pub requires_export_validation: bool,
    /// Requires audit coverage.
    pub requires_audit_coverage: bool,
    /// Requires snapshot reproducibility.
    pub requires_snapshot_reproducibility: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Benchmark fixture.
pub struct BenchmarkFixture {
    /// Fixture id.
    pub fixture_id: String,
    /// Description.
    pub description: String,
    /// Baseline.
    pub baseline: BenchmarkWorkflowInput,
    /// Graph.
    pub graph: BenchmarkWorkflowInput,
    /// Expected.
    pub expected: BenchmarkExpectation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Benchmark corpus.
pub struct BenchmarkCorpus {
    /// Schema version.
    pub schema_version: String,
    /// Fixtures.
    pub fixtures: Vec<BenchmarkFixture>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Benchmark observed metrics.
pub struct BenchmarkObservedMetrics {
    /// Token usage.
    pub token_usage: u64,
    /// Context size.
    pub context_size: u64,
    /// Correction iterations.
    pub correction_iterations: u32,
    /// Export valid.
    pub export_valid: bool,
    /// Snapshot reproducible.
    pub snapshot_reproducible: bool,
    /// Audit coverage.
    pub audit_coverage: bool,
    /// Invalid write rejections.
    pub invalid_write_rejections: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Benchmark run.
pub struct BenchmarkRun {
    /// Fixture id.
    pub fixture_id: String,
    /// Workflow name.
    pub workflow_name: String,
    /// Metrics.
    pub metrics: BenchmarkObservedMetrics,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Benchmark fixture comparison.
pub struct BenchmarkFixtureComparison {
    /// Fixture id.
    pub fixture_id: String,
    /// Token usage delta.
    pub token_usage_delta: i64,
    /// Context size delta.
    pub context_size_delta: i64,
    /// Correction iterations delta.
    pub correction_iterations_delta: i32,
    /// Graph export valid.
    pub graph_export_valid: bool,
    /// Graph audit coverage.
    pub graph_audit_coverage: bool,
    /// Graph snapshot reproducible.
    pub graph_snapshot_reproducible: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
/// Benchmark comparison summary.
pub struct BenchmarkComparisonSummary {
    /// Fixture count.
    pub fixture_count: usize,
    /// Graph export valid rate percent.
    pub graph_export_valid_rate_percent: u8,
    /// Graph audit coverage rate percent.
    pub graph_audit_coverage_rate_percent: u8,
    /// Graph snapshot reproducibility rate percent.
    pub graph_snapshot_reproducibility_rate_percent: u8,
}

impl BenchmarkComparisonSummary {
    /// Creates an instance from comparisons.
    pub fn from_comparisons(comparisons: &[BenchmarkFixtureComparison]) -> Self {
        if comparisons.is_empty() {
            return Self::default();
        }

        let fixture_count = comparisons.len();
        let graph_export_valid_count = comparisons
            .iter()
            .filter(|comparison| comparison.graph_export_valid)
            .count();
        let graph_audit_coverage_count = comparisons
            .iter()
            .filter(|comparison| comparison.graph_audit_coverage)
            .count();
        let graph_snapshot_reproducibility_count = comparisons
            .iter()
            .filter(|comparison| comparison.graph_snapshot_reproducible)
            .count();

        Self {
            fixture_count,
            // Graph export valid rate percent.
            graph_export_valid_rate_percent: percentage(graph_export_valid_count, fixture_count),
            // Graph audit coverage rate percent.
            graph_audit_coverage_rate_percent: percentage(
                graph_audit_coverage_count,
                fixture_count,
            ),
            // Graph snapshot reproducibility rate percent.
            graph_snapshot_reproducibility_rate_percent: percentage(
                graph_snapshot_reproducibility_count,
                fixture_count,
            ),
        }
    }
}

#[derive(Clone, Debug, Default)]
/// Flat json baseline runner.
pub struct FlatJsonBaselineRunner;

impl FlatJsonBaselineRunner {
    /// Run fixture.
    pub fn run_fixture(
        &self,
        fixture: &BenchmarkFixture,
        metrics: BenchmarkObservedMetrics,
    ) -> Result<BenchmarkRun, RuntimeError> {
        if fixture.fixture_id.trim().is_empty() {
            return Err(RuntimeError::InvalidBenchmarkRun("fixture_id"));
        }

        if fixture.baseline.workflow_name.trim().is_empty() {
            return Err(RuntimeError::InvalidBenchmarkRun("baseline.workflow_name"));
        }

        if fixture.baseline.steps.is_empty() {
            return Err(RuntimeError::InvalidBenchmarkRun("baseline.steps"));
        }

        Ok(BenchmarkRun {
            fixture_id: fixture.fixture_id.clone(),
            workflow_name: fixture.baseline.workflow_name.clone(),
            metrics,
        })
    }
}

#[derive(Clone, Debug, Default)]
/// Graph workflow runner.
pub struct GraphWorkflowRunner;

impl GraphWorkflowRunner {
    /// Run fixture.
    pub fn run_fixture(
        &self,
        fixture: &BenchmarkFixture,
        metrics: BenchmarkObservedMetrics,
    ) -> Result<BenchmarkRun, RuntimeError> {
        if fixture.fixture_id.trim().is_empty() {
            return Err(RuntimeError::InvalidBenchmarkRun("fixture_id"));
        }

        if fixture.graph.workflow_name.trim().is_empty() {
            return Err(RuntimeError::InvalidBenchmarkRun("graph.workflow_name"));
        }

        if fixture.graph.steps.is_empty() {
            return Err(RuntimeError::InvalidBenchmarkRun("graph.steps"));
        }

        Ok(BenchmarkRun {
            fixture_id: fixture.fixture_id.clone(),
            workflow_name: fixture.graph.workflow_name.clone(),
            metrics,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Multi agent benchmark scenario.
pub struct MultiAgentBenchmarkScenario {
    /// Scenario id.
    pub scenario_id: String,
    /// Workspace id.
    pub workspace_id: WorkspaceId,
    /// Agent ids.
    pub agent_ids: Vec<ActorId>,
    /// Expected handoffs.
    pub expected_handoffs: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Benchmark scenario observation.
pub struct BenchmarkScenarioObservation {
    /// Completed handoffs.
    pub completed_handoffs: u32,
    /// Persisted state reads.
    pub persisted_state_reads: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Multi agent benchmark result.
pub struct MultiAgentBenchmarkResult {
    /// Scenario id.
    pub scenario_id: String,
    /// Handoff completion rate percent.
    pub handoff_completion_rate_percent: u8,
    /// Shared state persistence confirmed.
    pub shared_state_persistence_confirmed: bool,
}

#[derive(Clone, Debug, Default)]
/// Benchmark harness.
pub struct BenchmarkHarness;

impl BenchmarkHarness {
    /// Compare fixture runs.
    pub fn compare_fixture_runs(
        &self,
        baseline: &BenchmarkRun,
        graph: &BenchmarkRun,
    ) -> Result<BenchmarkFixtureComparison, RuntimeError> {
        if baseline.fixture_id != graph.fixture_id {
            return Err(RuntimeError::BenchmarkFixtureMismatch {
                baseline_fixture_id: baseline.fixture_id.clone(),
                graph_fixture_id: graph.fixture_id.clone(),
            });
        }

        let token_usage_delta = i64::try_from(
            i128::from(graph.metrics.token_usage) - i128::from(baseline.metrics.token_usage),
        )
        .map_err(|_| RuntimeError::InvalidBenchmarkRun("token_usage_delta"))?;

        let context_size_delta = i64::try_from(
            i128::from(graph.metrics.context_size) - i128::from(baseline.metrics.context_size),
        )
        .map_err(|_| RuntimeError::InvalidBenchmarkRun("context_size_delta"))?;

        let correction_iterations_delta = i32::try_from(
            i64::from(graph.metrics.correction_iterations)
                - i64::from(baseline.metrics.correction_iterations),
        )
        .map_err(|_| RuntimeError::InvalidBenchmarkRun("correction_iterations_delta"))?;

        Ok(BenchmarkFixtureComparison {
            fixture_id: baseline.fixture_id.clone(),
            token_usage_delta,
            context_size_delta,
            correction_iterations_delta,
            graph_export_valid: graph.metrics.export_valid,
            graph_audit_coverage: graph.metrics.audit_coverage,
            graph_snapshot_reproducible: graph.metrics.snapshot_reproducible,
        })
    }

    /// Evaluate multi agent scenario.
    pub fn evaluate_multi_agent_scenario(
        &self,
        scenario: &MultiAgentBenchmarkScenario,
        observation: &BenchmarkScenarioObservation,
    ) -> Result<MultiAgentBenchmarkResult, RuntimeError> {
        if scenario.scenario_id.trim().is_empty() {
            return Err(RuntimeError::InvalidMultiAgentBenchmarkScenario(
                "scenario_id",
            ));
        }

        if scenario.agent_ids.is_empty() {
            return Err(RuntimeError::InvalidMultiAgentBenchmarkScenario(
                "agent_ids",
            ));
        }

        if scenario.expected_handoffs == 0 {
            return Err(RuntimeError::InvalidMultiAgentBenchmarkScenario(
                "expected_handoffs",
            ));
        }

        let completion_basis = usize::try_from(scenario.expected_handoffs)
            .map_err(|_| RuntimeError::InvalidMultiAgentBenchmarkScenario("expected_handoffs"))?;
        let completed_basis = usize::try_from(observation.completed_handoffs)
            .map_err(|_| RuntimeError::InvalidMultiAgentBenchmarkScenario("completed_handoffs"))?;

        let completion_rate = percentage(completed_basis.min(completion_basis), completion_basis);
        let shared_state_persistence_confirmed = observation.completed_handoffs
            >= scenario.expected_handoffs
            && observation.persisted_state_reads > 0;

        Ok(MultiAgentBenchmarkResult {
            scenario_id: scenario.scenario_id.clone(),
            handoff_completion_rate_percent: completion_rate,
            shared_state_persistence_confirmed,
        })
    }
}

pub(crate) fn percentage(numerator: usize, denominator: usize) -> u8 {
    if denominator == 0 {
        return 0;
    }

    let value = (numerator.saturating_mul(100)) / denominator;
    u8::try_from(value).unwrap_or(100)
}

impl BenchmarkCorpus {
    // Validation keeps benchmark fixtures deterministic and machine-checkable
    // before they are consumed by baseline/graph runners.
    /// Validate.
    pub fn validate(&self) -> Result<(), RuntimeError> {
        if self.schema_version.trim().is_empty() {
            return Err(RuntimeError::InvalidBenchmarkCorpus("schema_version"));
        }

        if self.fixtures.is_empty() {
            return Err(RuntimeError::InvalidBenchmarkCorpus("fixtures"));
        }

        let mut fixture_ids = HashSet::new();
        for fixture in &self.fixtures {
            if fixture.fixture_id.trim().is_empty() {
                return Err(RuntimeError::InvalidBenchmarkCorpus("fixture_id"));
            }

            if !fixture_ids.insert(fixture.fixture_id.clone()) {
                return Err(RuntimeError::DuplicateBenchmarkFixtureId(
                    fixture.fixture_id.clone(),
                ));
            }

            if fixture.description.trim().is_empty() {
                return Err(RuntimeError::InvalidBenchmarkCorpus("description"));
            }

            if fixture.baseline.workflow_name.trim().is_empty() {
                return Err(RuntimeError::InvalidBenchmarkCorpus(
                    "baseline.workflow_name",
                ));
            }

            if fixture.baseline.steps.is_empty() {
                return Err(RuntimeError::InvalidBenchmarkCorpus("baseline.steps"));
            }

            if fixture.graph.workflow_name.trim().is_empty() {
                return Err(RuntimeError::InvalidBenchmarkCorpus("graph.workflow_name"));
            }

            if fixture.graph.steps.is_empty() {
                return Err(RuntimeError::InvalidBenchmarkCorpus("graph.steps"));
            }
        }

        Ok(())
    }
}

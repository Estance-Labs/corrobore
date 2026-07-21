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
use cypher_parser::NormalizedThreshold;
use cypher_planner::{InvestigationPhysicalPlan, InvestigationPhysicalStage, PhysicalStageKind};
use serde::Serialize;
use thiserror::Error;

/// Resource consumption measured by the engine during an investigation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InvestigationResourceUsage {
    /// Peak resident investigation memory in bytes.
    pub memory_bytes: u64,
    /// Elapsed execution time in milliseconds.
    pub elapsed_millis: u64,
    /// Completed external retrieval operations.
    pub external_retrievals: u32,
}

/// Typed facts observed while executing an investigation plan.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InvestigationExecutionObservation {
    /// Temporal snapshot actually resolved by the engine.
    pub resolved_at_time: Option<String>,
    /// Number of mutually independent supporting sources.
    pub independent_sources: u32,
    /// Lowest reliability among the sources used by the result.
    pub minimum_source_reliability: Option<NormalizedThreshold>,
    /// Measured evidence completeness.
    pub evidence_completeness: Option<NormalizedThreshold>,
    /// Whether the result contains hypotheses.
    pub contains_hypotheses: bool,
    /// Whether the result contains contradictory evidence.
    pub contains_contradictory_evidence: bool,
    /// Resources consumed by execution.
    pub resources: InvestigationResourceUsage,
}

/// Declarative contract evaluated by the executor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum InvestigationContractKind {
    /// Requested temporal snapshot.
    TemporalSnapshot,
    /// Minimum independent-source count.
    IndependentSources,
    /// Minimum source reliability.
    SourceReliability,
    /// Maximum memory.
    MemoryBudget,
    /// Maximum latency.
    LatencyBudget,
    /// Maximum external retrieval count.
    ExternalRetrievalBudget,
    /// Hypothesis allowance.
    HypothesesAllowance,
    /// Contradictory-evidence allowance.
    ContradictoryEvidenceAllowance,
    /// Minimum evidence completeness.
    EvidenceCompleteness,
}

/// Result of evaluating one contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum ContractOutcomeStatus {
    /// Observed execution satisfies the contract.
    Satisfied,
    /// Observed execution violates the contract.
    Violated,
}

/// Auditable evaluation of one contract.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct InvestigationContractOutcome {
    /// Contract that was evaluated.
    pub contract: InvestigationContractKind,
    /// Evaluation result.
    pub status: ContractOutcomeStatus,
    /// Canonical expected value.
    pub expected: String,
    /// Canonical observed value.
    pub observed: String,
}

/// Stable typed reason for one contract violation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvestigationContractViolationCode {
    /// Requested temporal snapshot was not available.
    TemporalSnapshotUnavailable,
    /// Engine resolved a different temporal snapshot.
    TemporalSnapshotMismatch,
    /// Too few independent sources were observed.
    IndependentSourcesUnsatisfied,
    /// Source reliability was absent or below the threshold.
    SourceReliabilityUnsatisfied,
    /// Memory consumption exceeded the bound.
    MemoryBudgetExceeded,
    /// Elapsed time exceeded the bound.
    LatencyBudgetExceeded,
    /// External retrieval count exceeded the bound.
    ExternalRetrievalBudgetExceeded,
    /// Hypotheses were present without permission.
    HypothesesNotAllowed,
    /// Contradictory evidence was present without permission.
    ContradictoryEvidenceNotAllowed,
    /// Evidence completeness was absent or below the threshold.
    EvidenceCompletenessUnsatisfied,
}

/// Typed contract violation with expected and observed evidence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvestigationContractViolation {
    /// Stable violation category.
    pub code: InvestigationContractViolationCode,
    /// Contract that failed.
    pub contract: InvestigationContractKind,
    /// Human-readable failure description.
    pub message: String,
}

/// Deterministic reason execution cannot continue successfully.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum InvestigationStopReason {
    /// Temporal snapshot could not be honored.
    TemporalSnapshotUnavailable,
    /// One or more hard evidence requirements failed.
    EvidenceRequirementUnsatisfied,
    /// A disallowed evidence category was present.
    AllowanceViolation,
    /// Memory budget was exceeded.
    MemoryBudgetExceeded,
    /// Latency budget was exceeded.
    LatencyBudgetExceeded,
    /// External retrieval budget was exceeded.
    ExternalRetrievalBudgetExceeded,
}

/// Successful proof that all applicable contracts were evaluated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvestigationContractReport {
    /// Per-contract audit outcomes.
    pub outcomes: Vec<InvestigationContractOutcome>,
    /// Every physical stage inspected by the executor in order.
    pub evaluated_stage_kinds: Vec<PhysicalStageKind>,
}

impl InvestigationContractReport {
    /// Serializes the report using stable contract and stage names.
    pub fn to_canonical_string(&self) -> String {
        let stages = self
            .evaluated_stage_kinds
            .iter()
            .map(|kind| kind.canonical_name())
            .collect::<Vec<_>>()
            .join(",");
        let outcomes = self
            .outcomes
            .iter()
            .map(canonical_outcome)
            .collect::<Vec<_>>()
            .join(",");
        format!("stages=[{stages}];outcomes=[{outcomes}]")
    }
}

/// Failed enforcement result retaining all deterministic audit outcomes.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("investigation contract enforcement failed")]
pub struct InvestigationContractError {
    /// Violations in physical execution order.
    pub violations: Vec<InvestigationContractViolation>,
    /// Outcomes for every contract that was evaluated.
    pub outcomes: Vec<InvestigationContractOutcome>,
    /// Deterministic first stop reason.
    pub stop_reason: InvestigationStopReason,
}

impl InvestigationContractError {
    /// Serializes violations, outcomes, and stop reason canonically.
    pub fn to_canonical_string(&self) -> String {
        let violations = self
            .violations
            .iter()
            .map(|violation| {
                format!(
                    "{}:{}",
                    violation_code_name(violation.code),
                    contract_name(violation.contract)
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let outcomes = self
            .outcomes
            .iter()
            .map(canonical_outcome)
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "stop={};violations=[{violations}];outcomes=[{outcomes}]",
            stop_reason_name(self.stop_reason)
        )
    }
}

/// Enforces all contracts represented by an investigation physical plan.
///
/// The implementation will inspect stages in execution order, use safe-deny
/// defaults for undeclared allowances, retain all outcomes and violations, and
/// choose the first violated stage as the deterministic stop reason.
pub fn enforce_investigation_contracts(
    plan: &InvestigationPhysicalPlan,
    observation: &InvestigationExecutionObservation,
) -> Result<InvestigationContractReport, InvestigationContractError> {
    let evaluated_stage_kinds = plan.stage_kinds();
    let mut outcomes = Vec::new();
    let mut violations = Vec::new();
    let mut stop_reason = None;

    for stage in &plan.stages {
        match stage {
            InvestigationPhysicalStage::TemporalFilter { at_time } => {
                let satisfied = observation.resolved_at_time.as_deref() == Some(at_time.as_str());
                push_outcome(
                    &mut outcomes,
                    InvestigationContractKind::TemporalSnapshot,
                    satisfied,
                    at_time.clone(),
                    observation
                        .resolved_at_time
                        .clone()
                        .unwrap_or_else(|| "unavailable".to_owned()),
                );
                if !satisfied {
                    let code = if observation.resolved_at_time.is_some() {
                        InvestigationContractViolationCode::TemporalSnapshotMismatch
                    } else {
                        InvestigationContractViolationCode::TemporalSnapshotUnavailable
                    };
                    push_violation(
                        &mut violations,
                        &mut stop_reason,
                        code,
                        InvestigationContractKind::TemporalSnapshot,
                        InvestigationStopReason::TemporalSnapshotUnavailable,
                        "the requested temporal snapshot was not resolved",
                    );
                }
            }
            InvestigationPhysicalStage::IndependentSourceFilter { minimum } => {
                let satisfied = observation.independent_sources >= *minimum;
                push_outcome(
                    &mut outcomes,
                    InvestigationContractKind::IndependentSources,
                    satisfied,
                    format!(">={minimum}"),
                    observation.independent_sources.to_string(),
                );
                if !satisfied {
                    push_violation(
                        &mut violations,
                        &mut stop_reason,
                        InvestigationContractViolationCode::IndependentSourcesUnsatisfied,
                        InvestigationContractKind::IndependentSources,
                        InvestigationStopReason::EvidenceRequirementUnsatisfied,
                        "independent-source count is below the required minimum",
                    );
                }
            }
            InvestigationPhysicalStage::SourceReliabilityFilter {
                minimum_parts_per_million,
            } => {
                let observed = observation
                    .minimum_source_reliability
                    .map(NormalizedThreshold::parts_per_million);
                let satisfied = observed.is_some_and(|value| value >= *minimum_parts_per_million);
                push_outcome(
                    &mut outcomes,
                    InvestigationContractKind::SourceReliability,
                    satisfied,
                    format!(">={minimum_parts_per_million}ppm"),
                    optional_number(observed, "unavailable"),
                );
                if !satisfied {
                    push_violation(
                        &mut violations,
                        &mut stop_reason,
                        InvestigationContractViolationCode::SourceReliabilityUnsatisfied,
                        InvestigationContractKind::SourceReliability,
                        InvestigationStopReason::EvidenceRequirementUnsatisfied,
                        "source reliability is unavailable or below the required minimum",
                    );
                }
            }
            InvestigationPhysicalStage::BudgetGuard {
                memory_bytes,
                latency_millis,
                external_retrievals,
            } => {
                if let Some(maximum) = memory_bytes {
                    let satisfied = observation.resources.memory_bytes <= *maximum;
                    push_outcome(
                        &mut outcomes,
                        InvestigationContractKind::MemoryBudget,
                        satisfied,
                        format!("<={maximum}B"),
                        format!("{}B", observation.resources.memory_bytes),
                    );
                    if !satisfied {
                        push_violation(
                            &mut violations,
                            &mut stop_reason,
                            InvestigationContractViolationCode::MemoryBudgetExceeded,
                            InvestigationContractKind::MemoryBudget,
                            InvestigationStopReason::MemoryBudgetExceeded,
                            "memory consumption exceeded the declared budget",
                        );
                    }
                }
                if let Some(maximum) = latency_millis {
                    let satisfied = observation.resources.elapsed_millis <= *maximum;
                    push_outcome(
                        &mut outcomes,
                        InvestigationContractKind::LatencyBudget,
                        satisfied,
                        format!("<={maximum}ms"),
                        format!("{}ms", observation.resources.elapsed_millis),
                    );
                    if !satisfied {
                        push_violation(
                            &mut violations,
                            &mut stop_reason,
                            InvestigationContractViolationCode::LatencyBudgetExceeded,
                            InvestigationContractKind::LatencyBudget,
                            InvestigationStopReason::LatencyBudgetExceeded,
                            "elapsed time exceeded the declared budget",
                        );
                    }
                }
                if let Some(maximum) = external_retrievals {
                    let satisfied = observation.resources.external_retrievals <= *maximum;
                    push_outcome(
                        &mut outcomes,
                        InvestigationContractKind::ExternalRetrievalBudget,
                        satisfied,
                        format!("<={maximum}"),
                        observation.resources.external_retrievals.to_string(),
                    );
                    if !satisfied {
                        push_violation(
                            &mut violations,
                            &mut stop_reason,
                            InvestigationContractViolationCode::ExternalRetrievalBudgetExceeded,
                            InvestigationContractKind::ExternalRetrievalBudget,
                            InvestigationStopReason::ExternalRetrievalBudgetExceeded,
                            "external retrieval count exceeded the declared budget",
                        );
                    }
                }
            }
            InvestigationPhysicalStage::EvidenceArbitration {
                allow_hypotheses,
                allow_contradictory_evidence,
            } => {
                let hypotheses_allowed = allow_hypotheses.unwrap_or(false);
                let hypotheses_satisfied = hypotheses_allowed || !observation.contains_hypotheses;
                push_outcome(
                    &mut outcomes,
                    InvestigationContractKind::HypothesesAllowance,
                    hypotheses_satisfied,
                    format!("allowed={hypotheses_allowed}"),
                    format!("present={}", observation.contains_hypotheses),
                );
                if !hypotheses_satisfied {
                    push_violation(
                        &mut violations,
                        &mut stop_reason,
                        InvestigationContractViolationCode::HypothesesNotAllowed,
                        InvestigationContractKind::HypothesesAllowance,
                        InvestigationStopReason::AllowanceViolation,
                        "hypotheses were present without an explicit allowance",
                    );
                }

                let contradictory_allowed = allow_contradictory_evidence.unwrap_or(false);
                let contradictory_satisfied =
                    contradictory_allowed || !observation.contains_contradictory_evidence;
                push_outcome(
                    &mut outcomes,
                    InvestigationContractKind::ContradictoryEvidenceAllowance,
                    contradictory_satisfied,
                    format!("allowed={contradictory_allowed}"),
                    format!("present={}", observation.contains_contradictory_evidence),
                );
                if !contradictory_satisfied {
                    push_violation(
                        &mut violations,
                        &mut stop_reason,
                        InvestigationContractViolationCode::ContradictoryEvidenceNotAllowed,
                        InvestigationContractKind::ContradictoryEvidenceAllowance,
                        InvestigationStopReason::AllowanceViolation,
                        "contradictory evidence was present without an explicit allowance",
                    );
                }
            }
            InvestigationPhysicalStage::CompletenessVerification {
                minimum_parts_per_million: Some(minimum),
            } => {
                let observed = observation
                    .evidence_completeness
                    .map(NormalizedThreshold::parts_per_million);
                let satisfied = observed.is_some_and(|value| value >= *minimum);
                push_outcome(
                    &mut outcomes,
                    InvestigationContractKind::EvidenceCompleteness,
                    satisfied,
                    format!(">={minimum}ppm"),
                    optional_number(observed, "unavailable"),
                );
                if !satisfied {
                    push_violation(
                        &mut violations,
                        &mut stop_reason,
                        InvestigationContractViolationCode::EvidenceCompletenessUnsatisfied,
                        InvestigationContractKind::EvidenceCompleteness,
                        InvestigationStopReason::EvidenceRequirementUnsatisfied,
                        "evidence completeness is unavailable or below the required minimum",
                    );
                }
            }
            InvestigationPhysicalStage::SeedSelection { .. }
            | InvestigationPhysicalStage::WorkingSetConstruction
            | InvestigationPhysicalStage::EvidenceTraversal { .. }
            | InvestigationPhysicalStage::CompletenessVerification {
                minimum_parts_per_million: None,
            }
            | InvestigationPhysicalStage::ResponseProjection { .. } => {}
        }
    }

    match stop_reason {
        None => Ok(InvestigationContractReport {
            outcomes,
            evaluated_stage_kinds,
        }),
        Some(stop_reason) => Err(InvestigationContractError {
            violations,
            outcomes,
            stop_reason,
        }),
    }
}

fn push_outcome(
    outcomes: &mut Vec<InvestigationContractOutcome>,
    contract: InvestigationContractKind,
    satisfied: bool,
    expected: String,
    observed: String,
) {
    outcomes.push(InvestigationContractOutcome {
        contract,
        status: if satisfied {
            ContractOutcomeStatus::Satisfied
        } else {
            ContractOutcomeStatus::Violated
        },
        expected,
        observed,
    });
}

fn push_violation(
    violations: &mut Vec<InvestigationContractViolation>,
    first_stop_reason: &mut Option<InvestigationStopReason>,
    code: InvestigationContractViolationCode,
    contract: InvestigationContractKind,
    stop_reason: InvestigationStopReason,
    message: &str,
) {
    if first_stop_reason.is_none() {
        *first_stop_reason = Some(stop_reason);
    }
    violations.push(InvestigationContractViolation {
        code,
        contract,
        message: message.to_owned(),
    });
}

fn canonical_outcome(outcome: &InvestigationContractOutcome) -> String {
    format!(
        "{}:{}:{}:{}",
        contract_name(outcome.contract),
        outcome_status_name(outcome.status),
        outcome.expected,
        outcome.observed
    )
}

fn contract_name(contract: InvestigationContractKind) -> &'static str {
    match contract {
        InvestigationContractKind::TemporalSnapshot => "temporal_snapshot",
        InvestigationContractKind::IndependentSources => "independent_sources",
        InvestigationContractKind::SourceReliability => "source_reliability",
        InvestigationContractKind::MemoryBudget => "memory_budget",
        InvestigationContractKind::LatencyBudget => "latency_budget",
        InvestigationContractKind::ExternalRetrievalBudget => "external_retrieval_budget",
        InvestigationContractKind::HypothesesAllowance => "hypotheses_allowance",
        InvestigationContractKind::ContradictoryEvidenceAllowance => {
            "contradictory_evidence_allowance"
        }
        InvestigationContractKind::EvidenceCompleteness => "evidence_completeness",
    }
}

fn outcome_status_name(status: ContractOutcomeStatus) -> &'static str {
    match status {
        ContractOutcomeStatus::Satisfied => "satisfied",
        ContractOutcomeStatus::Violated => "violated",
    }
}

fn violation_code_name(code: InvestigationContractViolationCode) -> &'static str {
    match code {
        InvestigationContractViolationCode::TemporalSnapshotUnavailable => {
            "temporal_snapshot_unavailable"
        }
        InvestigationContractViolationCode::TemporalSnapshotMismatch => {
            "temporal_snapshot_mismatch"
        }
        InvestigationContractViolationCode::IndependentSourcesUnsatisfied => {
            "independent_sources_unsatisfied"
        }
        InvestigationContractViolationCode::SourceReliabilityUnsatisfied => {
            "source_reliability_unsatisfied"
        }
        InvestigationContractViolationCode::MemoryBudgetExceeded => "memory_budget_exceeded",
        InvestigationContractViolationCode::LatencyBudgetExceeded => "latency_budget_exceeded",
        InvestigationContractViolationCode::ExternalRetrievalBudgetExceeded => {
            "external_retrieval_budget_exceeded"
        }
        InvestigationContractViolationCode::HypothesesNotAllowed => "hypotheses_not_allowed",
        InvestigationContractViolationCode::ContradictoryEvidenceNotAllowed => {
            "contradictory_evidence_not_allowed"
        }
        InvestigationContractViolationCode::EvidenceCompletenessUnsatisfied => {
            "evidence_completeness_unsatisfied"
        }
    }
}

fn stop_reason_name(reason: InvestigationStopReason) -> &'static str {
    match reason {
        InvestigationStopReason::TemporalSnapshotUnavailable => "temporal_snapshot_unavailable",
        InvestigationStopReason::EvidenceRequirementUnsatisfied => {
            "evidence_requirement_unsatisfied"
        }
        InvestigationStopReason::AllowanceViolation => "allowance_violation",
        InvestigationStopReason::MemoryBudgetExceeded => "memory_budget_exceeded",
        InvestigationStopReason::LatencyBudgetExceeded => "latency_budget_exceeded",
        InvestigationStopReason::ExternalRetrievalBudgetExceeded => {
            "external_retrieval_budget_exceeded"
        }
    }
}

fn optional_number<T: ToString>(value: Option<T>, unavailable: &str) -> String {
    value.map_or_else(|| unavailable.to_owned(), |number| number.to_string())
}

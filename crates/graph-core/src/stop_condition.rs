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
//! Deterministic budget-aware stop-condition policy for active investigations.
//!
//! This module evaluates whether investigation should stop or continue using
//! calibrated confidence/completeness, unresolved counter-evidence, remaining
//! budget, and the highest-ranked eligible Next Best Evidence action.

use serde::{Deserialize, Serialize};

use crate::{
    Confidence, EvidenceSubgraph, GraphError, InvestigationAction, NextBestEvidenceProposalScope,
    NextBestEvidenceRanking, RetrievalCompleteness, UnresolvedUnknown,
};

/// Remaining investigation budget in the inclusive unit interval.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct StopConditionBudget(f64);

impl StopConditionBudget {
    /// Creates a validated normalized budget value.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::InvalidStopConditionBudget`] when `value` is not
    /// finite or lies outside the inclusive unit interval.
    pub fn new(value: f64) -> Result<Self, GraphError> {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(GraphError::InvalidStopConditionBudget(value));
        }

        Ok(Self(value))
    }

    /// Returns the normalized budget value.
    #[must_use]
    pub const fn value(self) -> f64 {
        self.0
    }
}

impl TryFrom<f64> for StopConditionBudget {
    type Error = GraphError;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<StopConditionBudget> for f64 {
    fn from(value: StopConditionBudget) -> Self {
        value.value()
    }
}

/// Policy thresholds used to decide whether investigation stops or continues.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "InvestigationStopThresholdsWire")]
pub struct InvestigationStopThresholds {
    minimum_confidence_to_stop: Confidence,
    minimum_completeness_to_stop: RetrievalCompleteness,
    minimum_expected_action_value_to_continue: f64,
    minimum_remaining_budget_to_continue: StopConditionBudget,
}

#[derive(Deserialize)]
struct InvestigationStopThresholdsWire {
    minimum_confidence_to_stop: Confidence,
    minimum_completeness_to_stop: RetrievalCompleteness,
    minimum_expected_action_value_to_continue: f64,
    minimum_remaining_budget_to_continue: StopConditionBudget,
}

impl InvestigationStopThresholds {
    /// Creates validated stop-condition thresholds.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::InvalidStopConditionPolicy`] when
    /// `minimum_expected_action_value_to_continue` is not finite.
    pub fn new(
        minimum_confidence_to_stop: Confidence,
        minimum_completeness_to_stop: RetrievalCompleteness,
        minimum_expected_action_value_to_continue: f64,
        minimum_remaining_budget_to_continue: StopConditionBudget,
    ) -> Result<Self, GraphError> {
        if !minimum_expected_action_value_to_continue.is_finite() {
            return Err(GraphError::InvalidStopConditionPolicy(
                "minimum expected action value must be finite".to_owned(),
            ));
        }

        Ok(Self {
            minimum_confidence_to_stop,
            minimum_completeness_to_stop,
            minimum_expected_action_value_to_continue,
            minimum_remaining_budget_to_continue,
        })
    }

    /// Returns the minimum confidence needed for an evidence-sufficient stop.
    #[must_use]
    pub const fn minimum_confidence_to_stop(self) -> Confidence {
        self.minimum_confidence_to_stop
    }

    /// Returns the minimum retrieval completeness for an evidence-sufficient stop.
    #[must_use]
    pub const fn minimum_completeness_to_stop(self) -> RetrievalCompleteness {
        self.minimum_completeness_to_stop
    }

    /// Returns the minimum action value required to continue investigation.
    #[must_use]
    pub const fn minimum_expected_action_value_to_continue(self) -> f64 {
        self.minimum_expected_action_value_to_continue
    }

    /// Returns the minimum remaining budget required to continue investigation.
    #[must_use]
    pub const fn minimum_remaining_budget_to_continue(self) -> StopConditionBudget {
        self.minimum_remaining_budget_to_continue
    }
}

impl TryFrom<InvestigationStopThresholdsWire> for InvestigationStopThresholds {
    type Error = GraphError;

    fn try_from(value: InvestigationStopThresholdsWire) -> Result<Self, Self::Error> {
        Self::new(
            value.minimum_confidence_to_stop,
            value.minimum_completeness_to_stop,
            value.minimum_expected_action_value_to_continue,
            value.minimum_remaining_budget_to_continue,
        )
    }
}

/// Typed reasons for stopping an investigation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InvestigationStopReason {
    /// Confidence/completeness thresholds are met and no material counter-evidence remains.
    EvidenceSufficient,
    /// Remaining eligible action value is below the continuation threshold.
    MarginalGainBelowThreshold,
    /// Remaining budget is below the continuation threshold.
    BudgetExhausted,
    /// Policy forbids autonomous continuation.
    PolicyRestricted,
}

/// Typed stop/continue decision for one policy evaluation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum InvestigationStopConditionDecision {
    /// Investigation should stop for the typed reason.
    Stop {
        /// Why investigation should stop.
        reason: InvestigationStopReason,
    },
    /// Investigation should continue with the selected next action.
    Continue {
        /// Stable identifier of the selected ranked candidate.
        selected_candidate_id: String,
        /// Selected next best evidence action.
        selected_action: InvestigationAction,
        /// Selected candidate expected value at evaluation time.
        selected_expected_value: f64,
    },
}

/// Auditable stop-condition evaluation surface attached to calibrated assessments.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InvestigationStopCondition {
    decision: InvestigationStopConditionDecision,
    thresholds: InvestigationStopThresholds,
    remaining_budget: StopConditionBudget,
    highest_eligible_action_value: Option<f64>,
    has_material_counter_evidence: bool,
}

impl InvestigationStopCondition {
    /// Returns the typed stop/continue decision.
    #[must_use]
    pub const fn decision(&self) -> &InvestigationStopConditionDecision {
        &self.decision
    }

    /// Returns the policy thresholds used by this evaluation.
    #[must_use]
    pub const fn thresholds(&self) -> InvestigationStopThresholds {
        self.thresholds
    }

    /// Returns the remaining budget used by this evaluation.
    #[must_use]
    pub const fn remaining_budget(&self) -> StopConditionBudget {
        self.remaining_budget
    }

    /// Returns the highest eligible action value considered by the policy.
    #[must_use]
    pub const fn highest_eligible_action_value(&self) -> Option<f64> {
        self.highest_eligible_action_value
    }

    /// Returns whether material counter-evidence remained unresolved.
    #[must_use]
    pub const fn has_material_counter_evidence(&self) -> bool {
        self.has_material_counter_evidence
    }
}

/// Evaluates whether an active investigation should stop or continue.
///
/// The policy is deterministic:
/// 1. Stop as evidence-sufficient only when confidence and completeness meet
///    thresholds and no material counter-evidence remains unresolved.
/// 2. Otherwise stop when remaining budget is below threshold.
/// 3. Otherwise continue only with an internal eligible ranked action whose
///    value meets the continuation threshold.
/// 4. Otherwise stop for a typed policy or marginal-gain reason.
#[allow(clippy::too_many_arguments)]
pub fn evaluate_investigation_stop_condition(
    current_confidence: Confidence,
    retrieval_completeness: RetrievalCompleteness,
    counter_evidence: &EvidenceSubgraph,
    unresolved_unknowns: &[UnresolvedUnknown],
    remaining_budget: StopConditionBudget,
    next_best_evidence: &NextBestEvidenceRanking,
    thresholds: InvestigationStopThresholds,
) -> Result<InvestigationStopCondition, GraphError> {
    let has_material_counter_evidence = !counter_evidence.is_empty()
        || unresolved_unknowns
            .iter()
            .any(|unknown| matches!(unknown, UnresolvedUnknown::UnresolvedContradiction { .. }));

    let selected = next_best_evidence.selected();
    let highest_eligible_action_value =
        selected.map(|candidate| candidate.score_breakdown().expected_value());

    let decision = if current_confidence.value() >= thresholds.minimum_confidence_to_stop().value()
        && retrieval_completeness.value() >= thresholds.minimum_completeness_to_stop().value()
        && !has_material_counter_evidence
    {
        InvestigationStopConditionDecision::Stop {
            reason: InvestigationStopReason::EvidenceSufficient,
        }
    } else if remaining_budget.value() < thresholds.minimum_remaining_budget_to_continue().value() {
        InvestigationStopConditionDecision::Stop {
            reason: InvestigationStopReason::BudgetExhausted,
        }
    } else if let Some(candidate) = selected {
        let expected_value = candidate.score_breakdown().expected_value();
        if candidate.action().proposal_scope() == NextBestEvidenceProposalScope::External {
            InvestigationStopConditionDecision::Stop {
                reason: InvestigationStopReason::PolicyRestricted,
            }
        } else if expected_value < thresholds.minimum_expected_action_value_to_continue() {
            InvestigationStopConditionDecision::Stop {
                reason: InvestigationStopReason::MarginalGainBelowThreshold,
            }
        } else {
            InvestigationStopConditionDecision::Continue {
                selected_candidate_id: candidate.candidate_id().to_owned(),
                selected_action: candidate.action(),
                selected_expected_value: expected_value,
            }
        }
    } else {
        // No eligible continuation path remains. Prefer budget exhaustion when all
        // remaining candidates are blocked by budget; otherwise classify as policy.
        let all_budget_blocked = next_best_evidence
            .ranked_candidates()
            .iter()
            .all(|candidate| {
                !candidate.ineligibility_reasons().is_empty()
                    && candidate.ineligibility_reasons().iter().all(|reason| {
                        matches!(
                            reason,
                            crate::NextBestEvidenceIneligibilityReason::BudgetExceeded
                        )
                    })
            });

        if all_budget_blocked {
            InvestigationStopConditionDecision::Stop {
                reason: InvestigationStopReason::BudgetExhausted,
            }
        } else {
            InvestigationStopConditionDecision::Stop {
                reason: InvestigationStopReason::PolicyRestricted,
            }
        }
    };

    Ok(InvestigationStopCondition {
        decision,
        thresholds,
        remaining_budget,
        highest_eligible_action_value,
        has_material_counter_evidence,
    })
}

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
//! Deterministic Next Best Evidence action ranking.
//!
//! This module scores investigation proposals without executing them. Benefit,
//! cost, latency, risk, budget, and policy terms remain explicit so selection
//! can be audited and external actions stay behind an execution boundary.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::GraphError;

/// An investigation action that can be proposed by the ranking model.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InvestigationAction {
    /// Expand a relationship from the current working-set frontier.
    ExpandRelation,
    /// Load a known graph page.
    LoadPage,
    /// Search a corpus outside the currently loaded graph.
    SearchCorpus,
    /// Request evidence from a source.
    RequestSource,
    /// Verify an unresolved claim.
    VerifyClaim,
    /// Compare evidence across timelines.
    CompareTimelines,
    /// Ask an analyst for a decision or additional context.
    AskAnalyst,
    /// Stop the investigation.
    Stop,
}

impl InvestigationAction {
    /// The complete stable action vocabulary.
    pub const ALL: [Self; 8] = [
        Self::ExpandRelation,
        Self::LoadPage,
        Self::SearchCorpus,
        Self::RequestSource,
        Self::VerifyClaim,
        Self::CompareTimelines,
        Self::AskAnalyst,
        Self::Stop,
    ];

    /// Returns whether this proposal crosses an external execution boundary.
    #[must_use]
    pub const fn proposal_scope(self) -> NextBestEvidenceProposalScope {
        match self {
            Self::SearchCorpus | Self::RequestSource | Self::AskAnalyst => {
                NextBestEvidenceProposalScope::External
            }
            Self::ExpandRelation
            | Self::LoadPage
            | Self::VerifyClaim
            | Self::CompareTimelines
            | Self::Stop => NextBestEvidenceProposalScope::Internal,
        }
    }
}

/// Execution boundary associated with an investigation proposal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NextBestEvidenceProposalScope {
    /// The proposal stays within the Corrobore runtime.
    Internal,
    /// The proposal requires a separately authorized external action.
    External,
}

/// A finite normalized term in the inclusive unit interval.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct NextBestEvidenceScoreTerm(f64);

impl NextBestEvidenceScoreTerm {
    /// Creates a validated normalized score term.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::InvalidNextBestEvidenceScoreTerm`] when `value`
    /// is not finite or lies outside the inclusive unit interval.
    pub fn new(value: f64) -> Result<Self, GraphError> {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(GraphError::InvalidNextBestEvidenceScoreTerm(value));
        }

        Ok(Self(value))
    }

    /// Returns the normalized scalar.
    #[must_use]
    pub const fn value(self) -> f64 {
        self.0
    }
}

impl TryFrom<f64> for NextBestEvidenceScoreTerm {
    type Error = GraphError;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<NextBestEvidenceScoreTerm> for f64 {
    fn from(term: NextBestEvidenceScoreTerm) -> Self {
        term.value()
    }
}

/// Explainable positive and negative terms for one candidate action.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct NextBestEvidenceScoreBreakdown {
    expected_evidence_gain: NextBestEvidenceScoreTerm,
    expected_uncertainty_reduction: NextBestEvidenceScoreTerm,
    expected_decision_improvement: NextBestEvidenceScoreTerm,
    retrieval_cost: NextBestEvidenceScoreTerm,
    latency_cost: NextBestEvidenceScoreTerm,
    source_risk: NextBestEvidenceScoreTerm,
}

impl NextBestEvidenceScoreBreakdown {
    /// Creates a complete score breakdown.
    #[must_use]
    pub const fn new(
        expected_evidence_gain: NextBestEvidenceScoreTerm,
        expected_uncertainty_reduction: NextBestEvidenceScoreTerm,
        expected_decision_improvement: NextBestEvidenceScoreTerm,
        retrieval_cost: NextBestEvidenceScoreTerm,
        latency_cost: NextBestEvidenceScoreTerm,
        source_risk: NextBestEvidenceScoreTerm,
    ) -> Self {
        Self {
            expected_evidence_gain,
            expected_uncertainty_reduction,
            expected_decision_improvement,
            retrieval_cost,
            latency_cost,
            source_risk,
        }
    }

    /// Returns the expected evidence-gain term.
    #[must_use]
    pub const fn expected_evidence_gain(self) -> NextBestEvidenceScoreTerm {
        self.expected_evidence_gain
    }

    /// Returns the expected uncertainty-reduction term.
    #[must_use]
    pub const fn expected_uncertainty_reduction(self) -> NextBestEvidenceScoreTerm {
        self.expected_uncertainty_reduction
    }

    /// Returns the expected decision-improvement term.
    #[must_use]
    pub const fn expected_decision_improvement(self) -> NextBestEvidenceScoreTerm {
        self.expected_decision_improvement
    }

    /// Returns the normalized retrieval-cost penalty.
    #[must_use]
    pub const fn retrieval_cost(self) -> NextBestEvidenceScoreTerm {
        self.retrieval_cost
    }

    /// Returns the normalized latency-cost penalty.
    #[must_use]
    pub const fn latency_cost(self) -> NextBestEvidenceScoreTerm {
        self.latency_cost
    }

    /// Returns the normalized source-risk penalty.
    #[must_use]
    pub const fn source_risk(self) -> NextBestEvidenceScoreTerm {
        self.source_risk
    }

    /// Returns benefits minus retrieval, latency, and risk penalties.
    #[must_use]
    pub fn expected_value(self) -> f64 {
        self.expected_evidence_gain.value()
            + self.expected_uncertainty_reduction.value()
            + self.expected_decision_improvement.value()
            - self.retrieval_cost.value()
            - self.latency_cost.value()
            - self.source_risk.value()
    }
}

/// Hard constraints applied before a candidate can be selected.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct NextBestEvidenceConstraints {
    within_budget: bool,
    allowed_by_policy: bool,
    maximum_source_risk: NextBestEvidenceScoreTerm,
}

impl NextBestEvidenceConstraints {
    /// Creates the hard constraints for one candidate proposal.
    #[must_use]
    pub const fn new(
        within_budget: bool,
        allowed_by_policy: bool,
        maximum_source_risk: NextBestEvidenceScoreTerm,
    ) -> Self {
        Self {
            within_budget,
            allowed_by_policy,
            maximum_source_risk,
        }
    }

    /// Returns whether the proposal fits within the remaining budget.
    #[must_use]
    pub const fn within_budget(self) -> bool {
        self.within_budget
    }

    /// Returns whether policy permits proposing the action.
    #[must_use]
    pub const fn allowed_by_policy(self) -> bool {
        self.allowed_by_policy
    }

    /// Returns the maximum permitted normalized source risk.
    #[must_use]
    pub const fn maximum_source_risk(self) -> NextBestEvidenceScoreTerm {
        self.maximum_source_risk
    }
}

/// Validated input describing one candidate action proposal.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "NextBestEvidenceCandidateInputWire")]
pub struct NextBestEvidenceCandidateInput {
    candidate_id: String,
    action: InvestigationAction,
    score_breakdown: NextBestEvidenceScoreBreakdown,
    constraints: NextBestEvidenceConstraints,
}

#[derive(Deserialize)]
struct NextBestEvidenceCandidateInputWire {
    candidate_id: String,
    action: InvestigationAction,
    score_breakdown: NextBestEvidenceScoreBreakdown,
    constraints: NextBestEvidenceConstraints,
}

impl NextBestEvidenceCandidateInput {
    /// Creates a candidate proposal with a stable non-blank identifier.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::InvalidNextBestEvidenceInput`] when the candidate
    /// identifier is blank.
    pub fn new(
        candidate_id: impl Into<String>,
        action: InvestigationAction,
        score_breakdown: NextBestEvidenceScoreBreakdown,
        constraints: NextBestEvidenceConstraints,
    ) -> Result<Self, GraphError> {
        let candidate_id = candidate_id.into();
        if candidate_id.trim().is_empty() {
            return Err(GraphError::InvalidNextBestEvidenceInput(
                "candidate identifier must not be blank".to_owned(),
            ));
        }

        Ok(Self {
            candidate_id,
            action,
            score_breakdown,
            constraints,
        })
    }
}

impl TryFrom<NextBestEvidenceCandidateInputWire> for NextBestEvidenceCandidateInput {
    type Error = GraphError;

    fn try_from(input: NextBestEvidenceCandidateInputWire) -> Result<Self, Self::Error> {
        Self::new(
            input.candidate_id,
            input.action,
            input.score_breakdown,
            input.constraints,
        )
    }
}

/// A hard reason why a candidate cannot be selected.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum NextBestEvidenceIneligibilityReason {
    /// The action exceeds the remaining investigation budget.
    BudgetExceeded,
    /// Investigation policy denies the action.
    PolicyDenied,
    /// The action's source risk exceeds the permitted maximum.
    SourceRiskExceeded {
        /// Candidate source risk.
        observed: NextBestEvidenceScoreTerm,
        /// Maximum source risk allowed by policy.
        maximum: NextBestEvidenceScoreTerm,
    },
}

/// One scored and eligibility-annotated proposal in ranked order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RankedNextBestEvidenceCandidate {
    candidate_id: String,
    action: InvestigationAction,
    score_breakdown: NextBestEvidenceScoreBreakdown,
    ineligibility_reasons: Vec<NextBestEvidenceIneligibilityReason>,
}

impl RankedNextBestEvidenceCandidate {
    /// Returns the candidate's stable identifier.
    #[must_use]
    pub fn candidate_id(&self) -> &str {
        &self.candidate_id
    }

    /// Returns the proposed investigation action.
    #[must_use]
    pub const fn action(&self) -> InvestigationAction {
        self.action
    }

    /// Returns the complete explainable score.
    #[must_use]
    pub const fn score_breakdown(&self) -> NextBestEvidenceScoreBreakdown {
        self.score_breakdown
    }

    /// Returns every hard reason preventing selection.
    #[must_use]
    pub fn ineligibility_reasons(&self) -> &[NextBestEvidenceIneligibilityReason] {
        &self.ineligibility_reasons
    }

    /// Returns whether this candidate can be selected.
    #[must_use]
    pub fn is_eligible(&self) -> bool {
        self.ineligibility_reasons.is_empty()
    }
}

/// Deterministically ordered Next Best Evidence proposals.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NextBestEvidenceRanking {
    ranked_candidates: Vec<RankedNextBestEvidenceCandidate>,
}

impl NextBestEvidenceRanking {
    /// Returns every candidate in deterministic ranking order.
    #[must_use]
    pub fn ranked_candidates(&self) -> &[RankedNextBestEvidenceCandidate] {
        &self.ranked_candidates
    }

    /// Returns the highest-ranked eligible proposal, if one exists.
    #[must_use]
    pub fn selected(&self) -> Option<&RankedNextBestEvidenceCandidate> {
        self.ranked_candidates
            .iter()
            .find(|candidate| candidate.is_eligible())
    }
}

/// Ranks candidate investigation proposals without executing any action.
///
/// Eligible candidates precede ineligible candidates. Within each group,
/// expected value sorts descending and the stable candidate identifier breaks
/// ties lexically.
///
/// # Errors
///
/// Returns [`GraphError::InvalidNextBestEvidenceInput`] when the candidate set
/// is empty or contains duplicate identifiers.
pub fn rank_next_best_evidence(
    candidates: Vec<NextBestEvidenceCandidateInput>,
) -> Result<NextBestEvidenceRanking, GraphError> {
    if candidates.is_empty() {
        return Err(GraphError::InvalidNextBestEvidenceInput(
            "candidate set must not be empty".to_owned(),
        ));
    }

    let mut candidate_ids = HashSet::with_capacity(candidates.len());
    let mut ranked_candidates = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        if !candidate_ids.insert(candidate.candidate_id.clone()) {
            return Err(GraphError::InvalidNextBestEvidenceInput(format!(
                "duplicate candidate identifier: {}",
                candidate.candidate_id
            )));
        }

        let mut ineligibility_reasons = Vec::with_capacity(3);
        if !candidate.constraints.within_budget() {
            ineligibility_reasons.push(NextBestEvidenceIneligibilityReason::BudgetExceeded);
        }
        if !candidate.constraints.allowed_by_policy() {
            ineligibility_reasons.push(NextBestEvidenceIneligibilityReason::PolicyDenied);
        }

        let observed_risk = candidate.score_breakdown.source_risk();
        let maximum_risk = candidate.constraints.maximum_source_risk();
        if observed_risk.value() > maximum_risk.value() {
            ineligibility_reasons.push(NextBestEvidenceIneligibilityReason::SourceRiskExceeded {
                observed: observed_risk,
                maximum: maximum_risk,
            });
        }

        ranked_candidates.push(RankedNextBestEvidenceCandidate {
            candidate_id: candidate.candidate_id,
            action: candidate.action,
            score_breakdown: candidate.score_breakdown,
            ineligibility_reasons,
        });
    }

    ranked_candidates.sort_by(|left, right| {
        right
            .is_eligible()
            .cmp(&left.is_eligible())
            .then_with(|| {
                right
                    .score_breakdown()
                    .expected_value()
                    .total_cmp(&left.score_breakdown().expected_value())
            })
            .then_with(|| left.candidate_id().cmp(right.candidate_id()))
    });

    Ok(NextBestEvidenceRanking { ranked_candidates })
}

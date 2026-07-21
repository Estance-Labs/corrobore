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
use graph_core::{
    CandidateEvidenceOutcome, ClaimId, Confidence, EvidenceId, EvidenceSubgraph,
    InformationGainInput, InvestigationAction, InvestigationStopConditionDecision,
    InvestigationStopReason, InvestigationStopThresholds, NextBestEvidenceCandidateInput,
    NextBestEvidenceConstraints, NextBestEvidenceRanking, NextBestEvidenceScoreBreakdown,
    NextBestEvidenceScoreTerm, NodeId, OutcomeProbability, RetrievalCompleteness,
    StopConditionBudget, UnresolvedUnknown, estimate_information_gain,
    evaluate_investigation_stop_condition, rank_next_best_evidence,
};
use serde::{Deserialize, Serialize};

fn confidence(value: f64) -> Confidence {
    Confidence::new(value).expect("test confidence should be valid")
}

fn completeness(value: f64) -> RetrievalCompleteness {
    RetrievalCompleteness::new(value).expect("test completeness should be valid")
}

fn term(value: f64) -> NextBestEvidenceScoreTerm {
    NextBestEvidenceScoreTerm::new(value).expect("test score term should be valid")
}

fn thresholds(
    confidence_threshold: f64,
    completeness_threshold: f64,
    min_action_value: f64,
    min_remaining_budget: f64,
) -> InvestigationStopThresholds {
    InvestigationStopThresholds::new(
        confidence(confidence_threshold),
        completeness(completeness_threshold),
        min_action_value,
        StopConditionBudget::new(min_remaining_budget).expect("valid budget threshold"),
    )
    .expect("valid stop thresholds")
}

fn ranking(
    candidate_id: &str,
    action: InvestigationAction,
    expected_value: f64,
) -> NextBestEvidenceRanking {
    let score = NextBestEvidenceScoreBreakdown::new(
        term(expected_value),
        term(0.0),
        term(0.0),
        term(0.0),
        term(0.0),
        term(0.0),
    );
    let constraints = NextBestEvidenceConstraints::new(true, true, term(1.0));
    let candidate = NextBestEvidenceCandidateInput::new(candidate_id, action, score, constraints)
        .expect("valid candidate");
    rank_next_best_evidence(vec![candidate]).expect("valid ranking")
}

fn information_gain() -> graph_core::InformationGainEstimate {
    let outcomes = vec![
        CandidateEvidenceOutcome::new(
            OutcomeProbability::new(0.5).expect("valid probability"),
            confidence(0.1),
        ),
        CandidateEvidenceOutcome::new(
            OutcomeProbability::new(0.5).expect("valid probability"),
            confidence(0.9),
        ),
    ];
    let input =
        InformationGainInput::new(confidence(0.5), outcomes).expect("valid estimator input");
    estimate_information_gain(&input)
}

#[test]
fn evidence_sufficient_stops_at_threshold_boundaries() {
    let decision = evaluate_investigation_stop_condition(
        confidence(0.90),
        completeness(0.80),
        &EvidenceSubgraph::default(),
        &[],
        StopConditionBudget::new(0.50).expect("valid budget"),
        &ranking("verify-claim--1", InvestigationAction::VerifyClaim, 0.40),
        thresholds(0.90, 0.80, 0.40, 0.10),
    )
    .expect("stop evaluation should succeed");

    assert!(matches!(
        decision.decision(),
        InvestigationStopConditionDecision::Stop {
            reason: InvestigationStopReason::EvidenceSufficient
        }
    ));
}

#[test]
fn unresolved_material_counter_evidence_blocks_evidence_sufficient_stop() {
    let counter_evidence = EvidenceSubgraph {
        claim_ids: vec![ClaimId::new("claim--counter").expect("valid claim id")],
        evidence_ids: vec![EvidenceId::new("evidence--counter").expect("valid evidence id")],
        ..EvidenceSubgraph::default()
    };
    let unresolved_unknowns = vec![UnresolvedUnknown::UnresolvedContradiction {
        claim_id: ClaimId::new("claim--main").expect("valid claim id"),
        contradicting_claim_id: ClaimId::new("claim--counter").expect("valid claim id"),
    }];
    let decision = evaluate_investigation_stop_condition(
        confidence(0.95),
        completeness(0.95),
        &counter_evidence,
        &unresolved_unknowns,
        StopConditionBudget::new(0.60).expect("valid budget"),
        &ranking("verify-claim--2", InvestigationAction::VerifyClaim, 0.75),
        thresholds(0.90, 0.80, 0.40, 0.10),
    )
    .expect("stop evaluation should succeed");

    assert!(matches!(
        decision.decision(),
        InvestigationStopConditionDecision::Continue {
            selected_candidate_id,
            selected_action: InvestigationAction::VerifyClaim,
            ..
        } if selected_candidate_id == "verify-claim--2"
    ));
}

#[test]
fn budget_exhaustion_is_distinct_from_marginal_gain_stops() {
    let exhausted = evaluate_investigation_stop_condition(
        confidence(0.30),
        completeness(0.60),
        &EvidenceSubgraph::default(),
        &[],
        StopConditionBudget::new(0.0).expect("valid budget"),
        &ranking("verify-claim--3", InvestigationAction::VerifyClaim, 0.90),
        thresholds(0.90, 0.80, 0.40, 0.10),
    )
    .expect("stop evaluation should succeed");
    let marginal = evaluate_investigation_stop_condition(
        confidence(0.30),
        completeness(0.60),
        &EvidenceSubgraph::default(),
        &[],
        StopConditionBudget::new(0.50).expect("valid budget"),
        &ranking("verify-claim--4", InvestigationAction::VerifyClaim, 0.20),
        thresholds(0.90, 0.80, 0.40, 0.10),
    )
    .expect("stop evaluation should succeed");

    assert!(matches!(
        exhausted.decision(),
        InvestigationStopConditionDecision::Stop {
            reason: InvestigationStopReason::BudgetExhausted
        }
    ));
    assert!(matches!(
        marginal.decision(),
        InvestigationStopConditionDecision::Stop {
            reason: InvestigationStopReason::MarginalGainBelowThreshold
        }
    ));
}

#[test]
fn continue_decision_exposes_selected_next_best_evidence_action() {
    let decision = evaluate_investigation_stop_condition(
        confidence(0.30),
        completeness(0.60),
        &EvidenceSubgraph::default(),
        &[UnresolvedUnknown::UnexpandedFrontier {
            node_id: NodeId::new("node--frontier").expect("valid node id"),
        }],
        StopConditionBudget::new(0.50).expect("valid budget"),
        &ranking(
            "compare-timelines--1",
            InvestigationAction::CompareTimelines,
            0.80,
        ),
        thresholds(0.90, 0.80, 0.40, 0.10),
    )
    .expect("stop evaluation should succeed");

    assert!(matches!(
        decision.decision(),
        InvestigationStopConditionDecision::Continue {
            selected_candidate_id,
            selected_action: InvestigationAction::CompareTimelines,
            ..
        } if selected_candidate_id == "compare-timelines--1"
    ));
}

#[test]
fn external_action_requires_authorization_and_forces_policy_stop() {
    let decision = evaluate_investigation_stop_condition(
        confidence(0.30),
        completeness(0.60),
        &EvidenceSubgraph::default(),
        &[],
        StopConditionBudget::new(0.50).expect("valid budget"),
        &ranking("ask-analyst--1", InvestigationAction::AskAnalyst, 0.90),
        thresholds(0.90, 0.80, 0.40, 0.10),
    )
    .expect("stop evaluation should succeed");

    assert!(matches!(
        decision.decision(),
        InvestigationStopConditionDecision::Stop {
            reason: InvestigationStopReason::PolicyRestricted
        }
    ));
}

#[test]
fn stop_evaluation_is_deterministic_and_serializable() {
    let first = evaluate_investigation_stop_condition(
        confidence(0.30),
        completeness(0.60),
        &EvidenceSubgraph::default(),
        &[],
        StopConditionBudget::new(0.50).expect("valid budget"),
        &ranking("verify-claim--5", InvestigationAction::VerifyClaim, 0.80),
        thresholds(0.90, 0.80, 0.40, 0.10),
    )
    .expect("stop evaluation should succeed");
    let second = evaluate_investigation_stop_condition(
        confidence(0.30),
        completeness(0.60),
        &EvidenceSubgraph::default(),
        &[],
        StopConditionBudget::new(0.50).expect("valid budget"),
        &ranking("verify-claim--5", InvestigationAction::VerifyClaim, 0.80),
        thresholds(0.90, 0.80, 0.40, 0.10),
    )
    .expect("stop evaluation should succeed");

    assert_eq!(first, second);
    assert_eq!(
        serde_json::to_string(&first).expect("stop condition should serialize"),
        serde_json::to_string(&second).expect("stop condition should serialize")
    );
}

#[test]
fn stop_condition_types_are_serde_compatible() {
    fn assert_serializable<T: Serialize + for<'de> Deserialize<'de>>() {}

    assert_serializable::<graph_core::InvestigationStopCondition>();
    assert_serializable::<graph_core::InvestigationStopThresholds>();
    assert_serializable::<graph_core::StopConditionBudget>();
    assert!(
        !information_gain()
            .expected_information_gain_bits()
            .is_sign_negative()
    );
}

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
//! Epic 0020 acceptance suite: active investigation and next best evidence.
//!
//! Validates deterministic FIMI and CTI scenarios end to end at the public
//! graph-core boundary: information-gain estimation, explainable ranking,
//! calibrated assessment envelope, and budget-aware stop decisions.

use graph_core::{
    CalibratedAssessment, CandidateEvidenceOutcome, ClaimId, Confidence, EvidenceId,
    EvidenceSubgraph, InformationGainEstimate, InformationGainInput, InvestigationAction,
    InvestigationStopConditionDecision, InvestigationStopReason, InvestigationStopThresholds,
    NextBestEvidenceCandidateInput, NextBestEvidenceConstraints, NextBestEvidenceRanking,
    NextBestEvidenceScoreBreakdown, NextBestEvidenceScoreTerm, NodeId, OutcomeProbability,
    RequestId, RetrievalCompleteness, SourceProvenanceRef, StopConditionBudget, UnresolvedUnknown,
    estimate_information_gain, evaluate_investigation_stop_condition, rank_next_best_evidence,
};

const EPSILON: f64 = 1e-12;

fn confidence(value: f64) -> Confidence {
    Confidence::new(value).expect("acceptance confidence should be valid")
}

fn completeness(value: f64) -> RetrievalCompleteness {
    RetrievalCompleteness::new(value).expect("acceptance completeness should be valid")
}

fn term(value: f64) -> NextBestEvidenceScoreTerm {
    NextBestEvidenceScoreTerm::new(value).expect("acceptance score term should be valid")
}

fn probability(value: f64) -> OutcomeProbability {
    OutcomeProbability::new(value).expect("acceptance probability should be valid")
}

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= EPSILON,
        "expected {expected}, got {actual}"
    );
}

fn expected_entropy(confidence: f64) -> f64 {
    if confidence == 0.0 || confidence == 1.0 {
        0.0
    } else {
        -(confidence * confidence.log2() + (1.0 - confidence) * (1.0 - confidence).log2())
    }
}

fn expected_gain_math(current_confidence: f64, outcomes: &[(f64, f64)]) -> (f64, f64) {
    let prior = expected_entropy(current_confidence).clamp(0.0, 1.0);
    let posterior = outcomes
        .iter()
        .map(|(p, posterior_confidence)| p * expected_entropy(*posterior_confidence))
        .sum::<f64>()
        .clamp(0.0, 1.0);
    let gain = (prior - posterior).max(0.0).clamp(0.0, 1.0);
    let reduction = if prior == 0.0 {
        0.0
    } else {
        (gain / prior).clamp(0.0, 1.0)
    };
    (gain, reduction)
}

#[derive(Clone)]
struct CandidateScenario {
    id: &'static str,
    action: InvestigationAction,
    outcomes: Vec<(f64, f64)>,
    decision_improvement: f64,
    retrieval_cost: f64,
    latency_cost: f64,
    source_risk: f64,
    within_budget: bool,
    allowed_by_policy: bool,
    maximum_source_risk: f64,
}

fn estimate_for_candidate(
    current_confidence: Confidence,
    candidate: &CandidateScenario,
) -> InformationGainEstimate {
    let outcomes = candidate
        .outcomes
        .iter()
        .map(|(p, posterior)| {
            CandidateEvidenceOutcome::new(probability(*p), confidence(*posterior))
        })
        .collect();
    let input = InformationGainInput::new(current_confidence, outcomes)
        .expect("candidate input should be valid");
    estimate_information_gain(&input)
}

fn rank_candidates(
    current_confidence: Confidence,
    candidates: &[CandidateScenario],
) -> NextBestEvidenceRanking {
    let inputs = candidates
        .iter()
        .map(|candidate| {
            let estimate = estimate_for_candidate(current_confidence, candidate);
            NextBestEvidenceCandidateInput::new(
                candidate.id,
                candidate.action,
                NextBestEvidenceScoreBreakdown::new(
                    term(estimate.expected_information_gain_bits()),
                    term(estimate.expected_uncertainty_reduction()),
                    term(candidate.decision_improvement),
                    term(candidate.retrieval_cost),
                    term(candidate.latency_cost),
                    term(candidate.source_risk),
                ),
                NextBestEvidenceConstraints::new(
                    candidate.within_budget,
                    candidate.allowed_by_policy,
                    term(candidate.maximum_source_risk),
                ),
            )
            .expect("candidate should be valid")
        })
        .collect();
    rank_next_best_evidence(inputs).expect("ranking should be valid")
}

fn thresholds(
    min_confidence: f64,
    min_completeness: f64,
    min_action_value: f64,
    min_budget: f64,
) -> InvestigationStopThresholds {
    InvestigationStopThresholds::new(
        confidence(min_confidence),
        completeness(min_completeness),
        min_action_value,
        StopConditionBudget::new(min_budget).expect("minimum budget should be valid"),
    )
    .expect("stop thresholds should be valid")
}

#[derive(Clone)]
struct InvestigationScenario {
    question: &'static str,
    current_confidence: Confidence,
    retrieval_completeness: RetrievalCompleteness,
    supporting_evidence: EvidenceSubgraph,
    counter_evidence: EvidenceSubgraph,
    unresolved_unknowns: Vec<UnresolvedUnknown>,
    source_provenance: SourceProvenanceRef,
    remaining_budget: StopConditionBudget,
    thresholds: InvestigationStopThresholds,
    candidates: Vec<CandidateScenario>,
}

fn fimi_open_scenario() -> InvestigationScenario {
    InvestigationScenario {
        question: "Is the payment stream a coordinated laundering campaign?",
        current_confidence: confidence(0.58),
        retrieval_completeness: completeness(0.52),
        supporting_evidence: EvidenceSubgraph {
            claim_ids: vec![
                ClaimId::new("claim--fimi-campaign").expect("claim id should be valid"),
            ],
            evidence_ids: vec![
                EvidenceId::new("evidence--fimi-ledger-pattern")
                    .expect("evidence id should be valid"),
            ],
            ..EvidenceSubgraph::default()
        },
        counter_evidence: EvidenceSubgraph {
            evidence_ids: vec![
                EvidenceId::new("evidence--fimi-benign-pattern")
                    .expect("evidence id should be valid"),
            ],
            ..EvidenceSubgraph::default()
        },
        unresolved_unknowns: vec![UnresolvedUnknown::UnexpandedFrontier {
            node_id: NodeId::new("node--fimi-frontier").expect("node id should be valid"),
        }],
        source_provenance: SourceProvenanceRef {
            retrieval_ids: vec![
                RequestId::new("request--fimi-open").expect("request id should be valid"),
            ],
            source_refs: vec!["source://fimi/ledger-batch-42".to_owned()],
        },
        remaining_budget: StopConditionBudget::new(0.65).expect("remaining budget should be valid"),
        thresholds: thresholds(0.90, 0.85, 0.20, 0.10),
        candidates: vec![
            CandidateScenario {
                id: "verify-claim--fimi-ledger",
                action: InvestigationAction::VerifyClaim,
                outcomes: vec![(0.5, 0.15), (0.5, 0.92)],
                decision_improvement: 0.55,
                retrieval_cost: 0.15,
                latency_cost: 0.10,
                source_risk: 0.05,
                within_budget: true,
                allowed_by_policy: true,
                maximum_source_risk: 0.50,
            },
            CandidateScenario {
                id: "compare-timelines--fimi",
                action: InvestigationAction::CompareTimelines,
                outcomes: vec![(0.5, 0.35), (0.5, 0.75)],
                decision_improvement: 0.35,
                retrieval_cost: 0.20,
                latency_cost: 0.10,
                source_risk: 0.05,
                within_budget: true,
                allowed_by_policy: true,
                maximum_source_risk: 0.50,
            },
        ],
    }
}

fn cti_closed_scenario() -> InvestigationScenario {
    InvestigationScenario {
        question: "Is the malware family attributed to actor X?",
        current_confidence: confidence(0.94),
        retrieval_completeness: completeness(0.93),
        supporting_evidence: EvidenceSubgraph {
            claim_ids: vec![
                ClaimId::new("claim--cti-attribution").expect("claim id should be valid"),
            ],
            evidence_ids: vec![
                EvidenceId::new("evidence--cti-signature").expect("evidence id should be valid"),
                EvidenceId::new("evidence--cti-infra-overlap")
                    .expect("evidence id should be valid"),
            ],
            ..EvidenceSubgraph::default()
        },
        counter_evidence: EvidenceSubgraph::default(),
        unresolved_unknowns: Vec::new(),
        source_provenance: SourceProvenanceRef {
            retrieval_ids: vec![
                RequestId::new("request--cti-closed").expect("request id should be valid"),
            ],
            source_refs: vec![
                "source://cti/report-alpha".to_owned(),
                "source://cti/report-beta".to_owned(),
            ],
        },
        remaining_budget: StopConditionBudget::new(0.40).expect("remaining budget should be valid"),
        thresholds: thresholds(0.90, 0.85, 0.20, 0.10),
        candidates: vec![CandidateScenario {
            id: "compare-timelines--cti",
            action: InvestigationAction::CompareTimelines,
            outcomes: vec![(0.5, 0.90), (0.5, 0.96)],
            decision_improvement: 0.25,
            retrieval_cost: 0.10,
            latency_cost: 0.05,
            source_risk: 0.05,
            within_budget: true,
            allowed_by_policy: true,
            maximum_source_risk: 0.50,
        }],
    }
}

fn external_proposal_scenario() -> InvestigationScenario {
    let mut scenario = fimi_open_scenario();
    scenario.candidates = vec![
        CandidateScenario {
            id: "request-source--external-regulator",
            action: InvestigationAction::RequestSource,
            outcomes: vec![(0.5, 0.10), (0.5, 0.95)],
            decision_improvement: 0.70,
            retrieval_cost: 0.05,
            latency_cost: 0.05,
            source_risk: 0.10,
            within_budget: true,
            allowed_by_policy: true,
            maximum_source_risk: 0.50,
        },
        CandidateScenario {
            id: "verify-claim--fimi-ledger-low",
            action: InvestigationAction::VerifyClaim,
            outcomes: vec![(0.5, 0.40), (0.5, 0.72)],
            decision_improvement: 0.20,
            retrieval_cost: 0.25,
            latency_cost: 0.20,
            source_risk: 0.05,
            within_budget: true,
            allowed_by_policy: true,
            maximum_source_risk: 0.50,
        },
    ];
    scenario
}

fn build_assessment(
    scenario: &InvestigationScenario,
) -> (CalibratedAssessment, NextBestEvidenceRanking) {
    let ranking = rank_candidates(scenario.current_confidence, &scenario.candidates);
    let selected = ranking.selected().expect("at least one eligible candidate");
    let selected_candidate = scenario
        .candidates
        .iter()
        .find(|candidate| candidate.id == selected.candidate_id())
        .expect("selected candidate should be present");
    let selected_gain = estimate_for_candidate(scenario.current_confidence, selected_candidate);

    let stop_condition = evaluate_investigation_stop_condition(
        scenario.current_confidence,
        scenario.retrieval_completeness,
        &scenario.counter_evidence,
        &scenario.unresolved_unknowns,
        scenario.remaining_budget,
        &ranking,
        scenario.thresholds,
    )
    .expect("stop condition should evaluate");

    let assessment = CalibratedAssessment::new(
        scenario.question,
        scenario.current_confidence,
        scenario.supporting_evidence.clone(),
        scenario.counter_evidence.clone(),
        scenario.source_provenance.clone(),
        scenario.retrieval_completeness,
        scenario.unresolved_unknowns.clone(),
        selected_gain,
        ranking.clone(),
        stop_condition,
    )
    .expect("assessment should build");

    (assessment, ranking)
}

//
// Acceptance: FIMI open investigations expose deterministic gain estimates,
// deterministic ranking explanations, calibrated assessment/provenance, and a
// continue decision with the selected next internal action.
#[test]
fn acceptance_fimi_open_investigation_continues_with_deterministic_next_action() {
    let scenario = fimi_open_scenario();
    let (first_assessment, first_ranking) = build_assessment(&scenario);
    let (second_assessment, second_ranking) = build_assessment(&scenario);

    assert_eq!(first_assessment, second_assessment);
    assert_eq!(first_ranking, second_ranking);
    assert_eq!(
        first_assessment.source_provenance().retrieval_ids[0].as_str(),
        "request--fimi-open"
    );
    assert_eq!(
        first_ranking
            .selected()
            .expect("selected candidate should exist")
            .candidate_id(),
        "verify-claim--fimi-ledger"
    );
    assert!(matches!(
        first_assessment.stop_condition().decision(),
        InvestigationStopConditionDecision::Continue {
            selected_candidate_id,
            selected_action: InvestigationAction::VerifyClaim,
            ..
        } if selected_candidate_id == "verify-claim--fimi-ledger"
    ));

    for candidate in &scenario.candidates {
        let estimate = estimate_for_candidate(scenario.current_confidence, candidate);
        let (expected_gain, expected_reduction) =
            expected_gain_math(scenario.current_confidence.value(), &candidate.outcomes);
        assert_close(estimate.expected_information_gain_bits(), expected_gain);
        assert_close(
            estimate.expected_uncertainty_reduction(),
            expected_reduction,
        );
    }
}

//
// Acceptance: CTI completed investigations stop for evidence sufficiency under
// budget and carry the calibrated assessment envelope content and provenance.
#[test]
fn acceptance_cti_closed_investigation_stops_as_evidence_sufficient() {
    let scenario = cti_closed_scenario();
    let (assessment, ranking) = build_assessment(&scenario);

    assert_eq!(
        ranking
            .selected()
            .expect("selected candidate should exist")
            .candidate_id(),
        "compare-timelines--cti"
    );
    assert_eq!(
        assessment
            .supporting_evidence()
            .evidence_ids
            .iter()
            .map(|id| id.as_str())
            .collect::<Vec<_>>(),
        vec!["evidence--cti-signature", "evidence--cti-infra-overlap"]
    );
    assert_eq!(assessment.counter_evidence().evidence_ids.len(), 0);
    assert_eq!(assessment.unresolved_unknowns().len(), 0);
    assert_eq!(assessment.source_provenance().source_refs.len(), 2);
    assert!(matches!(
        assessment.stop_condition().decision(),
        InvestigationStopConditionDecision::Stop {
            reason: InvestigationStopReason::EvidenceSufficient
        }
    ));
}

//
// Acceptance: when the highest-value eligible action is external, ranking keeps
// it as the top proposal and stop policy blocks autonomous execution.
#[test]
fn acceptance_external_top_action_remains_proposal_and_forces_policy_stop() {
    let scenario = external_proposal_scenario();
    let (assessment, ranking) = build_assessment(&scenario);
    let selected = ranking.selected().expect("selected candidate should exist");

    assert_eq!(
        selected.candidate_id(),
        "request-source--external-regulator"
    );
    assert_eq!(selected.action(), InvestigationAction::RequestSource);
    assert!(matches!(
        assessment.stop_condition().decision(),
        InvestigationStopConditionDecision::Stop {
            reason: InvestigationStopReason::PolicyRestricted
        }
    ));
}

//
// Acceptance: budget-exhausted and marginal-gain stop paths remain distinct.
#[test]
fn acceptance_stop_paths_cover_budget_exhausted_and_marginal_gain() {
    let scenario = fimi_open_scenario();
    let ranking = rank_candidates(scenario.current_confidence, &scenario.candidates);

    let marginal_stop = evaluate_investigation_stop_condition(
        scenario.current_confidence,
        scenario.retrieval_completeness,
        &scenario.counter_evidence,
        &scenario.unresolved_unknowns,
        StopConditionBudget::new(0.50).expect("budget should be valid"),
        &ranking,
        thresholds(0.90, 0.85, 1.30, 0.10),
    )
    .expect("marginal stop should evaluate");
    let budget_stop = evaluate_investigation_stop_condition(
        scenario.current_confidence,
        scenario.retrieval_completeness,
        &scenario.counter_evidence,
        &scenario.unresolved_unknowns,
        StopConditionBudget::new(0.05).expect("budget should be valid"),
        &ranking,
        thresholds(0.90, 0.85, 0.20, 0.10),
    )
    .expect("budget stop should evaluate");

    assert!(matches!(
        marginal_stop.decision(),
        InvestigationStopConditionDecision::Stop {
            reason: InvestigationStopReason::MarginalGainBelowThreshold
        }
    ));
    assert!(matches!(
        budget_stop.decision(),
        InvestigationStopConditionDecision::Stop {
            reason: InvestigationStopReason::BudgetExhausted
        }
    ));
}

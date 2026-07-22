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
    GraphError, InvestigationAction, NextBestEvidenceCandidateInput, NextBestEvidenceConstraints,
    NextBestEvidenceIneligibilityReason, NextBestEvidenceProposalScope, NextBestEvidenceRanking,
    NextBestEvidenceScoreBreakdown, NextBestEvidenceScoreTerm, rank_next_best_evidence,
};
use serde::{Deserialize, Serialize};

const EPSILON: f64 = 1e-12;

fn term(value: f64) -> NextBestEvidenceScoreTerm {
    NextBestEvidenceScoreTerm::new(value).expect("test score term should be valid")
}

fn breakdown(
    evidence_gain: f64,
    uncertainty_reduction: f64,
    decision_improvement: f64,
    retrieval_cost: f64,
    latency_cost: f64,
    source_risk: f64,
) -> NextBestEvidenceScoreBreakdown {
    NextBestEvidenceScoreBreakdown::new(
        term(evidence_gain),
        term(uncertainty_reduction),
        term(decision_improvement),
        term(retrieval_cost),
        term(latency_cost),
        term(source_risk),
    )
}

fn constraints(
    within_budget: bool,
    allowed_by_policy: bool,
    maximum_source_risk: f64,
) -> NextBestEvidenceConstraints {
    NextBestEvidenceConstraints::new(within_budget, allowed_by_policy, term(maximum_source_risk))
}

fn candidate(
    candidate_id: &str,
    action: InvestigationAction,
    score_breakdown: NextBestEvidenceScoreBreakdown,
    candidate_constraints: NextBestEvidenceConstraints,
) -> NextBestEvidenceCandidateInput {
    NextBestEvidenceCandidateInput::new(
        candidate_id,
        action,
        score_breakdown,
        candidate_constraints,
    )
    .expect("test candidate should be valid")
}

fn rank(candidates: Vec<NextBestEvidenceCandidateInput>) -> NextBestEvidenceRanking {
    rank_next_best_evidence(candidates).expect("test ranking should be valid")
}

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= EPSILON,
        "expected {expected}, got {actual}"
    );
}

//
// Verify the public action vocabulary covers every Epic 0020 investigation
// choice.
//
// Given the complete action list,
// when callers enumerate it,
// then every documented internal, external, analyst, and stop proposal should
// be represented exactly once.
#[test]
fn action_vocabulary_is_complete_and_stable() {
    assert_eq!(
        InvestigationAction::ALL,
        [
            InvestigationAction::ExpandRelation,
            InvestigationAction::LoadPage,
            InvestigationAction::SearchCorpus,
            InvestigationAction::RequestSource,
            InvestigationAction::VerifyClaim,
            InvestigationAction::CompareTimelines,
            InvestigationAction::AskAnalyst,
            InvestigationAction::Stop,
        ]
    );

    assert_eq!(
        InvestigationAction::RequestSource.proposal_scope(),
        NextBestEvidenceProposalScope::External
    );
    assert_eq!(
        InvestigationAction::AskAnalyst.proposal_scope(),
        NextBestEvidenceProposalScope::External
    );
    assert_eq!(
        InvestigationAction::ExpandRelation.proposal_scope(),
        NextBestEvidenceProposalScope::Internal
    );
}

//
// Verify normalized ranking terms reject invalid values through a typed error.
//
// Given values outside the finite unit interval,
// when score terms are constructed,
// then the public API should reject them before ranking.
#[test]
fn score_terms_are_typed_finite_and_bounded() {
    assert_eq!(term(0.0).value(), 0.0);
    assert_eq!(term(1.0).value(), 1.0);

    for invalid in [-0.01, 1.01, f64::INFINITY, f64::NAN] {
        let error =
            NextBestEvidenceScoreTerm::new(invalid).expect_err("invalid score term should fail");
        assert!(
            matches!(error, GraphError::InvalidNextBestEvidenceScoreTerm(value)
                if (value.is_nan() && invalid.is_nan()) || value == invalid)
        );
    }
}

//
// Verify the documented expected-value formula and its explanation surface.
//
// Given one candidate with all six score terms,
// when it is ranked,
// then its total should add the three benefits, subtract the three penalties,
// and preserve every typed term for inspection.
#[test]
fn score_breakdown_is_complete_and_queryable() {
    let ranking = rank(vec![candidate(
        "verify-claim-a",
        InvestigationAction::VerifyClaim,
        breakdown(0.8, 0.6, 0.4, 0.2, 0.1, 0.3),
        constraints(true, true, 0.5),
    )]);
    let selected = ranking
        .selected()
        .expect("eligible candidate should be selected");
    let score = selected.score_breakdown();

    assert_close(score.expected_evidence_gain().value(), 0.8);
    assert_close(score.expected_uncertainty_reduction().value(), 0.6);
    assert_close(score.expected_decision_improvement().value(), 0.4);
    assert_close(score.retrieval_cost().value(), 0.2);
    assert_close(score.latency_cost().value(), 0.1);
    assert_close(score.source_risk().value(), 0.3);
    assert_close(score.expected_value(), 1.2);
}

//
// Verify eligible candidates are ordered by expected value.
//
// Given lower- and higher-value candidates under equal constraints,
// when the ranking is computed,
// then the higher-value proposal should rank first and become the selected
// next action.
#[test]
fn higher_value_eligible_action_ranks_first() {
    let lower = candidate(
        "load-page",
        InvestigationAction::LoadPage,
        breakdown(0.4, 0.3, 0.2, 0.2, 0.1, 0.1),
        constraints(true, true, 0.5),
    );
    let higher = candidate(
        "verify-claim",
        InvestigationAction::VerifyClaim,
        breakdown(0.8, 0.7, 0.5, 0.2, 0.1, 0.1),
        constraints(true, true, 0.5),
    );

    let ranking = rank(vec![lower, higher]);

    assert_eq!(
        ranking.ranked_candidates()[0].candidate_id(),
        "verify-claim"
    );
    assert_eq!(
        ranking.selected().map(|selected| selected.action()),
        Some(InvestigationAction::VerifyClaim)
    );
}

//
// Verify ties have an input-order-independent stable resolution.
//
// Given equal-score candidates in opposite input orders,
// when each set is ranked,
// then the lexical candidate identifier should produce identical outputs.
#[test]
fn equal_scores_use_deterministic_identifier_tie_breaking() {
    let alpha = candidate(
        "alpha",
        InvestigationAction::SearchCorpus,
        breakdown(0.6, 0.4, 0.2, 0.2, 0.1, 0.1),
        constraints(true, true, 0.5),
    );
    let beta = candidate(
        "beta",
        InvestigationAction::SearchCorpus,
        breakdown(0.6, 0.4, 0.2, 0.2, 0.1, 0.1),
        constraints(true, true, 0.5),
    );

    let forward = rank(vec![beta.clone(), alpha.clone()]);
    let reverse = rank(vec![alpha, beta]);

    assert_eq!(forward, reverse);
    assert_eq!(forward.ranked_candidates()[0].candidate_id(), "alpha");
    assert_eq!(forward.ranked_candidates()[1].candidate_id(), "beta");
}

//
// Verify hard budget, policy, and risk constraints override raw utility.
//
// Given ineligible high-value actions and an eligible stop proposal,
// when candidates are ranked,
// then no ineligible action should be selected and every rejection reason
// should remain explicit.
#[test]
fn ineligible_actions_cannot_be_selected() {
    let over_budget = candidate(
        "search-everything",
        InvestigationAction::SearchCorpus,
        breakdown(1.0, 1.0, 1.0, 0.0, 0.0, 0.1),
        constraints(false, true, 0.5),
    );
    let policy_denied = candidate(
        "request-restricted-source",
        InvestigationAction::RequestSource,
        breakdown(1.0, 0.9, 0.8, 0.0, 0.0, 0.1),
        constraints(true, false, 0.5),
    );
    let too_risky = candidate(
        "ask-unsafe-source",
        InvestigationAction::RequestSource,
        breakdown(1.0, 0.8, 0.7, 0.0, 0.0, 0.9),
        constraints(true, true, 0.4),
    );
    let stop = candidate(
        "stop",
        InvestigationAction::Stop,
        breakdown(0.0, 0.0, 0.0, 0.0, 0.0, 0.0),
        constraints(true, true, 0.0),
    );

    let ranking = rank(vec![over_budget, policy_denied, too_risky, stop]);

    assert_eq!(
        ranking.selected().map(|selected| selected.action()),
        Some(InvestigationAction::Stop)
    );
    assert_eq!(ranking.ranked_candidates()[0].candidate_id(), "stop");

    let budget_rejection = ranking
        .ranked_candidates()
        .iter()
        .find(|ranked| ranked.candidate_id() == "search-everything")
        .expect("budget candidate should remain explainable");
    assert_eq!(
        budget_rejection.ineligibility_reasons(),
        &[NextBestEvidenceIneligibilityReason::BudgetExceeded]
    );

    let policy_rejection = ranking
        .ranked_candidates()
        .iter()
        .find(|ranked| ranked.candidate_id() == "request-restricted-source")
        .expect("policy candidate should remain explainable");
    assert_eq!(
        policy_rejection.ineligibility_reasons(),
        &[NextBestEvidenceIneligibilityReason::PolicyDenied]
    );

    let risk_rejection = ranking
        .ranked_candidates()
        .iter()
        .find(|ranked| ranked.candidate_id() == "ask-unsafe-source")
        .expect("risk candidate should remain explainable");
    assert!(matches!(
        risk_rejection.ineligibility_reasons(),
        [NextBestEvidenceIneligibilityReason::SourceRiskExceeded {
            observed,
            maximum
        }] if observed.value() == 0.9 && maximum.value() == 0.4
    ));
}

//
// Verify malformed candidate collections fail deterministically.
//
// Given no candidates, a blank identifier, or duplicate identifiers,
// when inputs are constructed or ranked,
// then the API should return its dedicated typed input error.
#[test]
fn malformed_candidate_sets_return_typed_errors() {
    let empty_error =
        rank_next_best_evidence(Vec::new()).expect_err("empty candidate set should fail");
    assert!(matches!(
        empty_error,
        GraphError::InvalidNextBestEvidenceInput(_)
    ));

    let blank_error = NextBestEvidenceCandidateInput::new(
        " ",
        InvestigationAction::Stop,
        breakdown(0.0, 0.0, 0.0, 0.0, 0.0, 0.0),
        constraints(true, true, 0.0),
    )
    .expect_err("blank candidate id should fail");
    assert!(matches!(
        blank_error,
        GraphError::InvalidNextBestEvidenceInput(_)
    ));

    let duplicate = candidate(
        "same-id",
        InvestigationAction::Stop,
        breakdown(0.0, 0.0, 0.0, 0.0, 0.0, 0.0),
        constraints(true, true, 0.0),
    );
    let duplicate_error = rank_next_best_evidence(vec![duplicate.clone(), duplicate])
        .expect_err("duplicate candidate ids should fail");
    assert!(matches!(
        duplicate_error,
        GraphError::InvalidNextBestEvidenceInput(_)
    ));
}

//
// Verify public ranking contracts support deterministic audit persistence.
//
// Given every public input and output type,
// when serde bounds are required,
// then the action proposals and ranking explanations should be serializable
// without introducing an execution boundary.
#[test]
fn ranking_contracts_are_serializable_proposals() {
    fn assert_serializable<T: Serialize + for<'de> Deserialize<'de>>() {}

    assert_serializable::<InvestigationAction>();
    assert_serializable::<NextBestEvidenceProposalScope>();
    assert_serializable::<NextBestEvidenceScoreTerm>();
    assert_serializable::<NextBestEvidenceScoreBreakdown>();
    assert_serializable::<NextBestEvidenceConstraints>();
    assert_serializable::<NextBestEvidenceCandidateInput>();
    assert_serializable::<NextBestEvidenceIneligibilityReason>();
    assert_serializable::<NextBestEvidenceRanking>();
}

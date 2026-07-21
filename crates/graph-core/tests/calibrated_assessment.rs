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
    CalibratedAssessment, CandidateEvidenceOutcome, ClaimId, Confidence, EvidenceId,
    EvidenceSubgraph, GraphError, InformationGainInput, InvestigationAction,
    InvestigationStopConditionDecision, InvestigationStopReason, InvestigationStopThresholds,
    NextBestEvidenceCandidateInput, NextBestEvidenceConstraints, NextBestEvidenceRanking,
    NextBestEvidenceScoreBreakdown, NextBestEvidenceScoreTerm, NodeId, OutcomeProbability,
    RequestId, RetrievalCompleteness, SourceProvenanceRef, StopConditionBudget, UnresolvedUnknown,
    estimate_information_gain, evaluate_investigation_stop_condition, rank_next_best_evidence,
};
use serde::{Deserialize, Serialize};

fn confidence(value: f64) -> Confidence {
    Confidence::new(value).expect("test confidence should be valid")
}

fn completeness(value: f64) -> RetrievalCompleteness {
    RetrievalCompleteness::new(value).expect("test completeness should be valid")
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

fn term(value: f64) -> NextBestEvidenceScoreTerm {
    NextBestEvidenceScoreTerm::new(value).expect("test score term should be valid")
}

fn ranking() -> NextBestEvidenceRanking {
    let score = NextBestEvidenceScoreBreakdown::new(
        term(0.8),
        term(0.7),
        term(0.4),
        term(0.2),
        term(0.1),
        term(0.1),
    );
    let constraints = NextBestEvidenceConstraints::new(true, true, term(0.5));
    let candidate = NextBestEvidenceCandidateInput::new(
        "verify-claim--campaign",
        InvestigationAction::VerifyClaim,
        score,
        constraints,
    )
    .expect("valid candidate");

    rank_next_best_evidence(vec![candidate]).expect("valid ranking")
}

fn stop_thresholds() -> InvestigationStopThresholds {
    InvestigationStopThresholds::new(
        confidence(0.9),
        completeness(0.8),
        0.4,
        StopConditionBudget::new(0.1).expect("valid budget threshold"),
    )
    .expect("valid stop thresholds")
}

fn stop_condition(
    current_confidence: Confidence,
    retrieval_completeness: RetrievalCompleteness,
    counter_evidence: &EvidenceSubgraph,
    unresolved_unknowns: &[UnresolvedUnknown],
    next_best_evidence: &NextBestEvidenceRanking,
) -> graph_core::InvestigationStopCondition {
    evaluate_investigation_stop_condition(
        current_confidence,
        retrieval_completeness,
        counter_evidence,
        unresolved_unknowns,
        StopConditionBudget::new(0.5).expect("valid remaining budget"),
        next_best_evidence,
        stop_thresholds(),
    )
    .expect("valid stop condition")
}

fn assessment(current_confidence: f64, retrieval_completeness: f64) -> CalibratedAssessment {
    let supporting_evidence = EvidenceSubgraph {
        claim_ids: vec![ClaimId::new("claim--coordination").expect("valid claim id")],
        evidence_ids: vec![EvidenceId::new("evidence--support").expect("valid evidence id")],
        ..EvidenceSubgraph::default()
    };
    let counter_evidence = EvidenceSubgraph {
        evidence_ids: vec![EvidenceId::new("evidence--counter").expect("valid evidence id")],
        ..EvidenceSubgraph::default()
    };
    let unresolved_unknowns = vec![UnresolvedUnknown::UnexpandedFrontier {
        node_id: NodeId::new("node--unexpanded").expect("valid node id"),
    }];
    let current_confidence = confidence(current_confidence);
    let retrieval_completeness = completeness(retrieval_completeness);
    let next_best_evidence = ranking();
    let stop_condition = stop_condition(
        current_confidence,
        retrieval_completeness,
        &counter_evidence,
        &unresolved_unknowns,
        &next_best_evidence,
    );

    CalibratedAssessment::new(
        "Is the campaign coordinated?",
        current_confidence,
        supporting_evidence,
        counter_evidence,
        SourceProvenanceRef {
            retrieval_ids: vec![
                RequestId::new("request--investigation").expect("valid request id"),
            ],
            source_refs: vec!["source://analyst-report/42".to_owned()],
        },
        retrieval_completeness,
        unresolved_unknowns,
        information_gain(),
        next_best_evidence,
        stop_condition,
    )
    .expect("valid assessment")
}

//
// Verify an assessment exposes every calibrated and proof-carrying component.
//
// Given supporting and counter-evidence, provenance, uncertainty, and a ranked
// next action,
// when the assessment is constructed,
// then all evidence and action explanations should remain directly auditable.
#[test]
fn assessment_carries_evidence_uncertainty_gain_and_ranked_actions() {
    let assessment = assessment(0.72, 0.65);

    assert_eq!(assessment.question(), "Is the campaign coordinated?");
    assert_eq!(assessment.current_confidence().value(), 0.72);
    assert_eq!(assessment.retrieval_completeness().value(), 0.65);
    assert_eq!(
        assessment.supporting_evidence().evidence_ids[0].as_str(),
        "evidence--support"
    );
    assert_eq!(
        assessment.counter_evidence().evidence_ids[0].as_str(),
        "evidence--counter"
    );
    assert_eq!(
        assessment.source_provenance().retrieval_ids[0].as_str(),
        "request--investigation"
    );
    assert_eq!(assessment.unresolved_unknowns().len(), 1);
    assert!(
        assessment
            .expected_information_gain()
            .expected_information_gain_bits()
            > 0.0
    );

    let selected = assessment
        .next_best_evidence()
        .selected()
        .expect("one eligible proposal");
    assert_eq!(selected.action(), InvestigationAction::VerifyClaim);
    assert_eq!(selected.candidate_id(), "verify-claim--campaign");
    assert_eq!(
        selected.score_breakdown().expected_evidence_gain().value(),
        0.8
    );
    assert!(matches!(
        assessment.stop_condition().decision(),
        InvestigationStopConditionDecision::Continue {
            selected_candidate_id,
            selected_action: InvestigationAction::VerifyClaim,
            ..
        } if selected_candidate_id == "verify-claim--campaign"
    ));
    assert!(!matches!(
        assessment.stop_condition().decision(),
        InvestigationStopConditionDecision::Stop {
            reason: InvestigationStopReason::PolicyRestricted
        }
    ));
    assert_eq!(
        assessment
            .stop_condition()
            .thresholds()
            .minimum_confidence_to_stop()
            .value(),
        0.9
    );
}

//
// Verify confidence and completeness remain independent calibrated signals.
//
// Given confident-but-incomplete and low-confidence-but-complete states,
// when assessments are represented,
// then neither signal should overwrite or derive the other.
#[test]
fn confidence_and_retrieval_completeness_are_independent() {
    let confident_incomplete = assessment(0.95, 0.25);
    let uncertain_complete = assessment(0.30, 1.0);

    assert_eq!(confident_incomplete.current_confidence().value(), 0.95);
    assert_eq!(confident_incomplete.retrieval_completeness().value(), 0.25);
    assert_eq!(uncertain_complete.current_confidence().value(), 0.30);
    assert_eq!(uncertain_complete.retrieval_completeness().value(), 1.0);
}

//
// Verify equality and serialization are deterministic for audit regression.
//
// Given identical ordered inputs,
// when envelopes are constructed and serialized independently,
// then both typed and serialized representations should be identical.
#[test]
fn identical_inputs_produce_identical_assessments_and_serialization() {
    let first = assessment(0.72, 0.65);
    let second = assessment(0.72, 0.65);

    assert_eq!(first, second);
    assert_eq!(
        serde_json::to_string(&first).expect("assessment should serialize"),
        serde_json::to_string(&second).expect("assessment should serialize")
    );
}

//
// Verify malformed questions cannot cross the public assessment boundary.
//
// Given a blank investigation question,
// when an assessment is constructed,
// then a typed calibrated-assessment error should be returned.
#[test]
fn blank_questions_return_a_typed_error() {
    let valid = assessment(0.72, 0.65);
    let error = CalibratedAssessment::new(
        "  ",
        valid.current_confidence(),
        valid.supporting_evidence().clone(),
        valid.counter_evidence().clone(),
        valid.source_provenance().clone(),
        valid.retrieval_completeness(),
        valid.unresolved_unknowns().to_vec(),
        valid.expected_information_gain(),
        valid.next_best_evidence().clone(),
        valid.stop_condition().clone(),
    )
    .expect_err("blank question should fail");

    assert!(matches!(error, GraphError::InvalidCalibratedAssessment(_)));
}

//
// Verify the public envelope remains available at persistence boundaries.
//
// Given the calibrated assessment type,
// when serde bounds are required,
// then it should support serialization and validated deserialization.
#[test]
fn calibrated_assessment_is_serializable() {
    fn assert_serializable<T: Serialize + for<'de> Deserialize<'de>>() {}

    assert_serializable::<CalibratedAssessment>();
}

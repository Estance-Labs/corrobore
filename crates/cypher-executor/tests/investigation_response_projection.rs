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
use cypher_executor::{
    ContractOutcomeStatus, InvestigationContractKind, InvestigationContractOutcome,
    InvestigationProjectedField, InvestigationProjectionSource, InvestigationProjectionValue,
    InvestigationResponseMetadata, project_investigation_response,
};
use cypher_parser::{NormalizedThreshold, ReturnProjection};
use graph_core::{
    CalibratedAssessment, CandidateEvidenceOutcome, ClaimId, Confidence, EvidenceId,
    EvidenceSubgraph, InformationGainInput, InvestigationAction, InvestigationStopThresholds,
    NextBestEvidenceCandidateInput, NextBestEvidenceConstraints, NextBestEvidenceRanking,
    NextBestEvidenceScoreBreakdown, NextBestEvidenceScoreTerm, NodeId, OutcomeProbability,
    RequestId, RetrievalCompleteness, SourceProvenanceRef, StopConditionBudget, UnresolvedUnknown,
    estimate_information_gain, evaluate_investigation_stop_condition, rank_next_best_evidence,
};

fn confidence(value: f64) -> Confidence {
    Confidence::new(value).expect("confidence should be valid")
}

fn completeness(value: f64) -> RetrievalCompleteness {
    RetrievalCompleteness::new(value).expect("completeness should be valid")
}

fn term(value: f64) -> NextBestEvidenceScoreTerm {
    NextBestEvidenceScoreTerm::new(value).expect("score term should be valid")
}

fn ranking() -> NextBestEvidenceRanking {
    let candidate = NextBestEvidenceCandidateInput::new(
        "verify-claim--campaign",
        InvestigationAction::VerifyClaim,
        NextBestEvidenceScoreBreakdown::new(
            term(0.8),
            term(0.7),
            term(0.4),
            term(0.2),
            term(0.1),
            term(0.1),
        ),
        NextBestEvidenceConstraints::new(true, true, term(0.5)),
    )
    .expect("candidate should be valid");
    rank_next_best_evidence(vec![candidate]).expect("ranking should be valid")
}

fn assessment() -> CalibratedAssessment {
    let supporting_evidence = EvidenceSubgraph {
        claim_ids: vec![ClaimId::new("claim--coordination").expect("claim ID should be valid")],
        evidence_ids: vec![
            EvidenceId::new("evidence--support").expect("evidence ID should be valid"),
        ],
        ..EvidenceSubgraph::default()
    };
    let counter_evidence = EvidenceSubgraph {
        evidence_ids: vec![
            EvidenceId::new("evidence--counter").expect("evidence ID should be valid"),
        ],
        ..EvidenceSubgraph::default()
    };
    let unknowns = vec![UnresolvedUnknown::UnexpandedFrontier {
        node_id: NodeId::new("node--unexpanded").expect("node ID should be valid"),
    }];
    let ranking = ranking();
    let current_confidence = confidence(0.75);
    let retrieval_completeness = completeness(0.85);
    let stop_thresholds = InvestigationStopThresholds::new(
        confidence(0.90),
        completeness(0.80),
        0.40,
        StopConditionBudget::new(0.10).expect("budget should be valid"),
    )
    .expect("thresholds should be valid");
    let stop_condition = evaluate_investigation_stop_condition(
        current_confidence,
        retrieval_completeness,
        &counter_evidence,
        &unknowns,
        StopConditionBudget::new(0.50).expect("remaining budget should be valid"),
        &ranking,
        stop_thresholds,
    )
    .expect("stop condition should be valid");
    let gain_input = InformationGainInput::new(
        confidence(0.50),
        vec![
            CandidateEvidenceOutcome::new(
                OutcomeProbability::new(0.5).expect("probability should be valid"),
                confidence(0.1),
            ),
            CandidateEvidenceOutcome::new(
                OutcomeProbability::new(0.5).expect("probability should be valid"),
                confidence(0.9),
            ),
        ],
    )
    .expect("gain input should be valid");

    CalibratedAssessment::new(
        "Is the campaign coordinated?",
        current_confidence,
        supporting_evidence,
        counter_evidence,
        SourceProvenanceRef {
            retrieval_ids: vec![
                RequestId::new("request--investigation").expect("request ID should be valid"),
            ],
            source_refs: vec!["source://analyst-report/42".to_owned()],
        },
        retrieval_completeness,
        unknowns,
        estimate_information_gain(&gain_input),
        ranking,
        stop_condition,
    )
    .expect("assessment should be valid")
}

fn metadata() -> InvestigationResponseMetadata {
    InvestigationResponseMetadata {
        contract_outcomes: vec![InvestigationContractOutcome {
            contract: InvestigationContractKind::EvidenceCompleteness,
            status: ContractOutcomeStatus::Satisfied,
            expected: ">=800000ppm".to_owned(),
            observed: "850000".to_owned(),
        }],
        completeness: Some(
            NormalizedThreshold::from_parts_per_million(850_000)
                .expect("threshold should be valid"),
        ),
        temporal_context: Some("2026-06-01".to_owned()),
        stop_reason: None,
    }
}

#[test]
fn return_contract_projects_exact_requested_fields_with_provenance_and_metadata() {
    let assessment = assessment();
    let expected_provenance = assessment.source_provenance().clone();
    let response = project_investigation_response(
        &[ReturnProjection::ProofGraph, ReturnProjection::Unknowns],
        &InvestigationProjectionSource::from_assessment(assessment),
        metadata(),
    );

    assert_eq!(response.fields.len(), 2);
    assert!(matches!(
        &response.fields[0],
        InvestigationProjectedField::ProofGraph(InvestigationProjectionValue::Available(graph))
            if !graph.is_empty()
    ));
    assert!(matches!(
        &response.fields[1],
        InvestigationProjectedField::Unknowns(InvestigationProjectionValue::Available(unknowns))
            if unknowns.len() == 1
    ));
    assert_eq!(response.provenance, Some(expected_provenance));
    assert_eq!(response.metadata, metadata());
}

#[test]
fn all_return_variants_project_typed_assessment_evidence_unknowns_and_next_action() {
    let response = project_investigation_response(
        &[
            ReturnProjection::Assessment,
            ReturnProjection::ProofGraph,
            ReturnProjection::CounterEvidence,
            ReturnProjection::Unknowns,
            ReturnProjection::NextBestEvidence,
        ],
        &InvestigationProjectionSource::from_assessment(assessment()),
        metadata(),
    );

    assert!(matches!(
        &response.fields[0],
        InvestigationProjectedField::Assessment(InvestigationProjectionValue::Available(_))
    ));
    assert!(matches!(
        &response.fields[2],
        InvestigationProjectedField::CounterEvidence(
            InvestigationProjectionValue::Available(graph)
        ) if !graph.is_empty()
    ));
    assert!(matches!(
        &response.fields[4],
        InvestigationProjectedField::NextBestEvidence(
            InvestigationProjectionValue::Available(ranking)
        ) if ranking.selected().is_some()
    ));
}

#[test]
fn unavailable_requested_projection_is_explicit_and_byte_stable() {
    let first = project_investigation_response(
        &[ReturnProjection::Assessment, ReturnProjection::ProofGraph],
        &InvestigationProjectionSource::default(),
        metadata(),
    );
    let second = project_investigation_response(
        &[ReturnProjection::Assessment, ReturnProjection::ProofGraph],
        &InvestigationProjectionSource::default(),
        metadata(),
    );

    assert!(first.fields.iter().all(|field| field.is_unavailable()));
    assert_eq!(first, second);
    assert_eq!(
        serde_json::to_string(&first).expect("first response should serialize"),
        serde_json::to_string(&second).expect("second response should serialize")
    );
}

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
    ContractOutcomeStatus, InvestigationContractError, InvestigationContractReport,
    InvestigationExecutionObservation, InvestigationProjectedField, InvestigationProjectionSource,
    InvestigationResourceUsage, InvestigationResponse, InvestigationResponseMetadata,
    InvestigationStopReason, enforce_investigation_contracts, project_investigation_response,
};
use cypher_parser::{NormalizedThreshold, parse_investigation_query};
use cypher_planner::{InvestigationPlannerCapabilities, compile_investigation_plan};
use graph_core::{
    CalibratedAssessment, CandidateEvidenceOutcome, ClaimId, Confidence, EvidenceId,
    EvidenceSubgraph, InformationGainInput, InvestigationAction, InvestigationStopThresholds,
    NextBestEvidenceCandidateInput, NextBestEvidenceConstraints, NextBestEvidenceScoreBreakdown,
    NextBestEvidenceScoreTerm, NodeId, OutcomeProbability, RequestId, RetrievalCompleteness,
    SourceProvenanceRef, StopConditionBudget, UnresolvedUnknown, estimate_information_gain,
    evaluate_investigation_stop_condition, rank_next_best_evidence,
};

const FIMI_QUERY: &str = r#"
    INVESTIGATE attribution OF Campaign("campaign--fimi")
    AT TIME 2026-06-01
    REQUIRE independent_sources >= 2, source_reliability >= 0.70, evidence_completeness >= 0.80
    ALLOW hypotheses = true, contradictory_evidence = true
    BUDGET memory = 256 MB, latency = 3 s, external_retrievals = 4
    RETURN assessment, proof_graph, counter_evidence, unknowns, next_best_evidence
"#;

const CTI_QUERY: &str = r#"
    INVESTIGATE attribution OF Actor("actor--cti")
    AT TIME 2026-06-02
    REQUIRE independent_sources >= 3, source_reliability >= 0.75, evidence_completeness >= 0.85
    ALLOW hypotheses = false, contradictory_evidence = true
    BUDGET memory = 128 MB, latency = 2 s, external_retrievals = 2
    RETURN assessment, unknowns
"#;

fn threshold(parts_per_million: u32) -> NormalizedThreshold {
    NormalizedThreshold::from_parts_per_million(parts_per_million)
        .expect("test threshold should be valid")
}

fn confidence(value: f64) -> Confidence {
    Confidence::new(value).expect("confidence should be valid")
}

fn completeness(value: f64) -> RetrievalCompleteness {
    RetrievalCompleteness::new(value).expect("completeness should be valid")
}

fn term(value: f64) -> NextBestEvidenceScoreTerm {
    NextBestEvidenceScoreTerm::new(value).expect("score term should be valid")
}

fn assessment() -> CalibratedAssessment {
    let supporting_evidence = EvidenceSubgraph {
        claim_ids: vec![ClaimId::new("claim--campaign").expect("claim ID should be valid")],
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
    let ranking = rank_next_best_evidence(vec![candidate]).expect("ranking should be valid");
    let current_confidence = confidence(0.75);
    let retrieval_completeness = completeness(0.85);
    let stop_condition = evaluate_investigation_stop_condition(
        current_confidence,
        retrieval_completeness,
        &counter_evidence,
        &unknowns,
        StopConditionBudget::new(0.50).expect("remaining budget should be valid"),
        &ranking,
        InvestigationStopThresholds::new(
            confidence(0.90),
            completeness(0.80),
            0.40,
            StopConditionBudget::new(0.10).expect("budget should be valid"),
        )
        .expect("thresholds should be valid"),
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
        "Is the activity coordinated?",
        current_confidence,
        supporting_evidence,
        counter_evidence,
        SourceProvenanceRef {
            retrieval_ids: vec![
                RequestId::new("request--acceptance").expect("request ID should be valid"),
            ],
            source_refs: vec!["source://acceptance/1".to_owned()],
        },
        retrieval_completeness,
        unknowns,
        estimate_information_gain(&gain_input),
        ranking,
        stop_condition,
    )
    .expect("assessment should be valid")
}

fn fimi_observation() -> InvestigationExecutionObservation {
    InvestigationExecutionObservation {
        resolved_at_time: Some("2026-06-01".to_owned()),
        independent_sources: 3,
        minimum_source_reliability: Some(threshold(750_000)),
        evidence_completeness: Some(threshold(850_000)),
        contains_hypotheses: true,
        contains_contradictory_evidence: true,
        resources: InvestigationResourceUsage {
            memory_bytes: 200 * 1024 * 1024,
            elapsed_millis: 2_500,
            external_retrievals: 3,
        },
    }
}

fn cti_observation() -> InvestigationExecutionObservation {
    InvestigationExecutionObservation {
        resolved_at_time: Some("2026-06-02".to_owned()),
        independent_sources: 4,
        minimum_source_reliability: Some(threshold(800_000)),
        evidence_completeness: Some(threshold(900_000)),
        contains_hypotheses: false,
        contains_contradictory_evidence: true,
        resources: InvestigationResourceUsage {
            memory_bytes: 96 * 1024 * 1024,
            elapsed_millis: 1_500,
            external_retrievals: 2,
        },
    }
}

struct AcceptanceResult {
    report: InvestigationContractReport,
    response: InvestigationResponse,
    plan_bytes: String,
    contract_bytes: String,
    response_bytes: String,
}

fn run_acceptance_scenario(
    query: &str,
    observation: InvestigationExecutionObservation,
) -> AcceptanceResult {
    let ast = parse_investigation_query(query).expect("acceptance query should parse");
    let plan = compile_investigation_plan(&ast, &InvestigationPlannerCapabilities::all())
        .expect("acceptance query should compile");
    let report = enforce_investigation_contracts(&plan.physical, &observation)
        .expect("acceptance observation should satisfy every contract");
    let response = project_investigation_response(
        &ast.returns,
        &InvestigationProjectionSource::from_assessment(assessment()),
        InvestigationResponseMetadata {
            contract_outcomes: report.outcomes.clone(),
            completeness: observation.evidence_completeness,
            temporal_context: observation.resolved_at_time,
            stop_reason: None,
        },
    );
    let plan_bytes = plan.to_canonical_string();
    let contract_bytes = report.to_canonical_string();
    let response_bytes =
        serde_json::to_string(&response).expect("acceptance response should serialize");

    AcceptanceResult {
        report,
        response,
        plan_bytes,
        contract_bytes,
        response_bytes,
    }
}

fn run_failing_acceptance_scenario(
    query: &str,
    observation: InvestigationExecutionObservation,
) -> InvestigationContractError {
    let ast = parse_investigation_query(query).expect("acceptance query should parse");
    let plan = compile_investigation_plan(&ast, &InvestigationPlannerCapabilities::all())
        .expect("acceptance query should compile");
    enforce_investigation_contracts(&plan.physical, &observation)
        .expect_err("failing observation must retain typed violations")
}

#[test]
fn fimi_scenario_is_auditable_across_parse_plan_enforcement_and_all_projections() {
    let first = run_acceptance_scenario(FIMI_QUERY, fimi_observation());
    let second = run_acceptance_scenario(FIMI_QUERY, fimi_observation());

    assert_eq!(first.plan_bytes, second.plan_bytes);
    assert_eq!(first.contract_bytes, second.contract_bytes);
    assert_eq!(first.response_bytes, second.response_bytes);
    assert_eq!(first.response.fields.len(), 5);
    assert!(
        first
            .response
            .fields
            .iter()
            .all(|field| !field.is_unavailable())
    );
    assert!(
        first
            .report
            .outcomes
            .iter()
            .all(|outcome| outcome.status == ContractOutcomeStatus::Satisfied)
    );
}

#[test]
fn cti_scenario_projects_exact_selection_and_surfaces_typed_contract_failure() {
    let success = run_acceptance_scenario(CTI_QUERY, cti_observation());

    assert_eq!(success.response.fields.len(), 2);
    assert!(matches!(
        success.response.fields[0],
        InvestigationProjectedField::Assessment(_)
    ));
    assert!(matches!(
        success.response.fields[1],
        InvestigationProjectedField::Unknowns(_)
    ));

    let mut failing = cti_observation();
    failing.contains_hypotheses = true;
    failing.resources.memory_bytes = 256 * 1024 * 1024;
    let error = run_failing_acceptance_scenario(CTI_QUERY, failing);

    assert_eq!(
        error.stop_reason,
        InvestigationStopReason::MemoryBudgetExceeded
    );
    assert_eq!(error.violations.len(), 2);
}

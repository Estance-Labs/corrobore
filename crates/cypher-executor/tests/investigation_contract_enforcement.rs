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
    ContractOutcomeStatus, InvestigationContractKind, InvestigationContractViolationCode,
    InvestigationExecutionObservation, InvestigationResourceUsage, InvestigationStopReason,
    enforce_investigation_contracts,
};
use cypher_parser::{NormalizedThreshold, parse_investigation_query};
use cypher_planner::{
    InvestigationPlan, InvestigationPlannerCapabilities, compile_investigation_plan,
};

const COMPLETE_QUERY: &str = r#"
    INVESTIGATE attribution OF Campaign("C-42")
    AT TIME 2026-06-01
    REQUIRE independent_sources >= 2, source_reliability >= 0.70, evidence_completeness >= 0.80
    ALLOW hypotheses = true, contradictory_evidence = true
    BUDGET memory = 256 MB, latency = 3 s, external_retrievals = 4
    RETURN assessment, proof_graph, counter_evidence, unknowns, next_best_evidence
"#;

fn compile(query: &str) -> InvestigationPlan {
    let ast = parse_investigation_query(query).expect("investigation query should parse");
    compile_investigation_plan(&ast, &InvestigationPlannerCapabilities::all())
        .expect("investigation query should compile")
}

fn threshold(parts_per_million: u32) -> NormalizedThreshold {
    NormalizedThreshold::from_parts_per_million(parts_per_million)
        .expect("test threshold should be valid")
}

fn passing_observation() -> InvestigationExecutionObservation {
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

#[test]
fn successful_enforcement_proves_every_declared_contract_was_satisfied() {
    let plan = compile(COMPLETE_QUERY);
    let report = enforce_investigation_contracts(&plan.physical, &passing_observation())
        .expect("all contracts should be satisfied");

    assert_eq!(
        report
            .outcomes
            .iter()
            .map(|outcome| outcome.contract)
            .collect::<Vec<_>>(),
        vec![
            InvestigationContractKind::TemporalSnapshot,
            InvestigationContractKind::IndependentSources,
            InvestigationContractKind::SourceReliability,
            InvestigationContractKind::MemoryBudget,
            InvestigationContractKind::LatencyBudget,
            InvestigationContractKind::ExternalRetrievalBudget,
            InvestigationContractKind::HypothesesAllowance,
            InvestigationContractKind::ContradictoryEvidenceAllowance,
            InvestigationContractKind::EvidenceCompleteness,
        ]
    );
    assert!(
        report
            .outcomes
            .iter()
            .all(|outcome| outcome.status == ContractOutcomeStatus::Satisfied)
    );
    assert_eq!(report.evaluated_stage_kinds, plan.physical.stage_kinds());
}

#[test]
fn unsatisfied_hard_requirements_fail_with_all_typed_violations() {
    let plan = compile(COMPLETE_QUERY);
    let mut observation = passing_observation();
    observation.independent_sources = 1;
    observation.minimum_source_reliability = Some(threshold(600_000));
    observation.evidence_completeness = Some(threshold(700_000));

    let error = enforce_investigation_contracts(&plan.physical, &observation)
        .expect_err("hard requirements must not degrade to success");

    assert_eq!(
        error
            .violations
            .iter()
            .map(|violation| violation.code)
            .collect::<Vec<_>>(),
        vec![
            InvestigationContractViolationCode::IndependentSourcesUnsatisfied,
            InvestigationContractViolationCode::SourceReliabilityUnsatisfied,
            InvestigationContractViolationCode::EvidenceCompletenessUnsatisfied,
        ]
    );
    assert_eq!(
        error.stop_reason,
        InvestigationStopReason::EvidenceRequirementUnsatisfied
    );
    assert_eq!(error.outcomes.len(), 9);
}

#[test]
fn undeclared_allowances_use_safe_defaults_and_never_silently_broaden() {
    let plan = compile(r#"INVESTIGATE attribution OF Actor("A-7") RETURN assessment"#);
    let observation = InvestigationExecutionObservation {
        contains_hypotheses: true,
        contains_contradictory_evidence: true,
        ..InvestigationExecutionObservation::default()
    };

    let error = enforce_investigation_contracts(&plan.physical, &observation)
        .expect_err("undeclared allowances should default to false");

    assert_eq!(
        error
            .violations
            .iter()
            .map(|violation| violation.code)
            .collect::<Vec<_>>(),
        vec![
            InvestigationContractViolationCode::HypothesesNotAllowed,
            InvestigationContractViolationCode::ContradictoryEvidenceNotAllowed,
        ]
    );
    assert_eq!(
        error.stop_reason,
        InvestigationStopReason::AllowanceViolation
    );
}

#[test]
fn exceeded_budgets_report_all_violations_and_deterministic_stop_reason() {
    let plan = compile(COMPLETE_QUERY);
    let mut observation = passing_observation();
    observation.resources = InvestigationResourceUsage {
        memory_bytes: 300 * 1024 * 1024,
        elapsed_millis: 4_000,
        external_retrievals: 5,
    };

    let error = enforce_investigation_contracts(&plan.physical, &observation)
        .expect_err("exceeded budgets should stop execution");

    assert_eq!(
        error
            .violations
            .iter()
            .map(|violation| violation.code)
            .collect::<Vec<_>>(),
        vec![
            InvestigationContractViolationCode::MemoryBudgetExceeded,
            InvestigationContractViolationCode::LatencyBudgetExceeded,
            InvestigationContractViolationCode::ExternalRetrievalBudgetExceeded,
        ]
    );
    assert_eq!(
        error.stop_reason,
        InvestigationStopReason::MemoryBudgetExceeded
    );
}

#[test]
fn unavailable_or_mismatched_temporal_snapshot_is_rejected() {
    let plan = compile(COMPLETE_QUERY);

    let mut unavailable = passing_observation();
    unavailable.resolved_at_time = None;
    let unavailable_error = enforce_investigation_contracts(&plan.physical, &unavailable)
        .expect_err("missing temporal snapshot should fail");
    assert_eq!(
        unavailable_error.violations[0].code,
        InvestigationContractViolationCode::TemporalSnapshotUnavailable
    );
    assert_eq!(
        unavailable_error.stop_reason,
        InvestigationStopReason::TemporalSnapshotUnavailable
    );

    let mut mismatched = passing_observation();
    mismatched.resolved_at_time = Some("2026-05-31".to_owned());
    let mismatched_error = enforce_investigation_contracts(&plan.physical, &mismatched)
        .expect_err("mismatched temporal snapshot should fail");
    assert_eq!(
        mismatched_error.violations[0].code,
        InvestigationContractViolationCode::TemporalSnapshotMismatch
    );
    assert_eq!(
        mismatched_error.stop_reason,
        InvestigationStopReason::TemporalSnapshotUnavailable
    );
}

#[test]
fn enforcement_reports_and_failures_are_byte_stable() {
    let plan = compile(COMPLETE_QUERY);
    let first = enforce_investigation_contracts(&plan.physical, &passing_observation())
        .expect("first execution should satisfy contracts");
    let second = enforce_investigation_contracts(&plan.physical, &passing_observation())
        .expect("second execution should satisfy contracts");
    assert_eq!(first, second);
    assert_eq!(first.to_canonical_string(), second.to_canonical_string());

    let mut failing = passing_observation();
    failing.independent_sources = 0;
    let first_error = enforce_investigation_contracts(&plan.physical, &failing)
        .expect_err("first execution should fail");
    let second_error = enforce_investigation_contracts(&plan.physical, &failing)
        .expect_err("second execution should fail");
    assert_eq!(first_error, second_error);
    assert_eq!(
        first_error.to_canonical_string(),
        second_error.to_canonical_string()
    );
}

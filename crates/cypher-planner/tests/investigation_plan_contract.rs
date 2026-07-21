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
use cypher_parser::{
    InvestigationIntent, InvestigationTargetKind, ReturnProjection, parse_investigation_query,
};
use cypher_planner::{
    InvestigationLogicalStage, InvestigationPhysicalStage, InvestigationPlanErrorCode,
    InvestigationPlannerCapabilities, PhysicalStageKind, PlannerCapability,
    compile_investigation_plan,
};

const COMPLETE_QUERY: &str = r#"
    INVESTIGATE attribution OF Campaign("C-42")
    AT TIME 2026-06-01
    REQUIRE independent_sources >= 2, source_reliability >= 0.70, evidence_completeness >= 0.80
    ALLOW hypotheses = true, contradictory_evidence = true
    BUDGET memory = 256 MB, latency = 3 s, external_retrievals = 4
    RETURN assessment, proof_graph, counter_evidence, unknowns, next_best_evidence
"#;

#[test]
fn compiles_complete_intent_into_ordered_logical_and_physical_stages() {
    let query =
        parse_investigation_query(COMPLETE_QUERY).expect("investigation query should parse");
    let plan = compile_investigation_plan(&query, &InvestigationPlannerCapabilities::all())
        .expect("complete query should compile");

    assert_eq!(
        plan.logical.stages,
        vec![
            InvestigationLogicalStage::SeedSelection,
            InvestigationLogicalStage::WorkingSetConstruction,
            InvestigationLogicalStage::TemporalFiltering,
            InvestigationLogicalStage::EvidenceFiltering,
            InvestigationLogicalStage::BudgetEnforcement,
            InvestigationLogicalStage::EvidenceTraversal,
            InvestigationLogicalStage::EvidenceArbitration,
            InvestigationLogicalStage::CompletenessVerification,
            InvestigationLogicalStage::ResponseProjection,
        ]
    );
    assert_eq!(
        plan.physical.stage_kinds(),
        vec![
            PhysicalStageKind::SeedSelection,
            PhysicalStageKind::WorkingSetConstruction,
            PhysicalStageKind::TemporalFilter,
            PhysicalStageKind::IndependentSourceFilter,
            PhysicalStageKind::SourceReliabilityFilter,
            PhysicalStageKind::BudgetGuard,
            PhysicalStageKind::EvidenceTraversal,
            PhysicalStageKind::EvidenceArbitration,
            PhysicalStageKind::CompletenessVerification,
            PhysicalStageKind::ResponseProjection,
        ]
    );
}

#[test]
fn physical_stages_retain_every_declared_contract_exactly() {
    let query =
        parse_investigation_query(COMPLETE_QUERY).expect("investigation query should parse");
    let plan = compile_investigation_plan(&query, &InvestigationPlannerCapabilities::all())
        .expect("complete query should compile");

    assert!(matches!(
        &plan.physical.stages[0],
        InvestigationPhysicalStage::SeedSelection {
            intent: InvestigationIntent::Attribution,
            target_kind: InvestigationTargetKind::Campaign,
            target_identifier,
        } if target_identifier == "C-42"
    ));
    assert!(matches!(
        &plan.physical.stages[2],
        InvestigationPhysicalStage::TemporalFilter { at_time }
            if at_time == "2026-06-01"
    ));
    assert!(matches!(
        plan.physical.stages[3],
        InvestigationPhysicalStage::IndependentSourceFilter { minimum: 2 }
    ));
    assert!(matches!(
        plan.physical.stages[4],
        InvestigationPhysicalStage::SourceReliabilityFilter {
            minimum_parts_per_million: 700_000
        }
    ));
    assert!(matches!(
        plan.physical.stages[5],
        InvestigationPhysicalStage::BudgetGuard {
            memory_bytes: Some(268_435_456),
            latency_millis: Some(3_000),
            external_retrievals: Some(4),
        }
    ));
    assert!(matches!(
        plan.physical.stages[7],
        InvestigationPhysicalStage::EvidenceArbitration {
            allow_hypotheses: Some(true),
            allow_contradictory_evidence: Some(true),
        }
    ));
    assert!(matches!(
        plan.physical.stages[8],
        InvestigationPhysicalStage::CompletenessVerification {
            minimum_parts_per_million: Some(800_000)
        }
    ));
    assert!(matches!(
        &plan.physical.stages[9],
        InvestigationPhysicalStage::ResponseProjection { projections }
            if projections == &vec![
                ReturnProjection::Assessment,
                ReturnProjection::ProofGraph,
                ReturnProjection::CounterEvidence,
                ReturnProjection::Unknowns,
                ReturnProjection::NextBestEvidence,
            ]
    ));
}

#[test]
fn minimal_intent_still_compiles_into_complete_execution_pipeline() {
    let query =
        parse_investigation_query(r#"INVESTIGATE attribution OF Actor("A-7") RETURN assessment"#)
            .expect("minimal investigation query should parse");
    let plan = compile_investigation_plan(&query, &InvestigationPlannerCapabilities::all())
        .expect("minimal query should compile");

    assert_eq!(
        plan.physical.stage_kinds(),
        vec![
            PhysicalStageKind::SeedSelection,
            PhysicalStageKind::WorkingSetConstruction,
            PhysicalStageKind::EvidenceTraversal,
            PhysicalStageKind::EvidenceArbitration,
            PhysicalStageKind::CompletenessVerification,
            PhysicalStageKind::ResponseProjection,
        ]
    );
    assert!(matches!(
        plan.physical.stages[3],
        InvestigationPhysicalStage::EvidenceArbitration {
            allow_hypotheses: None,
            allow_contradictory_evidence: None,
        }
    ));
    assert!(matches!(
        plan.physical.stages[4],
        InvestigationPhysicalStage::CompletenessVerification {
            minimum_parts_per_million: None
        }
    ));
}

#[test]
fn unavailable_engine_capability_is_rejected_explicitly() {
    let query =
        parse_investigation_query(COMPLETE_QUERY).expect("investigation query should parse");

    for capability in [
        PlannerCapability::SeedSelection,
        PlannerCapability::WorkingSetConstruction,
        PlannerCapability::TemporalFiltering,
        PlannerCapability::TrustFiltering,
        PlannerCapability::BudgetEnforcement,
        PlannerCapability::EvidenceTraversal,
        PlannerCapability::EvidenceArbitration,
        PlannerCapability::CompletenessVerification,
        PlannerCapability::ResponseProjection,
    ] {
        let capabilities = InvestigationPlannerCapabilities::all().without(capability);
        let error = compile_investigation_plan(&query, &capabilities)
            .expect_err("missing required capability should fail");

        assert_eq!(
            error.code,
            InvestigationPlanErrorCode::UnsupportedCapability
        );
        assert_eq!(error.capability, Some(capability));
        assert!(error.message.contains(capability.canonical_name()));
    }
}

#[test]
fn normalized_equivalent_intents_produce_byte_stable_plans_and_explanations() {
    let first =
        parse_investigation_query(COMPLETE_QUERY).expect("investigation query should parse");
    let reordered = parse_investigation_query(
        r#"INVESTIGATE attribution OF Campaign("C-42")
           RETURN unknowns, assessment, next_best_evidence, proof_graph, counter_evidence
           ALLOW contradictory_evidence = true, hypotheses = true
           REQUIRE evidence_completeness >= .8, source_reliability >= .7, independent_sources >= 2
           BUDGET latency = 3000 ms, external_retrievals = 4, memory = 262144 KB
           AT TIME 2026-06-01"#,
    )
    .expect("equivalent query should parse");
    let capabilities = InvestigationPlannerCapabilities::all();

    let first_plan =
        compile_investigation_plan(&first, &capabilities).expect("first query should compile");
    let second_plan =
        compile_investigation_plan(&reordered, &capabilities).expect("second query should compile");

    assert_eq!(first_plan, second_plan);
    assert_eq!(
        first_plan.to_canonical_string(),
        second_plan.to_canonical_string()
    );
    assert_eq!(
        first_plan.explanations.len(),
        first_plan.physical.stages.len()
    );
    assert!(
        first_plan
            .explanations
            .iter()
            .enumerate()
            .all(|(index, explanation)| explanation.stage_index == index)
    );
}

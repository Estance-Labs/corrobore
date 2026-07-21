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
//! Epic 0021 acceptance suite: hypothetical branchable worlds.
//!
//! Exercises deterministic FIMI and CTI scenarios through shared immutable
//! facts, isolated overlays, explained comparison, counterfactual evidence
//! queries, and explicit audited merge/discard resolution.

use graph_core::{
    ActorId, BranchCreationInput, BranchEvidenceObservation, BranchId, BranchObservationAssessment,
    BranchObservationEffect, BranchOverlayReference, BranchPrediction, BranchPredictionId,
    BranchResolutionAuditMetadata, BranchResolutionDecision, BranchResolutionDecisionId,
    BranchResolutionKind, BranchResolutionLedger, BranchSelector, BranchStatus,
    BranchValidationAuditMetadata, CrossBranchScoreBreakdown, CrossBranchScoreInput,
    CrossBranchScoreTerm, EvidenceId, FactId, HypothesisWorldModel, OverlayHypothesis,
    OverlayHypothesisId, TemporalTimestamp, WorldId, query_counterfactual_expected_facts,
    query_discriminating_observations, query_smallest_disproving_evidence,
    rank_cross_branch_scores,
};

const EPSILON: f64 = 1e-12;

/// Complete deterministic input and expected observations for one domain.
///
/// Public Epic 0021 contracts populate the fixture: shared facts and branch
/// overlays form the model, score inputs drive comparison, evidence observations
/// drive counterfactual queries, and explicit decisions drive audited
/// resolution.
#[derive(Clone, Debug, PartialEq)]
struct Epic0021Scenario {
    model: HypothesisWorldModel,
    winner: BranchSelector,
    alternative: BranchSelector,
    score_inputs: Vec<CrossBranchScoreInput>,
    expected_winner_score: (f64, f64, f64),
    observations: Vec<BranchEvidenceObservation>,
    expected_winner_evidence: Vec<EvidenceId>,
    winner_counter_evidence: EvidenceId,
    discriminating_evidence: Vec<EvidenceId>,
    alternative_disproof: EvidenceId,
    merge_decision: BranchResolutionDecision,
    discard_decision: BranchResolutionDecision,
}

/// Declares the FIMI competing-explanation scenario.
///
/// Models coordinated-campaign and organic-convergence worlds over identical
/// confirmed observations, with domain-specific overlays, evidence, scores, and
/// audited terminal decisions.
fn fimi_scenario() -> Epic0021Scenario {
    build_scenario(ScenarioSpec {
        domain: "fimi",
        winner_slug: "coordinated",
        alternative_slug: "organic",
        winner_title: "Coordinated influence campaign",
        alternative_title: "Organic narrative convergence",
        winner_hypothesis: "One operator coordinates the observed narratives",
        alternative_hypothesis: "Independent communities converged organically",
        winner_prediction: "The same command schedule will recur across channels",
        alternative_prediction: "Posting schedules will diverge by community",
        winner_evidence_slug: "command-schedule",
        winner_evidence_description: "Channels reuse a private command schedule",
        alternative_evidence_slug: "organic-origin",
        alternative_evidence_description: "Independent origin records predate coordination",
        shared_evidence_description: "The same narrative appears across channels",
        winner_score: (0.9, 0.8, 0.1),
        alternative_score: (0.5, 0.4, 0.2),
    })
}

/// Declares the CTI competing-attribution scenario.
///
/// Models actor-attribution and commodity-malware worlds over the same
/// immutable telemetry, with domain-specific overlay conclusions, evidence
/// effects, scores, and audited terminal decisions.
fn cti_scenario() -> Epic0021Scenario {
    build_scenario(ScenarioSpec {
        domain: "cti",
        winner_slug: "actor-north",
        alternative_slug: "commodity-malware",
        winner_title: "Actor North intrusion",
        alternative_title: "Commodity malware intrusion",
        winner_hypothesis: "Actor North operated the observed infrastructure",
        alternative_hypothesis: "Commodity malware operators reused public tooling",
        winner_prediction: "A private signing key will link the next payload",
        alternative_prediction: "Future payloads will use public builder defaults",
        winner_evidence_slug: "private-key",
        winner_evidence_description: "Payloads share Actor North's private signing key",
        alternative_evidence_slug: "public-builder",
        alternative_evidence_description: "The sample matches public builder defaults",
        shared_evidence_description: "The same malware sample reached both sensors",
        winner_score: (0.85, 0.75, 0.05),
        alternative_score: (0.45, 0.5, 0.15),
    })
}

/// Applies the fixture's explicit merge and discard decisions.
///
/// Routes both decisions through [`BranchResolutionLedger`] so tests validate
/// the real validation, promotion, history, and lifecycle boundaries instead
/// of constructing expected terminal state directly.
fn resolve_scenario(scenario: Epic0021Scenario) -> BranchResolutionLedger {
    BranchResolutionLedger::new(scenario.model)
        .apply_decision(scenario.merge_decision)
        .expect("validated winner merge should succeed")
        .apply_decision(scenario.discard_decision)
        .expect("audited alternative discard should succeed")
}

/// Domain-specific values used to seed the shared acceptance pipeline.
struct ScenarioSpec {
    domain: &'static str,
    winner_slug: &'static str,
    alternative_slug: &'static str,
    winner_title: &'static str,
    alternative_title: &'static str,
    winner_hypothesis: &'static str,
    alternative_hypothesis: &'static str,
    winner_prediction: &'static str,
    alternative_prediction: &'static str,
    winner_evidence_slug: &'static str,
    winner_evidence_description: &'static str,
    alternative_evidence_slug: &'static str,
    alternative_evidence_description: &'static str,
    shared_evidence_description: &'static str,
    winner_score: (f64, f64, f64),
    alternative_score: (f64, f64, f64),
}

fn build_scenario(spec: ScenarioSpec) -> Epic0021Scenario {
    let winner = selector(spec.domain, spec.winner_slug);
    let alternative = selector(spec.domain, spec.alternative_slug);
    let winner_hypothesis_id = hypothesis_id(spec.domain, spec.winner_slug);
    let alternative_hypothesis_id = hypothesis_id(spec.domain, spec.alternative_slug);
    let winner_prediction_id = prediction_id(spec.domain, spec.winner_slug);
    let alternative_prediction_id = prediction_id(spec.domain, spec.alternative_slug);

    let model = HypothesisWorldModel::new(vec![
        fact_id(&format!("fact--{}-shared-a", spec.domain)),
        fact_id(&format!("fact--{}-shared-b", spec.domain)),
    ])
    .expect("scenario world model should be valid")
    .create_world(winner.world_id().clone(), spec.winner_title.to_owned())
    .expect("winner world should be created")
    .create_branch(
        winner.world_id(),
        BranchCreationInput::new(winner.branch_id().clone(), "Primary explanation".to_owned()),
    )
    .expect("winner branch should be created")
    .add_branch_hypothesis(
        winner.world_id(),
        winner.branch_id(),
        OverlayHypothesis::new(winner_hypothesis_id.clone(), spec.winner_hypothesis)
            .expect("winner hypothesis should be valid"),
    )
    .expect("winner hypothesis should be added")
    .add_branch_prediction(
        winner.world_id(),
        winner.branch_id(),
        BranchPrediction::new(winner_prediction_id.clone(), spec.winner_prediction)
            .expect("winner prediction should be valid"),
    )
    .expect("winner prediction should be added")
    .create_world(
        alternative.world_id().clone(),
        spec.alternative_title.to_owned(),
    )
    .expect("alternative world should be created")
    .create_branch(
        alternative.world_id(),
        BranchCreationInput::new(
            alternative.branch_id().clone(),
            "Competing explanation".to_owned(),
        ),
    )
    .expect("alternative branch should be created")
    .add_branch_hypothesis(
        alternative.world_id(),
        alternative.branch_id(),
        OverlayHypothesis::new(alternative_hypothesis_id, spec.alternative_hypothesis)
            .expect("alternative hypothesis should be valid"),
    )
    .expect("alternative hypothesis should be added")
    .add_branch_prediction(
        alternative.world_id(),
        alternative.branch_id(),
        BranchPrediction::new(alternative_prediction_id, spec.alternative_prediction)
            .expect("alternative prediction should be valid"),
    )
    .expect("alternative prediction should be added");

    let shared_evidence = evidence_id(&format!("evidence--{}-shared", spec.domain));
    let winner_evidence = evidence_id(&format!(
        "evidence--{}-{}",
        spec.domain, spec.winner_evidence_slug
    ));
    let alternative_evidence = evidence_id(&format!(
        "evidence--{}-{}",
        spec.domain, spec.alternative_evidence_slug
    ));
    let observations = vec![
        observation(
            shared_evidence.clone(),
            &format!("source://{}/shared", spec.domain),
            spec.shared_evidence_description,
            vec![
                assessment(winner.clone(), BranchObservationEffect::Expected),
                assessment(alternative.clone(), BranchObservationEffect::Expected),
            ],
        ),
        observation(
            winner_evidence.clone(),
            &format!("source://{}/winner", spec.domain),
            spec.winner_evidence_description,
            vec![
                assessment(winner.clone(), BranchObservationEffect::Expected),
                assessment(alternative.clone(), BranchObservationEffect::Contradicts),
            ],
        ),
        observation(
            alternative_evidence.clone(),
            &format!("source://{}/alternative", spec.domain),
            spec.alternative_evidence_description,
            vec![
                assessment(winner.clone(), BranchObservationEffect::Contradicts),
                assessment(alternative.clone(), BranchObservationEffect::Expected),
            ],
        ),
    ];

    let mut expected_winner_evidence = vec![shared_evidence, winner_evidence.clone()];
    expected_winner_evidence.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    let mut discriminating_evidence = vec![winner_evidence.clone(), alternative_evidence.clone()];
    discriminating_evidence.sort_by(|left, right| left.as_str().cmp(right.as_str()));

    Epic0021Scenario {
        model,
        winner: winner.clone(),
        alternative: alternative.clone(),
        score_inputs: vec![
            score_input(alternative.clone(), spec.alternative_score),
            score_input(winner.clone(), spec.winner_score),
        ],
        expected_winner_score: spec.winner_score,
        observations,
        expected_winner_evidence,
        winner_counter_evidence: alternative_evidence,
        discriminating_evidence,
        alternative_disproof: winner_evidence,
        merge_decision: BranchResolutionDecision::new(
            decision_id(spec.domain, "merge"),
            winner,
            BranchResolutionKind::Merge,
            audit_metadata(spec.domain, "Winner selected after domain review"),
        )
        .with_validation(validation_metadata(spec.domain))
        .with_promoted_reference(BranchOverlayReference::Prediction(winner_prediction_id))
        .with_promoted_reference(BranchOverlayReference::Hypothesis(winner_hypothesis_id)),
        discard_decision: BranchResolutionDecision::new(
            decision_id(spec.domain, "discard"),
            alternative,
            BranchResolutionKind::Discard,
            audit_metadata(spec.domain, "Alternative rejected after comparative review"),
        ),
    }
}

#[test]
fn acceptance_fimi_and_cti_worlds_share_facts_but_isolate_branch_overlays() {
    let fimi = fimi_scenario();
    let cti = cti_scenario();
    assert_eq!(fimi.winner.world_id().as_str(), "world--fimi-coordinated");
    assert_eq!(fimi.alternative.world_id().as_str(), "world--fimi-organic");
    assert_eq!(cti.winner.world_id().as_str(), "world--cti-actor-north");
    assert_eq!(
        cti.alternative.world_id().as_str(),
        "world--cti-commodity-malware"
    );

    for scenario in [fimi, cti] {
        assert_eq!(scenario.model.worlds().len(), 2);
        assert_eq!(scenario.model.base_facts().len(), 2);

        for selector in [&scenario.winner, &scenario.alternative] {
            let world = scenario
                .model
                .world(selector.world_id())
                .expect("scenario world should exist");
            assert_eq!(world.base_facts(), scenario.model.base_facts());
            assert_eq!(
                world
                    .branch(selector.branch_id())
                    .expect("scenario branch should exist")
                    .status(),
                BranchStatus::Active
            );
        }

        let winner_overlay = scenario
            .model
            .branch_overlay(scenario.winner.world_id(), scenario.winner.branch_id())
            .expect("winner overlay should exist");
        let alternative_overlay = scenario
            .model
            .branch_overlay(
                scenario.alternative.world_id(),
                scenario.alternative.branch_id(),
            )
            .expect("alternative overlay should exist");
        assert_eq!(winner_overlay.hypotheses().len(), 1);
        assert_eq!(alternative_overlay.hypotheses().len(), 1);
        assert_ne!(winner_overlay, alternative_overlay);
    }
}

#[test]
fn acceptance_cross_branch_rankings_are_explained_and_reproducible() {
    for scenario in [fimi_scenario(), cti_scenario()] {
        let forward = rank_cross_branch_scores(&scenario.model, scenario.score_inputs.clone())
            .expect("forward ranking should succeed");
        let mut reversed_inputs = scenario.score_inputs.clone();
        reversed_inputs.reverse();
        let reverse = rank_cross_branch_scores(&scenario.model, reversed_inputs)
            .expect("reverse ranking should succeed");

        assert_eq!(forward, reverse);
        let winner = &forward.ranked_branches()[0];
        assert_eq!(winner.world_id(), scenario.winner.world_id());
        assert_eq!(winner.branch_id(), scenario.winner.branch_id());
        assert_close(
            winner.score_breakdown().evidence_support().value(),
            scenario.expected_winner_score.0,
        );
        assert_close(
            winner.score_breakdown().prediction_quality().value(),
            scenario.expected_winner_score.1,
        );
        assert_close(
            winner.score_breakdown().contradiction_penalty().value(),
            scenario.expected_winner_score.2,
        );
    }
}

#[test]
fn acceptance_counterfactual_queries_return_typed_discriminating_evidence() {
    for scenario in [fimi_scenario(), cti_scenario()] {
        let winner_query = query_counterfactual_expected_facts(
            &scenario.model,
            &scenario.winner,
            &scenario.observations,
        )
        .expect("winner counterfactual query should succeed");
        assert_eq!(
            evidence_ids(winner_query.expected_observations()),
            scenario.expected_winner_evidence
        );
        assert_eq!(
            evidence_ids(winner_query.contradicting_observations()),
            vec![scenario.winner_counter_evidence.clone()]
        );

        let forward = query_discriminating_observations(
            &scenario.model,
            vec![scenario.alternative.clone(), scenario.winner.clone()],
            &scenario.observations,
        )
        .expect("forward discriminating query should succeed");
        let mut reversed_observations = scenario.observations.clone();
        reversed_observations.reverse();
        let reverse = query_discriminating_observations(
            &scenario.model,
            vec![scenario.winner.clone(), scenario.alternative.clone()],
            &reversed_observations,
        )
        .expect("reverse discriminating query should succeed");
        assert_eq!(forward, reverse);
        assert_eq!(
            forward
                .observations()
                .iter()
                .map(|record| record.evidence_id().clone())
                .collect::<Vec<_>>(),
            scenario.discriminating_evidence
        );

        let disproof = query_smallest_disproving_evidence(
            &scenario.model,
            &scenario.alternative,
            &scenario.observations,
        )
        .expect("alternative disproof query should succeed");
        assert_eq!(
            disproof
                .evidence()
                .expect("alternative should have a stable disproof")
                .evidence_id(),
            &scenario.alternative_disproof
        );
    }
}

#[test]
fn acceptance_merge_and_discard_are_explicit_audited_and_non_destructive() {
    for scenario in [fimi_scenario(), cti_scenario()] {
        let original_base = scenario.model.base_facts().to_vec();
        let alternative_overlay = scenario
            .model
            .branch_overlay(
                scenario.alternative.world_id(),
                scenario.alternative.branch_id(),
            )
            .expect("alternative overlay should exist")
            .clone();
        let ledger = resolve_scenario(scenario.clone());

        assert_eq!(ledger.world_model().base_facts(), original_base);
        assert_eq!(ledger.audit_trail().len(), 2);
        assert!(ledger.audit_trail().iter().any(|decision| {
            decision.kind() == BranchResolutionKind::Merge
                && decision.validation().is_some()
                && decision.audit().rationale().contains("review")
        }));
        assert!(ledger.audit_trail().iter().any(|decision| {
            decision.kind() == BranchResolutionKind::Discard
                && decision.validation().is_none()
                && decision.audit().rationale().contains("review")
        }));
        assert_eq!(ledger.canonical_promotions().len(), 1);
        assert_eq!(
            ledger.canonical_promotions()[0].promoted_references().len(),
            2
        );
        assert_eq!(
            branch_status(&ledger, &scenario.winner),
            BranchStatus::Merged
        );
        assert_eq!(
            branch_status(&ledger, &scenario.alternative),
            BranchStatus::Discarded
        );
        assert_eq!(
            ledger
                .world_model()
                .branch_overlay(
                    scenario.alternative.world_id(),
                    scenario.alternative.branch_id(),
                )
                .expect("discarded overlay should remain queryable"),
            &alternative_overlay
        );
    }
}

#[test]
fn acceptance_complete_branchable_world_runs_are_byte_stable() {
    for build in [
        fimi_scenario as fn() -> Epic0021Scenario,
        cti_scenario as fn() -> Epic0021Scenario,
    ] {
        let first_scenario = build();
        let second_scenario = build();
        assert_eq!(first_scenario, second_scenario);

        let first = resolve_scenario(first_scenario);
        let second = resolve_scenario(second_scenario);
        assert_eq!(first, second);
        assert_eq!(
            serde_json::to_string(&first).expect("first ledger should serialize"),
            serde_json::to_string(&second).expect("second ledger should serialize")
        );
    }
}

fn world_id(domain: &str, slug: &str) -> WorldId {
    WorldId::new(format!("world--{domain}-{slug}")).expect("scenario world ID should be valid")
}

fn branch_id() -> BranchId {
    BranchId::new("branch--primary").expect("scenario branch ID should be valid")
}

fn selector(domain: &str, slug: &str) -> BranchSelector {
    BranchSelector::new(world_id(domain, slug), branch_id())
}

fn fact_id(value: &str) -> FactId {
    FactId::new(value).expect("scenario fact ID should be valid")
}

fn hypothesis_id(domain: &str, slug: &str) -> OverlayHypothesisId {
    OverlayHypothesisId::new(format!("hypothesis--{domain}-{slug}"))
        .expect("scenario hypothesis ID should be valid")
}

fn prediction_id(domain: &str, slug: &str) -> BranchPredictionId {
    BranchPredictionId::new(format!("prediction--{domain}-{slug}"))
        .expect("scenario prediction ID should be valid")
}

fn evidence_id(value: &str) -> EvidenceId {
    EvidenceId::new(value).expect("scenario evidence ID should be valid")
}

fn score_input(selector: BranchSelector, score: (f64, f64, f64)) -> CrossBranchScoreInput {
    CrossBranchScoreInput::new(
        selector.world_id().clone(),
        selector.branch_id().clone(),
        CrossBranchScoreBreakdown::new(
            score_term(score.0),
            score_term(score.1),
            score_term(score.2),
        ),
    )
}

fn score_term(value: f64) -> CrossBranchScoreTerm {
    CrossBranchScoreTerm::new(value).expect("scenario score term should be valid")
}

fn assessment(
    selector: BranchSelector,
    effect: BranchObservationEffect,
) -> BranchObservationAssessment {
    BranchObservationAssessment::new(selector, effect)
}

fn observation(
    evidence_id: EvidenceId,
    source_ref: &str,
    description: &str,
    assessments: Vec<BranchObservationAssessment>,
) -> BranchEvidenceObservation {
    BranchEvidenceObservation::new(evidence_id, source_ref, description, assessments)
        .expect("scenario observation should be valid")
}

fn decision_id(domain: &str, kind: &str) -> BranchResolutionDecisionId {
    BranchResolutionDecisionId::new(format!("decision--{domain}-{kind}"))
        .expect("scenario decision ID should be valid")
}

fn audit_metadata(domain: &str, rationale: &str) -> BranchResolutionAuditMetadata {
    BranchResolutionAuditMetadata::new(
        ActorId::new(format!("actor--{domain}-decider"))
            .expect("scenario decision actor should be valid"),
        timestamp("2026-07-19T01:00:00Z"),
        rationale,
    )
    .expect("scenario audit metadata should be valid")
}

fn validation_metadata(domain: &str) -> BranchValidationAuditMetadata {
    BranchValidationAuditMetadata::new(
        ActorId::new(format!("actor--{domain}-validator"))
            .expect("scenario validation actor should be valid"),
        timestamp("2026-07-19T01:05:00Z"),
        "Independent domain review validated canonical promotion",
    )
    .expect("scenario validation metadata should be valid")
}

fn timestamp(value: &str) -> TemporalTimestamp {
    TemporalTimestamp::new(value).expect("scenario timestamp should be valid")
}

fn evidence_ids(observations: &[graph_core::BranchEvidenceObservation]) -> Vec<EvidenceId> {
    observations
        .iter()
        .map(|record| record.evidence_id().clone())
        .collect()
}

fn branch_status(
    ledger: &BranchResolutionLedger,
    selector: &graph_core::BranchSelector,
) -> BranchStatus {
    ledger
        .world_model()
        .world(selector.world_id())
        .and_then(|world| world.branch(selector.branch_id()))
        .expect("resolved branch should remain queryable")
        .status()
}

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= EPSILON,
        "expected {expected}, got {actual}"
    );
}

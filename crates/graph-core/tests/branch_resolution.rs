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
    ActorId, BranchContradiction, BranchContradictionId, BranchCreationInput, BranchId,
    BranchOverlayReference, BranchPrediction, BranchPredictionId, BranchResolutionAuditMetadata,
    BranchResolutionDecision, BranchResolutionDecisionId, BranchResolutionKind,
    BranchResolutionLedger, BranchSelector, BranchStatus, BranchValidationAuditMetadata,
    GraphError, HypothesisWorldModel, OverlayHypothesis, OverlayHypothesisId, TemporalTimestamp,
    WorldId,
};

fn world_id(value: &str) -> WorldId {
    WorldId::new(value).expect("test world ID should be valid")
}

fn branch_id(value: &str) -> BranchId {
    BranchId::new(value).expect("test branch ID should be valid")
}

fn selector() -> BranchSelector {
    BranchSelector::new(world_id("world--alpha"), branch_id("branch--main"))
}

fn hypothesis_id(value: &str) -> OverlayHypothesisId {
    OverlayHypothesisId::new(value).expect("test hypothesis ID should be valid")
}

fn decision_id(value: &str) -> BranchResolutionDecisionId {
    BranchResolutionDecisionId::new(value).expect("test decision ID should be valid")
}

fn timestamp(value: &str) -> TemporalTimestamp {
    TemporalTimestamp::new(value).expect("test timestamp should be valid")
}

fn audit() -> BranchResolutionAuditMetadata {
    BranchResolutionAuditMetadata::new(
        ActorId::new("actor--decider").expect("test actor ID should be valid"),
        timestamp("2026-07-19T00:00:00Z"),
        "Evidence and counterfactual review completed",
    )
    .expect("audit metadata should be valid")
}

fn validation() -> BranchValidationAuditMetadata {
    BranchValidationAuditMetadata::new(
        ActorId::new("actor--validator").expect("test actor ID should be valid"),
        timestamp("2026-07-19T00:05:00Z"),
        "Independent validation approved promotion",
    )
    .expect("validation metadata should be valid")
}

fn model() -> HypothesisWorldModel {
    HypothesisWorldModel::new(vec![
        graph_core::FactId::new("fact--shared").expect("test fact ID should be valid"),
    ])
    .expect("world model should be valid")
    .create_world(world_id("world--alpha"), "Alpha attribution".to_owned())
    .expect("world should be created")
    .create_branch(
        &world_id("world--alpha"),
        BranchCreationInput::new(branch_id("branch--main"), "Alpha branch".to_owned()),
    )
    .expect("branch should be created")
    .add_branch_hypothesis(
        &world_id("world--alpha"),
        &branch_id("branch--main"),
        OverlayHypothesis::new(
            hypothesis_id("hypothesis--alpha"),
            "Actor Alpha controls the infrastructure",
        )
        .expect("hypothesis should be valid"),
    )
    .expect("hypothesis should be added")
    .add_branch_prediction(
        &world_id("world--alpha"),
        &branch_id("branch--main"),
        BranchPrediction::new(
            BranchPredictionId::new("prediction--alpha")
                .expect("test prediction ID should be valid"),
            "The infrastructure will reuse Alpha certificates",
        )
        .expect("prediction should be valid"),
    )
    .expect("prediction should be added")
}

fn merge_decision() -> BranchResolutionDecision {
    BranchResolutionDecision::new(
        decision_id("decision--merge-alpha"),
        selector(),
        BranchResolutionKind::Merge,
        audit(),
    )
    .with_validation(validation())
    .with_promoted_reference(BranchOverlayReference::Hypothesis(hypothesis_id(
        "hypothesis--alpha",
    )))
}

#[test]
fn merge_requires_explicit_validated_decision_and_records_canonical_promotion() {
    let original_base = model().base_facts().to_vec();
    let ledger = BranchResolutionLedger::new(model())
        .apply_decision(merge_decision())
        .expect("validated merge should succeed");

    let branch = ledger
        .world_model()
        .world(&world_id("world--alpha"))
        .and_then(|world| world.branch(&branch_id("branch--main")))
        .expect("merged branch should remain queryable");
    assert_eq!(branch.status(), BranchStatus::Merged);
    assert_eq!(ledger.world_model().base_facts(), original_base);
    assert_eq!(ledger.audit_trail().len(), 1);
    assert_eq!(ledger.canonical_promotions().len(), 1);
    assert_eq!(
        ledger.canonical_promotions()[0].promoted_references(),
        &[BranchOverlayReference::Hypothesis(hypothesis_id(
            "hypothesis--alpha"
        ))]
    );
}

#[test]
fn promotion_without_explicit_validation_is_rejected() {
    let unvalidated = BranchResolutionDecision::new(
        decision_id("decision--unvalidated"),
        selector(),
        BranchResolutionKind::Merge,
        audit(),
    )
    .with_promoted_reference(BranchOverlayReference::Hypothesis(hypothesis_id(
        "hypothesis--alpha",
    )));

    let error = BranchResolutionLedger::new(model())
        .apply_decision(unvalidated)
        .expect_err("unvalidated promotion should fail");
    assert!(matches!(error, GraphError::InvalidBranchResolution(_)));
}

#[test]
fn unresolved_conflicts_and_invalid_promotion_references_block_merge() {
    let conflicted_model = model()
        .add_branch_contradiction(
            &world_id("world--alpha"),
            &branch_id("branch--main"),
            BranchContradiction::new(
                BranchContradictionId::new("contradiction--alpha")
                    .expect("test contradiction ID should be valid"),
                BranchOverlayReference::Hypothesis(hypothesis_id("hypothesis--alpha")),
                BranchOverlayReference::BaseFact(
                    graph_core::FactId::new("fact--shared").expect("test fact ID should be valid"),
                ),
                "Canonical ownership conflicts with the hypothesis",
            )
            .expect("contradiction should be valid"),
        )
        .expect("contradiction should be added");
    let conflict_error = BranchResolutionLedger::new(conflicted_model)
        .apply_decision(merge_decision())
        .expect_err("unresolved contradiction should block merge");
    assert!(matches!(
        conflict_error,
        GraphError::InvalidBranchResolution(_)
    ));

    let invalid_reference = BranchResolutionDecision::new(
        decision_id("decision--invalid-reference"),
        selector(),
        BranchResolutionKind::Merge,
        audit(),
    )
    .with_validation(validation())
    .with_promoted_reference(BranchOverlayReference::Hypothesis(hypothesis_id(
        "hypothesis--missing",
    )));
    let reference_error = BranchResolutionLedger::new(model())
        .apply_decision(invalid_reference)
        .expect_err("unknown promotion reference should fail");
    assert!(matches!(
        reference_error,
        GraphError::InvalidBranchResolution(_)
    ));
}

#[test]
fn discard_preserves_overlay_and_audit_history_without_canonical_mutation() {
    let original = model();
    let original_overlay = original
        .branch_overlay(&world_id("world--alpha"), &branch_id("branch--main"))
        .expect("branch overlay should exist")
        .clone();
    let discard = BranchResolutionDecision::new(
        decision_id("decision--discard-alpha"),
        selector(),
        BranchResolutionKind::Discard,
        audit(),
    );

    let ledger = BranchResolutionLedger::new(original.clone())
        .apply_decision(discard)
        .expect("discard should succeed");

    assert_eq!(ledger.audit_trail().len(), 1);
    assert!(ledger.canonical_promotions().is_empty());
    assert_eq!(ledger.world_model().base_facts(), original.base_facts());
    assert_eq!(
        ledger
            .world_model()
            .branch_overlay(&world_id("world--alpha"), &branch_id("branch--main"))
            .expect("discarded overlay should remain queryable"),
        &original_overlay
    );
    assert_eq!(
        ledger
            .world_model()
            .world(&world_id("world--alpha"))
            .and_then(|world| world.branch(&branch_id("branch--main")))
            .expect("discarded branch should remain queryable")
            .status(),
        BranchStatus::Discarded
    );
}

#[test]
fn merge_and_discard_results_are_deterministic_for_identical_inputs() {
    let prediction = BranchOverlayReference::Prediction(
        BranchPredictionId::new("prediction--alpha").expect("test prediction ID should be valid"),
    );
    let first = BranchResolutionLedger::new(model())
        .apply_decision(merge_decision().with_promoted_reference(prediction.clone()))
        .expect("first merge should succeed");
    let second = BranchResolutionLedger::new(model())
        .apply_decision(
            BranchResolutionDecision::new(
                decision_id("decision--merge-alpha"),
                selector(),
                BranchResolutionKind::Merge,
                audit(),
            )
            .with_validation(validation())
            .with_promoted_reference(prediction)
            .with_promoted_reference(BranchOverlayReference::Hypothesis(hypothesis_id(
                "hypothesis--alpha",
            ))),
        )
        .expect("second merge should succeed");

    assert_eq!(first, second);
    assert_eq!(
        serde_json::to_string(&first).expect("first ledger should serialize"),
        serde_json::to_string(&second).expect("second ledger should serialize")
    );
}

#[test]
fn audit_metadata_and_terminal_branch_transitions_are_validated() {
    let blank_reason = BranchResolutionAuditMetadata::new(
        ActorId::new("actor--decider").expect("test actor ID should be valid"),
        timestamp("2026-07-19T00:00:00Z"),
        " ",
    )
    .expect_err("blank audit rationale should fail");
    assert!(matches!(
        blank_reason,
        GraphError::InvalidBranchResolution(_)
    ));

    let ledger = BranchResolutionLedger::new(model())
        .apply_decision(merge_decision())
        .expect("initial merge should succeed");
    let second_decision = BranchResolutionDecision::new(
        decision_id("decision--discard-after-merge"),
        selector(),
        BranchResolutionKind::Discard,
        audit(),
    );
    let terminal_error = ledger
        .world_model()
        .clone()
        .add_branch_hypothesis(
            &world_id("world--alpha"),
            &branch_id("branch--main"),
            OverlayHypothesis::new(
                hypothesis_id("hypothesis--late"),
                "Late mutation must not be accepted",
            )
            .expect("hypothesis should be valid"),
        )
        .expect_err("terminal branch overlay should be immutable");
    assert!(matches!(
        terminal_error,
        GraphError::InvalidBranchOverlay(_)
    ));

    let terminal_error = ledger
        .apply_decision(second_decision)
        .expect_err("terminal branch should reject another decision");
    assert!(matches!(
        terminal_error,
        GraphError::InvalidBranchResolution(_)
    ));
}

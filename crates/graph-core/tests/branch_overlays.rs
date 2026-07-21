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
    BranchContradiction, BranchContradictionId, BranchCreationInput, BranchDerivedRelation,
    BranchDerivedRelationId, BranchExpectedEvidence, BranchId, BranchOverlayReference,
    BranchPrediction, BranchPredictionId, ExpectedEvidenceMarkerId, FactId, GraphError,
    HypothesisWorldModel, OverlayHypothesis, OverlayHypothesisId, RelationshipType, WorldId,
};

fn world_id(value: &str) -> WorldId {
    WorldId::new(value).expect("test world ID should be valid")
}

fn branch_id(value: &str) -> BranchId {
    BranchId::new(value).expect("test branch ID should be valid")
}

fn fact_id(value: &str) -> FactId {
    FactId::new(value).expect("test fact ID should be valid")
}

fn hypothesis_id(value: &str) -> OverlayHypothesisId {
    OverlayHypothesisId::new(value).expect("test hypothesis ID should be valid")
}

fn prediction_id(value: &str) -> BranchPredictionId {
    BranchPredictionId::new(value).expect("test prediction ID should be valid")
}

fn relation_id(value: &str) -> BranchDerivedRelationId {
    BranchDerivedRelationId::new(value).expect("test derived relation ID should be valid")
}

fn expected_evidence_id(value: &str) -> ExpectedEvidenceMarkerId {
    ExpectedEvidenceMarkerId::new(value).expect("test expected-evidence ID should be valid")
}

fn contradiction_id(value: &str) -> BranchContradictionId {
    BranchContradictionId::new(value).expect("test contradiction ID should be valid")
}

fn model_with_branches() -> HypothesisWorldModel {
    HypothesisWorldModel::new(vec![fact_id("fact--a"), fact_id("fact--b")])
        .expect("world model should be valid")
        .create_world(
            world_id("world--attribution"),
            "Attribution alternatives".to_owned(),
        )
        .expect("world should be created")
        .create_branch(
            &world_id("world--attribution"),
            BranchCreationInput::new(branch_id("branch--alpha"), "Actor Alpha".to_owned()),
        )
        .expect("alpha branch should be created")
        .create_branch(
            &world_id("world--attribution"),
            BranchCreationInput::new(branch_id("branch--beta"), "Actor Beta".to_owned()),
        )
        .expect("beta branch should be created")
}

fn alpha_overlay_with_all_record_kinds() -> HypothesisWorldModel {
    model_with_branches()
        .add_branch_hypothesis(
            &world_id("world--attribution"),
            &branch_id("branch--alpha"),
            OverlayHypothesis::new(
                hypothesis_id("hypothesis--alpha"),
                "Actor Alpha operates the observed infrastructure",
            )
            .expect("hypothesis should be valid"),
        )
        .expect("hypothesis should be added")
        .add_branch_derived_relation(
            &world_id("world--attribution"),
            &branch_id("branch--alpha"),
            BranchDerivedRelation::new(
                relation_id("derived--uses"),
                BranchOverlayReference::Hypothesis(hypothesis_id("hypothesis--alpha")),
                BranchOverlayReference::BaseFact(fact_id("fact--a")),
                RelationshipType::new("USES").expect("relationship type should be valid"),
            ),
        )
        .expect("derived relation should be added")
        .add_branch_prediction(
            &world_id("world--attribution"),
            &branch_id("branch--alpha"),
            BranchPrediction::new(
                prediction_id("prediction--reuse"),
                "Actor Alpha will reuse the infrastructure",
            )
            .expect("prediction should be valid"),
        )
        .expect("prediction should be added")
        .add_branch_expected_evidence(
            &world_id("world--attribution"),
            &branch_id("branch--alpha"),
            BranchExpectedEvidence::new(
                expected_evidence_id("expected--telemetry"),
                "Telemetry linking Actor Alpha to the infrastructure",
                BranchOverlayReference::Prediction(prediction_id("prediction--reuse")),
            )
            .expect("expected evidence should be valid"),
        )
        .expect("expected evidence should be added")
        .add_branch_contradiction(
            &world_id("world--attribution"),
            &branch_id("branch--alpha"),
            BranchContradiction::new(
                contradiction_id("contradiction--ownership"),
                BranchOverlayReference::Hypothesis(hypothesis_id("hypothesis--alpha")),
                BranchOverlayReference::BaseFact(fact_id("fact--b")),
                "Confirmed ownership conflicts with the attribution hypothesis",
            )
            .expect("contradiction should be valid"),
        )
        .expect("contradiction should be added")
}

#[test]
fn branch_overlay_represents_all_branch_local_record_kinds() {
    let model = alpha_overlay_with_all_record_kinds();
    let overlay = model
        .branch_overlay(&world_id("world--attribution"), &branch_id("branch--alpha"))
        .expect("alpha overlay should be queryable");

    assert_eq!(overlay.hypotheses().len(), 1);
    assert_eq!(overlay.derived_relations().len(), 1);
    assert_eq!(overlay.predictions().len(), 1);
    assert_eq!(overlay.expected_evidence().len(), 1);
    assert_eq!(overlay.contradictions().len(), 1);
}

#[test]
fn overlay_state_is_isolated_per_branch() {
    let model = alpha_overlay_with_all_record_kinds();

    let alpha = model
        .branch_overlay(&world_id("world--attribution"), &branch_id("branch--alpha"))
        .expect("alpha overlay should exist");
    let beta = model
        .branch_overlay(&world_id("world--attribution"), &branch_id("branch--beta"))
        .expect("beta overlay should exist");

    assert_eq!(alpha.hypotheses().len(), 1);
    assert!(beta.hypotheses().is_empty());
    assert!(beta.derived_relations().is_empty());
    assert!(beta.predictions().is_empty());
    assert!(beta.expected_evidence().is_empty());
    assert!(beta.contradictions().is_empty());
}

#[test]
fn overlay_writes_preserve_the_canonical_base_layer() {
    let model = alpha_overlay_with_all_record_kinds();

    assert_eq!(
        model.base_facts(),
        &[fact_id("fact--a"), fact_id("fact--b")]
    );
    assert_eq!(
        model
            .world(&world_id("world--attribution"))
            .expect("world should exist")
            .base_facts(),
        model.base_facts()
    );
}

#[test]
fn invalid_overlay_references_and_scopes_return_typed_errors() {
    let unknown_base_error = model_with_branches()
        .add_branch_derived_relation(
            &world_id("world--attribution"),
            &branch_id("branch--alpha"),
            BranchDerivedRelation::new(
                relation_id("derived--invalid"),
                BranchOverlayReference::BaseFact(fact_id("fact--missing")),
                BranchOverlayReference::BaseFact(fact_id("fact--a")),
                RelationshipType::new("USES").expect("relationship type should be valid"),
            ),
        )
        .expect_err("unknown base fact reference should fail");
    assert!(matches!(
        unknown_base_error,
        GraphError::InvalidBranchOverlay(_)
    ));

    let cross_branch_error = model_with_branches()
        .add_branch_hypothesis(
            &world_id("world--attribution"),
            &branch_id("branch--beta"),
            OverlayHypothesis::new(hypothesis_id("hypothesis--beta"), "Beta-only hypothesis")
                .expect("hypothesis should be valid"),
        )
        .expect("beta hypothesis should be added")
        .add_branch_derived_relation(
            &world_id("world--attribution"),
            &branch_id("branch--alpha"),
            BranchDerivedRelation::new(
                relation_id("derived--cross-branch"),
                BranchOverlayReference::Hypothesis(hypothesis_id("hypothesis--beta")),
                BranchOverlayReference::BaseFact(fact_id("fact--a")),
                RelationshipType::new("USES").expect("relationship type should be valid"),
            ),
        )
        .expect_err("cross-branch reference should fail");
    assert!(matches!(
        cross_branch_error,
        GraphError::InvalidBranchOverlay(_)
    ));

    let unknown_branch_error = model_with_branches()
        .add_branch_hypothesis(
            &world_id("world--attribution"),
            &branch_id("branch--missing"),
            OverlayHypothesis::new(hypothesis_id("hypothesis--x"), "Unknown branch")
                .expect("hypothesis should be valid"),
        )
        .expect_err("unknown branch scope should fail");
    assert!(matches!(
        unknown_branch_error,
        GraphError::InvalidBranchOverlay(_)
    ));
}

#[test]
fn overlay_equality_and_serialization_are_deterministic() {
    let first = model_with_branches()
        .add_branch_hypothesis(
            &world_id("world--attribution"),
            &branch_id("branch--alpha"),
            OverlayHypothesis::new(hypothesis_id("hypothesis--z"), "Hypothesis Z")
                .expect("hypothesis should be valid"),
        )
        .expect("hypothesis Z should be added")
        .add_branch_hypothesis(
            &world_id("world--attribution"),
            &branch_id("branch--alpha"),
            OverlayHypothesis::new(hypothesis_id("hypothesis--a"), "Hypothesis A")
                .expect("hypothesis should be valid"),
        )
        .expect("hypothesis A should be added");
    let second = model_with_branches()
        .add_branch_hypothesis(
            &world_id("world--attribution"),
            &branch_id("branch--alpha"),
            OverlayHypothesis::new(hypothesis_id("hypothesis--a"), "Hypothesis A")
                .expect("hypothesis should be valid"),
        )
        .expect("hypothesis A should be added")
        .add_branch_hypothesis(
            &world_id("world--attribution"),
            &branch_id("branch--alpha"),
            OverlayHypothesis::new(hypothesis_id("hypothesis--z"), "Hypothesis Z")
                .expect("hypothesis should be valid"),
        )
        .expect("hypothesis Z should be added");

    assert_eq!(first, second);
    assert_eq!(
        serde_json::to_string(&first).expect("first model should serialize"),
        serde_json::to_string(&second).expect("second model should serialize")
    );
}

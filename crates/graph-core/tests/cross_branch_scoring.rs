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
    BranchCreationInput, BranchId, CrossBranchRanking, CrossBranchScoreBreakdown,
    CrossBranchScoreInput, CrossBranchScoreTerm, GraphError, HypothesisWorldModel, WorldId,
    rank_cross_branch_scores,
};

const EPSILON: f64 = 1e-12;

fn world_id(value: &str) -> WorldId {
    WorldId::new(value).expect("test world ID should be valid")
}

fn branch_id(value: &str) -> BranchId {
    BranchId::new(value).expect("test branch ID should be valid")
}

fn fact_id(value: &str) -> graph_core::FactId {
    graph_core::FactId::new(value).expect("test fact ID should be valid")
}

fn term(value: f64) -> CrossBranchScoreTerm {
    CrossBranchScoreTerm::new(value).expect("test score term should be valid")
}

fn breakdown(
    evidence_support: f64,
    prediction_quality: f64,
    contradiction_penalty: f64,
) -> CrossBranchScoreBreakdown {
    CrossBranchScoreBreakdown::new(
        term(evidence_support),
        term(prediction_quality),
        term(contradiction_penalty),
    )
}

fn input(
    world: &str,
    branch: &str,
    score_breakdown: CrossBranchScoreBreakdown,
) -> CrossBranchScoreInput {
    CrossBranchScoreInput::new(world_id(world), branch_id(branch), score_breakdown)
}

fn model() -> HypothesisWorldModel {
    HypothesisWorldModel::new(vec![fact_id("fact--shared")])
        .expect("world model should be valid")
        .create_world(world_id("world--alpha"), "Alpha attribution".to_owned())
        .expect("alpha world should be created")
        .create_branch(
            &world_id("world--alpha"),
            BranchCreationInput::new(branch_id("branch--one"), "Alpha branch one".to_owned()),
        )
        .expect("alpha branch one should be created")
        .create_branch(
            &world_id("world--alpha"),
            BranchCreationInput::new(branch_id("branch--two"), "Alpha branch two".to_owned()),
        )
        .expect("alpha branch two should be created")
        .create_world(world_id("world--beta"), "Beta attribution".to_owned())
        .expect("beta world should be created")
        .create_branch(
            &world_id("world--beta"),
            BranchCreationInput::new(branch_id("branch--one"), "Beta branch one".to_owned()),
        )
        .expect("beta branch one should be created")
}

fn rank(model: &HypothesisWorldModel, inputs: Vec<CrossBranchScoreInput>) -> CrossBranchRanking {
    rank_cross_branch_scores(model, inputs).expect("cross-branch ranking should succeed")
}

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= EPSILON,
        "expected {expected}, got {actual}"
    );
}

#[test]
fn score_terms_are_typed_finite_and_bounded() {
    assert_eq!(term(0.0).value(), 0.0);
    assert_eq!(term(1.0).value(), 1.0);

    for invalid in [-0.01, 1.01, f64::INFINITY, f64::NAN] {
        let error = CrossBranchScoreTerm::new(invalid).expect_err("invalid score term should fail");
        assert!(
            matches!(error, GraphError::InvalidCrossBranchScoreTerm(value)
                if (value.is_nan() && invalid.is_nan()) || value == invalid)
        );
    }
}

#[test]
fn score_explanation_preserves_every_positive_and_negative_term() {
    let ranking = rank(
        &model(),
        vec![input(
            "world--alpha",
            "branch--one",
            breakdown(0.8, 0.6, 0.2),
        )],
    );
    let score = ranking.ranked_branches()[0].score_breakdown();

    assert_close(score.evidence_support().value(), 0.8);
    assert_close(score.prediction_quality().value(), 0.6);
    assert_close(score.contradiction_penalty().value(), 0.2);
    assert_close(score.total(), 1.2);
}

#[test]
fn higher_scored_branch_ranks_ahead_under_equal_constraints() {
    let ranking = rank(
        &model(),
        vec![
            input("world--alpha", "branch--one", breakdown(0.4, 0.3, 0.2)),
            input("world--beta", "branch--one", breakdown(0.9, 0.8, 0.1)),
        ],
    );

    assert_eq!(
        ranking.ranked_branches()[0].world_id(),
        &world_id("world--beta")
    );
    assert_eq!(
        ranking.ranked_branches()[0].branch_id(),
        &branch_id("branch--one")
    );
}

#[test]
fn equal_scores_use_world_then_branch_identifier_tie_breaking() {
    let alpha_one = input("world--alpha", "branch--one", breakdown(0.6, 0.5, 0.1));
    let alpha_two = input("world--alpha", "branch--two", breakdown(0.7, 0.4, 0.1));
    let beta_one = input("world--beta", "branch--one", breakdown(0.8, 0.3, 0.1));

    let forward = rank(
        &model(),
        vec![beta_one.clone(), alpha_two.clone(), alpha_one.clone()],
    );
    let reverse = rank(&model(), vec![alpha_one, alpha_two, beta_one]);

    assert_eq!(forward, reverse);
    assert_eq!(
        forward
            .ranked_branches()
            .iter()
            .map(|ranked| (ranked.world_id().as_str(), ranked.branch_id().as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("world--alpha", "branch--one"),
            ("world--alpha", "branch--two"),
            ("world--beta", "branch--one"),
        ]
    );
}

#[test]
fn missing_overlays_and_duplicate_inputs_return_typed_errors() {
    let missing = rank_cross_branch_scores(
        &model(),
        vec![input(
            "world--alpha",
            "branch--missing",
            breakdown(0.5, 0.5, 0.5),
        )],
    )
    .expect_err("missing branch overlay should fail");
    assert!(matches!(
        missing,
        GraphError::InvalidCrossBranchComparison(_)
    ));

    let duplicate_input = input("world--alpha", "branch--one", breakdown(0.5, 0.5, 0.5));
    let duplicate =
        rank_cross_branch_scores(&model(), vec![duplicate_input.clone(), duplicate_input])
            .expect_err("duplicate branch score input should fail");
    assert!(matches!(
        duplicate,
        GraphError::InvalidCrossBranchComparison(_)
    ));
}

#[test]
fn comparison_is_repeatable_and_does_not_mutate_branch_or_base_state() {
    let model = model();
    let original = model.clone();
    let inputs = vec![
        input("world--alpha", "branch--one", breakdown(0.7, 0.5, 0.2)),
        input("world--beta", "branch--one", breakdown(0.6, 0.4, 0.1)),
    ];

    let first = rank(&model, inputs.clone());
    let second = rank(&model, inputs);

    assert_eq!(first, second);
    assert_eq!(model, original);
    assert_eq!(model.base_facts(), &[fact_id("fact--shared")]);
}

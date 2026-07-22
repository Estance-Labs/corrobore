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
    BranchCreationInput, BranchEvidenceObservation, BranchId, BranchObservationAssessment,
    BranchObservationEffect, BranchSelector, EvidenceId, GraphError, HypothesisWorldModel, WorldId,
    query_counterfactual_expected_facts, query_discriminating_observations,
    query_smallest_disproving_evidence, query_source_removal_impact,
};

fn world_id(value: &str) -> WorldId {
    WorldId::new(value).expect("test world ID should be valid")
}

fn branch_id(value: &str) -> BranchId {
    BranchId::new(value).expect("test branch ID should be valid")
}

fn evidence_id(value: &str) -> EvidenceId {
    EvidenceId::new(value).expect("test evidence ID should be valid")
}

fn selector(world: &str, branch: &str) -> BranchSelector {
    BranchSelector::new(world_id(world), branch_id(branch))
}

fn assessment(
    world: &str,
    branch: &str,
    effect: BranchObservationEffect,
) -> BranchObservationAssessment {
    BranchObservationAssessment::new(selector(world, branch), effect)
}

fn observation(
    id: &str,
    source_ref: &str,
    description: &str,
    assessments: Vec<BranchObservationAssessment>,
) -> BranchEvidenceObservation {
    BranchEvidenceObservation::new(evidence_id(id), source_ref, description, assessments)
        .expect("test observation should be valid")
}

fn model() -> HypothesisWorldModel {
    HypothesisWorldModel::new(vec![
        graph_core::FactId::new("fact--shared").expect("test fact ID should be valid"),
    ])
    .expect("world model should be valid")
    .create_world(world_id("world--alpha"), "Alpha attribution".to_owned())
    .expect("alpha world should be created")
    .create_branch(
        &world_id("world--alpha"),
        BranchCreationInput::new(branch_id("branch--main"), "Alpha branch".to_owned()),
    )
    .expect("alpha branch should be created")
    .create_world(world_id("world--beta"), "Beta attribution".to_owned())
    .expect("beta world should be created")
    .create_branch(
        &world_id("world--beta"),
        BranchCreationInput::new(branch_id("branch--main"), "Beta branch".to_owned()),
    )
    .expect("beta branch should be created")
}

fn observations() -> Vec<BranchEvidenceObservation> {
    vec![
        observation(
            "evidence--shared",
            "source://shared",
            "Infrastructure telemetry is present",
            vec![
                assessment(
                    "world--alpha",
                    "branch--main",
                    BranchObservationEffect::Expected,
                ),
                assessment(
                    "world--beta",
                    "branch--main",
                    BranchObservationEffect::Expected,
                ),
            ],
        ),
        observation(
            "evidence--discriminator",
            "source://alpha",
            "Operator language matches Alpha",
            vec![
                assessment(
                    "world--alpha",
                    "branch--main",
                    BranchObservationEffect::Expected,
                ),
                assessment(
                    "world--beta",
                    "branch--main",
                    BranchObservationEffect::Contradicts,
                ),
            ],
        ),
        observation(
            "evidence--alpha-refutation",
            "source://alpha",
            "Ownership records exclude Alpha",
            vec![assessment(
                "world--alpha",
                "branch--main",
                BranchObservationEffect::Contradicts,
            )],
        ),
    ]
}

#[test]
fn expected_fact_query_returns_branch_aware_expected_and_contradictory_outputs() {
    let result = query_counterfactual_expected_facts(
        &model(),
        &selector("world--alpha", "branch--main"),
        &observations(),
    )
    .expect("counterfactual query should succeed");

    assert_eq!(result.selector(), &selector("world--alpha", "branch--main"));
    assert_eq!(
        result
            .expected_observations()
            .iter()
            .map(|item| item.evidence_id().as_str())
            .collect::<Vec<_>>(),
        vec!["evidence--discriminator", "evidence--shared"]
    );
    assert_eq!(
        result.contradicting_observations()[0]
            .evidence_id()
            .as_str(),
        "evidence--alpha-refutation"
    );
}

#[test]
fn discriminating_query_identifies_observations_that_separate_worlds() {
    let result = query_discriminating_observations(
        &model(),
        vec![
            selector("world--beta", "branch--main"),
            selector("world--alpha", "branch--main"),
        ],
        &observations(),
    )
    .expect("discriminating query should succeed");

    assert_eq!(
        result
            .observations()
            .iter()
            .map(|item| item.evidence_id().as_str())
            .collect::<Vec<_>>(),
        vec!["evidence--alpha-refutation", "evidence--discriminator"]
    );
    let discriminator = result
        .observations()
        .iter()
        .find(|item| item.evidence_id().as_str() == "evidence--discriminator")
        .expect("discriminator should be returned");
    assert_eq!(discriminator.assessments().len(), 2);
}

#[test]
fn smallest_disproving_evidence_is_stable_and_empty_when_none_exists() {
    let alpha = query_smallest_disproving_evidence(
        &model(),
        &selector("world--alpha", "branch--main"),
        &observations(),
    )
    .expect("disproving-evidence query should succeed");
    assert_eq!(
        alpha
            .evidence()
            .expect("alpha should have disproving evidence")
            .evidence_id()
            .as_str(),
        "evidence--alpha-refutation"
    );

    let no_contradictions = vec![observation(
        "evidence--expected",
        "source://shared",
        "Expected evidence",
        vec![assessment(
            "world--alpha",
            "branch--main",
            BranchObservationEffect::Expected,
        )],
    )];
    let empty = query_smallest_disproving_evidence(
        &model(),
        &selector("world--alpha", "branch--main"),
        &no_contradictions,
    )
    .expect("empty disproving-evidence query should succeed");
    assert!(empty.evidence().is_none());
}

#[test]
fn source_removal_impact_preserves_affected_evidence_per_branch() {
    let result = query_source_removal_impact(
        &model(),
        "source://alpha",
        vec![
            selector("world--beta", "branch--main"),
            selector("world--alpha", "branch--main"),
        ],
        &observations(),
    )
    .expect("source-removal query should succeed");

    assert_eq!(result.source_ref(), "source://alpha");
    assert_eq!(result.branch_impacts().len(), 2);
    let alpha = result
        .branch_impacts()
        .iter()
        .find(|impact| impact.selector() == &selector("world--alpha", "branch--main"))
        .expect("alpha impact should be present");
    assert_eq!(
        alpha
            .removed_expected_evidence()
            .iter()
            .map(EvidenceId::as_str)
            .collect::<Vec<_>>(),
        vec!["evidence--discriminator"]
    );
    assert_eq!(
        alpha
            .removed_contradicting_evidence()
            .iter()
            .map(EvidenceId::as_str)
            .collect::<Vec<_>>(),
        vec!["evidence--alpha-refutation"]
    );
}

#[test]
fn query_outputs_are_deterministic_and_queries_do_not_mutate_world_state() {
    let model = model();
    let original = model.clone();
    let forward = query_discriminating_observations(
        &model,
        vec![
            selector("world--beta", "branch--main"),
            selector("world--alpha", "branch--main"),
        ],
        &observations(),
    )
    .expect("forward query should succeed");
    let mut reversed_observations = observations();
    reversed_observations.reverse();
    let reverse = query_discriminating_observations(
        &model,
        vec![
            selector("world--alpha", "branch--main"),
            selector("world--beta", "branch--main"),
        ],
        &reversed_observations,
    )
    .expect("reverse query should succeed");

    assert_eq!(forward, reverse);
    assert_eq!(model, original);
}

#[test]
fn invalid_branch_selectors_return_typed_errors() {
    let error = query_counterfactual_expected_facts(
        &model(),
        &selector("world--missing", "branch--main"),
        &observations(),
    )
    .expect_err("unknown world selector should fail");
    assert!(matches!(error, GraphError::InvalidBranchEvidenceQuery(_)));

    let duplicate_selector = selector("world--alpha", "branch--main");
    let error = query_discriminating_observations(
        &model(),
        vec![duplicate_selector.clone(), duplicate_selector],
        &observations(),
    )
    .expect_err("duplicate selectors should fail");
    assert!(matches!(error, GraphError::InvalidBranchEvidenceQuery(_)));
}

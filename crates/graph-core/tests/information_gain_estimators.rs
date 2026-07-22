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
    CandidateEvidenceOutcome, Confidence, GraphError, InformationGainEstimate,
    InformationGainInput, OutcomeProbability, estimate_information_gain,
};
use serde::{Deserialize, Serialize};

const EPSILON: f64 = 1e-12;

fn confidence(value: f64) -> Confidence {
    Confidence::new(value).expect("test confidence should be valid")
}

fn probability(value: f64) -> OutcomeProbability {
    OutcomeProbability::new(value).expect("test probability should be valid")
}

fn outcome(probability_value: f64, posterior_confidence: f64) -> CandidateEvidenceOutcome {
    CandidateEvidenceOutcome::new(
        probability(probability_value),
        confidence(posterior_confidence),
    )
}

fn input(current_confidence: f64, outcomes: Vec<CandidateEvidenceOutcome>) -> InformationGainInput {
    InformationGainInput::new(confidence(current_confidence), outcomes)
        .expect("test information-gain input should be valid")
}

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= EPSILON,
        "expected {expected}, got {actual}"
    );
}

fn assert_bounded(estimate: &InformationGainEstimate) {
    for value in [
        estimate.current_uncertainty_bits(),
        estimate.expected_posterior_uncertainty_bits(),
        estimate.expected_information_gain_bits(),
        estimate.expected_uncertainty_reduction(),
    ] {
        assert!((0.0..=1.0).contains(&value), "{value} must be bounded");
    }
}

//
// Verify the probability primitive accepts exactly finite unit-interval values.
//
// Given valid boundary values and invalid values outside the unit interval,
// when outcome probabilities are constructed,
// then valid values should be preserved and invalid values should return the
// dedicated typed error.
#[test]
fn outcome_probability_is_typed_and_bounded() {
    assert_eq!(probability(0.0).value(), 0.0);
    assert_eq!(probability(1.0).value(), 1.0);
    assert_eq!(probability(0.35).value(), 0.35);

    for invalid in [-0.01, 1.01, f64::INFINITY, f64::NAN] {
        let error = OutcomeProbability::new(invalid).expect_err("invalid probability should fail");
        assert!(matches!(error, GraphError::InvalidOutcomeProbability(value)
                if (value.is_nan() && invalid.is_nan()) || value == invalid));
    }
}

//
// Verify candidate distributions are explicit and normalized.
//
// Given empty, under-normalized, and normalized candidate outcome sets,
// when information-gain inputs are constructed,
// then malformed distributions should return typed input errors while a
// floating-point-safe normalized distribution succeeds.
#[test]
fn candidate_outcome_distributions_are_non_empty_and_normalized() {
    let empty = InformationGainInput::new(confidence(0.5), Vec::new())
        .expect_err("empty outcomes should fail");
    assert!(matches!(empty, GraphError::InvalidInformationGainInput(_)));

    let under_normalized =
        InformationGainInput::new(confidence(0.5), vec![outcome(0.4, 0.2), outcome(0.5, 0.8)])
            .expect_err("probabilities that do not sum to one should fail");
    assert!(matches!(
        under_normalized,
        GraphError::InvalidInformationGainInput(_)
    ));

    InformationGainInput::new(
        confidence(0.5),
        vec![outcome(0.1, 0.1), outcome(0.2, 0.5), outcome(0.7, 0.9)],
    )
    .expect("normalized probabilities should succeed");

    let confidence_error =
        Confidence::new(1.1).expect_err("invalid posterior confidence should be typed");
    assert!(matches!(
        confidence_error,
        GraphError::InvalidConfidence(value) if value == 1.1
    ));
}

//
// Verify evidence that preserves the current uncertainty has no information
// value.
//
// Given a maximally uncertain question and two outcomes that leave confidence
// unchanged,
// when information gain is estimated,
// then prior and posterior uncertainty should both be one bit and both gain
// measures should be zero.
#[test]
fn uncertainty_preserving_evidence_has_zero_expected_gain() {
    let estimate =
        estimate_information_gain(&input(0.5, vec![outcome(0.5, 0.5), outcome(0.5, 0.5)]));

    assert_close(estimate.current_uncertainty_bits(), 1.0);
    assert_close(estimate.expected_posterior_uncertainty_bits(), 1.0);
    assert_close(estimate.expected_information_gain_bits(), 0.0);
    assert_close(estimate.expected_uncertainty_reduction(), 0.0);
    assert_bounded(&estimate);
}

//
// Verify decisive evidence eliminates binary uncertainty.
//
// Given a maximally uncertain question whose equally likely outcomes resolve
// confidence to zero or one,
// when information gain is estimated,
// then the expected gain and relative uncertainty reduction should both be
// maximal.
#[test]
fn decisive_evidence_has_maximal_expected_gain() {
    let estimate =
        estimate_information_gain(&input(0.5, vec![outcome(0.5, 0.0), outcome(0.5, 1.0)]));

    assert_close(estimate.current_uncertainty_bits(), 1.0);
    assert_close(estimate.expected_posterior_uncertainty_bits(), 0.0);
    assert_close(estimate.expected_information_gain_bits(), 1.0);
    assert_close(estimate.expected_uncertainty_reduction(), 1.0);
    assert_bounded(&estimate);
}

//
// Verify candidates expected to resolve uncertainty outrank ambiguous
// candidates before costs and risks are introduced.
//
// Given two candidate observations with the same outcome probabilities but
// different posterior confidence separation,
// when both are estimated,
// then the more decisive candidate should expose more information gain and
// uncertainty reduction.
#[test]
fn more_decisive_evidence_scores_above_ambiguous_evidence() {
    let ambiguous =
        estimate_information_gain(&input(0.5, vec![outcome(0.5, 0.4), outcome(0.5, 0.6)]));
    let decisive =
        estimate_information_gain(&input(0.5, vec![outcome(0.5, 0.1), outcome(0.5, 0.9)]));

    assert!(decisive.expected_information_gain_bits() > ambiguous.expected_information_gain_bits());
    assert!(decisive.expected_uncertainty_reduction() > ambiguous.expected_uncertainty_reduction());
    assert_bounded(&ambiguous);
    assert_bounded(&decisive);
}

//
// Verify already-certain questions remain finite and deterministic.
//
// Given a question at full confidence whose only possible outcome preserves
// that confidence,
// when information gain is estimated,
// then all uncertainty and gain measures should be zero without division
// artifacts.
#[test]
fn already_certain_questions_have_zero_finite_gain() {
    let request = input(1.0, vec![outcome(1.0, 1.0)]);
    let first = estimate_information_gain(&request);
    let second = estimate_information_gain(&request);

    assert_eq!(first, second);
    assert_close(first.current_uncertainty_bits(), 0.0);
    assert_close(first.expected_posterior_uncertainty_bits(), 0.0);
    assert_close(first.expected_information_gain_bits(), 0.0);
    assert_close(first.expected_uncertainty_reduction(), 0.0);
    assert_bounded(&first);
}

//
// Verify public estimator contracts remain serializable for future assessment
// and audit envelopes.
//
// Given the public input, probability, outcome, and estimate types,
// when serde trait bounds are required,
// then all types should support deterministic persistence boundaries.
#[test]
fn estimator_contracts_are_serializable() {
    fn assert_serializable<T: Serialize + for<'de> Deserialize<'de>>() {}

    assert_serializable::<OutcomeProbability>();
    assert_serializable::<CandidateEvidenceOutcome>();
    assert_serializable::<InformationGainInput>();
    assert_serializable::<InformationGainEstimate>();
}

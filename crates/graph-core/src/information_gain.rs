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
//! Deterministic information-gain estimation for candidate evidence.
//!
//! This module evaluates how much a possible observation is expected to reduce
//! binary epistemic uncertainty. It deliberately excludes action cost, risk,
//! and execution policy so callers can compose those concerns separately.

use serde::{Deserialize, Serialize};

use crate::{Confidence, GraphError};

const NORMALIZATION_TOLERANCE: f64 = 1e-9;

/// A finite probability in the inclusive unit interval.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct OutcomeProbability(f64);

impl OutcomeProbability {
    /// Creates a validated outcome probability.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::InvalidOutcomeProbability`] when `value` is not
    /// finite or lies outside the inclusive unit interval.
    pub fn new(value: f64) -> Result<Self, GraphError> {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(GraphError::InvalidOutcomeProbability(value));
        }

        Ok(Self(value))
    }

    /// Returns the probability as a scalar.
    #[must_use]
    pub const fn value(self) -> f64 {
        self.0
    }
}

impl TryFrom<f64> for OutcomeProbability {
    type Error = GraphError;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<OutcomeProbability> for f64 {
    fn from(probability: OutcomeProbability) -> Self {
        probability.value()
    }
}

/// One possible observation and its forecast posterior confidence.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CandidateEvidenceOutcome {
    probability: OutcomeProbability,
    post_observation_confidence: Confidence,
}

impl CandidateEvidenceOutcome {
    /// Creates a candidate evidence outcome.
    #[must_use]
    pub const fn new(
        probability: OutcomeProbability,
        post_observation_confidence: Confidence,
    ) -> Self {
        Self {
            probability,
            post_observation_confidence,
        }
    }

    /// Returns the probability assigned to this outcome.
    #[must_use]
    pub const fn probability(self) -> OutcomeProbability {
        self.probability
    }

    /// Returns the confidence expected after observing this outcome.
    #[must_use]
    pub const fn post_observation_confidence(self) -> Confidence {
        self.post_observation_confidence
    }
}

/// Validated inputs for estimating a candidate observation's information gain.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "InformationGainInputWire")]
pub struct InformationGainInput {
    current_confidence: Confidence,
    outcomes: Vec<CandidateEvidenceOutcome>,
}

#[derive(Deserialize)]
struct InformationGainInputWire {
    current_confidence: Confidence,
    outcomes: Vec<CandidateEvidenceOutcome>,
}

impl InformationGainInput {
    /// Creates an information-gain request from a prior and candidate outcomes.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::InvalidInformationGainInput`] when `outcomes` is
    /// empty or its probabilities do not sum to one.
    pub fn new(
        current_confidence: Confidence,
        outcomes: Vec<CandidateEvidenceOutcome>,
    ) -> Result<Self, GraphError> {
        if outcomes.is_empty() {
            return Err(GraphError::InvalidInformationGainInput(
                "candidate outcomes must not be empty".to_owned(),
            ));
        }

        let probability_sum = outcomes
            .iter()
            .map(|outcome| outcome.probability().value())
            .sum::<f64>();
        if (probability_sum - 1.0).abs() > NORMALIZATION_TOLERANCE {
            return Err(GraphError::InvalidInformationGainInput(format!(
                "candidate outcome probabilities must sum to one, got {probability_sum}"
            )));
        }

        Ok(Self {
            current_confidence,
            outcomes,
        })
    }

    /// Returns the confidence before observing the candidate evidence.
    #[must_use]
    pub const fn current_confidence(&self) -> Confidence {
        self.current_confidence
    }

    /// Returns the candidate outcome distribution.
    #[must_use]
    pub fn outcomes(&self) -> &[CandidateEvidenceOutcome] {
        &self.outcomes
    }
}

impl TryFrom<InformationGainInputWire> for InformationGainInput {
    type Error = GraphError;

    fn try_from(input: InformationGainInputWire) -> Result<Self, Self::Error> {
        Self::new(input.current_confidence, input.outcomes)
    }
}

/// Bounded information-gain measures for a candidate observation.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct InformationGainEstimate {
    current_uncertainty_bits: f64,
    expected_posterior_uncertainty_bits: f64,
    expected_information_gain_bits: f64,
    expected_uncertainty_reduction: f64,
}

impl InformationGainEstimate {
    /// Returns current binary Shannon uncertainty in bits.
    #[must_use]
    pub const fn current_uncertainty_bits(self) -> f64 {
        self.current_uncertainty_bits
    }

    /// Returns probability-weighted posterior uncertainty in bits.
    #[must_use]
    pub const fn expected_posterior_uncertainty_bits(self) -> f64 {
        self.expected_posterior_uncertainty_bits
    }

    /// Returns expected information gain in bits.
    #[must_use]
    pub const fn expected_information_gain_bits(self) -> f64 {
        self.expected_information_gain_bits
    }

    /// Returns expected uncertainty reduction relative to current uncertainty.
    #[must_use]
    pub const fn expected_uncertainty_reduction(self) -> f64 {
        self.expected_uncertainty_reduction
    }
}

/// Estimates the information value of observing a candidate outcome.
///
/// The calculation uses binary Shannon entropy for both the current and
/// forecast posterior confidence, then probability-weights the posterior
/// values. All outputs are bounded to the inclusive unit interval. Expected
/// information gain is floored at zero when a forecast would increase
/// uncertainty. Already-certain inputs have zero relative reduction.
#[must_use]
pub fn estimate_information_gain(input: &InformationGainInput) -> InformationGainEstimate {
    let current_uncertainty_bits = binary_entropy(input.current_confidence());
    let expected_posterior_uncertainty_bits = clamp_unit(
        input
            .outcomes()
            .iter()
            .map(|outcome| {
                outcome.probability().value()
                    * binary_entropy(outcome.post_observation_confidence())
            })
            .sum(),
    );
    let expected_information_gain_bits =
        clamp_unit((current_uncertainty_bits - expected_posterior_uncertainty_bits).max(0.0));
    let expected_uncertainty_reduction = if current_uncertainty_bits == 0.0 {
        0.0
    } else {
        clamp_unit(expected_information_gain_bits / current_uncertainty_bits)
    };

    InformationGainEstimate {
        current_uncertainty_bits,
        expected_posterior_uncertainty_bits,
        expected_information_gain_bits,
        expected_uncertainty_reduction,
    }
}

fn binary_entropy(confidence: Confidence) -> f64 {
    let probability = confidence.value();
    if probability == 0.0 || probability == 1.0 {
        return 0.0;
    }

    clamp_unit(
        -(probability * probability.log2() + (1.0 - probability) * (1.0 - probability).log2()),
    )
}

fn clamp_unit(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

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
//! Deterministic, explainable comparison of hypothetical branches.
//!
//! This module scores existing branch overlays without mutating their world,
//! branch, overlay, or canonical base-fact state.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::{BranchId, GraphError, HypothesisWorldModel, WorldId};

/// A finite normalized branch-score term in the inclusive unit interval.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct CrossBranchScoreTerm(f64);

impl CrossBranchScoreTerm {
    /// Creates a validated normalized score term.
    ///
    /// # Errors
    ///
    /// Rejects non-finite values and values outside `[0, 1]` with
    /// [`GraphError::InvalidCrossBranchScoreTerm`].
    pub fn new(value: f64) -> Result<Self, GraphError> {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(GraphError::InvalidCrossBranchScoreTerm(value));
        }
        Ok(Self(value))
    }

    /// Returns the normalized scalar.
    #[must_use]
    pub const fn value(self) -> f64 {
        self.0
    }
}

impl TryFrom<f64> for CrossBranchScoreTerm {
    type Error = GraphError;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<CrossBranchScoreTerm> for f64 {
    fn from(term: CrossBranchScoreTerm) -> Self {
        term.value()
    }
}

/// Complete positive and negative terms explaining one branch score.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CrossBranchScoreBreakdown {
    evidence_support: CrossBranchScoreTerm,
    prediction_quality: CrossBranchScoreTerm,
    contradiction_penalty: CrossBranchScoreTerm,
}

impl CrossBranchScoreBreakdown {
    /// Creates a complete branch-score explanation.
    #[must_use]
    pub const fn new(
        evidence_support: CrossBranchScoreTerm,
        prediction_quality: CrossBranchScoreTerm,
        contradiction_penalty: CrossBranchScoreTerm,
    ) -> Self {
        Self {
            evidence_support,
            prediction_quality,
            contradiction_penalty,
        }
    }

    /// Returns the evidence-support benefit.
    #[must_use]
    pub const fn evidence_support(self) -> CrossBranchScoreTerm {
        self.evidence_support
    }

    /// Returns the predictive-quality benefit.
    #[must_use]
    pub const fn prediction_quality(self) -> CrossBranchScoreTerm {
        self.prediction_quality
    }

    /// Returns the unresolved-contradiction penalty.
    #[must_use]
    pub const fn contradiction_penalty(self) -> CrossBranchScoreTerm {
        self.contradiction_penalty
    }

    /// Returns support plus prediction quality minus contradiction penalty.
    #[must_use]
    pub fn total(self) -> f64 {
        self.evidence_support.value() + self.prediction_quality.value()
            - self.contradiction_penalty.value()
    }
}

/// Typed input associating one branch scope with its score explanation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CrossBranchScoreInput {
    world_id: WorldId,
    branch_id: BranchId,
    score_breakdown: CrossBranchScoreBreakdown,
}

impl CrossBranchScoreInput {
    /// Creates one branch-scoring input.
    #[must_use]
    pub const fn new(
        world_id: WorldId,
        branch_id: BranchId,
        score_breakdown: CrossBranchScoreBreakdown,
    ) -> Self {
        Self {
            world_id,
            branch_id,
            score_breakdown,
        }
    }

    /// Returns the target world identifier.
    #[must_use]
    pub const fn world_id(&self) -> &WorldId {
        &self.world_id
    }

    /// Returns the target branch identifier.
    #[must_use]
    pub const fn branch_id(&self) -> &BranchId {
        &self.branch_id
    }

    /// Returns the complete score explanation.
    #[must_use]
    pub const fn score_breakdown(&self) -> CrossBranchScoreBreakdown {
        self.score_breakdown
    }
}

/// One scored branch in deterministic rank order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RankedBranchScore {
    world_id: WorldId,
    branch_id: BranchId,
    score_breakdown: CrossBranchScoreBreakdown,
}

impl RankedBranchScore {
    /// Returns the ranked world identifier.
    #[must_use]
    pub const fn world_id(&self) -> &WorldId {
        &self.world_id
    }

    /// Returns the ranked branch identifier.
    #[must_use]
    pub const fn branch_id(&self) -> &BranchId {
        &self.branch_id
    }

    /// Returns the preserved score explanation.
    #[must_use]
    pub const fn score_breakdown(&self) -> CrossBranchScoreBreakdown {
        self.score_breakdown
    }

    /// Returns the calculated branch score.
    #[must_use]
    pub fn total(&self) -> f64 {
        self.score_breakdown.total()
    }
}

/// Deterministic ranking of validated branch-scoring inputs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CrossBranchRanking {
    ranked_branches: Vec<RankedBranchScore>,
}

impl CrossBranchRanking {
    /// Returns all compared branches in rank order.
    #[must_use]
    pub fn ranked_branches(&self) -> &[RankedBranchScore] {
        self.ranked_branches.as_slice()
    }

    /// Returns the highest-ranked branch.
    #[must_use]
    pub fn selected(&self) -> Option<&RankedBranchScore> {
        self.ranked_branches.first()
    }
}

/// Validates and ranks branches without mutating the world model.
///
/// # Errors
///
/// Rejects empty or duplicate inputs and world/branch scopes whose overlay
/// cannot be resolved.
pub fn rank_cross_branch_scores(
    model: &HypothesisWorldModel,
    inputs: Vec<CrossBranchScoreInput>,
) -> Result<CrossBranchRanking, GraphError> {
    if inputs.is_empty() {
        return Err(GraphError::InvalidCrossBranchComparison(
            "at least one branch score input is required".to_owned(),
        ));
    }

    let mut seen_scopes = HashSet::with_capacity(inputs.len());
    let mut ranked_branches = Vec::with_capacity(inputs.len());
    for input in inputs {
        model
            .branch_overlay(input.world_id(), input.branch_id())
            .map_err(|_| {
                GraphError::InvalidCrossBranchComparison(format!(
                    "branch overlay not found for world {} and branch {}",
                    input.world_id().as_str(),
                    input.branch_id().as_str()
                ))
            })?;

        let scope = (input.world_id.clone(), input.branch_id.clone());
        if !seen_scopes.insert(scope) {
            return Err(GraphError::InvalidCrossBranchComparison(format!(
                "duplicate branch score input for world {} and branch {}",
                input.world_id().as_str(),
                input.branch_id().as_str()
            )));
        }

        ranked_branches.push(RankedBranchScore {
            world_id: input.world_id,
            branch_id: input.branch_id,
            score_breakdown: input.score_breakdown,
        });
    }

    ranked_branches.sort_by(|left, right| {
        right
            .total()
            .total_cmp(&left.total())
            .then_with(|| left.world_id.as_str().cmp(right.world_id.as_str()))
            .then_with(|| left.branch_id.as_str().cmp(right.branch_id.as_str()))
    });

    Ok(CrossBranchRanking { ranked_branches })
}

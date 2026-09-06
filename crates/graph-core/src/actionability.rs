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
//! Permission to act is a separate, fail-closed policy over named dimensions.
use crate::{Confidence, ConfidenceDimensions, GraphError, VerdictState};
use serde::{Deserialize, Serialize};
/// An explicit reason that prevents action or export.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionabilityBlocker {
    /// No current grounded deterministic pass covers the claim.
    DeterministicVerificationMissing,
    /// Too few independent, positively weighted support clusters.
    IndependentCorroborationMissing,
    /// Contradictions exceed the configured limit.
    ContradictionThresholdExceeded,
    /// Temporal evidence is stale.
    TemporalValidityStale,
    /// A required dimension is absent.
    RequiredDimensionMissing,
    /// Belief state is not supported or mixed.
    VerdictNotSupported,
    /// No positive evidence supports the claim.
    EvidenceInsufficient,
}
/// Versioned policy selected by the caller for a claim predicate class.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActionabilityPolicy {
    version: String,
    minimum_clusters: usize,
    maximum_contradiction: Confidence,
}
impl Default for ActionabilityPolicy {
    fn default() -> Self {
        Self {
            version: "actionability-v1".into(),
            minimum_clusters: 2,
            maximum_contradiction: Confidence::new(0.25).expect("constant"),
        }
    }
}
impl ActionabilityPolicy {
    /// Configure corroboration and contradiction requirements for a claim type.
    pub fn new(
        version: impl Into<String>,
        minimum_clusters: usize,
        maximum_contradiction: Confidence,
    ) -> Result<Self, GraphError> {
        let version = version.into();
        if version.trim().is_empty() || minimum_clusters == 0 {
            return Err(GraphError::InvalidPropertyValue(
                "actionability policy requires a version and at least one cluster".into(),
            ));
        }
        Ok(Self {
            version,
            minimum_clusters,
            maximum_contradiction,
        })
    }
    /// Evaluate every condition independently and retain all blockers.
    pub fn evaluate(
        &self,
        dimensions: &ConfidenceDimensions,
        independent_clusters: usize,
        deterministic_coverage: bool,
        state: VerdictState,
    ) -> ActionabilityAssessment {
        // Missing dimensions abstain. Explicit failures remain blocked even with
        // maximal support; caller supplies current grounded verifier coverage.
        let mut blockers = Vec::new();
        if !deterministic_coverage {
            blockers.push(ActionabilityBlocker::DeterministicVerificationMissing);
        }
        if independent_clusters < self.minimum_clusters {
            blockers.push(ActionabilityBlocker::IndependentCorroborationMissing);
        }
        if dimensions
            .contradiction_load
            .is_some_and(|v| v.value() > self.maximum_contradiction.value())
        {
            blockers.push(ActionabilityBlocker::ContradictionThresholdExceeded);
        }
        if dimensions
            .temporal_validity
            .is_some_and(|v| v.value() < 1.0)
        {
            blockers.push(ActionabilityBlocker::TemporalValidityStale);
        }
        let missing = dimensions.evidence_sufficiency.is_none()
            || dimensions.source_independence.is_none()
            || dimensions.contradiction_load.is_none()
            || dimensions.temporal_validity.is_none();
        if missing {
            blockers.push(ActionabilityBlocker::RequiredDimensionMissing);
        }
        if !matches!(state, VerdictState::Supported | VerdictState::Mixed) {
            blockers.push(ActionabilityBlocker::VerdictNotSupported);
        }
        if dimensions
            .evidence_sufficiency
            .is_some_and(|v| v.value() == 0.0)
        {
            blockers.push(ActionabilityBlocker::EvidenceInsufficient);
        }
        let dimension = (!missing).then(|| {
            Confidence::new(if blockers.is_empty() { 1.0 } else { 0.0 }).expect("binary permission")
        });
        ActionabilityAssessment {
            policy: self.clone(),
            blockers,
            dimension,
        }
    }
}
/// Persisted permission decision with the exact policy and blocking reasons.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActionabilityAssessment {
    policy: ActionabilityPolicy,
    blockers: Vec<ActionabilityBlocker>,
    dimension: Option<Confidence>,
}
impl ActionabilityAssessment {
    /// Whether all gates passed.
    pub fn is_actionable(&self) -> bool {
        self.dimension.is_some_and(|v| v.value() == 1.0) && self.blockers.is_empty()
    }
    /// Every blocking condition in stable policy order.
    pub fn blockers(&self) -> &[ActionabilityBlocker] {
        &self.blockers
    }
    /// Binary permission, absent when required dimensions are unavailable.
    pub fn dimension(&self) -> Option<Confidence> {
        self.dimension
    }
    /// Policy used to evaluate this snapshot.
    pub fn policy(&self) -> &ActionabilityPolicy {
        &self.policy
    }
}

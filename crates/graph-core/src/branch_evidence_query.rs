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
//! Deterministic counterfactual and discriminating-evidence queries.
//!
//! Query observations explicitly connect evidence provenance to expected,
//! contradictory, or neutral effects in selected hypothetical branches.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::{BranchId, EvidenceId, GraphError, HypothesisWorldModel, WorldId};

/// Typed selector for one branch overlay.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BranchSelector {
    world_id: WorldId,
    branch_id: BranchId,
}

impl BranchSelector {
    /// Creates a branch selector.
    #[must_use]
    pub const fn new(world_id: WorldId, branch_id: BranchId) -> Self {
        Self {
            world_id,
            branch_id,
        }
    }

    /// Returns the world identifier.
    #[must_use]
    pub const fn world_id(&self) -> &WorldId {
        &self.world_id
    }

    /// Returns the branch identifier.
    #[must_use]
    pub const fn branch_id(&self) -> &BranchId {
        &self.branch_id
    }
}

/// Effect of one evidence observation under a selected branch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BranchObservationEffect {
    /// The observation is expected if the branch hypothesis is correct.
    Expected,
    /// The observation contradicts the branch hypothesis.
    Contradicts,
    /// The observation neither supports nor contradicts the branch.
    Neutral,
}

/// One branch-specific assessment of an evidence observation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchObservationAssessment {
    selector: BranchSelector,
    effect: BranchObservationEffect,
}

impl BranchObservationAssessment {
    /// Creates a branch-specific assessment.
    #[must_use]
    pub const fn new(selector: BranchSelector, effect: BranchObservationEffect) -> Self {
        Self { selector, effect }
    }

    /// Returns the assessed branch.
    #[must_use]
    pub const fn selector(&self) -> &BranchSelector {
        &self.selector
    }

    /// Returns the observation effect.
    #[must_use]
    pub const fn effect(&self) -> BranchObservationEffect {
        self.effect
    }
}

/// Evidence observation with provenance and per-branch effects.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchEvidenceObservation {
    evidence_id: EvidenceId,
    source_ref: String,
    description: String,
    assessments: Vec<BranchObservationAssessment>,
}

impl BranchEvidenceObservation {
    /// Creates a validated branch evidence observation.
    ///
    /// # Errors
    ///
    /// Rejects blank provenance or description values, empty assessments, and
    /// duplicate branch assessments.
    pub fn new(
        evidence_id: EvidenceId,
        source_ref: impl Into<String>,
        description: impl Into<String>,
        mut assessments: Vec<BranchObservationAssessment>,
    ) -> Result<Self, GraphError> {
        let source_ref = source_ref.into();
        if source_ref.trim().is_empty() {
            return Err(invalid_query(
                "observation source reference must not be blank",
            ));
        }
        let description = description.into();
        if description.trim().is_empty() {
            return Err(invalid_query("observation description must not be blank"));
        }
        if assessments.is_empty() {
            return Err(invalid_query("observation must assess at least one branch"));
        }

        assessments.sort_by(|left, right| selector_cmp(left.selector(), right.selector()));
        if assessments
            .windows(2)
            .any(|pair| pair[0].selector() == pair[1].selector())
        {
            return Err(invalid_query(
                "observation contains duplicate branch assessments",
            ));
        }

        Ok(Self {
            evidence_id,
            source_ref,
            description,
            assessments,
        })
    }

    /// Returns the evidence identifier.
    #[must_use]
    pub const fn evidence_id(&self) -> &EvidenceId {
        &self.evidence_id
    }

    /// Returns the provenance source reference.
    #[must_use]
    pub fn source_ref(&self) -> &str {
        self.source_ref.as_str()
    }

    /// Returns the observation description.
    #[must_use]
    pub fn description(&self) -> &str {
        self.description.as_str()
    }

    /// Returns branch assessments in selector order.
    #[must_use]
    pub fn assessments(&self) -> &[BranchObservationAssessment] {
        self.assessments.as_slice()
    }
}

/// Expected and contradictory observations under one branch hypothesis.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CounterfactualExpectedFactsResult {
    selector: BranchSelector,
    expected_observations: Vec<BranchEvidenceObservation>,
    contradicting_observations: Vec<BranchEvidenceObservation>,
}

impl CounterfactualExpectedFactsResult {
    /// Returns the queried branch.
    #[must_use]
    pub const fn selector(&self) -> &BranchSelector {
        &self.selector
    }

    /// Returns expected observations in evidence-ID order.
    #[must_use]
    pub fn expected_observations(&self) -> &[BranchEvidenceObservation] {
        self.expected_observations.as_slice()
    }

    /// Returns contradictory observations in evidence-ID order.
    #[must_use]
    pub fn contradicting_observations(&self) -> &[BranchEvidenceObservation] {
        self.contradicting_observations.as_slice()
    }
}

/// Observation whose effects differ across selected branches.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscriminatingObservation {
    evidence_id: EvidenceId,
    source_ref: String,
    description: String,
    assessments: Vec<BranchObservationAssessment>,
}

impl DiscriminatingObservation {
    /// Returns the evidence identifier.
    #[must_use]
    pub const fn evidence_id(&self) -> &EvidenceId {
        &self.evidence_id
    }

    /// Returns the provenance source reference.
    #[must_use]
    pub fn source_ref(&self) -> &str {
        self.source_ref.as_str()
    }

    /// Returns the description.
    #[must_use]
    pub fn description(&self) -> &str {
        self.description.as_str()
    }

    /// Returns one normalized assessment per selected branch.
    #[must_use]
    pub fn assessments(&self) -> &[BranchObservationAssessment] {
        self.assessments.as_slice()
    }
}

/// Deterministic discriminating-evidence query output.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscriminatingEvidenceResult {
    selectors: Vec<BranchSelector>,
    observations: Vec<DiscriminatingObservation>,
}

impl DiscriminatingEvidenceResult {
    /// Returns selected branches in lexical order.
    #[must_use]
    pub fn selectors(&self) -> &[BranchSelector] {
        self.selectors.as_slice()
    }

    /// Returns discriminating observations in evidence-ID order.
    #[must_use]
    pub fn observations(&self) -> &[DiscriminatingObservation] {
        self.observations.as_slice()
    }
}

/// Smallest deterministic disproving-evidence query output.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmallestDisprovingEvidenceResult {
    selector: BranchSelector,
    evidence: Option<BranchEvidenceObservation>,
}

impl SmallestDisprovingEvidenceResult {
    /// Returns the queried branch.
    #[must_use]
    pub const fn selector(&self) -> &BranchSelector {
        &self.selector
    }

    /// Returns the lexically first one-record disproof, when one exists.
    #[must_use]
    pub const fn evidence(&self) -> Option<&BranchEvidenceObservation> {
        self.evidence.as_ref()
    }
}

/// Evidence removed from one branch assessment when a source is excluded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRemovalBranchImpact {
    selector: BranchSelector,
    removed_expected_evidence: Vec<EvidenceId>,
    removed_contradicting_evidence: Vec<EvidenceId>,
}

impl SourceRemovalBranchImpact {
    /// Returns the affected branch.
    #[must_use]
    pub const fn selector(&self) -> &BranchSelector {
        &self.selector
    }

    /// Returns expected evidence lost with the source.
    #[must_use]
    pub fn removed_expected_evidence(&self) -> &[EvidenceId] {
        self.removed_expected_evidence.as_slice()
    }

    /// Returns contradictory evidence lost with the source.
    #[must_use]
    pub fn removed_contradicting_evidence(&self) -> &[EvidenceId] {
        self.removed_contradicting_evidence.as_slice()
    }
}

/// Deterministic source-removal impact output.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRemovalImpactResult {
    source_ref: String,
    branch_impacts: Vec<SourceRemovalBranchImpact>,
}

impl SourceRemovalImpactResult {
    /// Returns the removed source reference.
    #[must_use]
    pub fn source_ref(&self) -> &str {
        self.source_ref.as_str()
    }

    /// Returns per-branch impacts in selector order.
    #[must_use]
    pub fn branch_impacts(&self) -> &[SourceRemovalBranchImpact] {
        self.branch_impacts.as_slice()
    }
}

/// Queries expected and contradictory observations for one branch.
///
/// # Errors
///
/// Rejects invalid selectors and malformed observation sets.
pub fn query_counterfactual_expected_facts(
    model: &HypothesisWorldModel,
    selector: &BranchSelector,
    observations: &[BranchEvidenceObservation],
) -> Result<CounterfactualExpectedFactsResult, GraphError> {
    validate_selector(model, selector)?;
    validate_observations(model, observations)?;

    let mut expected_observations = observations
        .iter()
        .filter(|observation| {
            effect_for(observation, selector) == BranchObservationEffect::Expected
        })
        .cloned()
        .collect::<Vec<_>>();
    let mut contradicting_observations = observations
        .iter()
        .filter(|observation| {
            effect_for(observation, selector) == BranchObservationEffect::Contradicts
        })
        .cloned()
        .collect::<Vec<_>>();
    sort_observations(&mut expected_observations);
    sort_observations(&mut contradicting_observations);

    Ok(CounterfactualExpectedFactsResult {
        selector: selector.clone(),
        expected_observations,
        contradicting_observations,
    })
}

/// Finds observations whose effects differ across selected branches.
///
/// # Errors
///
/// Requires at least two unique, valid selectors.
pub fn query_discriminating_observations(
    model: &HypothesisWorldModel,
    selectors: Vec<BranchSelector>,
    observations: &[BranchEvidenceObservation],
) -> Result<DiscriminatingEvidenceResult, GraphError> {
    let selectors = validate_selectors(model, selectors, 2)?;
    validate_observations(model, observations)?;

    let mut discriminating = Vec::new();
    for observation in observations {
        let assessments = selectors
            .iter()
            .map(|selector| {
                BranchObservationAssessment::new(
                    selector.clone(),
                    effect_for(observation, selector),
                )
            })
            .collect::<Vec<_>>();
        let first_effect = assessments[0].effect();
        if assessments
            .iter()
            .skip(1)
            .any(|assessment| assessment.effect() != first_effect)
        {
            discriminating.push(DiscriminatingObservation {
                evidence_id: observation.evidence_id.clone(),
                source_ref: observation.source_ref.clone(),
                description: observation.description.clone(),
                assessments,
            });
        }
    }
    discriminating.sort_by(|left, right| left.evidence_id.as_str().cmp(right.evidence_id.as_str()));

    Ok(DiscriminatingEvidenceResult {
        selectors,
        observations: discriminating,
    })
}

/// Finds the stable smallest one-record disproof for a branch.
///
/// # Errors
///
/// Rejects invalid selectors and malformed observation sets.
pub fn query_smallest_disproving_evidence(
    model: &HypothesisWorldModel,
    selector: &BranchSelector,
    observations: &[BranchEvidenceObservation],
) -> Result<SmallestDisprovingEvidenceResult, GraphError> {
    validate_selector(model, selector)?;
    validate_observations(model, observations)?;

    let evidence = observations
        .iter()
        .filter(|observation| {
            effect_for(observation, selector) == BranchObservationEffect::Contradicts
        })
        .min_by(|left, right| {
            left.evidence_id()
                .as_str()
                .cmp(right.evidence_id().as_str())
        })
        .cloned();

    Ok(SmallestDisprovingEvidenceResult {
        selector: selector.clone(),
        evidence,
    })
}

/// Calculates evidence effects removed with one provenance source.
///
/// # Errors
///
/// Rejects blank sources, invalid selectors, and malformed observation sets.
pub fn query_source_removal_impact(
    model: &HypothesisWorldModel,
    source_ref: &str,
    selectors: Vec<BranchSelector>,
    observations: &[BranchEvidenceObservation],
) -> Result<SourceRemovalImpactResult, GraphError> {
    if source_ref.trim().is_empty() {
        return Err(invalid_query("source-removal reference must not be blank"));
    }
    let selectors = validate_selectors(model, selectors, 1)?;
    validate_observations(model, observations)?;

    let branch_impacts = selectors
        .iter()
        .map(|selector| {
            let mut removed_expected_evidence = Vec::new();
            let mut removed_contradicting_evidence = Vec::new();
            for observation in observations
                .iter()
                .filter(|observation| observation.source_ref() == source_ref)
            {
                match effect_for(observation, selector) {
                    BranchObservationEffect::Expected => {
                        removed_expected_evidence.push(observation.evidence_id.clone());
                    }
                    BranchObservationEffect::Contradicts => {
                        removed_contradicting_evidence.push(observation.evidence_id.clone());
                    }
                    BranchObservationEffect::Neutral => {}
                }
            }
            sort_evidence_ids(&mut removed_expected_evidence);
            sort_evidence_ids(&mut removed_contradicting_evidence);
            SourceRemovalBranchImpact {
                selector: selector.clone(),
                removed_expected_evidence,
                removed_contradicting_evidence,
            }
        })
        .collect();

    Ok(SourceRemovalImpactResult {
        source_ref: source_ref.to_owned(),
        branch_impacts,
    })
}

fn effect_for(
    observation: &BranchEvidenceObservation,
    selector: &BranchSelector,
) -> BranchObservationEffect {
    observation
        .assessments()
        .iter()
        .find(|assessment| assessment.selector() == selector)
        .map_or(BranchObservationEffect::Neutral, |assessment| {
            assessment.effect()
        })
}

fn validate_selector(
    model: &HypothesisWorldModel,
    selector: &BranchSelector,
) -> Result<(), GraphError> {
    model
        .branch_overlay(selector.world_id(), selector.branch_id())
        .map(|_| ())
        .map_err(|_| {
            invalid_query(format!(
                "branch overlay not found for world {} and branch {}",
                selector.world_id().as_str(),
                selector.branch_id().as_str()
            ))
        })
}

fn validate_selectors(
    model: &HypothesisWorldModel,
    mut selectors: Vec<BranchSelector>,
    minimum: usize,
) -> Result<Vec<BranchSelector>, GraphError> {
    if selectors.len() < minimum {
        return Err(invalid_query(format!(
            "query requires at least {minimum} branch selector(s)"
        )));
    }
    selectors.sort_by(selector_cmp);
    if selectors.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(invalid_query("query contains duplicate branch selectors"));
    }
    for selector in &selectors {
        validate_selector(model, selector)?;
    }
    Ok(selectors)
}

fn validate_observations(
    model: &HypothesisWorldModel,
    observations: &[BranchEvidenceObservation],
) -> Result<(), GraphError> {
    let mut evidence_ids = HashSet::with_capacity(observations.len());
    for observation in observations {
        if !evidence_ids.insert(observation.evidence_id().clone()) {
            return Err(invalid_query(format!(
                "duplicate observation evidence identifier: {}",
                observation.evidence_id().as_str()
            )));
        }
        for assessment in observation.assessments() {
            validate_selector(model, assessment.selector())?;
        }
    }
    Ok(())
}

fn selector_cmp(left: &BranchSelector, right: &BranchSelector) -> std::cmp::Ordering {
    left.world_id()
        .as_str()
        .cmp(right.world_id().as_str())
        .then_with(|| left.branch_id().as_str().cmp(right.branch_id().as_str()))
}

fn sort_observations(observations: &mut [BranchEvidenceObservation]) {
    observations.sort_by(|left, right| {
        left.evidence_id()
            .as_str()
            .cmp(right.evidence_id().as_str())
    });
}

fn sort_evidence_ids(evidence_ids: &mut [EvidenceId]) {
    evidence_ids.sort_by(|left, right| left.as_str().cmp(right.as_str()));
}

fn invalid_query(message: impl Into<String>) -> GraphError {
    GraphError::InvalidBranchEvidenceQuery(message.into())
}

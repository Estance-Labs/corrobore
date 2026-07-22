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
//! Explicit, audited resolution of hypothetical branches.
//!
//! Resolution is modeled separately from branch-local overlays: a terminal
//! decision changes branch lifecycle state while preserving the overlay and
//! immutable shared base. Successful merges create canonical promotion records;
//! discards create audit history only.

use serde::{Deserialize, Serialize};

use crate::{
    ActorId, BranchOverlayReference, BranchSelector, BranchStatus, GraphError,
    HypothesisWorldModel, TemporalTimestamp,
};

/// Typed identifier for one audited branch-resolution decision.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BranchResolutionDecisionId {
    value: String,
}

impl BranchResolutionDecisionId {
    /// Creates a validated branch-resolution decision identifier.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::InvalidIdentifier`] when `value` is blank.
    pub fn new(value: impl Into<String>) -> Result<Self, GraphError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(GraphError::InvalidIdentifier(
                "BranchResolutionDecisionId".to_owned(),
            ));
        }
        Ok(Self { value })
    }

    /// Returns the identifier as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.value.as_str()
    }
}

/// Terminal outcome explicitly selected for one active branch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BranchResolutionKind {
    /// Promote selected branch conclusions into canonical promotion records.
    Merge,
    /// Close the branch without changing canonical promotion state.
    Discard,
}

/// Mandatory provenance explaining who made a resolution decision and why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchResolutionAuditMetadata {
    decided_by: ActorId,
    decided_at: TemporalTimestamp,
    rationale: String,
}

impl BranchResolutionAuditMetadata {
    /// Creates mandatory resolution audit metadata.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::InvalidBranchResolution`] when `rationale` is
    /// blank.
    pub fn new(
        decided_by: ActorId,
        decided_at: TemporalTimestamp,
        rationale: impl Into<String>,
    ) -> Result<Self, GraphError> {
        let rationale = rationale.into();
        validate_non_blank("resolution rationale", &rationale)?;
        Ok(Self {
            decided_by,
            decided_at,
            rationale,
        })
    }

    /// Returns the actor who made the decision.
    #[must_use]
    pub const fn decided_by(&self) -> &ActorId {
        &self.decided_by
    }

    /// Returns when the decision was made.
    #[must_use]
    pub const fn decided_at(&self) -> &TemporalTimestamp {
        &self.decided_at
    }

    /// Returns the decision rationale.
    #[must_use]
    pub fn rationale(&self) -> &str {
        self.rationale.as_str()
    }
}

/// Independent validation required before branch conclusions may be promoted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchValidationAuditMetadata {
    validated_by: ActorId,
    validated_at: TemporalTimestamp,
    rationale: String,
}

impl BranchValidationAuditMetadata {
    /// Creates mandatory validation provenance for a merge decision.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::InvalidBranchResolution`] when `rationale` is
    /// blank.
    pub fn new(
        validated_by: ActorId,
        validated_at: TemporalTimestamp,
        rationale: impl Into<String>,
    ) -> Result<Self, GraphError> {
        let rationale = rationale.into();
        validate_non_blank("validation rationale", &rationale)?;
        Ok(Self {
            validated_by,
            validated_at,
            rationale,
        })
    }

    /// Returns the actor who validated the promotion.
    #[must_use]
    pub const fn validated_by(&self) -> &ActorId {
        &self.validated_by
    }

    /// Returns when the promotion was validated.
    #[must_use]
    pub const fn validated_at(&self) -> &TemporalTimestamp {
        &self.validated_at
    }

    /// Returns the validation rationale.
    #[must_use]
    pub fn rationale(&self) -> &str {
        self.rationale.as_str()
    }
}

/// Complete explicit request to merge or discard one branch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchResolutionDecision {
    decision_id: BranchResolutionDecisionId,
    selector: BranchSelector,
    kind: BranchResolutionKind,
    audit: BranchResolutionAuditMetadata,
    validation: Option<BranchValidationAuditMetadata>,
    promoted_references: Vec<BranchOverlayReference>,
}

impl BranchResolutionDecision {
    /// Creates an audited decision before optional merge validation and
    /// promotion references are attached.
    #[must_use]
    pub const fn new(
        decision_id: BranchResolutionDecisionId,
        selector: BranchSelector,
        kind: BranchResolutionKind,
        audit: BranchResolutionAuditMetadata,
    ) -> Self {
        Self {
            decision_id,
            selector,
            kind,
            audit,
            validation: None,
            promoted_references: Vec::new(),
        }
    }

    /// Attaches the explicit validation gate required by merge decisions.
    #[must_use]
    pub fn with_validation(mut self, validation: BranchValidationAuditMetadata) -> Self {
        self.validation = Some(validation);
        self
    }

    /// Selects one branch-local conclusion for canonical promotion.
    #[must_use]
    pub fn with_promoted_reference(mut self, reference: BranchOverlayReference) -> Self {
        self.promoted_references.push(reference);
        self
    }

    /// Returns the decision identifier.
    #[must_use]
    pub const fn decision_id(&self) -> &BranchResolutionDecisionId {
        &self.decision_id
    }

    /// Returns the selected branch.
    #[must_use]
    pub const fn selector(&self) -> &BranchSelector {
        &self.selector
    }

    /// Returns the selected terminal outcome.
    #[must_use]
    pub const fn kind(&self) -> BranchResolutionKind {
        self.kind
    }

    /// Returns mandatory resolution provenance.
    #[must_use]
    pub const fn audit(&self) -> &BranchResolutionAuditMetadata {
        &self.audit
    }

    /// Returns optional merge-validation provenance.
    #[must_use]
    pub const fn validation(&self) -> Option<&BranchValidationAuditMetadata> {
        self.validation.as_ref()
    }

    /// Returns selected conclusions in canonical deterministic order after
    /// application.
    #[must_use]
    pub fn promoted_references(&self) -> &[BranchOverlayReference] {
        self.promoted_references.as_slice()
    }
}

/// Canonical record linking a validated merge to promoted branch conclusions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalPromotionRecord {
    decision_id: BranchResolutionDecisionId,
    selector: BranchSelector,
    promoted_references: Vec<BranchOverlayReference>,
}

impl CanonicalPromotionRecord {
    /// Returns the decision that authorized this promotion.
    #[must_use]
    pub const fn decision_id(&self) -> &BranchResolutionDecisionId {
        &self.decision_id
    }

    /// Returns the source branch.
    #[must_use]
    pub const fn selector(&self) -> &BranchSelector {
        &self.selector
    }

    /// Returns promoted conclusions in deterministic order.
    #[must_use]
    pub fn promoted_references(&self) -> &[BranchOverlayReference] {
        self.promoted_references.as_slice()
    }
}

/// Immutable-by-value world model plus its deterministic resolution history.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchResolutionLedger {
    world_model: HypothesisWorldModel,
    audit_trail: Vec<BranchResolutionDecision>,
    canonical_promotions: Vec<CanonicalPromotionRecord>,
}

impl BranchResolutionLedger {
    /// Starts an empty resolution ledger around an existing world model.
    #[must_use]
    pub const fn new(world_model: HypothesisWorldModel) -> Self {
        Self {
            world_model,
            audit_trail: Vec::new(),
            canonical_promotions: Vec::new(),
        }
    }

    /// Validates and applies one explicit terminal branch decision.
    ///
    /// Merge implementation will require validation provenance, at least one
    /// resolvable non-base reference, and no unresolved contradiction. Discard
    /// implementation will reject promotion data. Both paths will reject
    /// duplicate decision IDs and non-active branches before recording history.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::InvalidBranchResolution`] for invalid resolution
    /// input.
    pub fn apply_decision(
        mut self,
        mut decision: BranchResolutionDecision,
    ) -> Result<Self, GraphError> {
        if self
            .audit_trail
            .iter()
            .any(|record| record.decision_id == decision.decision_id)
        {
            return Err(GraphError::InvalidBranchResolution(format!(
                "duplicate branch-resolution decision identifier: {}",
                decision.decision_id.as_str()
            )));
        }

        let selector = decision.selector.clone();
        let overlay = self
            .world_model
            .branch_overlay(selector.world_id(), selector.branch_id())
            .map_err(|_| {
                GraphError::InvalidBranchResolution(format!(
                    "unknown branch scope: {}/{}",
                    selector.world_id().as_str(),
                    selector.branch_id().as_str()
                ))
            })?;
        let branch = self
            .world_model
            .world(selector.world_id())
            .and_then(|world| world.branch(selector.branch_id()))
            .ok_or_else(|| {
                GraphError::InvalidBranchResolution(format!(
                    "unknown branch scope: {}/{}",
                    selector.world_id().as_str(),
                    selector.branch_id().as_str()
                ))
            })?;
        if branch.status() != BranchStatus::Active {
            return Err(GraphError::InvalidBranchResolution(format!(
                "branch {}/{} is already terminal",
                selector.world_id().as_str(),
                selector.branch_id().as_str()
            )));
        }

        match decision.kind {
            BranchResolutionKind::Merge => {
                if decision.validation.is_none() {
                    return Err(GraphError::InvalidBranchResolution(
                        "merge requires explicit validation metadata".to_owned(),
                    ));
                }
                if decision.promoted_references.is_empty() {
                    return Err(GraphError::InvalidBranchResolution(
                        "merge requires at least one promoted branch conclusion".to_owned(),
                    ));
                }
                if !overlay.contradictions().is_empty() {
                    return Err(GraphError::InvalidBranchResolution(
                        "merge is blocked by unresolved branch contradictions".to_owned(),
                    ));
                }
                for reference in &decision.promoted_references {
                    if matches!(reference, BranchOverlayReference::BaseFact(_)) {
                        return Err(GraphError::InvalidBranchResolution(
                            "immutable base facts cannot be promoted from a branch".to_owned(),
                        ));
                    }
                    if !overlay.contains_reference(reference) {
                        return Err(GraphError::InvalidBranchResolution(format!(
                            "promoted branch reference does not resolve: {reference:?}"
                        )));
                    }
                }
                normalize_references(&mut decision.promoted_references)?;
            }
            BranchResolutionKind::Discard => {
                if decision.validation.is_some() {
                    return Err(GraphError::InvalidBranchResolution(
                        "discard must not include merge validation metadata".to_owned(),
                    ));
                }
                if !decision.promoted_references.is_empty() {
                    return Err(GraphError::InvalidBranchResolution(
                        "discard must not include promoted references".to_owned(),
                    ));
                }
            }
        }

        let terminal_status = match decision.kind {
            BranchResolutionKind::Merge => BranchStatus::Merged,
            BranchResolutionKind::Discard => BranchStatus::Discarded,
        };
        self.world_model.transition_branch_status(
            selector.world_id(),
            selector.branch_id(),
            terminal_status,
        )?;

        if decision.kind == BranchResolutionKind::Merge {
            self.canonical_promotions.push(CanonicalPromotionRecord {
                decision_id: decision.decision_id.clone(),
                selector,
                promoted_references: decision.promoted_references.clone(),
            });
            self.canonical_promotions
                .sort_by(|left, right| left.decision_id.as_str().cmp(right.decision_id.as_str()));
        }

        self.audit_trail.push(decision);
        self.audit_trail
            .sort_by(|left, right| left.decision_id.as_str().cmp(right.decision_id.as_str()));
        Ok(self)
    }

    /// Returns the world model with terminal branch statuses applied.
    #[must_use]
    pub const fn world_model(&self) -> &HypothesisWorldModel {
        &self.world_model
    }

    /// Returns complete decisions in deterministic decision-ID order.
    #[must_use]
    pub fn audit_trail(&self) -> &[BranchResolutionDecision] {
        self.audit_trail.as_slice()
    }

    /// Returns successful canonical promotions in deterministic decision-ID
    /// order.
    #[must_use]
    pub fn canonical_promotions(&self) -> &[CanonicalPromotionRecord] {
        self.canonical_promotions.as_slice()
    }
}

fn validate_non_blank(field: &str, value: &str) -> Result<(), GraphError> {
    if value.trim().is_empty() {
        return Err(GraphError::InvalidBranchResolution(format!(
            "{field} must not be blank"
        )));
    }
    Ok(())
}

fn normalize_references(references: &mut [BranchOverlayReference]) -> Result<(), GraphError> {
    references.sort_by(|left, right| reference_key(left).cmp(&reference_key(right)));
    if references.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(GraphError::InvalidBranchResolution(
            "duplicate promoted branch reference".to_owned(),
        ));
    }
    Ok(())
}

fn reference_key(reference: &BranchOverlayReference) -> (u8, &str) {
    match reference {
        BranchOverlayReference::BaseFact(id) => (0, id.as_str()),
        BranchOverlayReference::Hypothesis(id) => (1, id.as_str()),
        BranchOverlayReference::DerivedRelation(id) => (2, id.as_str()),
        BranchOverlayReference::Prediction(id) => (3, id.as_str()),
    }
}

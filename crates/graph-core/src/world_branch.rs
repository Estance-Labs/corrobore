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
//! Hypothetical world and branch model contracts (Epic 0021).
//!
//! This module defines typed world/branch identifiers, immutable shared base
//! fact references, branch metadata, and deterministic fork lifecycle
//! operations.

use serde::{Deserialize, Serialize};

use crate::{
    BranchContradiction, BranchDerivedRelation, BranchExpectedEvidence, BranchOverlay,
    BranchPrediction, FactId, GraphError, OverlayHypothesis,
};

/// Typed identifier of one hypothetical world.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct WorldId {
    value: String,
}

impl WorldId {
    /// Creates a validated world identifier.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::InvalidIdentifier`] when `value` is blank.
    pub fn new(value: impl Into<String>) -> Result<Self, GraphError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(GraphError::InvalidIdentifier("WorldId".to_owned()));
        }
        Ok(Self { value })
    }

    /// Returns the world identifier as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.value.as_str()
    }
}

/// Typed identifier of one branch within a world.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BranchId {
    value: String,
}

impl BranchId {
    /// Creates a validated branch identifier.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::InvalidIdentifier`] when `value` is blank.
    pub fn new(value: impl Into<String>) -> Result<Self, GraphError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(GraphError::InvalidIdentifier("BranchId".to_owned()));
        }
        Ok(Self { value })
    }

    /// Returns the branch identifier as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.value.as_str()
    }
}

/// Lifecycle status of a branch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BranchStatus {
    /// Branch is open for investigation.
    Active,
    /// Branch has been merged into a trusted target.
    Merged,
    /// Branch has been discarded.
    Discarded,
}

/// Input payload for creating a branch within a world.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchCreationInput {
    branch_id: BranchId,
    title: String,
    parent_branch_id: Option<BranchId>,
}

impl BranchCreationInput {
    /// Creates branch creation input.
    #[must_use]
    pub fn new(branch_id: BranchId, title: String) -> Self {
        Self {
            branch_id,
            title,
            parent_branch_id: None,
        }
    }

    /// Sets the parent branch.
    #[must_use]
    pub fn with_parent_branch_id(mut self, parent_branch_id: BranchId) -> Self {
        self.parent_branch_id = Some(parent_branch_id);
        self
    }
}

/// Deterministic branch descriptor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchDescriptor {
    branch_id: BranchId,
    title: String,
    parent_branch_id: Option<BranchId>,
    lineage: Vec<BranchId>,
    status: BranchStatus,
    created_sequence: u64,
    #[serde(default)]
    overlay: BranchOverlay,
}

impl BranchDescriptor {
    /// Returns branch identifier.
    #[must_use]
    pub fn branch_id(&self) -> &BranchId {
        &self.branch_id
    }

    /// Returns optional parent branch identifier.
    #[must_use]
    pub fn parent_branch_id(&self) -> Option<&BranchId> {
        self.parent_branch_id.as_ref()
    }

    /// Returns lineage from the root parent to the direct parent.
    #[must_use]
    pub fn lineage(&self) -> &[BranchId] {
        self.lineage.as_slice()
    }

    /// Returns branch status.
    #[must_use]
    pub const fn status(&self) -> BranchStatus {
        self.status
    }

    /// Returns the branch-local overlay.
    #[must_use]
    pub fn overlay(&self) -> &BranchOverlay {
        &self.overlay
    }
}

/// One world descriptor over immutable shared base facts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HypothesisWorldDescriptor {
    world_id: WorldId,
    title: String,
    base_facts: Vec<FactId>,
    branches: Vec<BranchDescriptor>,
    next_branch_sequence: u64,
}

impl HypothesisWorldDescriptor {
    /// Returns world identifier.
    #[must_use]
    pub fn world_id(&self) -> &WorldId {
        &self.world_id
    }

    /// Returns immutable base facts shared by this world.
    #[must_use]
    pub fn base_facts(&self) -> &[FactId] {
        self.base_facts.as_slice()
    }

    /// Returns branches in deterministic creation order.
    #[must_use]
    pub fn branches(&self) -> &[BranchDescriptor] {
        self.branches.as_slice()
    }

    /// Returns one branch descriptor by identifier.
    #[must_use]
    pub fn branch(&self, branch_id: &BranchId) -> Option<&BranchDescriptor> {
        self.branches
            .iter()
            .find(|branch| branch.branch_id() == branch_id)
    }
}

/// Top-level world/branch model over one immutable shared base.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HypothesisWorldModel {
    base_facts: Vec<FactId>,
    worlds: Vec<HypothesisWorldDescriptor>,
}

impl HypothesisWorldModel {
    /// Creates a world model over immutable shared base facts.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::InvalidWorldBranchModel`] when `base_facts` is
    /// empty.
    pub fn new(base_facts: Vec<FactId>) -> Result<Self, GraphError> {
        if base_facts.is_empty() {
            return Err(GraphError::InvalidWorldBranchModel(
                "shared immutable base facts must not be empty".to_owned(),
            ));
        }

        let mut base_facts = base_facts;
        base_facts.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        base_facts.dedup_by(|left, right| left == right);
        Ok(Self {
            base_facts,
            worlds: Vec::new(),
        })
    }

    /// Returns immutable shared base facts.
    #[must_use]
    pub fn base_facts(&self) -> &[FactId] {
        self.base_facts.as_slice()
    }

    /// Returns worlds in deterministic world-ID order.
    #[must_use]
    pub fn worlds(&self) -> &[HypothesisWorldDescriptor] {
        self.worlds.as_slice()
    }

    /// Returns one world by identifier.
    #[must_use]
    pub fn world(&self, world_id: &WorldId) -> Option<&HypothesisWorldDescriptor> {
        self.worlds
            .iter()
            .find(|world| world.world_id() == world_id)
    }

    /// Adds a new world that shares the model immutable base facts.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::InvalidWorldBranchModel`] when the world already
    /// exists or `title` is blank.
    pub fn create_world(mut self, world_id: WorldId, title: String) -> Result<Self, GraphError> {
        if title.trim().is_empty() {
            return Err(GraphError::InvalidWorldBranchModel(
                "world title must not be blank".to_owned(),
            ));
        }
        if self.world(&world_id).is_some() {
            return Err(GraphError::InvalidWorldBranchModel(format!(
                "duplicate world identifier: {}",
                world_id.as_str()
            )));
        }

        self.worlds.push(HypothesisWorldDescriptor {
            world_id,
            title,
            base_facts: self.base_facts.clone(),
            branches: Vec::new(),
            next_branch_sequence: 0,
        });
        self.worlds
            .sort_by(|left, right| left.world_id.as_str().cmp(right.world_id.as_str()));
        Ok(self)
    }

    /// Creates a branch in a target world under deterministic lineage rules.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::InvalidWorldBranchModel`] when the world is
    /// missing, branch identifiers collide, or the parent lineage is invalid.
    pub fn create_branch(
        mut self,
        world_id: &WorldId,
        input: BranchCreationInput,
    ) -> Result<Self, GraphError> {
        let world = self
            .worlds
            .iter_mut()
            .find(|world| world.world_id() == world_id)
            .ok_or_else(|| {
                GraphError::InvalidWorldBranchModel(format!(
                    "unknown world identifier: {}",
                    world_id.as_str()
                ))
            })?;

        if input.title.trim().is_empty() {
            return Err(GraphError::InvalidWorldBranchModel(
                "branch title must not be blank".to_owned(),
            ));
        }
        if world.branch(&input.branch_id).is_some() {
            return Err(GraphError::InvalidWorldBranchModel(format!(
                "duplicate branch identifier in world {}: {}",
                world_id.as_str(),
                input.branch_id.as_str()
            )));
        }

        let lineage = if let Some(parent_branch_id) = &input.parent_branch_id {
            if parent_branch_id == &input.branch_id {
                return Err(GraphError::InvalidWorldBranchModel(format!(
                    "branch {} cannot parent itself",
                    input.branch_id.as_str()
                )));
            }
            let parent = world.branch(parent_branch_id).ok_or_else(|| {
                GraphError::InvalidWorldBranchModel(format!(
                    "parent branch {} not found in world {}",
                    parent_branch_id.as_str(),
                    world_id.as_str()
                ))
            })?;
            let mut lineage = parent.lineage().to_vec();
            lineage.push(parent.branch_id().clone());
            lineage
        } else {
            Vec::new()
        };

        world.branches.push(BranchDescriptor {
            branch_id: input.branch_id,
            title: input.title,
            parent_branch_id: input.parent_branch_id,
            lineage,
            status: BranchStatus::Active,
            created_sequence: world.next_branch_sequence,
            overlay: BranchOverlay::default(),
        });
        world.next_branch_sequence += 1;
        Ok(self)
    }

    /// Validates that a branch mutation does not attempt base-fact writes.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::InvalidWorldBranchModel`] when the target world or
    /// branch is missing, or when `attempted_base_fact_refs` is non-empty.
    pub fn attempt_branch_base_fact_mutation(
        &self,
        world_id: &WorldId,
        branch_id: &BranchId,
        attempted_base_fact_refs: Vec<FactId>,
    ) -> Result<(), GraphError> {
        let world = self.world(world_id).ok_or_else(|| {
            GraphError::InvalidWorldBranchModel(format!(
                "unknown world identifier: {}",
                world_id.as_str()
            ))
        })?;
        if world.branch(branch_id).is_none() {
            return Err(GraphError::InvalidWorldBranchModel(format!(
                "unknown branch identifier in world {}: {}",
                world_id.as_str(),
                branch_id.as_str()
            )));
        }
        if !attempted_base_fact_refs.is_empty() {
            return Err(GraphError::InvalidWorldBranchModel(
                "branch scope cannot mutate immutable base facts".to_owned(),
            ));
        }
        Ok(())
    }

    /// Returns the overlay owned by one branch.
    ///
    /// # Errors
    ///
    /// Rejects unknown world and branch scopes with
    /// [`GraphError::InvalidBranchOverlay`].
    pub fn branch_overlay(
        &self,
        world_id: &WorldId,
        branch_id: &BranchId,
    ) -> Result<&BranchOverlay, GraphError> {
        let world = self.world(world_id).ok_or_else(|| {
            GraphError::InvalidBranchOverlay(format!(
                "unknown world identifier: {}",
                world_id.as_str()
            ))
        })?;
        let branch = world.branch(branch_id).ok_or_else(|| {
            GraphError::InvalidBranchOverlay(format!(
                "unknown branch identifier in world {}: {}",
                world_id.as_str(),
                branch_id.as_str()
            ))
        })?;
        Ok(branch.overlay())
    }

    /// Adds a hypothesis to one branch without changing shared base facts.
    ///
    /// # Errors
    ///
    /// Rejects unknown scopes and duplicate identifiers.
    pub fn add_branch_hypothesis(
        mut self,
        world_id: &WorldId,
        branch_id: &BranchId,
        hypothesis: OverlayHypothesis,
    ) -> Result<Self, GraphError> {
        let overlay = self.branch_overlay_mut(world_id, branch_id)?;
        if overlay
            .hypotheses
            .iter()
            .any(|record| record.id() == hypothesis.id())
        {
            return Err(duplicate_overlay_id("hypothesis", hypothesis.id().as_str()));
        }
        overlay.hypotheses.push(hypothesis);
        overlay
            .hypotheses
            .sort_by(|left, right| left.id().as_str().cmp(right.id().as_str()));
        Ok(self)
    }

    /// Adds a derived relation after validating all branch-visible references.
    ///
    /// # Errors
    ///
    /// Rejects missing references and duplicate identifiers.
    pub fn add_branch_derived_relation(
        mut self,
        world_id: &WorldId,
        branch_id: &BranchId,
        relation: BranchDerivedRelation,
    ) -> Result<Self, GraphError> {
        let overlay = self.branch_overlay(world_id, branch_id)?;
        self.validate_overlay_reference(overlay, relation.source())?;
        self.validate_overlay_reference(overlay, relation.target())?;

        let overlay = self.branch_overlay_mut(world_id, branch_id)?;
        if overlay
            .derived_relations
            .iter()
            .any(|record| record.id() == relation.id())
        {
            return Err(duplicate_overlay_id(
                "derived relation",
                relation.id().as_str(),
            ));
        }
        overlay.derived_relations.push(relation);
        overlay
            .derived_relations
            .sort_by(|left, right| left.id().as_str().cmp(right.id().as_str()));
        Ok(self)
    }

    /// Adds a prediction to one branch.
    ///
    /// # Errors
    ///
    /// Rejects unknown scopes and duplicate identifiers.
    pub fn add_branch_prediction(
        mut self,
        world_id: &WorldId,
        branch_id: &BranchId,
        prediction: BranchPrediction,
    ) -> Result<Self, GraphError> {
        let overlay = self.branch_overlay_mut(world_id, branch_id)?;
        if overlay
            .predictions
            .iter()
            .any(|record| record.id() == prediction.id())
        {
            return Err(duplicate_overlay_id("prediction", prediction.id().as_str()));
        }
        overlay.predictions.push(prediction);
        overlay
            .predictions
            .sort_by(|left, right| left.id().as_str().cmp(right.id().as_str()));
        Ok(self)
    }

    /// Adds expected evidence targeting an assertion visible in the same branch.
    ///
    /// # Errors
    ///
    /// Rejects missing targets and duplicate identifiers.
    pub fn add_branch_expected_evidence(
        mut self,
        world_id: &WorldId,
        branch_id: &BranchId,
        expected_evidence: BranchExpectedEvidence,
    ) -> Result<Self, GraphError> {
        let overlay = self.branch_overlay(world_id, branch_id)?;
        self.validate_overlay_reference(overlay, expected_evidence.target())?;

        let overlay = self.branch_overlay_mut(world_id, branch_id)?;
        if overlay
            .expected_evidence
            .iter()
            .any(|record| record.id() == expected_evidence.id())
        {
            return Err(duplicate_overlay_id(
                "expected-evidence marker",
                expected_evidence.id().as_str(),
            ));
        }
        overlay.expected_evidence.push(expected_evidence);
        overlay
            .expected_evidence
            .sort_by(|left, right| left.id().as_str().cmp(right.id().as_str()));
        Ok(self)
    }

    /// Adds an unresolved contradiction between branch-visible assertions.
    ///
    /// # Errors
    ///
    /// Rejects missing references and duplicate identifiers.
    pub fn add_branch_contradiction(
        mut self,
        world_id: &WorldId,
        branch_id: &BranchId,
        contradiction: BranchContradiction,
    ) -> Result<Self, GraphError> {
        let overlay = self.branch_overlay(world_id, branch_id)?;
        self.validate_overlay_reference(overlay, contradiction.left())?;
        self.validate_overlay_reference(overlay, contradiction.right())?;

        let overlay = self.branch_overlay_mut(world_id, branch_id)?;
        if overlay
            .contradictions
            .iter()
            .any(|record| record.id() == contradiction.id())
        {
            return Err(duplicate_overlay_id(
                "contradiction",
                contradiction.id().as_str(),
            ));
        }
        overlay.contradictions.push(contradiction);
        overlay
            .contradictions
            .sort_by(|left, right| left.id().as_str().cmp(right.id().as_str()));
        Ok(self)
    }

    fn branch_overlay_mut(
        &mut self,
        world_id: &WorldId,
        branch_id: &BranchId,
    ) -> Result<&mut BranchOverlay, GraphError> {
        let world = self
            .worlds
            .iter_mut()
            .find(|world| world.world_id() == world_id)
            .ok_or_else(|| {
                GraphError::InvalidBranchOverlay(format!(
                    "unknown world identifier: {}",
                    world_id.as_str()
                ))
            })?;
        let branch = world
            .branches
            .iter_mut()
            .find(|branch| branch.branch_id() == branch_id)
            .ok_or_else(|| {
                GraphError::InvalidBranchOverlay(format!(
                    "unknown branch identifier in world {}: {}",
                    world_id.as_str(),
                    branch_id.as_str()
                ))
            })?;
        if branch.status != BranchStatus::Active {
            return Err(GraphError::InvalidBranchOverlay(format!(
                "branch {} in world {} is not active",
                branch_id.as_str(),
                world_id.as_str()
            )));
        }
        Ok(&mut branch.overlay)
    }

    pub(crate) fn transition_branch_status(
        &mut self,
        world_id: &WorldId,
        branch_id: &BranchId,
        status: BranchStatus,
    ) -> Result<(), GraphError> {
        let world = self
            .worlds
            .iter_mut()
            .find(|world| world.world_id() == world_id)
            .ok_or_else(|| {
                GraphError::InvalidBranchResolution(format!(
                    "unknown world identifier: {}",
                    world_id.as_str()
                ))
            })?;
        let branch = world
            .branches
            .iter_mut()
            .find(|branch| branch.branch_id() == branch_id)
            .ok_or_else(|| {
                GraphError::InvalidBranchResolution(format!(
                    "unknown branch identifier in world {}: {}",
                    world_id.as_str(),
                    branch_id.as_str()
                ))
            })?;
        if branch.status != BranchStatus::Active {
            return Err(GraphError::InvalidBranchResolution(format!(
                "branch {} in world {} is already terminal",
                branch_id.as_str(),
                world_id.as_str()
            )));
        }
        branch.status = status;
        Ok(())
    }

    fn validate_overlay_reference(
        &self,
        overlay: &BranchOverlay,
        reference: &crate::BranchOverlayReference,
    ) -> Result<(), GraphError> {
        let exists = match reference {
            crate::BranchOverlayReference::BaseFact(fact_id) => {
                self.base_facts.iter().any(|candidate| candidate == fact_id)
            }
            _ => overlay.contains_reference(reference),
        };
        if !exists {
            return Err(GraphError::InvalidBranchOverlay(format!(
                "overlay reference does not resolve: {reference:?}"
            )));
        }
        Ok(())
    }
}

fn duplicate_overlay_id(kind: &str, id: &str) -> GraphError {
    GraphError::InvalidBranchOverlay(format!("duplicate {kind} identifier: {id}"))
}

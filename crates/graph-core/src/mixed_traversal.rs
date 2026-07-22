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
//! Mixed traversal and query contracts across resolutions and hyperrelations.
//!
//! Paths use explicit operators for binary edges, derivation-backed resolution
//! changes, and hyperrelation entry/expansion. Execution produces deterministic
//! score and explanation records; ambiguous hyperrelation expansion requires an
//! explicit role filter instead of silently choosing a participant.

use serde::{Deserialize, Serialize};

use crate::{
    CoordinatedEventHyperrelation, DerivationLinkId, GraphError, HyperrelationId,
    HyperrelationParticipant, HyperrelationParticipantRole, NodeId, RelationshipId,
    ResolutionLevel, ResolutionRecordRef,
};

/// Stable traversal operation vocabulary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MixedTraversalOperator {
    /// Traverse one existing binary relationship.
    Binary,
    /// Move from a lower-resolution artifact to an adjacent higher level.
    AbstractResolution,
    /// Move from a higher-resolution artifact to an adjacent lower level.
    DrillDownResolution,
    /// Enter one hyperrelation through a participant.
    EnterHyperrelation,
    /// Expand from one hyperrelation to a participant.
    ExpandHyperrelation,
}

/// Typed endpoint of one mixed traversal step.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MixedTraversalEndpoint {
    /// Existing graph node.
    Node(NodeId),
    /// Artifact at an explicit graph resolution.
    Resolution(ResolutionRecordRef),
    /// First-class hyperrelation.
    Hyperrelation(HyperrelationId),
}

/// One explicit step in a mixed traversal path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MixedTraversalStep {
    /// Existing binary relationship traversal.
    Binary {
        /// Auditable binary relationship identifier.
        relationship_id: RelationshipId,
        /// Binary source node.
        source: NodeId,
        /// Binary target node.
        target: NodeId,
    },
    /// Derivation-backed cross-resolution traversal.
    AcrossResolution {
        /// Derivation link proving the resolution relationship.
        derivation_link_id: DerivationLinkId,
        /// Source resolution artifact.
        source: ResolutionRecordRef,
        /// Target resolution artifact.
        target: ResolutionRecordRef,
    },
    /// Enter one hyperrelation through a known participant.
    EnterHyperrelation {
        /// Hyperrelation being entered.
        hyperrelation_id: HyperrelationId,
        /// Participant used as entry point.
        participant: NodeId,
        /// Role captured for explanation.
        role: HyperrelationParticipantRole,
    },
    /// Expand from one hyperrelation to a known participant.
    ExpandHyperrelation {
        /// Hyperrelation being expanded.
        hyperrelation_id: HyperrelationId,
        /// Participant reached by expansion.
        participant: NodeId,
        /// Role captured for explanation.
        role: HyperrelationParticipantRole,
    },
}

impl MixedTraversalStep {
    /// Creates one binary traversal step.
    #[must_use]
    pub fn binary(relationship_id: RelationshipId, source: NodeId, target: NodeId) -> Self {
        Self::Binary {
            relationship_id,
            source,
            target,
        }
    }

    /// Creates one derivation-backed resolution traversal step.
    #[must_use]
    pub fn across_resolution(
        derivation_link_id: DerivationLinkId,
        source: ResolutionRecordRef,
        target: ResolutionRecordRef,
    ) -> Self {
        Self::AcrossResolution {
            derivation_link_id,
            source,
            target,
        }
    }

    /// Creates a validated hyperrelation-entry step.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::InvalidMixedTraversal`] when the node is not a
    /// participant in `hyperrelation`.
    pub fn enter_hyperrelation(
        hyperrelation: &CoordinatedEventHyperrelation,
        participant: NodeId,
    ) -> Result<Self, GraphError> {
        let role = participant_role(hyperrelation, &participant)?;
        Ok(Self::EnterHyperrelation {
            hyperrelation_id: hyperrelation.id().clone(),
            participant,
            role,
        })
    }

    /// Creates a validated hyperrelation-expansion step.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::InvalidMixedTraversal`] when the node is not a
    /// participant in `hyperrelation`.
    pub fn expand_hyperrelation(
        hyperrelation: &CoordinatedEventHyperrelation,
        participant: NodeId,
    ) -> Result<Self, GraphError> {
        let role = participant_role(hyperrelation, &participant)?;
        Ok(Self::ExpandHyperrelation {
            hyperrelation_id: hyperrelation.id().clone(),
            participant,
            role,
        })
    }

    fn endpoints(&self) -> (MixedTraversalEndpoint, MixedTraversalEndpoint) {
        match self {
            Self::Binary { source, target, .. } => (
                MixedTraversalEndpoint::Node(source.clone()),
                MixedTraversalEndpoint::Node(target.clone()),
            ),
            Self::AcrossResolution { source, target, .. } => (
                MixedTraversalEndpoint::Resolution(source.clone()),
                MixedTraversalEndpoint::Resolution(target.clone()),
            ),
            Self::EnterHyperrelation {
                hyperrelation_id,
                participant,
                ..
            } => (
                MixedTraversalEndpoint::Node(participant.clone()),
                MixedTraversalEndpoint::Hyperrelation(hyperrelation_id.clone()),
            ),
            Self::ExpandHyperrelation {
                hyperrelation_id,
                participant,
                ..
            } => (
                MixedTraversalEndpoint::Hyperrelation(hyperrelation_id.clone()),
                MixedTraversalEndpoint::Node(participant.clone()),
            ),
        }
    }

    fn operator(&self) -> MixedTraversalOperator {
        match self {
            Self::Binary { .. } => MixedTraversalOperator::Binary,
            Self::AcrossResolution { source, target, .. } => {
                if resolution_level_rank(target.level()) > resolution_level_rank(source.level()) {
                    MixedTraversalOperator::AbstractResolution
                } else {
                    MixedTraversalOperator::DrillDownResolution
                }
            }
            Self::EnterHyperrelation { .. } => MixedTraversalOperator::EnterHyperrelation,
            Self::ExpandHyperrelation { .. } => MixedTraversalOperator::ExpandHyperrelation,
        }
    }

    fn audit_ref(&self) -> &str {
        match self {
            Self::Binary {
                relationship_id, ..
            } => relationship_id.as_str(),
            Self::AcrossResolution {
                derivation_link_id, ..
            } => derivation_link_id.as_str(),
            Self::EnterHyperrelation {
                hyperrelation_id, ..
            }
            | Self::ExpandHyperrelation {
                hyperrelation_id, ..
            } => hyperrelation_id.as_str(),
        }
    }

    const fn score_contribution(&self) -> u64 {
        match self {
            Self::Binary { .. } => 100,
            Self::AcrossResolution { .. } => 80,
            Self::EnterHyperrelation { .. } | Self::ExpandHyperrelation { .. } => 90,
        }
    }
}

/// Deterministic traversal score hook.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MixedTraversalScore {
    total: u64,
}

impl MixedTraversalScore {
    /// Returns the deterministic total score.
    #[must_use]
    pub const fn total(self) -> u64 {
        self.total
    }
}

/// Auditable explanation for one mixed traversal step.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MixedTraversalExplanation {
    operator: MixedTraversalOperator,
    audit_ref: String,
    score_contribution: u64,
}

impl MixedTraversalExplanation {
    /// Returns the typed operator.
    #[must_use]
    pub const fn operator(&self) -> MixedTraversalOperator {
        self.operator
    }

    /// Returns the relationship, derivation, or hyperrelation audit reference.
    #[must_use]
    pub fn audit_ref(&self) -> &str {
        self.audit_ref.as_str()
    }

    /// Returns this step's deterministic score contribution.
    #[must_use]
    pub const fn score_contribution(&self) -> u64 {
        self.score_contribution
    }
}

/// Deterministic path result with score and explanations.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MixedTraversalResult {
    steps: Vec<MixedTraversalStep>,
    score: MixedTraversalScore,
    explanations: Vec<MixedTraversalExplanation>,
}

impl MixedTraversalResult {
    /// Returns the validated path steps.
    #[must_use]
    pub fn steps(&self) -> &[MixedTraversalStep] {
        self.steps.as_slice()
    }

    /// Returns the deterministic aggregate score.
    #[must_use]
    pub const fn score(&self) -> MixedTraversalScore {
        self.score
    }

    /// Returns one audit explanation per path step.
    #[must_use]
    pub fn explanations(&self) -> &[MixedTraversalExplanation] {
        self.explanations.as_slice()
    }
}

/// Hyperrelation expansion request with an optional explicit role filter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HyperrelationExpansionRequest {
    entry_participant: NodeId,
    role_filter: Option<Vec<HyperrelationParticipantRole>>,
}

impl HyperrelationExpansionRequest {
    /// Creates an unfiltered expansion request.
    #[must_use]
    pub fn new(entry_participant: NodeId) -> Self {
        Self {
            entry_participant,
            role_filter: None,
        }
    }

    /// Sets a deterministic non-empty role filter.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::InvalidMixedTraversal`] for an empty filter.
    pub fn with_role_filter(
        mut self,
        roles: Vec<HyperrelationParticipantRole>,
    ) -> Result<Self, GraphError> {
        if roles.is_empty() {
            return Err(GraphError::InvalidMixedTraversal(
                "hyperrelation role filter must not be empty".to_owned(),
            ));
        }
        let mut roles = roles;
        roles.sort();
        roles.dedup();
        self.role_filter = Some(roles);
        Ok(self)
    }
}

/// Explanation of one hyperrelation expansion query.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HyperrelationExpansionExplanation {
    hyperrelation_id: HyperrelationId,
    entry_participant: NodeId,
    role_filter: Vec<HyperrelationParticipantRole>,
}

impl HyperrelationExpansionExplanation {
    /// Returns the expanded hyperrelation identifier.
    #[must_use]
    pub fn hyperrelation_id(&self) -> &HyperrelationId {
        &self.hyperrelation_id
    }

    /// Returns the participant used to enter the hyperrelation.
    #[must_use]
    pub fn entry_participant(&self) -> &NodeId {
        &self.entry_participant
    }

    /// Returns the deterministic role filter applied to expansion.
    #[must_use]
    pub fn role_filter(&self) -> &[HyperrelationParticipantRole] {
        self.role_filter.as_slice()
    }
}

/// Deterministic hyperrelation expansion result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HyperrelationExpansionResult {
    participants: Vec<HyperrelationParticipant>,
    explanation: HyperrelationExpansionExplanation,
}

impl HyperrelationExpansionResult {
    /// Returns matching participants in deterministic event order.
    #[must_use]
    pub fn participants(&self) -> &[HyperrelationParticipant] {
        self.participants.as_slice()
    }

    /// Returns the expansion explanation.
    #[must_use]
    pub const fn explanation(&self) -> &HyperrelationExpansionExplanation {
        &self.explanation
    }
}

/// Executes and validates one mixed traversal path.
///
/// # Errors
///
/// Returns [`GraphError::InvalidMixedTraversal`] for empty, disconnected,
/// self-referential, or non-adjacent paths.
pub fn execute_mixed_traversal(
    steps: Vec<MixedTraversalStep>,
) -> Result<MixedTraversalResult, GraphError> {
    if steps.is_empty() {
        return Err(GraphError::InvalidMixedTraversal(
            "mixed traversal requires at least one step".to_owned(),
        ));
    }

    let mut previous_target = None;
    let mut total = 0_u64;
    let mut explanations = Vec::with_capacity(steps.len());

    for step in &steps {
        let (source, target) = step.endpoints();
        if source == target {
            return Err(GraphError::InvalidMixedTraversal(
                "mixed traversal step cannot target its own source".to_owned(),
            ));
        }
        if let MixedTraversalStep::AcrossResolution { source, target, .. } = step {
            let source_rank = resolution_level_rank(source.level());
            let target_rank = resolution_level_rank(target.level());
            if source_rank.abs_diff(target_rank) != 1 {
                return Err(GraphError::InvalidMixedTraversal(format!(
                    "resolution traversal must use adjacent levels: {:?} -> {:?}",
                    source.level(),
                    target.level()
                )));
            }
        }
        if let Some(previous_target) = &previous_target
            && previous_target != &source
        {
            return Err(GraphError::InvalidMixedTraversal(format!(
                "disconnected traversal step: expected source {previous_target:?}, got {source:?}"
            )));
        }

        let contribution = step.score_contribution();
        total = total.saturating_add(contribution);
        explanations.push(MixedTraversalExplanation {
            operator: step.operator(),
            audit_ref: step.audit_ref().to_owned(),
            score_contribution: contribution,
        });
        previous_target = Some(target);
    }

    Ok(MixedTraversalResult {
        steps,
        score: MixedTraversalScore { total },
        explanations,
    })
}

/// Expands a hyperrelation according to explicit role-filter semantics.
///
/// # Errors
///
/// Returns [`GraphError::InvalidMixedTraversal`] for invalid or ambiguous
/// expansion.
pub fn query_hyperrelation_expansion(
    hyperrelation: &CoordinatedEventHyperrelation,
    request: &HyperrelationExpansionRequest,
) -> Result<HyperrelationExpansionResult, GraphError> {
    participant_role(hyperrelation, &request.entry_participant)?;

    let candidates = hyperrelation
        .participants()
        .iter()
        .filter(|candidate| candidate.node_id() != &request.entry_participant)
        .collect::<Vec<_>>();

    let role_filter = if let Some(role_filter) = &request.role_filter {
        role_filter.clone()
    } else {
        let mut candidate_roles = candidates
            .iter()
            .map(|candidate| candidate.role())
            .collect::<Vec<_>>();
        candidate_roles.sort();
        candidate_roles.dedup();
        if candidate_roles.len() > 1 {
            return Err(GraphError::InvalidMixedTraversal(format!(
                "hyperrelation {} expansion is ambiguous across {} participant roles; \
                 provide an explicit role filter",
                hyperrelation.id().as_str(),
                candidate_roles.len()
            )));
        }
        candidate_roles
    };

    let participants = candidates
        .into_iter()
        .filter(|candidate| role_filter.contains(&candidate.role()))
        .cloned()
        .collect();

    Ok(HyperrelationExpansionResult {
        participants,
        explanation: HyperrelationExpansionExplanation {
            hyperrelation_id: hyperrelation.id().clone(),
            entry_participant: request.entry_participant.clone(),
            role_filter,
        },
    })
}

fn participant_role(
    hyperrelation: &CoordinatedEventHyperrelation,
    participant: &NodeId,
) -> Result<HyperrelationParticipantRole, GraphError> {
    hyperrelation
        .participants()
        .iter()
        .find(|candidate| candidate.node_id() == participant)
        .map(HyperrelationParticipant::role)
        .ok_or_else(|| {
            GraphError::InvalidMixedTraversal(format!(
                "node {} is not a participant of hyperrelation {}",
                participant.as_str(),
                hyperrelation.id().as_str()
            ))
        })
}

const fn resolution_level_rank(level: ResolutionLevel) -> u8 {
    match level {
        ResolutionLevel::Tactical => 0,
        ResolutionLevel::Operational => 1,
        ResolutionLevel::Strategic => 2,
    }
}

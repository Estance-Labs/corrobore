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
//! First-class n-ary hyperrelation contracts (Epic 0022).
//!
//! Coordinated events retain all participants, their roles, temporal scope, and
//! narrative context in one record. Optional binary projections provide an
//! explicit interoperability boundary without replacing or changing the
//! existing binary relationship model.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::{GraphError, NodeId, RelationshipInput, TemporalTimestamp};

/// Typed identifier of one first-class hyperrelation.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct HyperrelationId {
    value: String,
}

impl HyperrelationId {
    /// Creates a validated hyperrelation identifier.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::InvalidIdentifier`] when `value` is blank.
    pub fn new(value: impl Into<String>) -> Result<Self, GraphError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(GraphError::InvalidIdentifier("HyperrelationId".to_owned()));
        }
        Ok(Self { value })
    }

    /// Returns the identifier as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.value.as_str()
    }
}

/// Versioned schema of a supported hyperrelation kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HyperrelationSchema {
    /// Coordinated event schema with actor, narrative, and infrastructure roles.
    CoordinatedEventV1,
}

/// Semantic role of one coordinated-event participant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum HyperrelationParticipantRole {
    /// Actor or account participating in the coordinated event.
    Actor,
    /// Narrative coordinated by the event.
    Narrative,
    /// Infrastructure used by participants.
    Infrastructure,
    /// Optional related entity retained in the n-ary event context.
    RelatedEntity,
}

/// One node and its explicit role in an n-ary hyperrelation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HyperrelationParticipant {
    node_id: NodeId,
    role: HyperrelationParticipantRole,
}

impl HyperrelationParticipant {
    /// Creates one typed participant.
    #[must_use]
    pub fn new(node_id: NodeId, role: HyperrelationParticipantRole) -> Self {
        Self { node_id, role }
    }

    /// Returns the participant node identifier.
    #[must_use]
    pub fn node_id(&self) -> &NodeId {
        &self.node_id
    }

    /// Returns the participant role.
    #[must_use]
    pub const fn role(&self) -> HyperrelationParticipantRole {
        self.role
    }
}

/// Inclusive temporal window of one coordinated event.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HyperrelationTimeWindow {
    start: TemporalTimestamp,
    end: TemporalTimestamp,
}

impl HyperrelationTimeWindow {
    /// Creates a chronologically valid time window.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::InvalidHyperrelation`] when `start` follows `end`.
    pub fn new(start: TemporalTimestamp, end: TemporalTimestamp) -> Result<Self, GraphError> {
        if start.as_str() > end.as_str() {
            return Err(GraphError::InvalidHyperrelation(
                "hyperrelation time-window start must be <= end".to_owned(),
            ));
        }
        Ok(Self { start, end })
    }

    /// Returns the inclusive window start.
    #[must_use]
    pub const fn start(&self) -> &TemporalTimestamp {
        &self.start
    }

    /// Returns the inclusive window end.
    #[must_use]
    pub const fn end(&self) -> &TemporalTimestamp {
        &self.end
    }
}

/// First-class coordinated-event hyperrelation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoordinatedEventHyperrelation {
    id: HyperrelationId,
    schema: HyperrelationSchema,
    participants: Vec<HyperrelationParticipant>,
    time_window: HyperrelationTimeWindow,
    narrative_context: String,
}

impl CoordinatedEventHyperrelation {
    /// Creates and validates one coordinated event.
    ///
    /// Enforces minimum arity, required role cardinalities, unique participant
    /// nodes, deterministic ordering, and non-blank narrative context.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::InvalidHyperrelation`] for schema violations.
    pub fn new(
        id: HyperrelationId,
        participants: Vec<HyperrelationParticipant>,
        time_window: HyperrelationTimeWindow,
        narrative_context: impl Into<String>,
    ) -> Result<Self, GraphError> {
        let narrative_context = narrative_context.into();
        if narrative_context.trim().is_empty() {
            return Err(GraphError::InvalidHyperrelation(
                "coordinated-event narrative context must not be blank".to_owned(),
            ));
        }
        if participants.len() < 4 {
            return Err(GraphError::InvalidHyperrelation(format!(
                "coordinated-event hyperrelation requires at least four participants, got {}",
                participants.len()
            )));
        }

        let mut participants = participants;
        participants.sort_by(|left, right| {
            left.role()
                .cmp(&right.role())
                .then_with(|| left.node_id().as_str().cmp(right.node_id().as_str()))
        });

        let mut seen_participants = HashSet::with_capacity(participants.len());
        for participant in &participants {
            if !seen_participants.insert(participant.node_id()) {
                return Err(GraphError::InvalidHyperrelation(format!(
                    "participant node {} cannot appear more than once or carry multiple roles",
                    participant.node_id().as_str()
                )));
            }
        }

        let actor_count = participants
            .iter()
            .filter(|participant| participant.role() == HyperrelationParticipantRole::Actor)
            .count();
        if actor_count < 2 {
            return Err(GraphError::InvalidHyperrelation(format!(
                "coordinated-event hyperrelation requires at least two actors, got {actor_count}"
            )));
        }

        let narrative_count = participants
            .iter()
            .filter(|participant| participant.role() == HyperrelationParticipantRole::Narrative)
            .count();
        if narrative_count != 1 {
            return Err(GraphError::InvalidHyperrelation(format!(
                "coordinated-event hyperrelation requires exactly one narrative, got \
                 {narrative_count}"
            )));
        }

        let infrastructure_count = participants
            .iter()
            .filter(|participant| {
                participant.role() == HyperrelationParticipantRole::Infrastructure
            })
            .count();
        if infrastructure_count == 0 {
            return Err(GraphError::InvalidHyperrelation(
                "coordinated-event hyperrelation requires at least one infrastructure participant"
                    .to_owned(),
            ));
        }

        Ok(Self {
            id,
            schema: HyperrelationSchema::CoordinatedEventV1,
            participants,
            time_window,
            narrative_context,
        })
    }

    /// Returns the typed hyperrelation identifier.
    #[must_use]
    pub fn id(&self) -> &HyperrelationId {
        &self.id
    }

    /// Returns the versioned hyperrelation schema.
    #[must_use]
    pub const fn schema(&self) -> HyperrelationSchema {
        self.schema
    }

    /// Returns participants in deterministic role and node order.
    #[must_use]
    pub fn participants(&self) -> &[HyperrelationParticipant] {
        self.participants.as_slice()
    }

    /// Returns participants having one role.
    #[must_use]
    pub fn participants_for_role(
        &self,
        role: HyperrelationParticipantRole,
    ) -> Vec<&HyperrelationParticipant> {
        self.participants
            .iter()
            .filter(|participant| participant.role() == role)
            .collect()
    }

    /// Returns the narrative context.
    #[must_use]
    pub fn narrative_context(&self) -> &str {
        self.narrative_context.as_str()
    }

    /// Returns the event time window.
    #[must_use]
    pub const fn time_window(&self) -> &HyperrelationTimeWindow {
        &self.time_window
    }

    /// Builds explicit binary projections from a reified event anchor.
    #[must_use]
    pub fn binary_projections(&self, event_anchor: NodeId) -> Vec<HyperrelationBinaryProjection> {
        self.participants
            .iter()
            .map(|participant| HyperrelationBinaryProjection {
                source: event_anchor.clone(),
                target: participant.node_id().clone(),
                relationship_type: HyperrelationProjectionType::from(participant.role()),
            })
            .collect()
    }
}

/// Role-specific relationship type used by a binary compatibility projection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HyperrelationProjectionType {
    /// Event anchor to actor projection.
    HasActor,
    /// Event anchor to narrative projection.
    HasNarrative,
    /// Event anchor to infrastructure projection.
    UsesInfrastructure,
    /// Event anchor to optional related entity projection.
    HasRelatedEntity,
}

impl HyperrelationProjectionType {
    /// Returns the stable binary relationship type string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::HasActor => "HAS_ACTOR",
            Self::HasNarrative => "HAS_NARRATIVE",
            Self::UsesInfrastructure => "USES_INFRASTRUCTURE",
            Self::HasRelatedEntity => "HAS_RELATED_ENTITY",
        }
    }
}

impl From<HyperrelationParticipantRole> for HyperrelationProjectionType {
    fn from(role: HyperrelationParticipantRole) -> Self {
        match role {
            HyperrelationParticipantRole::Actor => Self::HasActor,
            HyperrelationParticipantRole::Narrative => Self::HasNarrative,
            HyperrelationParticipantRole::Infrastructure => Self::UsesInfrastructure,
            HyperrelationParticipantRole::RelatedEntity => Self::HasRelatedEntity,
        }
    }
}

/// One explicit participant projection compatible with binary relationship input.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HyperrelationBinaryProjection {
    source: NodeId,
    target: NodeId,
    relationship_type: HyperrelationProjectionType,
}

impl HyperrelationBinaryProjection {
    /// Returns the reified event anchor source.
    #[must_use]
    pub fn source(&self) -> &NodeId {
        &self.source
    }

    /// Returns the projected participant target.
    #[must_use]
    pub fn target(&self) -> &NodeId {
        &self.target
    }

    /// Returns the role-specific binary relationship type.
    #[must_use]
    pub const fn relationship_type(&self) -> HyperrelationProjectionType {
        self.relationship_type
    }

    /// Converts this projection to the existing binary relationship input.
    ///
    /// # Errors
    ///
    /// Propagates relationship-type validation errors.
    pub fn into_relationship_input(self) -> Result<RelationshipInput, GraphError> {
        RelationshipInput::new(self.source, self.relationship_type.as_str(), self.target)
    }
}

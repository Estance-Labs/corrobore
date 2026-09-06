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
//! Graph tier model of the immune system (Epic 0019).
//!
//!
//!
//! - Never mutate the canonical graph directly: immune corrections,
//!   proposals, and suspect data live in explicit tiers, and every movement
//!   between tiers is an audited, append-only transition.
//! - Existing data is canonical by default; entering the canonical tier
//!   requires the explicit audited-promotion reason, so nothing validated is
//!   ever replaced silently.
//! - Silent deletion is unrepresentable: the registry has no removal API, and
//!   no-op transitions are rejected so the audit trail records only real
//!   movements.
//! - Keep validators, responses, and probes out of this module; they consume
//!   the tier model through its typed transitions.
//!
//! # Determinism
//!
//! Transitions carry a monotonic sequence instead of wall-clock time, and
//! tier listings follow first-transition order, so identically built
//! registries are equal and reports diff cleanly.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{
    GraphError,
    ids::{ClaimId, EvidenceId, NodeId, RelationshipId},
};

/// One tier of the immune system's graph separation.
///
///
/// name the epic's four tiers explicitly so every record's trust standing is
/// a typed, queryable state instead of a convention.
///
///
/// enumerate the tiers; `ALL` fixes the report order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GraphTier {
    /// Validated data the engine reasons over by default.
    Canonical,

    /// Proposed extractions and corrections awaiting review.
    Shadow,

    /// Suspect data isolated pending verification.
    Quarantine,

    /// Possible relations not yet validated.
    Hypothesis,
}

impl GraphTier {
    /// The complete, stable tier vocabulary.
    ///
    ///
    /// fix the report order so tier listings diff cleanly across runs.
    pub const ALL: [GraphTier; 4] = [
        GraphTier::Canonical,
        GraphTier::Shadow,
        GraphTier::Quarantine,
        GraphTier::Hypothesis,
    ];
}

/// Typed reference to a tier-tracked record.
///
///
/// track graph records and epistemic records under one tier model: nodes,
/// relationships, claims, and evidence all carry a trust standing.
///
///
/// reference each record kind by its validated identifier.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TierRecordRef {
    /// An extraction proposal awaiting explicit promotion.
    Candidate(crate::CandidateId),
    /// A graph node.
    Node(NodeId),

    /// A graph relationship.
    Relationship(RelationshipId),

    /// An epistemic claim.
    Claim(ClaimId),

    /// An evidence record.
    Evidence(EvidenceId),
}

/// Typed reason of one tier transition.
///
///
/// make every movement explainable: the audit trail names why a record moved,
/// not only where.
///
///
/// enumerate the immune movement reasons.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TierTransitionReason {
    /// Initial non-canonical placement of an extraction proposal.
    CandidateSubmission,
    /// A validator finding isolated or flagged the record.
    ValidatorFinding,

    /// An immune repair proposal placed the record in review.
    RepairProposal,

    /// A verification probe outcome justified the movement.
    VerificationOutcome,

    /// An explicit audited promotion into a more trusted tier.
    AuditedPromotion,
}

/// One audited tier transition.
///
///
/// keep the immune audit trail complete: every movement records the record,
/// both endpoints, the acting component, the typed reason, and its order.
///
///
/// carry the transition context with a monotonic sequence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TierTransition {
    /// Record that moved.
    pub record: TierRecordRef,

    /// Tier the record moved from.
    pub from: GraphTier,

    /// Tier the record moved to.
    pub to: GraphTier,

    /// Component or actor that moved it.
    pub actor_ref: String,

    /// Typed reason of the movement.
    pub reason: TierTransitionReason,

    /// Monotonic order of the transition in the registry.
    pub sequence: u64,
}

/// Registry owning tier membership and the audited transition trail.
///
///
/// own the epic's non-destructive contract: tiers move only through audited
/// transitions, canonical entry requires explicit promotion, and nothing is
/// ever deleted.
///
///
/// keep current tiers, the append-only audit trail, and first-transition
/// order for deterministic listings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphTierRegistry {
    tiers: HashMap<TierRecordRef, GraphTier>,
    known_records: Vec<TierRecordRef>,
    audit: Vec<TierTransition>,
    next_sequence: u64,
}

impl GraphTierRegistry {
    /// Create an empty registry.
    ///
    ///
    /// provide the stable constructor used before any immune action.
    ///
    ///
    /// start with no tracked records: everything reads canonical by default.
    ///
    /// # Errors
    ///
    /// none expected because an empty registry has no external dependency.
    pub fn new() -> Self {
        Self::default()
    }

    /// Return the current tier of a record.
    ///
    ///
    /// give every consumer the same trust standing for a record, tracked or
    /// not.
    ///
    ///
    /// return the recorded tier, or canonical for records with no transition
    /// history.
    ///
    /// # Errors
    ///
    /// none expected because absence deterministically reads canonical.
    pub fn tier_of(&self, record: &TierRecordRef) -> GraphTier {
        self.tiers
            .get(record)
            .copied()
            .unwrap_or(GraphTier::Canonical)
    }

    /// Record one audited tier transition.
    ///
    ///
    /// make every immune movement explicit, explainable, and irreversible in
    /// history: the trail only grows.
    ///
    ///
    /// append the transition from the record's current tier to the target
    /// tier with its actor, reason, and sequence.
    ///
    /// # Errors
    ///
    /// return `GraphError::InvalidTierTransition` when the target equals the
    /// current tier (no-op) or when entering canonical without the
    /// audited-promotion reason.
    pub fn transition(
        &mut self,
        record: TierRecordRef,
        to: GraphTier,
        actor_ref: impl Into<String>,
        reason: TierTransitionReason,
    ) -> Result<u64, GraphError> {
        let from = self.tier_of(&record);

        if to == from {
            return Err(GraphError::InvalidTierTransition(format!(
                "record already in tier {to:?}"
            )));
        }
        if to == GraphTier::Canonical && reason != TierTransitionReason::AuditedPromotion {
            return Err(GraphError::InvalidTierTransition(format!(
                "entering the canonical tier requires an audited promotion, got {reason:?}"
            )));
        }

        if !self.tiers.contains_key(&record) {
            self.known_records.push(record.clone());
        }
        self.tiers.insert(record.clone(), to);

        let sequence = self.next_sequence;
        self.next_sequence += 1;
        self.audit.push(TierTransition {
            record,
            from,
            to,
            actor_ref: actor_ref.into(),
            reason,
            sequence,
        });
        Ok(sequence)
    }

    /// Return the full audited transition trail in order.
    ///
    ///
    /// expose the complete immune history for audit and downstream
    /// explanation.
    ///
    ///
    /// return the append-only trail.
    ///
    /// # Errors
    ///
    /// none expected because reading the trail cannot fail.
    pub fn audit_trail(&self) -> &[TierTransition] {
        &self.audit
    }

    /// Return the audited transitions of one record in order.
    ///
    ///
    /// let reviewers follow one record's trust history without scanning the
    /// whole trail.
    ///
    ///
    /// filter the trail by record, preserving order.
    ///
    /// # Errors
    ///
    /// none expected because absence is a deterministic empty result.
    pub fn audit_for(&self, record: &TierRecordRef) -> Vec<&TierTransition> {
        self.audit
            .iter()
            .filter(|transition| &transition.record == record)
            .collect()
    }

    /// List the records currently in one tier, in first-transition order.
    ///
    ///
    /// give tier reports a deterministic, diffable order independent of map
    /// internals.
    ///
    ///
    /// walk the known records in first-transition order and keep those whose
    /// current tier matches.
    ///
    /// # Errors
    ///
    /// none expected because absence is a deterministic empty result.
    pub fn records_in_tier(&self, tier: GraphTier) -> Vec<TierRecordRef> {
        self.known_records
            .iter()
            .filter(|record| self.tier_of(record) == tier)
            .cloned()
            .collect()
    }
}

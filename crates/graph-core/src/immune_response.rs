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
//! Non-destructive immune response and repair proposals (Epic 0019).
//!
//!
//!
//! - Turn validator findings into audited, non-destructive responses: every
//!   action routes through the tier model's audited transitions or the
//!   anti-pheromone field's typed reporting path — never silent deletion,
//!   never a direct canonical rewrite.
//! - Preserve a complete, ordered audit linking finding -> response ->
//!   transition: each response names the finding it addresses and, when a
//!   tier movement happened, the transition's sequence in the registry.
//! - Repair proposals are shadow-tier records referencing their finding; the
//!   defective canonical record stays untouched, and the proposal reaches
//!   canonical only through the registry's explicit audited promotion.
//! - Probe generation lives in its dedicated issue; verification requests
//!   here carry the probe reference that will answer them.

use serde::{Deserialize, Serialize};

use crate::{
    GraphError,
    anti_pheromone::{AntiPheromoneField, AntiPheromoneSignal},
    graph_tiers::{GraphTier, GraphTierRegistry, TierRecordRef, TierTransitionReason},
    ids::{ClaimId, NodeId, RelationshipId},
    pheromone_trace::PheromoneTaskScope,
    validation::{ValidationErrorRecord, ValidationTarget},
};

/// Typed outcome of one immune response.
///
///
/// name the epic's four response actions explicitly so audits read as typed
/// decisions instead of free text.
///
///
/// carry the action-specific context: the quarantined record, the reported
/// signal, the probe reference, or the proposed repair record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImmuneResponseAction {
    /// The finding's target was quarantined through the tier model.
    Quarantine {
        /// Quarantined record.
        record: TierRecordRef,
    },

    /// The finding's relationship had its traversal priority reduced.
    ReducePriority {
        /// Signal reported into the anti-pheromone field.
        signal: AntiPheromoneSignal,
    },

    /// Verification was requested from a probe.
    RequestVerification {
        /// Reference of the probe that will answer the request.
        probe_ref: String,
    },

    /// A repair was proposed as a shadow-tier record.
    ProposeRepair {
        /// Proposed replacement record placed in the shadow tier.
        proposal: TierRecordRef,
    },
}

/// One audited immune response.
///
///
/// keep the finding -> response -> transition chain explicit: reviewers can
/// follow any immune action back to the defect that justified it.
///
///
/// carry the addressed finding's code and target, the typed action, the
/// response order, and the linked tier-transition sequence when one happened.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImmuneResponse {
    /// Stable code of the finding this response addresses.
    pub finding_code: String,

    /// Target of the finding this response addresses.
    pub finding_target: ValidationTarget,

    /// Typed action taken.
    pub action: ImmuneResponseAction,

    /// Monotonic order of the response in the responder's audit.
    pub sequence: u64,

    /// Sequence of the linked tier transition, when the action moved a tier.
    pub tier_transition_sequence: Option<u64>,
}

/// Responder owning the ordered immune-response audit.
///
///
/// centralize response recording so every immune action is auditable in one
/// ordered trail, while tier state and pheromone fields stay owned by their
/// own modules and are only borrowed per action.
///
///
/// append responses with monotonic sequences; the four actions apply their
/// side effects through the borrowed registry or field.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImmuneResponder {
    responses: Vec<ImmuneResponse>,
    next_sequence: u64,
}

impl ImmuneResponder {
    /// Create an empty responder.
    ///
    ///
    /// provide the stable constructor used before any immune action.
    ///
    ///
    /// start with an empty ordered audit.
    ///
    /// # Errors
    ///
    /// none expected because an empty responder has no external dependency.
    pub fn new() -> Self {
        Self::default()
    }

    /// Quarantine the finding's target through the tier model.
    ///
    ///
    /// isolate suspect data without destroying it: the movement is an audited
    /// tier transition with the validator-finding reason.
    ///
    ///
    /// map the finding target onto a tier record, transition it to
    /// quarantine, and record the response linked to the transition.
    ///
    /// # Errors
    ///
    /// return `GraphError::InvalidTierTransition` when the target is not
    /// tier-trackable (retrievals, export records) or when the registry
    /// rejects the movement; nothing is recorded on failure.
    pub fn quarantine(
        &mut self,
        registry: &mut GraphTierRegistry,
        finding: &ValidationErrorRecord,
        actor_ref: impl Into<String>,
    ) -> Result<&ImmuneResponse, GraphError> {
        let record = tier_record_for(finding.target())?;
        let transition_sequence = registry.transition(
            record.clone(),
            GraphTier::Quarantine,
            actor_ref,
            TierTransitionReason::ValidatorFinding,
        )?;

        Ok(self.record(
            finding,
            ImmuneResponseAction::Quarantine { record },
            Some(transition_sequence),
        ))
    }

    /// Reduce the traversal priority of the finding's relationship.
    ///
    ///
    /// make learned navigation avoid suspect edges without touching any tier:
    /// the reduction is a typed report into the anti-pheromone field.
    ///
    ///
    /// report the signal on the finding's relationship in the given task
    /// scope and record the response.
    ///
    /// # Errors
    ///
    /// return `GraphError::InvalidTierTransition` when the finding does not
    /// target a relationship; nothing is recorded on failure.
    pub fn reduce_priority(
        &mut self,
        field: &mut AntiPheromoneField,
        scope: &PheromoneTaskScope,
        finding: &ValidationErrorRecord,
        signal: AntiPheromoneSignal,
    ) -> Result<&ImmuneResponse, GraphError> {
        let ValidationTarget::Relationship(relationship_value) = finding.target() else {
            return Err(GraphError::InvalidTierTransition(format!(
                "priority reduction requires a relationship target, got {:?}",
                finding.target()
            )));
        };
        let relationship_id = RelationshipId::new(relationship_value.clone())?;

        field.report_negative_observation(scope, &relationship_id, signal);
        Ok(self.record(
            finding,
            ImmuneResponseAction::ReducePriority { signal },
            None,
        ))
    }

    /// Request verification of the finding from a probe.
    ///
    ///
    /// defer judgment instead of acting destructively: the response names the
    /// probe whose answer will justify the follow-up action.
    ///
    ///
    /// record the response with the probe reference; no tier or field is
    /// touched.
    ///
    /// # Errors
    ///
    /// none expected because the request only appends to the audit.
    pub fn request_verification(
        &mut self,
        finding: &ValidationErrorRecord,
        probe_ref: impl Into<String>,
    ) -> &ImmuneResponse {
        self.record(
            finding,
            ImmuneResponseAction::RequestVerification {
                probe_ref: probe_ref.into(),
            },
            None,
        )
    }

    /// Propose a repair as a shadow-tier record.
    ///
    ///
    /// never rewrite canonical data: the proposal is a distinct record placed
    /// in the shadow tier, referencing the finding it addresses, and only an
    /// explicit audited promotion can make it canonical.
    ///
    ///
    /// transition the proposal record into the shadow tier with the
    /// repair-proposal reason and record the linked response.
    ///
    /// # Errors
    ///
    /// return `GraphError::InvalidTierTransition` when the registry rejects
    /// the movement; nothing is recorded on failure.
    pub fn propose_repair(
        &mut self,
        registry: &mut GraphTierRegistry,
        finding: &ValidationErrorRecord,
        proposal: TierRecordRef,
        actor_ref: impl Into<String>,
    ) -> Result<&ImmuneResponse, GraphError> {
        let transition_sequence = registry.transition(
            proposal.clone(),
            GraphTier::Shadow,
            actor_ref,
            TierTransitionReason::RepairProposal,
        )?;

        Ok(self.record(
            finding,
            ImmuneResponseAction::ProposeRepair { proposal },
            Some(transition_sequence),
        ))
    }

    /// Return the ordered response audit.
    ///
    ///
    /// expose the complete finding -> response -> transition chain for review.
    ///
    ///
    /// return the append-only response list.
    ///
    /// # Errors
    ///
    /// none expected because reading the audit cannot fail.
    pub fn audit(&self) -> &[ImmuneResponse] {
        &self.responses
    }

    fn record(
        &mut self,
        finding: &ValidationErrorRecord,
        action: ImmuneResponseAction,
        tier_transition_sequence: Option<u64>,
    ) -> &ImmuneResponse {
        let sequence = self.next_sequence;
        self.next_sequence += 1;
        self.responses.push(ImmuneResponse {
            finding_code: finding.code().to_owned(),
            finding_target: finding.target().clone(),
            action,
            sequence,
            tier_transition_sequence,
        });
        self.responses.last().expect("a response was just recorded")
    }
}

/// Map a validation target onto a tier-trackable record.
fn tier_record_for(target: &ValidationTarget) -> Result<TierRecordRef, GraphError> {
    match target {
        ValidationTarget::Node(value) => Ok(TierRecordRef::Node(NodeId::new(value.clone())?)),
        ValidationTarget::Relationship(value) => Ok(TierRecordRef::Relationship(
            RelationshipId::new(value.clone())?,
        )),
        ValidationTarget::Claim(value) => Ok(TierRecordRef::Claim(ClaimId::new(value.clone())?)),
        ValidationTarget::ExportRecord(_) | ValidationTarget::Retrieval(_) => Err(
            GraphError::InvalidTierTransition(format!("target {target:?} is not tier-trackable")),
        ),
    }
}

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
//! Candidate records remain outside canonical graph materialization until promotion.
use crate::{
    ActorId, CandidateId, ExtractionRunId, Graph, GraphError, GraphTier, GraphTierRegistry,
    NodeInput, RelationshipInput, TierRecordRef, TierTransition, TierTransitionReason,
};
use serde::{Deserialize, Serialize};
/// Immutable extraction proposal, preserving the raw payload as a string.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateInput {
    id: CandidateId,
    extraction_run_id: ExtractionRunId,
    raw_payload: String,
    actor: ActorId,
    landing_tier: GraphTier,
}
impl CandidateInput {
    /// Create a shadow-tier proposal. Extraction run and actor are mandatory.
    pub fn new(
        id: impl Into<String>,
        extraction_run_id: ExtractionRunId,
        raw_payload: impl Into<String>,
        actor: ActorId,
    ) -> Result<Self, GraphError> {
        Ok(Self {
            id: CandidateId::new(id)?,
            extraction_run_id,
            raw_payload: raw_payload.into(),
            actor,
            landing_tier: GraphTier::Shadow,
        })
    }
    /// Choose shadow or hypothesis; other tiers are rejected on submission.
    pub fn with_tier(mut self, tier: GraphTier) -> Self {
        self.landing_tier = tier;
        self
    }
    /// Stable candidate identifier.
    pub fn id(&self) -> &CandidateId {
        &self.id
    }
    /// Verbatim extractor output.
    pub fn raw_payload(&self) -> &str {
        &self.raw_payload
    }
    /// Extraction execution that produced the proposal.
    pub fn extraction_run_id(&self) -> &ExtractionRunId {
        &self.extraction_run_id
    }
}
/// Reviewed graph record to materialize only during explicit promotion.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum CandidatePromotionInput {
    /// A new canonical node.
    Node(NodeInput),
    /// A new canonical relationship.
    Relationship(RelationshipInput),
}
/// Immutable evidence of who promoted a candidate and the resulting record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CandidatePromotion {
    candidate_id: CandidateId,
    actor: ActorId,
    reason: String,
    target: TierRecordRef,
    input: CandidatePromotionInput,
}
impl CandidatePromotion {
    /// Reviewing actor.
    pub fn actor(&self) -> &ActorId {
        &self.actor
    }
    /// Explicit review justification.
    pub fn reason(&self) -> &str {
        &self.reason
    }
    /// Canonical record created by this operation.
    pub fn target(&self) -> &TierRecordRef {
        &self.target
    }
    /// Original proposal.
    pub fn candidate_id(&self) -> &CandidateId {
        &self.candidate_id
    }
}
/// Append-only candidate ledger. The existing tier registry is rebuilt from
/// immutable submissions/promotions on restoration, avoiding parallel tier state.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "StoredCandidates")]
pub struct CandidateStore {
    records: Vec<CandidateInput>,
    promotions: Vec<CandidatePromotion>,
    transitions: Vec<TierTransition>,
    #[serde(skip)]
    tiers: GraphTierRegistry,
}
#[derive(Deserialize)]
struct StoredCandidates {
    records: Vec<CandidateInput>,
    promotions: Vec<CandidatePromotion>,
    transitions: Vec<TierTransition>,
}
impl TryFrom<StoredCandidates> for CandidateStore {
    type Error = GraphError;
    fn try_from(stored: StoredCandidates) -> Result<Self, Self::Error> {
        // Replay in original event order and verify every persisted transition.
        // Separate record arrays alone lose interleaved submission/promotion order.
        let mut result = Self::default();
        let mut records = stored.records.into_iter();
        let mut promotions = stored.promotions.into_iter();
        for transition in stored.transitions {
            let before = result.transitions.len();
            match transition.reason {
                TierTransitionReason::CandidateSubmission => {
                    result.submit(
                        records
                            .next()
                            .ok_or_else(|| invalid("missing stored candidate"))?,
                    )?;
                }
                TierTransitionReason::AuditedPromotion => {
                    result.record_promotion(
                        promotions
                            .next()
                            .ok_or_else(|| invalid("missing stored promotion"))?,
                    )?;
                }
                _ => return Err(invalid("invalid candidate transition reason")),
            }
            if result.transitions.len() != before + 1
                || result.transitions.last() != Some(&transition)
            {
                return Err(invalid("candidate audit mismatch"));
            }
        }
        if records.next().is_some() || promotions.next().is_some() {
            return Err(invalid("unaudited candidate records"));
        }
        Ok(result)
    }
}
fn invalid(message: &str) -> GraphError {
    GraphError::InvalidPropertyValue(message.into())
}
impl CandidateStore {
    fn submit(&mut self, input: CandidateInput) -> Result<CandidateInput, GraphError> {
        CandidateId::new(input.id.as_str())?;
        ExtractionRunId::new(input.extraction_run_id.as_str())?;
        ActorId::new(input.actor.as_str())?;
        if !matches!(
            input.landing_tier,
            GraphTier::Shadow | GraphTier::Hypothesis
        ) {
            return Err(invalid("candidate must land in Shadow or Hypothesis"));
        }
        if let Some(existing) = self.get(input.id()) {
            return if existing == &input {
                Ok(existing.clone())
            } else {
                Err(invalid("immutable candidate identifier conflict"))
            };
        }
        self.tiers.transition(
            TierRecordRef::Candidate(input.id.clone()),
            input.landing_tier,
            input.actor.as_str(),
            TierTransitionReason::CandidateSubmission,
        )?;
        self.transitions
            .extend(self.tiers.audit_trail().last().cloned());
        self.records.push(input.clone());
        Ok(input)
    }
    fn record_promotion(&mut self, promotion: CandidatePromotion) -> Result<(), GraphError> {
        ActorId::new(promotion.actor.as_str())?;
        if promotion.reason.trim().is_empty() {
            return Err(invalid("promotion requires a review reason"));
        }
        if self.get(&promotion.candidate_id).is_none() {
            return Err(invalid("unknown candidate"));
        }
        if !matches!(
            (&promotion.input, &promotion.target),
            (CandidatePromotionInput::Node(_), TierRecordRef::Node(_))
                | (
                    CandidatePromotionInput::Relationship(_),
                    TierRecordRef::Relationship(_)
                )
        ) {
            return Err(invalid("promotion target kind mismatch"));
        }
        self.tiers.transition(
            TierRecordRef::Candidate(promotion.candidate_id.clone()),
            GraphTier::Canonical,
            promotion.actor.as_str(),
            TierTransitionReason::AuditedPromotion,
        )?;
        self.transitions
            .extend(self.tiers.audit_trail().last().cloned());
        self.promotions.push(promotion);
        Ok(())
    }

    /// Whether no proposal has been recorded.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
    /// Look up the original proposal, including after promotion.
    pub fn get(&self, id: &CandidateId) -> Option<&CandidateInput> {
        self.records.iter().find(|r| r.id() == id)
    }
    /// Current tier, absent for an unknown candidate rather than canonical.
    pub fn tier_of(&self, id: &CandidateId) -> Option<GraphTier> {
        self.get(id)
            .map(|_| self.tiers.tier_of(&TierRecordRef::Candidate(id.clone())))
    }
    /// Retained promotion records.
    pub fn promotions(&self) -> &[CandidatePromotion] {
        &self.promotions
    }
    /// Tier transition audit from Epic 0019.
    pub fn tier_audit(&self) -> &[TierTransition] {
        self.tiers.audit_trail()
    }
}
impl Graph {
    /// Submit a proposal without materializing a canonical node or relationship.
    pub fn submit_candidate(
        &mut self,
        input: CandidateInput,
    ) -> Result<CandidateInput, GraphError> {
        // Validate provenance and non-canonical destination before storing raw
        // bytes; exact retries must not append duplicate records or tier events.
        self.epistemic_stores_mut().candidates.submit(input)
    }
    /// Explicitly materialize a reviewed record and append its promotion receipt.
    pub fn promote_candidate(
        &mut self,
        id: &CandidateId,
        actor: ActorId,
        reason: impl Into<String>,
        input: CandidatePromotionInput,
    ) -> Result<CandidatePromotion, GraphError> {
        // Stage record creation and audited promotion together. Validation errors
        // leave both graph and candidate ledger unchanged; raw input never changes.
        let reason = reason.into();
        ActorId::new(actor.as_str())?;
        if reason.trim().is_empty() {
            return Err(invalid("promotion requires a review reason"));
        }
        let store = &self.epistemic_stores().candidates;
        let run = store
            .get(id)
            .ok_or_else(|| invalid("unknown candidate"))?
            .extraction_run_id
            .clone();
        if let Some(existing) = store.promotions.iter().find(|p| &p.candidate_id == id) {
            return if existing.actor == actor
                && existing.reason == reason
                && existing.input == input
            {
                Ok(existing.clone())
            } else {
                Err(invalid(
                    "candidate already promoted with different review input",
                ))
            };
        }
        let mut staged = self.clone();
        let target = match input.clone() {
            CandidatePromotionInput::Node(node) => {
                TierRecordRef::Node(staged.create_node(node.with_extraction_run_id(run))?)
            }
            CandidatePromotionInput::Relationship(relationship) => TierRecordRef::Relationship(
                staged.create_relationship(relationship.with_extraction_run_id(run))?,
            ),
        };
        let promotion = CandidatePromotion {
            candidate_id: id.clone(),
            actor,
            reason,
            target,
            input,
        };
        staged
            .epistemic_stores_mut()
            .candidates
            .record_promotion(promotion.clone())?;
        *self = staged;
        Ok(promotion)
    }
}

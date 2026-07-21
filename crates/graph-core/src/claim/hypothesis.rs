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
use super::*;

/// Explicit lifecycle state for bounded hypothesis workspaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HypothesisWorkspaceStatus {
    /// Active.
    Active,
    /// Deferred.
    Deferred,
    /// Rejected.
    Rejected,
    /// Merged later.
    MergedLater,
}

/// Input payload for creating a hypothesis workspace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HypothesisWorkspaceInput {
    pub(crate) id: HypothesisWorkspaceId,
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) owner_actor: ActorId,
    pub(crate) created_at: Option<String>,
    pub(crate) parent_context_ref: Option<String>,
}

impl HypothesisWorkspaceInput {
    /// Creates a new instance.
    pub fn new(
        id: HypothesisWorkspaceId,
        title: String,
        description: String,
        owner_actor: ActorId,
    ) -> Self {
        Self {
            id,
            title,
            description,
            owner_actor,
            // Created at.
            created_at: None,
            // Parent context ref.
            parent_context_ref: None,
        }
    }

    /// Sets the created at.
    pub fn with_created_at(mut self, created_at: String) -> Self {
        self.created_at = Some(created_at);
        self
    }

    /// Sets the parent context ref.
    pub fn with_parent_context_ref(mut self, parent_context_ref: String) -> Self {
        self.parent_context_ref = Some(parent_context_ref);
        self
    }
}

/// Trust-input category used as policy input for future deterministic
/// resolution, not as an automatic truth decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrustInputKind {
    /// Source reliability.
    SourceReliability,
    /// Extractor reliability.
    ExtractorReliability,
    /// Model reliability.
    ModelReliability,
    /// Agent reliability.
    AgentReliability,
    /// Validation rule reliability.
    ValidationRuleReliability,
    /// Historical correction.
    HistoricalCorrection,
}

/// Input payload for recording a trust signal.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrustInputInput {
    pub(crate) kind: TrustInputKind,
    pub(crate) subject_ref: String,
    pub(crate) value: f64,
    pub(crate) provenance_ref: Option<String>,
    pub(crate) reason_ref: Option<String>,
    pub(crate) temporal: TemporalMetadata,
    pub(crate) claim_refs: Vec<ClaimId>,
}

impl TrustInputInput {
    /// Creates a new instance.
    pub fn new(kind: TrustInputKind, subject_ref: String, value: f64) -> Self {
        Self {
            kind,
            subject_ref,
            value,
            // Provenance ref.
            provenance_ref: None,
            // Reason ref.
            reason_ref: None,
            // Temporal.
            temporal: TemporalMetadata::default(),
            // Claim refs.
            claim_refs: Vec::new(),
        }
    }

    /// Sets the provenance ref.
    pub fn with_provenance_ref(mut self, provenance_ref: String) -> Self {
        self.provenance_ref = Some(provenance_ref);
        self
    }

    /// Sets the reason ref.
    pub fn with_reason_ref(mut self, reason_ref: String) -> Self {
        self.reason_ref = Some(reason_ref);
        self
    }

    /// Sets the temporal.
    pub fn with_temporal(mut self, temporal: TemporalMetadata) -> Self {
        self.temporal = temporal;
        self
    }

    /// Sets the claim ref.
    pub fn with_claim_ref(mut self, claim_id: ClaimId) -> Self {
        self.claim_refs.push(claim_id);
        self
    }
}

/// Bounded hypothesis container for competing analytical interpretations.
///
/// This model is separate from working sets: working sets control what is
/// loaded for traversal, while hypothesis workspaces control epistemic context.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HypothesisWorkspace {
    pub(crate) id: HypothesisWorkspaceId,
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) status: HypothesisWorkspaceStatus,
    pub(crate) owner_actor: ActorId,
    pub(crate) created_at: Option<String>,
    pub(crate) parent_context_ref: Option<String>,
}

impl HypothesisWorkspace {
    /// Id.
    pub fn id(&self) -> &HypothesisWorkspaceId {
        &self.id
    }

    /// Title.
    pub fn title(&self) -> &str {
        self.title.as_str()
    }

    /// Description.
    pub fn description(&self) -> &str {
        self.description.as_str()
    }

    /// Status.
    pub fn status(&self) -> HypothesisWorkspaceStatus {
        self.status
    }

    /// Owner actor.
    pub fn owner_actor(&self) -> &ActorId {
        &self.owner_actor
    }

    /// Created at.
    pub fn created_at(&self) -> Option<&str> {
        self.created_at.as_deref()
    }

    /// Parent context ref.
    pub fn parent_context_ref(&self) -> Option<&str> {
        self.parent_context_ref.as_deref()
    }
}

/// First-class trust input record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrustInput {
    pub(crate) trust_input_id: String,
    pub(crate) kind: TrustInputKind,
    pub(crate) subject_ref: String,
    pub(crate) value: f64,
    pub(crate) provenance_ref: Option<String>,
    pub(crate) reason_ref: Option<String>,
    pub(crate) temporal: TemporalMetadata,
    pub(crate) claim_refs: Vec<ClaimId>,
}

impl TrustInput {
    /// Trust input id.
    pub fn trust_input_id(&self) -> &str {
        self.trust_input_id.as_str()
    }

    /// Kind.
    pub fn kind(&self) -> TrustInputKind {
        self.kind
    }

    /// Subject ref.
    pub fn subject_ref(&self) -> &str {
        self.subject_ref.as_str()
    }

    /// Value.
    pub fn value(&self) -> f64 {
        self.value
    }

    /// Provenance ref.
    pub fn provenance_ref(&self) -> Option<&str> {
        self.provenance_ref.as_deref()
    }

    /// Reason ref.
    pub fn reason_ref(&self) -> Option<&str> {
        self.reason_ref.as_deref()
    }

    /// Temporal.
    pub fn temporal(&self) -> &TemporalMetadata {
        &self.temporal
    }

    /// Claim refs.
    pub fn claim_refs(&self) -> &[ClaimId] {
        self.claim_refs.as_slice()
    }
}

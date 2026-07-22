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

/// Explicit semantic intent for links between evidence/claims and target
/// claims.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClaimLinkKind {
    /// Indicates supporting context for a target claim.
    Supports,
    /// Indicates refuting context for a target claim.
    Refutes,
    /// Indicates explicit conflict context between two claims without deciding
    /// final claim resolution.
    Contradicts,
    /// Indicates one claim replaces another while preserving historical claim
    /// records for audit and versioning boundaries.
    Supersedes,
}

/// Typed source for a claim link so evidence-to-claim and claim-to-claim links
/// are represented explicitly.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClaimLinkSource {
    /// Evidence.
    Evidence(EvidenceId),
    /// Claim.
    Claim(ClaimId),
}

/// Normalized support/refutation link record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimLink {
    pub(crate) source: ClaimLinkSource,
    pub(crate) target_claim_id: ClaimId,
    pub(crate) kind: ClaimLinkKind,
    pub(crate) explanation_ref: Option<String>,
}

/// Decision semantics for explicit lifecycle adjudication events.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClaimDecisionKind {
    /// Retraction.
    Retraction,
    /// Rejection.
    Rejection,
}

/// Reason metadata captured when a claim is retracted or rejected.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimDecision {
    pub(crate) kind: ClaimDecisionKind,
    pub(crate) reason: String,
    pub(crate) actor_ref: Option<String>,
    pub(crate) session_ref: Option<String>,
}

impl ClaimDecision {
    /// Creates a new instance.
    pub fn new(
        kind: ClaimDecisionKind,
        reason: String,
        actor_ref: Option<String>,
        session_ref: Option<String>,
    ) -> Self {
        Self {
            kind,
            reason,
            actor_ref,
            session_ref,
        }
    }

    /// Kind.
    pub fn kind(&self) -> ClaimDecisionKind {
        self.kind
    }

    /// Reason.
    pub fn reason(&self) -> &str {
        self.reason.as_str()
    }

    /// Actor ref.
    pub fn actor_ref(&self) -> Option<&str> {
        self.actor_ref.as_deref()
    }

    /// Session ref.
    pub fn session_ref(&self) -> Option<&str> {
        self.session_ref.as_deref()
    }
}

impl ClaimLink {
    /// Creates a new instance.
    pub fn new(source: ClaimLinkSource, target_claim_id: ClaimId, kind: ClaimLinkKind) -> Self {
        Self {
            source,
            target_claim_id,
            kind,
            // Explanation ref.
            explanation_ref: None,
        }
    }

    /// Sets the explanation ref.
    pub fn with_explanation_ref(mut self, explanation_ref: Option<String>) -> Self {
        self.explanation_ref = explanation_ref;
        self
    }

    /// Source.
    pub fn source(&self) -> &ClaimLinkSource {
        &self.source
    }

    /// Target claim id.
    pub fn target_claim_id(&self) -> &ClaimId {
        &self.target_claim_id
    }

    /// Kind.
    pub fn kind(&self) -> ClaimLinkKind {
        self.kind
    }

    /// Explanation ref.
    pub fn explanation_ref(&self) -> Option<&str> {
        self.explanation_ref.as_deref()
    }
}

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
    /// Provides context for a target claim without supporting or refuting it.
    ContextFor,
    /// Marks the source as a duplicate of the target claim (same proposition
    /// from another extraction or source).
    Duplicates,
    /// Marks the target claim as derived from the source by inference or
    /// aggregation rather than directly observed.
    DerivedFrom,
    /// Marks the target claim as depending on the source holding.
    DependsOn,
}

impl ClaimLinkKind {
    /// Closed vocabulary in canonical order: the four Epic 0005 kinds first,
    /// then the four Epic 0029 kinds.
    pub const ALL: [Self; 8] = [
        Self::Supports,
        Self::Refutes,
        Self::Contradicts,
        Self::Supersedes,
        Self::ContextFor,
        Self::Duplicates,
        Self::DerivedFrom,
        Self::DependsOn,
    ];

    /// Canonical lowercase token used in explanation keys and projections.
    pub fn as_str(self) -> &'static str {
        claim_link_kind_token(self)
    }

    /// Explanation kind recorded when a link of this kind is attached.
    pub fn explanation_kind(self) -> EpistemicExplanationKind {
        claim_link_kind_to_explanation_kind(self)
    }
}

/// Typed source for a claim link so evidence-to-claim and claim-to-claim links
/// are represented explicitly.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClaimLinkSource {
    /// Evidence.
    Evidence(EvidenceId),
    /// Claim.
    Claim(ClaimId),
    /// Observation (Epic 0029): the exact span behind the link, which
    /// resolves to its `Source` through the observation store.
    Observation(ObservationId),
}

impl ClaimLinkSource {
    /// Evidence identifier when the source is evidence.
    pub fn evidence_id(&self) -> Option<&EvidenceId> {
        match self {
            Self::Evidence(id) => Some(id),
            Self::Claim(_) | Self::Observation(_) => None,
        }
    }

    /// Claim identifier when the source is a claim.
    pub fn claim_id(&self) -> Option<&ClaimId> {
        match self {
            Self::Claim(id) => Some(id),
            Self::Evidence(_) | Self::Observation(_) => None,
        }
    }

    /// Observation identifier when the source is an observation.
    pub fn observation_id(&self) -> Option<&ObservationId> {
        match self {
            Self::Observation(id) => Some(id),
            Self::Evidence(_) | Self::Claim(_) => None,
        }
    }

    /// Canonical lowercase token of the source kind.
    pub fn kind_token(&self) -> &'static str {
        match self {
            Self::Evidence(_) => "evidence",
            Self::Claim(_) => "claim",
            Self::Observation(_) => "observation",
        }
    }

    /// Identifier string of the source record.
    pub fn id_str(&self) -> &str {
        match self {
            Self::Evidence(id) => id.as_str(),
            Self::Claim(id) => id.as_str(),
            Self::Observation(id) => id.as_str(),
        }
    }
}

/// Evidence link record (ADR-0016): a typed epistemic relation from evidence,
/// an observation, or a claim to a target claim, with optional governance
/// fields. The type keeps its Epic 0005 name and serialization; the
/// documentation refers to it as the evidence link.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClaimLink {
    pub(crate) source: ClaimLinkSource,
    pub(crate) target_claim_id: ClaimId,
    pub(crate) kind: ClaimLinkKind,
    pub(crate) explanation_ref: Option<String>,
    /// Governance fields (Epic 0029 WS-A item 4). Stored and returned here,
    /// populated by the corroboration engine of WS-D. Absent on every link
    /// created before this change and skipped on serialization so stored
    /// payloads stay byte-stable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) strength: Option<Confidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) authority: Option<Confidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) independence_cluster: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) bitemporal: Option<BitemporalStamp>,
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
            strength: None,
            authority: None,
            independence_cluster: None,
            bitemporal: None,
        }
    }

    /// Sets the per-link strength signal (unit interval).
    pub fn with_strength(mut self, strength: Confidence) -> Self {
        self.strength = Some(strength);
        self
    }

    /// Sets the per-link source authority signal (unit interval).
    pub fn with_authority(mut self, authority: Confidence) -> Self {
        self.authority = Some(authority);
        self
    }

    /// Sets the opaque independence-cluster identifier the source belongs to.
    pub fn with_independence_cluster(mut self, cluster: impl Into<String>) -> Self {
        self.independence_cluster = Some(cluster.into());
        self
    }

    /// Sets the bitemporal stamp of the link.
    pub fn with_bitemporal(mut self, bitemporal: BitemporalStamp) -> Self {
        self.bitemporal = Some(bitemporal);
        self
    }

    /// Strength, when set.
    pub fn strength(&self) -> Option<Confidence> {
        self.strength
    }

    /// Authority, when set.
    pub fn authority(&self) -> Option<Confidence> {
        self.authority
    }

    /// Independence cluster, when set.
    pub fn independence_cluster(&self) -> Option<&str> {
        self.independence_cluster.as_deref()
    }

    /// Bitemporal stamp, when set.
    pub fn bitemporal(&self) -> Option<&BitemporalStamp> {
        self.bitemporal.as_ref()
    }

    /// Stable reference key of the link: `<source kind>:<source id>:<target
    /// claim>:<kind token>`. Used by explanations and verification inputs.
    pub fn reference_key(&self) -> String {
        claim_link_explanation_key(self)
    }

    /// Whether the link is active at an as-of point. Links without a stamp are
    /// always active; stamped links follow the bitemporal rule.
    pub fn is_active_at(&self, as_of: &VerdictAsOf) -> bool {
        self.bitemporal
            .as_ref()
            .is_none_or(|stamp| as_of.covers(stamp))
    }

    /// Project the link into additive, namespaced properties for a
    /// relationship in the epistemic vocabulary.
    ///
    /// Keys are prefixed `evidence_link_`; optional fields are omitted.
    pub fn to_property_map(&self) -> PropertyMap {
        let mut properties = PropertyMap::new();
        let mut put = |key: &str, value: PropertyValue| {
            properties.insert(key.to_owned(), value);
        };

        put(
            "evidence_link_kind",
            PropertyValue::String(self.kind.as_str().to_owned()),
        );
        put(
            "evidence_link_source_kind",
            PropertyValue::String(self.source.kind_token().to_owned()),
        );
        put(
            "evidence_link_source",
            PropertyValue::String(self.source.id_str().to_owned()),
        );
        put(
            "evidence_link_target_claim",
            PropertyValue::String(self.target_claim_id.as_str().to_owned()),
        );
        if let Some(explanation_ref) = &self.explanation_ref {
            put(
                "evidence_link_explanation_ref",
                PropertyValue::String(explanation_ref.clone()),
            );
        }
        if let Some(strength) = self.strength {
            put(
                "evidence_link_strength",
                PropertyValue::Float(strength.value()),
            );
        }
        if let Some(authority) = self.authority {
            put(
                "evidence_link_authority",
                PropertyValue::Float(authority.value()),
            );
        }
        if let Some(cluster) = &self.independence_cluster {
            put(
                "evidence_link_independence_cluster",
                PropertyValue::String(cluster.clone()),
            );
        }
        if let Some(stamp) = &self.bitemporal {
            put(
                "evidence_link_valid_from",
                PropertyValue::String(stamp.valid_from.as_str().to_owned()),
            );
            if let Some(valid_to) = &stamp.valid_to {
                put(
                    "evidence_link_valid_to",
                    PropertyValue::String(valid_to.as_str().to_owned()),
                );
            }
            put(
                "evidence_link_transaction_time",
                PropertyValue::String(stamp.transaction_time.as_str().to_owned()),
            );
        }

        properties
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

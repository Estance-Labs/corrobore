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
//! Coordination signals over a neutral collection, stored as evidence.
//!
//! Module boundary: this module observes production-side patterns across the
//! claims of one `Narrative` or `Campaign` and retains them as attributed
//! evidence annotations. It owns no verdict, no tier, no claim, and no
//! attribution. Two boundaries are structural, not conventional:
//!
//! - A signal says content shares a production pattern. It never says who
//!   produced it. Attribution is assessed separately and requires a claim the
//!   engine holds supported; no number of signals substitutes for one.
//! - A signal that reflects curation rather than production never collapses
//!   independence. Otherwise grouping content into a collection would deflate
//!   the support of the very claims it collects.
//!
//! The scope is what a single-claim risk assessment cannot reach: `WS-E`
//! evidence risk requires every record to be linked to one assessed claim, so a
//! pattern spread across the claims of a campaign is invisible to it.
use crate::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Token overlap above which two records from distinct sources are redundant.
///
/// Deliberately below the `WS-E` duplication threshold: campaign redundancy is
/// recycled material across articles, not two copies of one article.
pub const CROSS_CONTENT_REDUNDANCY_JACCARD: f64 = 0.5;
/// Reason prefix identifying the detector version in every retained signal.
pub const CAMPAIGN_SIGNAL_REASON_PREFIX: &str = "ws-g-signal-v1";

/// A production-side pattern observed across a collection's content.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CampaignSignal {
    /// The same attributed generation-prompt residue in several records.
    RepeatedPromptArtifact,
    /// The same attributed generation-style fingerprint in several records.
    GenerationStyleFingerprint,
    /// Recycled material across records from distinct sources.
    CrossContentRedundancy,
    /// The same attributed infrastructure identity behind distinct sources.
    SharedInfrastructure,
    /// Distinct content sources declared as members of one collection.
    NarrativeCoMembership,
}

/// The collection an assessment covers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalScope {
    /// One narrative and the claims it collects.
    Narrative(NarrativeId),
    /// One campaign, its own claims, and those of every narrative it collects.
    Campaign(CampaignId),
}

/// Attributed metadata the host supplies for one stored record.
///
/// Nothing here is an attestation about a producer: it is instrumentation
/// output, and its `attribution` names the instrument.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CampaignSignalFeatures {
    /// Existing immutable evidence record.
    pub evidence_id: EvidenceId,
    /// Instrumentation or assessment provenance, not an attestation.
    pub attribution: String,
    /// Attributed prompt residues; generic phrasing is not a residue.
    pub prompt_artifacts: Vec<String>,
    /// Attributed style fingerprint; a bare model name is not a fingerprint.
    pub generation_style_fingerprint: Option<String>,
    /// Precise infrastructure identities; a hosting category is unsuitable.
    pub infrastructure: Vec<String>,
}

/// A retained signal with its exact records, their sources, and its reason.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CampaignSignalFinding {
    signal: CampaignSignal,
    group_id: String,
    scope: SignalScope,
    evidence_ids: Vec<EvidenceId>,
    source_ids: Vec<SourceId>,
    reason: String,
}

/// Append-only coordination evidence, stored once per finding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CampaignSignalAnnotation {
    /// Original detection.
    pub finding: CampaignSignalFinding,
    /// When the signal became known; historical resolutions exclude later knowledge.
    pub stamp: BitemporalStamp,
}

/// One content-addressed coordination receipt held by the evidence store.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredCampaignSignal {
    /// Stable identity of the complete annotation.
    pub id: String,
    /// Retained annotation.
    pub annotation: CampaignSignalAnnotation,
}

/// A request to attribute a collection to an actor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttributionRequest {
    actor: NodeId,
    scope: SignalScope,
    cited_signals: Vec<String>,
    corroborating_claims: Vec<ClaimId>,
}

/// Why the cited support cannot carry an attribution.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttributionRefusal {
    /// Nothing was cited.
    NoSupportCited,
    /// A cited signal is not retained for this collection.
    UnknownSignal(String),
    /// Only coordination signals were cited; production is not authorship.
    CoordinationSignalsOnly,
    /// A cited claim is not supported at the assessment point.
    CorroborationNotSupported(ClaimId),
}

/// Whether an attribution may be recorded, and on what.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttributionAdmissibility {
    /// Refused, with the reason retained for the audit path.
    Refused(AttributionRefusal),
    /// Admissible on the cited supported claims. The core records nothing: the
    /// caller asserts the attribution as an ordinary governed claim.
    Admissible {
        /// Cited claims the engine holds supported at the assessment point.
        corroborating: Vec<ClaimId>,
    },
}

impl CampaignSignal {
    /// Closed vocabulary in canonical order.
    pub const ALL: [Self; 5] = [
        Self::RepeatedPromptArtifact,
        Self::GenerationStyleFingerprint,
        Self::CrossContentRedundancy,
        Self::SharedInfrastructure,
        Self::NarrativeCoMembership,
    ];

    /// Canonical snake_case token used in reasons and dependency explanations.
    pub fn as_str(self) -> &'static str {
        unimplemented!("phase 3")
    }

    /// Whether the signal is production-side evidence of a shared pipeline and
    /// may therefore join dependent links into one cluster.
    ///
    /// Co-membership is curation. Letting it collapse independence would mean an
    /// analyst grouping content could reduce the support of its own claims.
    pub fn affects_independence(self) -> bool {
        unimplemented!("phase 3")
    }
}

impl SignalScope {
    /// Identity of the collection, exactly as recorded.
    pub fn id(&self) -> &str {
        unimplemented!("phase 3")
    }

    /// Canonical snake_case collection kind.
    pub fn kind(&self) -> &'static str {
        unimplemented!("phase 3")
    }
}

impl CampaignSignalFeatures {
    /// Start with no measured metadata; content comes from the graph.
    pub fn new(evidence_id: EvidenceId, attribution: impl Into<String>) -> Self {
        Self {
            evidence_id,
            attribution: attribution.into(),
            prompt_artifacts: Vec::new(),
            generation_style_fingerprint: None,
            infrastructure: Vec::new(),
        }
    }
}

impl CampaignSignalFinding {
    /// Detected pattern.
    pub fn signal(&self) -> CampaignSignal {
        unimplemented!("phase 3")
    }

    /// Stable content-derived identity used for grouping and idempotency.
    pub fn group_id(&self) -> &str {
        unimplemented!("phase 3")
    }

    /// Collection the detection covered.
    pub fn scope(&self) -> &SignalScope {
        unimplemented!("phase 3")
    }

    /// Exact records carrying the pattern, in identity order.
    pub fn evidence_ids(&self) -> &[EvidenceId] {
        unimplemented!("phase 3")
    }

    /// Distinct sources behind those records, in identity order.
    pub fn source_ids(&self) -> &[SourceId] {
        unimplemented!("phase 3")
    }

    /// Measurement, threshold and attribution explaining the detection.
    pub fn reason(&self) -> &str {
        unimplemented!("phase 3")
    }

    /// Whether this finding may join dependent links into one cluster.
    pub fn affects_independence(&self) -> bool {
        unimplemented!("phase 3")
    }
}

impl AttributionRequest {
    /// Build a request naming the actor, the collection, and the cited support.
    ///
    /// The actor is an opaque canonical reference: assessing admissibility never
    /// requires the entity to be resident, and never creates it.
    pub fn new(
        actor: NodeId,
        scope: SignalScope,
        cited_signals: Vec<String>,
        corroborating_claims: Vec<ClaimId>,
    ) -> Self {
        Self {
            actor,
            scope,
            cited_signals,
            corroborating_claims,
        }
    }

    /// Actor the attribution would name.
    pub fn actor(&self) -> &NodeId {
        unimplemented!("phase 3")
    }

    /// Collection under assessment.
    pub fn scope(&self) -> &SignalScope {
        unimplemented!("phase 3")
    }

    /// Cited coordination signal identities.
    pub fn cited_signals(&self) -> &[String] {
        unimplemented!("phase 3")
    }

    /// Cited corroborating claims.
    pub fn corroborating_claims(&self) -> &[ClaimId] {
        unimplemented!("phase 3")
    }
}

/// Detect coordination signals over the claims of one collection.
///
/// Expected behavior, one deterministic rule per signal, each needing at least
/// two records from two distinct sources so a single source repeating itself is
/// never coordination:
///
/// - repeated prompt artifact: the same attributed residue in several records;
/// - generation-style fingerprint: the same attributed fingerprint;
/// - cross-content redundancy: token overlap at or above
///   [`CROSS_CONTENT_REDUNDANCY_JACCARD`] between records from distinct sources,
///   coalesced into one finding per connected group;
/// - shared infrastructure: a common attributed infrastructure identity;
/// - co-membership: distinct content sources declared by the collection.
///
/// Validation targets: every record exists, is linked to a claim of the
/// collection, and carries bounded attributed metadata; the collection resolves;
/// the request is bounded; identities are unique.
///
/// # Errors
/// Reject an unknown collection, unknown or out-of-scope records, duplicate
/// identities, unbounded requests and malformed metadata before detection.
pub fn detect_campaign_signals(
    graph: &Graph,
    scope: &SignalScope,
    features: &[CampaignSignalFeatures],
) -> Result<Vec<CampaignSignalFinding>, GraphError> {
    unimplemented!("phase 3")
}

impl Graph {
    /// Detect and retain coordination signals as append-only evidence.
    ///
    /// Expected behavior: retain one content-addressed annotation per finding so
    /// replay is idempotent, and change nothing else. No claim, verdict, tier,
    /// immune response or canonical node is touched. The dependency effect
    /// appears on the next cluster assignment, which is where independence is
    /// computed.
    ///
    /// # Errors
    /// Reject detection failures and a knowledge stamp that is not open-ended.
    pub fn record_campaign_signals(
        &mut self,
        scope: &SignalScope,
        features: &[CampaignSignalFeatures],
        stamp: BitemporalStamp,
    ) -> Result<Vec<CampaignSignalFinding>, GraphError> {
        unimplemented!("phase 3")
    }

    /// Decide whether an attribution may rest on the cited support.
    ///
    /// Expected behavior: refuse an empty request, refuse a citation that is not
    /// retained for the collection, refuse coordination signals as the only
    /// support, and refuse a corroborating claim the engine does not hold
    /// supported at the assessment point. Otherwise report the supported claims
    /// the attribution may rest on, and record nothing.
    ///
    /// Validation target: no set of coordination signals is ever admissible on
    /// its own, which is what keeps a fingerprint from becoming an accusation.
    ///
    /// # Errors
    /// Reject an unknown collection or an unknown cited claim.
    pub fn assess_campaign_attribution(
        &self,
        request: &AttributionRequest,
        as_of: &VerdictAsOf,
    ) -> Result<AttributionAdmissibility, GraphError> {
        unimplemented!("phase 3")
    }
}

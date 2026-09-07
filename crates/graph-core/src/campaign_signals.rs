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

/// Records a single assessment may examine together.
const MAX_ASSESSED_RECORDS: usize = 64;
/// Bound on one attributed metadata value.
const MAX_METADATA_LENGTH: usize = 1024;
/// Bound on one attributed metadata list.
const MAX_METADATA_ITEMS: usize = 64;
/// Tokens a record needs before its content overlap means anything.
const MIN_REDUNDANCY_TOKENS: usize = 4;

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
        match self {
            Self::RepeatedPromptArtifact => "repeated_prompt_artifact",
            Self::GenerationStyleFingerprint => "generation_style_fingerprint",
            Self::CrossContentRedundancy => "cross_content_redundancy",
            Self::SharedInfrastructure => "shared_infrastructure",
            Self::NarrativeCoMembership => "narrative_co_membership",
        }
    }

    /// Whether the signal is production-side evidence of a shared pipeline and
    /// may therefore join dependent links into one cluster.
    ///
    /// Co-membership is curation. Letting it collapse independence would mean an
    /// analyst grouping content could reduce the support of its own claims.
    pub fn affects_independence(self) -> bool {
        !matches!(self, Self::NarrativeCoMembership)
    }
}

impl SignalScope {
    /// Identity of the collection, exactly as recorded.
    pub fn id(&self) -> &str {
        match self {
            Self::Narrative(id) => id.as_str(),
            Self::Campaign(id) => id.as_str(),
        }
    }

    /// Canonical snake_case collection kind.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Narrative(_) => "narrative",
            Self::Campaign(_) => "campaign",
        }
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
        self.signal
    }

    /// Stable content-derived identity used for grouping and idempotency.
    pub fn group_id(&self) -> &str {
        &self.group_id
    }

    /// Collection the detection covered.
    pub fn scope(&self) -> &SignalScope {
        &self.scope
    }

    /// Exact records carrying the pattern, in identity order.
    pub fn evidence_ids(&self) -> &[EvidenceId] {
        &self.evidence_ids
    }

    /// Distinct sources behind those records, in identity order.
    pub fn source_ids(&self) -> &[SourceId] {
        &self.source_ids
    }

    /// Measurement, threshold and attribution explaining the detection.
    pub fn reason(&self) -> &str {
        &self.reason
    }

    /// Whether this finding may join dependent links into one cluster.
    pub fn affects_independence(&self) -> bool {
        self.signal.affects_independence()
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
        &self.actor
    }

    /// Collection under assessment.
    pub fn scope(&self) -> &SignalScope {
        &self.scope
    }

    /// Cited coordination signal identities.
    pub fn cited_signals(&self) -> &[String] {
        &self.cited_signals
    }

    /// Cited corroborating claims.
    pub fn corroborating_claims(&self) -> &[ClaimId] {
        &self.corroborating_claims
    }
}

/// Claims and declared content of a resolved collection.
struct ScopedCollection {
    claims: BTreeSet<String>,
    content: BTreeSet<String>,
}

/// One assessed record with its resolved source and content tokens.
struct AssessedRecord<'a> {
    feature: &'a CampaignSignalFeatures,
    source: SourceId,
    tokens: BTreeSet<String>,
}

fn signal_error(message: &str) -> GraphError {
    GraphError::InvalidPropertyValue(message.into())
}

/// Resolve the claims and content a collection collects. A campaign covers its
/// own membership and that of every narrative it collects.
fn resolve_scope(graph: &Graph, scope: &SignalScope) -> Result<ScopedCollection, GraphError> {
    let store = &graph.epistemic_stores().narrative_campaigns;
    let mut collection = ScopedCollection {
        claims: BTreeSet::new(),
        content: BTreeSet::new(),
    };
    let mut absorb = |membership: &ContextMembership| {
        collection
            .claims
            .extend(membership.claims.iter().map(|id| id.as_str().to_owned()));
        collection
            .content
            .extend(membership.content.iter().map(|id| id.as_str().to_owned()));
    };
    match scope {
        SignalScope::Narrative(id) => {
            let record = store
                .narrative_by_id(id)
                .ok_or_else(|| signal_error("coordination scope narrative is missing"))?;
            absorb(record.membership());
        }
        SignalScope::Campaign(id) => {
            let record = store
                .campaign_by_id(id)
                .ok_or_else(|| signal_error("coordination scope campaign is missing"))?;
            absorb(record.membership());
            for narrative in record.narratives() {
                let narrative = store
                    .narrative_by_id(narrative)
                    .ok_or_else(|| signal_error("coordination scope narrative is missing"))?;
                absorb(narrative.membership());
            }
        }
    }
    Ok(collection)
}

/// The source behind a record: its bound source, its observation's source, or
/// its own reference. Coordination is a statement about sources, so a record
/// that resolves to none cannot take part.
fn resolve_source(graph: &Graph, record: &EvidenceRecord) -> Result<SourceId, GraphError> {
    record
        .source_id()
        .cloned()
        .or_else(|| {
            record
                .observation_id()
                .and_then(|id| graph.epistemic_stores().observations.observation_by_id(id))
                .map(|observation| observation.source_id().clone())
        })
        .or_else(|| SourceId::new(record.source_ref()).ok())
        .ok_or_else(|| signal_error("assessed record does not resolve to a source"))
}

fn normalize(value: &str) -> String {
    value
        .split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

fn tokens(value: &str) -> BTreeSet<String> {
    value
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Build one finding with its records, their distinct sources, the measurement,
/// and the attribution of every instrument that reported it.
fn build_finding(
    signal: CampaignSignal,
    scope: &SignalScope,
    members: &[&AssessedRecord<'_>],
    measurement: &str,
) -> CampaignSignalFinding {
    use sha2::{Digest, Sha256};
    let evidence_ids: Vec<_> = members
        .iter()
        .map(|member| member.feature.evidence_id.clone())
        .collect();
    let mut source_ids: Vec<_> = members
        .iter()
        .map(|member| member.source.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    source_ids.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    let attributions: Vec<_> = members
        .iter()
        .map(|member| {
            (
                member.feature.evidence_id.as_str(),
                member.feature.attribution.as_str(),
            )
        })
        .collect();
    let reason = format!(
        "{CAMPAIGN_SIGNAL_REASON_PREFIX}: {measurement}; attribution={}",
        serde_json::to_string(&attributions).expect("attributed strings")
    );
    let bytes = serde_json::to_vec(&(signal, scope, &evidence_ids, &reason))
        .expect("coordination signal identity");
    let group_id = format!(
        "campaign-signal--{}",
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    CampaignSignalFinding {
        signal,
        group_id,
        scope: scope.clone(),
        evidence_ids,
        source_ids,
        reason,
    }
}

/// Distinct sources behind a member set. Coordination needs at least two: one
/// source repeating itself is not several sources agreeing.
fn distinct_sources(members: &[&AssessedRecord<'_>]) -> usize {
    members
        .iter()
        .map(|member| member.source.as_str())
        .collect::<BTreeSet<_>>()
        .len()
}

fn root(parents: &mut [usize], index: usize) -> usize {
    let mut current = index;
    while parents[current] != current {
        parents[current] = parents[parents[current]];
        current = parents[current];
    }
    current
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
    let collection = resolve_scope(graph, scope)?;
    if features.len() > MAX_ASSESSED_RECORDS {
        return Err(signal_error(
            "at most 64 records per coordination assessment",
        ));
    }
    let mut ordered: Vec<&CampaignSignalFeatures> = features.iter().collect();
    ordered.sort_by(|left, right| left.evidence_id.as_str().cmp(right.evidence_id.as_str()));
    let mut seen = BTreeSet::new();
    let mut assessed = Vec::new();
    for feature in ordered {
        if !seen.insert(feature.evidence_id.as_str()) {
            return Err(signal_error("duplicate evidence ID"));
        }
        if feature.prompt_artifacts.len() > MAX_METADATA_ITEMS
            || feature.infrastructure.len() > MAX_METADATA_ITEMS
        {
            return Err(signal_error("coordination feature size limit exceeded"));
        }
        for text in std::iter::once(&feature.attribution)
            .chain(feature.prompt_artifacts.iter())
            .chain(feature.infrastructure.iter())
            .chain(feature.generation_style_fingerprint.iter())
        {
            if text.trim().is_empty()
                || text.len() > MAX_METADATA_LENGTH
                || text.chars().any(char::is_control)
            {
                return Err(signal_error(
                    "metadata needs nonblank bounded attributed identities",
                ));
            }
        }
        let record = graph
            .evidence_by_id(&feature.evidence_id)
            .ok_or_else(|| signal_error("unknown evidence"))?;
        // Scope is the collection, not one claim: a record takes part when any
        // claim of the collection uses it.
        if !graph
            .epistemic_stores()
            .claims
            .claim_links()
            .iter()
            .any(|link| {
                collection.claims.contains(link.target_claim_id().as_str())
                    && link_uses_record(link, record)
            })
        {
            return Err(signal_error(
                "assessed evidence must be linked to a claim of the collection",
            ));
        }
        assessed.push(AssessedRecord {
            feature,
            source: resolve_source(graph, record)?,
            tokens: tokens(record.payload()),
        });
    }

    let mut findings = Vec::new();

    // Shared attributed metadata: one key per artifact, fingerprint or
    // infrastructure identity, joined across the records that reported it.
    let mut keyed: BTreeMap<(CampaignSignal, String), Vec<usize>> = BTreeMap::new();
    for (index, member) in assessed.iter().enumerate() {
        for artifact in &member.feature.prompt_artifacts {
            keyed
                .entry((CampaignSignal::RepeatedPromptArtifact, normalize(artifact)))
                .or_default()
                .push(index);
        }
        if let Some(fingerprint) = &member.feature.generation_style_fingerprint {
            keyed
                .entry((
                    CampaignSignal::GenerationStyleFingerprint,
                    normalize(fingerprint),
                ))
                .or_default()
                .push(index);
        }
        for infrastructure in &member.feature.infrastructure {
            keyed
                .entry((
                    CampaignSignal::SharedInfrastructure,
                    normalize(infrastructure),
                ))
                .or_default()
                .push(index);
        }
    }
    for ((signal, key), indices) in keyed {
        let members: Vec<_> = indices.iter().map(|&index| &assessed[index]).collect();
        let sources = distinct_sources(&members);
        if members.len() >= 2 && sources >= 2 {
            findings.push(build_finding(
                signal,
                scope,
                &members,
                &format!(
                    "shared attributed {} \"{key}\" in {} records from {sources} distinct sources",
                    signal.as_str(),
                    members.len()
                ),
            ));
        }
    }

    // Recycled material: measure every cross-source pair, then report one
    // finding per connected group instead of one per pair.
    let mut parents: Vec<usize> = (0..assessed.len()).collect();
    let mut witnesses = Vec::new();
    for (left, right) in (0..assessed.len())
        .flat_map(|left| ((left + 1)..assessed.len()).map(move |right| (left, right)))
    {
        if assessed[left].source == assessed[right].source
            || assessed[left].tokens.len() < MIN_REDUNDANCY_TOKENS
            || assessed[right].tokens.len() < MIN_REDUNDANCY_TOKENS
        {
            continue;
        }
        let shared = assessed[left]
            .tokens
            .intersection(&assessed[right].tokens)
            .count() as f64;
        let total = assessed[left].tokens.union(&assessed[right].tokens).count() as f64;
        let jaccard = shared / total;
        if jaccard >= CROSS_CONTENT_REDUNDANCY_JACCARD {
            let (a, b) = (root(&mut parents, left), root(&mut parents, right));
            parents[b] = a;
            witnesses.push((left, right, jaccard));
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for index in 0..assessed.len() {
        if witnesses
            .iter()
            .any(|(left, right, _)| *left == index || *right == index)
        {
            groups
                .entry(root(&mut parents, index))
                .or_default()
                .push(index);
        }
    }
    for indices in groups.into_values() {
        let members: Vec<_> = indices.iter().map(|&index| &assessed[index]).collect();
        let sources = distinct_sources(&members);
        if members.len() < 2 || sources < 2 {
            continue;
        }
        let measured: Vec<_> = witnesses
            .iter()
            .filter(|(left, right, _)| indices.contains(left) && indices.contains(right))
            .map(|(left, right, jaccard)| {
                format!(
                    "{}~{}={jaccard:.6}",
                    assessed[*left].feature.evidence_id.as_str(),
                    assessed[*right].feature.evidence_id.as_str()
                )
            })
            .collect();
        findings.push(build_finding(
            CampaignSignal::CrossContentRedundancy,
            scope,
            &members,
            &format!(
                "token Jaccard >= {CROSS_CONTENT_REDUNDANCY_JACCARD} across {sources} distinct sources: {}",
                measured.join(", ")
            ),
        ));
    }

    // Co-membership is recorded as context and excluded from clustering by the
    // signal itself, so curation can never collapse independence.
    let co_members: Vec<_> = assessed
        .iter()
        .filter(|member| collection.content.contains(member.source.as_str()))
        .collect();
    if co_members.len() >= 2 && distinct_sources(&co_members) >= 2 {
        let sources = distinct_sources(&co_members);
        findings.push(build_finding(
            CampaignSignal::NarrativeCoMembership,
            scope,
            &co_members,
            &format!(
                "{sources} declared content sources of {} {}",
                scope.kind(),
                scope.id()
            ),
        ));
    }

    findings.sort_by(|left, right| left.group_id.cmp(&right.group_id));
    Ok(findings)
}

/// Whether a link draws on this record, directly or through its observation.
fn link_uses_record(link: &ClaimLink, record: &EvidenceRecord) -> bool {
    match link.source() {
        ClaimLinkSource::Evidence(id) => id == record.id(),
        ClaimLinkSource::Observation(id) => record.observation_id() == Some(id),
        ClaimLinkSource::Claim(_) => false,
    }
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
        // Validate temporal input even when built through permissive deserialization.
        let validated =
            BitemporalStamp::new(stamp.valid_from.clone(), stamp.transaction_time.clone())?;
        if stamp.valid_to.is_some()
            || stamp.observation_time.is_some()
            || stamp.publication_time.is_some()
        {
            return Err(signal_error(
                "coordination signals require an open-ended knowledge stamp",
            ));
        }
        let findings = detect_campaign_signals(self, scope, features)?;
        if findings.is_empty() {
            return Ok(findings);
        }
        let mut evidence = self.evidence_store().clone();
        for finding in &findings {
            evidence.retain_campaign_signal(CampaignSignalAnnotation {
                finding: finding.clone(),
                stamp: validated.clone(),
            });
        }
        evidence.validate_risk_references()?;
        self.replace_evidence_store(evidence);
        Ok(findings)
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
        resolve_scope(self, &request.scope)?;
        if request.cited_signals.is_empty() && request.corroborating_claims.is_empty() {
            return Ok(AttributionAdmissibility::Refused(
                AttributionRefusal::NoSupportCited,
            ));
        }
        for cited in &request.cited_signals {
            let retained = self
                .evidence_store()
                .campaign_signal_by_group(cited)
                .is_some_and(|annotation| annotation.finding.scope() == &request.scope);
            if !retained {
                return Ok(AttributionAdmissibility::Refused(
                    AttributionRefusal::UnknownSignal(cited.clone()),
                ));
            }
        }
        // A shared production pattern is not authorship. Coordination evidence
        // can accompany an attribution; it can never carry one.
        if request.corroborating_claims.is_empty() {
            return Ok(AttributionAdmissibility::Refused(
                AttributionRefusal::CoordinationSignalsOnly,
            ));
        }
        for claim in &request.corroborating_claims {
            self.epistemic_stores().claims.claim_by_id(claim)?;
            let supported = self
                .epistemic_stores()
                .verdicts
                .verdict_as_of(claim, as_of)
                .is_some_and(|verdict| verdict.state() == VerdictState::Supported);
            if !supported {
                return Ok(AttributionAdmissibility::Refused(
                    AttributionRefusal::CorroborationNotSupported(claim.clone()),
                ));
            }
        }
        Ok(AttributionAdmissibility::Admissible {
            corroborating: request.corroborating_claims.clone(),
        })
    }
}

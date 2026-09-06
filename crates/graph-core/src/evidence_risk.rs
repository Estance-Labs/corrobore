//! Explainable evidence-risk detection and non-destructive immune responses.
use crate::*;
use serde::{Deserialize, Serialize};

/// Independent heuristic diagnostics, never a finding that a claim is false.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum EvidenceRiskSignal {
    /// Normalized lexical repetition.
    LexicalDuplication,
    /// High cosine similarity in the same embedding space.
    SemanticDuplication,
    /// Shared attributed infrastructure identifier.
    SharedInfrastructure,
    /// Shared upstream citation identifier.
    SharedUpstreamCitation,
    /// At least three sources published in a short window.
    TemporalBurst,
    /// Matching attributed generation watermark or fingerprint.
    GenerationFingerprint,
    /// Extreme embedding norm relative to the same model's peer set.
    EmbeddingGeometryAnomaly,
}
impl EvidenceRiskSignal {
    /// All seven independently testable signals.
    pub const ALL: [Self; 7] = [
        Self::LexicalDuplication,
        Self::SemanticDuplication,
        Self::SharedInfrastructure,
        Self::SharedUpstreamCitation,
        Self::TemporalBurst,
        Self::GenerationFingerprint,
        Self::EmbeddingGeometryAnomaly,
    ];
}
/// Attributed metadata supplied by the extraction/retrieval host for a stored record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EvidenceRiskFeatures {
    /// Existing immutable evidence record.
    pub evidence_id: EvidenceId,
    /// Instrumentation or assessment provenance, not an attestation.
    pub attribution: String,
    /// Semantic embedding, absent when not measured.
    pub embedding: Option<Vec<f64>>,
    /// Qualified model/version naming the embedding space.
    pub embedding_model: Option<String>,
    /// Precise infrastructure identities; broad hosting categories are unsuitable.
    pub infrastructure: Vec<String>,
    /// Canonical citation identities.
    pub upstream_citations: Vec<String>,
    /// UTC publication timestamp in seconds, absent when unknown.
    pub publication_seconds: Option<i64>,
    /// Attributed watermark/fingerprint; a generic model name is not a fingerprint.
    pub generation_fingerprint: Option<String>,
}
impl EvidenceRiskFeatures {
    /// Start with unknown optional metadata; lexical content comes from the graph.
    pub fn new(evidence_id: EvidenceId, attribution: impl Into<String>) -> Self {
        Self {
            evidence_id,
            attribution: attribution.into(),
            embedding: None,
            embedding_model: None,
            infrastructure: vec![],
            upstream_citations: vec![],
            publication_seconds: None,
            generation_fingerprint: None,
        }
    }
}
/// One pair/window measurement supporting a connected risk group.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRiskWitness {
    /// Exact records examined together, without assuming all group pairs match.
    pub evidence_ids: Vec<EvidenceId>,
    /// Original measurement, threshold and attribution.
    pub reason: String,
}
/// A retained signal with exact affected records and an inspectable reason.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRiskFinding {
    /// Stable diagnostic kind.
    pub signal: EvidenceRiskSignal,
    /// Stable content-derived identity for grouping and idempotency.
    pub group_id: String,
    /// Exact evidence records implicated by this diagnostic.
    pub evidence_ids: Vec<EvidenceId>,
    /// Measurement, threshold and attribution explaining the diagnostic.
    pub reason: String,
    /// Original pair/window measurements when several detections were coalesced.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub witnesses: Vec<EvidenceRiskWitness>,
}
/// One shared, content-addressed audit receipt referenced by evidence records.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredEvidenceRiskAssessment {
    /// Stable identity of the complete assessment and quarantine proof.
    pub id: String,
    /// Original signal and audit, stored once for the whole component.
    pub assessment: EvidenceRiskAnnotation,
}
/// Append-only risk assessment stored once and referenced by the original evidence records.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRiskAnnotation {
    /// Original diagnostic.
    pub finding: EvidenceRiskFinding,
    /// When this assessment became known; historical resolutions exclude future knowledge.
    pub stamp: BitemporalStamp,
    /// Audited quarantine responses captured from the existing immune system.
    pub quarantine_responses: Vec<ImmuneResponse>,
    /// Full dependency component quarantined, which may exceed the detector pair.
    pub quarantined_evidence_ids: Vec<EvidenceId>,
    /// Tier transitions retain actor, reason and sequence alongside the response.
    pub quarantine_transitions: Vec<TierTransition>,
}
/// Detect risk in up to 64 distinct evidence records linked to one claim.
///
/// # Errors
/// Reject missing records, unbound claim evidence and malformed metadata before detection.
pub fn detect_evidence_risks(
    graph: &Graph,
    claim: &ClaimId,
    features: &[EvidenceRiskFeatures],
) -> Result<Vec<EvidenceRiskFinding>, GraphError> {
    use std::collections::{BTreeMap, BTreeSet};
    graph.epistemic_stores().claims.claim_by_id(claim)?;
    if features.len() > 64 {
        return Err(risk_error("at most 64 records per claim assessment"));
    }
    let mut sorted: Vec<_> = features.iter().collect();
    sorted.sort_by_key(|f| f.evidence_id.as_str());
    let mut ids = BTreeSet::new();
    let mut dimensions = BTreeMap::new();
    for f in &sorted {
        if !ids.insert(f.evidence_id.clone()) {
            return Err(risk_error("duplicate evidence ID"));
        }
        let record = graph
            .evidence_by_id(&f.evidence_id)
            .ok_or_else(|| risk_error("unknown evidence"))?;
        if !graph
            .epistemic_stores()
            .claims
            .claim_links()
            .iter()
            .any(|l| l.target_claim_id() == claim && link_matches(l, record))
        {
            return Err(risk_error(
                "evidence must be directly linked to the assessed claim",
            ));
        }
        if record.payload().len() > 1_000_000
            || f.infrastructure.len() > 64
            || f.upstream_citations.len() > 64
        {
            return Err(risk_error("risk feature size limit exceeded"));
        }
        for text in std::iter::once(&f.attribution)
            .chain(f.infrastructure.iter())
            .chain(f.upstream_citations.iter())
            .chain(f.generation_fingerprint.iter())
            .chain(f.embedding_model.iter())
        {
            if text.trim().is_empty() || text.len() > 1024 || text.chars().any(char::is_control) {
                return Err(risk_error(
                    "metadata needs nonblank bounded attributed identities",
                ));
            }
        }
        if let Some(seconds) = f.publication_seconds
            && !(0..=253_402_300_799).contains(&seconds)
        {
            return Err(risk_error(
                "publication timestamp outside supported UTC range",
            ));
        }
        if let Some(v) = &f.embedding {
            let model = f
                .embedding_model
                .as_ref()
                .ok_or_else(|| risk_error("embedding requires a qualified model version"))?;
            if v.is_empty()
                || v.len() > 4096
                || v.iter().any(|x| !x.is_finite() || x.abs() > 1_000_000.0)
                || norm(v) == 0.0
            {
                return Err(risk_error("invalid embedding geometry"));
            }
            if let Some(previous) = dimensions.insert(model, v.len())
                && previous != v.len()
            {
                return Err(risk_error("inconsistent dimensions in one embedding space"));
            }
        }
    }
    let mut findings = Vec::new();
    let mut emit = |signal, members: Vec<&EvidenceRiskFeatures>, reason: String| {
        use sha2::{Digest, Sha256};
        let mut members = members;
        members.sort_by_key(|f| f.evidence_id.as_str());
        let evidence_ids: Vec<_> = members.iter().map(|f| f.evidence_id.clone()).collect();
        let attributions: Vec<_> = members
            .iter()
            .map(|f| (&f.evidence_id, &f.attribution))
            .collect();
        let reason = format!(
            "ws-e-risk-v1: {reason}; attribution={}",
            serde_json::to_string(&attributions).expect("strings")
        );
        let bytes = serde_json::to_vec(&(signal, &evidence_ids, &reason)).expect("risk identity");
        let group_id = format!(
            "risk--{}",
            Sha256::digest(bytes)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        findings.push(EvidenceRiskFinding {
            signal,
            group_id,
            evidence_ids,
            reason,
            witnesses: Vec::new(),
        });
    };
    let tokens: Vec<BTreeSet<String>> = sorted
        .iter()
        .map(|f| {
            graph
                .evidence_by_id(&f.evidence_id)
                .expect("validated record")
                .payload()
                .split(|c: char| !c.is_alphanumeric())
                .filter(|v| !v.is_empty())
                .map(str::to_lowercase)
                .collect()
        })
        .collect();
    for (i, left) in sorted.iter().enumerate() {
        for (j, right) in sorted.iter().enumerate().skip(i + 1) {
            let pair = || vec![*left, *right];
            if tokens[i].len() >= 4 && tokens[j].len() >= 4 {
                let similarity = tokens[i].intersection(&tokens[j]).count() as f64
                    / tokens[i].union(&tokens[j]).count() as f64;
                if similarity >= 0.9 {
                    emit(
                        EvidenceRiskSignal::LexicalDuplication,
                        pair(),
                        format!("token Jaccard={similarity:.6} >= 0.9"),
                    );
                }
            }
            if let (Some(a), Some(b)) = (&left.embedding, &right.embedding)
                && left.embedding_model == right.embedding_model
            {
                let cosine = a.iter().zip(b).map(|(a, b)| a * b).sum::<f64>() / (norm(a) * norm(b));
                if cosine >= 0.98 {
                    emit(
                        EvidenceRiskSignal::SemanticDuplication,
                        pair(),
                        format!(
                            "cosine={cosine:.6} >= 0.98; model={}",
                            left.embedding_model.as_deref().expect("validated model")
                        ),
                    );
                }
            }
            for (signal, a, b) in [
                (
                    EvidenceRiskSignal::SharedInfrastructure,
                    &left.infrastructure,
                    &right.infrastructure,
                ),
                (
                    EvidenceRiskSignal::SharedUpstreamCitation,
                    &left.upstream_citations,
                    &right.upstream_citations,
                ),
            ] {
                let common: BTreeSet<_> = a.iter().filter(|key| b.contains(key)).collect();
                if !common.is_empty() {
                    emit(signal, pair(), format!("shared identifiers={common:?}"));
                }
            }
            if let Some(a) = &left.generation_fingerprint
                && right.generation_fingerprint.as_ref() == Some(a)
            {
                emit(
                    EvidenceRiskSignal::GenerationFingerprint,
                    pair(),
                    format!("matching attributed fingerprint={a}"),
                );
            }
        }
    }
    let mut timed: Vec<_> = sorted
        .iter()
        .copied()
        .filter(|f| f.publication_seconds.is_some())
        .collect();
    timed.sort_by_key(|f| (f.publication_seconds, f.evidence_id.as_str()));
    let mut emitted_windows = BTreeSet::new();
    for first in &timed {
        let start = first.publication_seconds.expect("timed");
        let members: Vec<_> = timed
            .iter()
            .copied()
            .filter(|f| (start..=start + 60).contains(&f.publication_seconds.expect("timed")))
            .collect();
        let sources: BTreeSet<_> = members
            .iter()
            .map(|f| {
                let record = graph
                    .evidence_by_id(&f.evidence_id)
                    .expect("validated record");
                record
                    .source_id()
                    .map(|id| id.as_str().to_owned())
                    .or_else(|| {
                        record
                            .observation_id()
                            .and_then(|id| {
                                graph.epistemic_stores().observations.observation_by_id(id)
                            })
                            .map(|o| o.source_id().as_str().to_owned())
                    })
                    .unwrap_or_else(|| record.source_ref().to_owned())
            })
            .collect();
        let keys: Vec<_> = members.iter().map(|f| f.evidence_id.clone()).collect();
        if sources.len() >= 3 && emitted_windows.insert(keys) {
            emit(
                EvidenceRiskSignal::TemporalBurst,
                members,
                format!("at least 3 distinct source references in 60 seconds starting at {start}"),
            );
        }
    }
    for model in dimensions.keys() {
        let peers: Vec<_> = sorted
            .iter()
            .copied()
            .filter(|f| f.embedding_model.as_ref() == Some(*model) && f.embedding.is_some())
            .collect();
        if peers.len() < 3 {
            continue;
        }
        let mut norms: Vec<_> = peers
            .iter()
            .map(|f| norm(f.embedding.as_ref().expect("embedding")))
            .collect();
        norms.sort_by(f64::total_cmp);
        let median = if norms.len() % 2 == 0 {
            (norms[norms.len() / 2 - 1] + norms[norms.len() / 2]) / 2.0
        } else {
            norms[norms.len() / 2]
        };
        for f in peers {
            let ratio = norm(f.embedding.as_ref().expect("embedding")) / median;
            if !(0.05..=20.0).contains(&ratio) {
                emit(
                    EvidenceRiskSignal::EmbeddingGeometryAnomaly,
                    vec![f],
                    format!("norm/peer-median={ratio:.6} outside [0.05,20]; model={model}"),
                );
            }
        }
    }
    findings.sort_by(|a, b| a.group_id.cmp(&b.group_id));
    findings.dedup();
    Ok(coalesce_findings(findings))
}
// Pairwise detectors produce witnesses, not a separate persisted annotation
// for every pair in a dense cluster. Keep the exact witnesses while storing
// one diagnostic per connected component and signal.
fn coalesce_findings(findings: Vec<EvidenceRiskFinding>) -> Vec<EvidenceRiskFinding> {
    use sha2::{Digest, Sha256};
    use std::collections::BTreeSet;
    let mut result = Vec::new();
    for signal in EvidenceRiskSignal::ALL {
        let mut pending: Vec<_> = findings
            .iter()
            .filter(|f| f.signal == signal)
            .cloned()
            .collect();
        while let Some(seed) = pending.pop() {
            let mut ids: BTreeSet<_> = seed.evidence_ids.iter().cloned().collect();
            let mut group = vec![seed];
            loop {
                let before = pending.len();
                for index in (0..pending.len()).rev() {
                    if pending[index]
                        .evidence_ids
                        .iter()
                        .any(|id| ids.contains(id))
                    {
                        let finding = pending.remove(index);
                        ids.extend(finding.evidence_ids.iter().cloned());
                        group.push(finding);
                    }
                }
                if pending.len() == before {
                    break;
                }
            }
            group.sort_by(|a, b| a.group_id.cmp(&b.group_id));
            if group.len() == 1 {
                result.push(group.pop().expect("nonempty component"));
                continue;
            }
            let witnesses: Vec<_> = group
                .into_iter()
                .map(|f| EvidenceRiskWitness {
                    evidence_ids: f.evidence_ids,
                    reason: f.reason,
                })
                .collect();
            let evidence_ids: Vec<_> = ids.into_iter().collect();
            let reason = format!(
                "ws-e-risk-v1: {} measured pair/window witnesses form one connected risk group",
                witnesses.len()
            );
            let bytes = serde_json::to_vec(&(signal, &evidence_ids, &reason, &witnesses))
                .expect("risk group identity");
            let group_id = format!(
                "risk--{}",
                Sha256::digest(bytes)
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()
            );
            result.push(EvidenceRiskFinding {
                signal,
                group_id,
                evidence_ids,
                reason,
                witnesses,
            });
        }
    }
    result.sort_by(|a, b| a.group_id.cmp(&b.group_id));
    result
}
fn risk_error(message: &str) -> GraphError {
    GraphError::InvalidPropertyValue(message.into())
}
fn norm(v: &[f64]) -> f64 {
    v.iter().map(|x| x * x).sum::<f64>().sqrt()
}
fn link_matches(link: &ClaimLink, record: &EvidenceRecord) -> bool {
    match link.source() {
        ClaimLinkSource::Evidence(id) => id == record.id(),
        ClaimLinkSource::Observation(id) => record.observation_id() == Some(id),
        ClaimLinkSource::Claim(_) => false,
    }
}

impl Graph {
    /// Detect, quarantine and retain risk findings without removing evidence.
    ///
    /// # Errors
    /// Validate and stage all mutations before committing graph and immune state.
    pub fn apply_evidence_risks(
        &mut self,
        claim: &ClaimId,
        features: &[EvidenceRiskFeatures],
        stamp: BitemporalStamp,
        actor: &str,
        tiers: &mut GraphTierRegistry,
        immune: &mut ImmuneResponder,
    ) -> Result<Vec<EvidenceRiskFinding>, GraphError> {
        if actor.trim().is_empty() {
            return Err(risk_error("actor must not be blank"));
        }
        // Validate temporal input even if constructed through permissive deserialization.
        let validated =
            BitemporalStamp::new(stamp.valid_from.clone(), stamp.transaction_time.clone())?;
        if stamp.valid_to.is_some()
            || stamp.observation_time.is_some()
            || stamp.publication_time.is_some()
        {
            return Err(risk_error(
                "risk assessments require an open-ended knowledge stamp",
            ));
        }
        let at = VerdictAsOf::new(
            validated.valid_from.clone(),
            validated.transaction_time.clone(),
        );
        let findings = detect_evidence_risks(self, claim, features)?;
        for feature in features {
            let record = self
                .evidence_by_id(&feature.evidence_id)
                .expect("validated evidence");
            if !self
                .epistemic_stores()
                .claims
                .claim_links()
                .iter()
                .any(|l| {
                    l.target_claim_id() == claim && l.is_active_at(&at) && link_matches(l, record)
                })
            {
                return Err(risk_error("assessment requires an active evidence link"));
            }
        }
        if findings.is_empty() {
            return Ok(findings);
        }
        // Stage metadata separately from immutable evidence content. Risk follows
        // the evidence into every claim using it, not just this assessment's claim.
        let mut next_claims = self.epistemic_stores().claims.clone();
        let mut next_evidence = self.evidence_store().clone();
        let mut next_tiers = tiers.clone();
        let mut next_immune = immune.clone();
        for finding in &findings {
            let receipt = next_evidence.retain_risk_assessment(EvidenceRiskAnnotation {
                finding: finding.clone(),
                stamp: validated.clone(),
                quarantine_responses: vec![],
                quarantined_evidence_ids: finding.evidence_ids.clone(),
                quarantine_transitions: vec![],
            });
            for id in &finding.evidence_ids {
                next_evidence.attach_risk_reference(id, &receipt)?;
            }
        }
        let structure = next_claims.assign_independence_clusters(
            claim,
            &at,
            &next_evidence,
            &self.epistemic_stores().observations,
            &self.epistemic_stores().sources,
        )?;
        // Provisional receipts exist only to derive the full dependency closure.
        // Publish complete audited receipts, never their provisional copies.
        next_evidence = self.evidence_store().clone();
        for finding in &findings {
            let mut affected = std::collections::BTreeSet::new();
            for cluster in structure.clusters() {
                let links: Vec<_> = cluster
                    .members()
                    .iter()
                    .map(|&i| next_claims.claim_link_at_index(i).expect("derived member"))
                    .collect();
                if links.iter().any(|l| {
                    finding.evidence_ids.iter().any(|id| {
                        link_matches(l, self.evidence_by_id(id).expect("validated evidence"))
                    })
                }) {
                    for record in self.evidence_store().records() {
                        if links.iter().any(|l| link_matches(l, record)) {
                            affected.insert(record.id().clone());
                        }
                    }
                }
            }
            for id in &affected {
                let target = TierRecordRef::Evidence(id.clone());
                if next_tiers.tier_of(&target) != GraphTier::Quarantine {
                    let record = ValidationErrorRecord::new(
                        format!("evidence.risk.{:?}", finding.signal),
                        ValidationErrorSeverity::Warning,
                        &finding.reason,
                        ValidationTarget::Evidence(id.as_str().into()),
                    );
                    next_immune.quarantine(&mut next_tiers, &record, actor)?;
                }
            }
            let responses = next_immune.audit().iter().filter(|r| matches!(&r.action, ImmuneResponseAction::Quarantine { record: TierRecordRef::Evidence(id) } if affected.contains(id))).cloned().collect();
            let mut transitions: Vec<TierTransition> = affected
                .iter()
                .flat_map(|id| next_tiers.audit_for(&TierRecordRef::Evidence(id.clone())))
                .cloned()
                .collect();
            transitions.sort_by_key(|transition| transition.sequence);
            let annotation = EvidenceRiskAnnotation {
                finding: finding.clone(),
                stamp: validated.clone(),
                quarantine_responses: responses,
                quarantined_evidence_ids: affected.iter().cloned().collect(),
                quarantine_transitions: transitions,
            };
            let receipt = next_evidence.retain_risk_assessment(annotation);
            for id in affected {
                next_evidence.attach_risk_reference(&id, &receipt)?;
            }
        }
        self.epistemic_stores_mut().claims = next_claims;
        self.replace_evidence_store(next_evidence);
        *tiers = next_tiers;
        *immune = next_immune;
        Ok(findings)
    }
}

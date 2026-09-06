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
//! Explainable dependency components for evidence links. No authority or weighting policy.
use crate::*;
use serde::{Deserialize, Serialize};

/// Explicit artifact similarity assessment; cryptographic hash proximity is not similarity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NearDuplicateArtifact {
    /// Digest of the related artifact.
    pub sha256: String,
    /// Attribution or review reference explaining the near-duplicate assessment.
    pub reason: String,
}
/// Additional provenance dependencies retained with a source version.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceDependencySignals {
    /// Canonical upstream citation references.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub upstream_citations: Vec<String>,
    /// Shared extraction run identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extraction_run: Option<String>,
    /// Qualified extractor/model pipeline identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_pipeline: Option<String>,
    /// Explicit similarity assessments with their provenance.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub near_duplicate_artifacts: Vec<NearDuplicateArtifact>,
}
/// Why links belong to one dependency component.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum DependencySignal {
    /// Attributed evidence-risk assessment joining a suspected dependency group.
    EvidenceRisk,
    /// Same source identity.
    SharedSource,
    /// Shared publisher identity.
    SharedPublisher,
    /// Shared source ancestry.
    Syndication,
    /// Shared upstream citation.
    SharedUpstreamCitation,
    /// Same artifact digest.
    IdenticalArtifact,
    /// Explicitly assessed artifact similarity.
    NearDuplicateArtifact,
    /// Same extraction run.
    SharedExtractionRun,
    /// Same qualified model pipeline.
    SharedModelPipeline,
    /// Singleton fallback; independence is unknown, not established.
    UnknownIndependence,
}
/// An auditable connection between link positions in the append-only claim store.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyReason {
    signal: DependencySignal,
    value: String,
    left_link: usize,
    right_link: usize,
}
/// One connected component of dependent active links.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndependenceCluster {
    id: String,
    members: Vec<usize>,
    supporting: bool,
    reasons: Vec<DependencyReason>,
}
/// Source independence is a structure, not a probability or a record count.
/// Separate components mean no recorded dependency, not proven independence.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceIndependence {
    clusters: Vec<IndependenceCluster>,
}

impl SourceDependencySignals {
    /// Whether no additional provenance has been recorded.
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
    pub(crate) fn validate(&self) -> Result<(), GraphError> {
        if self
            .upstream_citations
            .iter()
            .chain(self.extraction_run.iter())
            .chain(self.model_pipeline.iter())
            .any(|s| s.trim().is_empty())
            || self.near_duplicate_artifacts.iter().any(|v| {
                v.reason.trim().is_empty()
                    || v.sha256.len() != 64
                    || !v
                        .sha256
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })
        {
            return Err(GraphError::InvalidPropertyValue("source dependency signals require nonblank identities and attributed lowercase SHA-256 similarity assessments".into()));
        }
        Ok(())
    }
}
impl DependencyReason {
    /// Kind of dependency or explicit singleton fallback.
    pub fn signal(&self) -> DependencySignal {
        self.signal
    }
    /// Shared key and, for similarity, the recorded attribution.
    pub fn value(&self) -> &str {
        &self.value
    }
    /// Stable ledger indices of the connected links.
    pub fn links(&self) -> (usize, usize) {
        (self.left_link, self.right_link)
    }
}
impl IndependenceCluster {
    /// Deterministic identifier derived from the component's link identities.
    pub fn id(&self) -> &str {
        &self.id
    }
    /// Member positions in the append-only claim store.
    pub fn members(&self) -> &[usize] {
        &self.members
    }
    /// Recorded connections explaining this component.
    pub fn reasons(&self) -> &[DependencyReason] {
        &self.reasons
    }
}
impl SourceIndependence {
    /// All dependency components, including refuting and contextual evidence.
    pub fn clusters(&self) -> &[IndependenceCluster] {
        &self.clusters
    }
    /// Components containing at least one active supporting link, never record count.
    /// Separate components do not establish real-world independence.
    pub fn supporting_cluster_count(&self) -> usize {
        self.clusters.iter().filter(|c| c.supporting).count()
    }
    /// Explicit unknown-independence singleton components.
    pub fn unknown_cluster_count(&self) -> usize {
        self.clusters
            .iter()
            .filter(|c| {
                c.reasons
                    .iter()
                    .any(|r| r.signal == DependencySignal::UnknownIndependence)
            })
            .count()
    }
    /// Membership lookup by stable claim-link ledger index.
    pub fn cluster_for_link(&self, index: usize) -> Option<&IndependenceCluster> {
        self.clusters.iter().find(|c| c.members.contains(&index))
    }
}

use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
type Keys = BTreeMap<(DependencySignal, String), (DependencySignal, String)>;
fn key(keys: &mut Keys, signal: DependencySignal, value: impl Into<String>) {
    let value = value.into();
    keys.entry((signal, value.clone()))
        .or_insert((signal, value));
}
fn evidence_keys(
    keys: &mut Keys,
    record: &EvidenceRecord,
    as_of: &VerdictAsOf,
    evidence: &EvidenceRecordStore,
) {
    for risk in evidence
        .risk_assessments_for(record.id())
        .iter()
        .filter(|r| as_of.covers(&r.stamp))
    {
        keys.insert(
            (
                DependencySignal::EvidenceRisk,
                risk.finding.group_id.clone(),
            ),
            (
                DependencySignal::EvidenceRisk,
                format!(
                    "{:?}: {}: {}",
                    risk.finding.signal, risk.finding.group_id, risk.finding.reason
                ),
            ),
        );
    }

    if let Some(run) = record.extraction_run_id() {
        key(keys, DependencySignal::SharedExtractionRun, run.as_str());
    }
    if let (Some(extractor), Some(model)) = (record.extractor_id(), record.model_version()) {
        key(
            keys,
            DependencySignal::SharedModelPipeline,
            serde_json::to_string(&(extractor, model)).expect("string tuple"),
        );
    }
    if let Some(digest) = record.content_sha256() {
        key(keys, DependencySignal::IdenticalArtifact, digest);
    }
}
fn source_keys(keys: &mut Keys, id: &SourceId, sources: &SourceStore) {
    let mut current = Some(id.clone());
    let mut seen = BTreeSet::new();
    let mut ancestor = false;
    while let Some(id) = current {
        if !seen.insert(id.as_str().to_owned()) {
            break;
        }
        key(keys, DependencySignal::SharedSource, id.as_str());
        if ancestor {
            keys.insert(
                (DependencySignal::SharedSource, id.as_str().to_owned()),
                (DependencySignal::Syndication, id.as_str().to_owned()),
            );
        }
        let Some(source) = sources.current_source(&id) else {
            break;
        };
        if let Some(publisher) = source.publisher() {
            key(keys, DependencySignal::SharedPublisher, publisher);
        }
        if let Some(digest) = source.artifact_sha256() {
            key(keys, DependencySignal::IdenticalArtifact, digest);
        }
        let signals = source.dependency_signals();
        for citation in &signals.upstream_citations {
            key(keys, DependencySignal::SharedUpstreamCitation, citation);
        }
        if let Some(run) = &signals.extraction_run {
            key(keys, DependencySignal::SharedExtractionRun, run);
        }
        if let Some(pipeline) = &signals.model_pipeline {
            key(keys, DependencySignal::SharedModelPipeline, pipeline);
        }
        for near in &signals.near_duplicate_artifacts {
            keys.entry((DependencySignal::IdenticalArtifact, near.sha256.clone()))
                .or_insert((
                    DependencySignal::NearDuplicateArtifact,
                    format!("{}: {}", near.sha256, near.reason),
                ));
        }
        current = source.parent_source().cloned();
        ancestor = true;
    }
}
fn profile(
    link: &ClaimLink,
    evidence: &EvidenceRecordStore,
    observations: &ObservationStore,
    sources: &SourceStore,
    as_of: &VerdictAsOf,
) -> Keys {
    let mut keys = Keys::new();
    match link.source() {
        ClaimLinkSource::Observation(id) => {
            if let Some(observation) = observations.observation_by_id(id) {
                source_keys(&mut keys, observation.source_id(), sources);
            }
            for record in evidence
                .records()
                .iter()
                .filter(|r| r.observation_id() == Some(id))
            {
                evidence_keys(&mut keys, record, as_of, evidence);
            }
        }
        ClaimLinkSource::Evidence(id) => {
            if let Some(record) = evidence.evidence_by_id(id) {
                let id = record
                    .source_id()
                    .cloned()
                    .or_else(|| {
                        record
                            .observation_id()
                            .and_then(|id| observations.observation_by_id(id))
                            .map(|o| o.source_id().clone())
                    })
                    .or_else(|| SourceId::new(record.source_ref()).ok());
                if let Some(id) = id {
                    source_keys(&mut keys, &id, sources);
                }
                evidence_keys(&mut keys, record, as_of, evidence);
            }
        }
        ClaimLinkSource::Claim(_) => {}
    }
    keys
}
fn root(parents: &mut [usize], index: usize) -> usize {
    let mut current = index;
    while parents[current] != current {
        parents[current] = parents[parents[current]];
        current = parents[current];
    }
    current
}
impl ClaimStore {
    /// Assign clusters to active links and return an explainable snapshot.
    /// Unknown independence is not established independence: a link with no
    /// provenance keys gets a private cluster of one and an explicit reason.
    ///
    /// # Errors
    /// Returns `ClaimNotFound` for an unknown claim.
    pub fn assign_independence_clusters(
        &mut self,
        claim: &ClaimId,
        as_of: &VerdictAsOf,
        evidence: &EvidenceRecordStore,
        observations: &ObservationStore,
        sources: &SourceStore,
    ) -> Result<SourceIndependence, GraphError> {
        self.claim_by_id(claim)?;
        self.validate_link_indices()?;
        evidence.validate_risk_references()?;
        let active: Vec<_> = self
            .claim_links
            .iter()
            .enumerate()
            .filter(|(_, link)| link.target_claim_id() == claim && link.is_active_at(as_of))
            .map(|(i, _)| i)
            .collect();
        let mut parents: Vec<_> = (0..self.claim_links.len()).collect();
        let mut owners =
            BTreeMap::<(DependencySignal, String), (usize, DependencySignal, String)>::new();
        let mut reasons = Vec::new();
        for &index in &active {
            let keys = profile(
                &self.claim_links[index],
                evidence,
                observations,
                sources,
                as_of,
            );
            if keys.is_empty() {
                reasons.push(DependencyReason {
                    signal: DependencySignal::UnknownIndependence,
                    value: "no assignable dependency; independence is unknown, not established"
                        .into(),
                    left_link: index,
                    right_link: index,
                });
            }
            for (token, (signal, value)) in keys {
                if let Some((other, other_signal, other_value)) = owners.get(&token) {
                    let a = root(&mut parents, *other);
                    let b = root(&mut parents, index);
                    parents[b] = a;
                    let signal = if signal == DependencySignal::NearDuplicateArtifact
                        || *other_signal == DependencySignal::NearDuplicateArtifact
                    {
                        DependencySignal::NearDuplicateArtifact
                    } else if signal == DependencySignal::Syndication
                        || *other_signal == DependencySignal::Syndication
                    {
                        DependencySignal::Syndication
                    } else {
                        signal
                    };
                    reasons.push(DependencyReason {
                        signal,
                        value: if value == *other_value {
                            value
                        } else {
                            format!("{other_value}; {value}")
                        },
                        left_link: *other,
                        right_link: index,
                    });
                } else {
                    if signal == DependencySignal::EvidenceRisk {
                        reasons.push(DependencyReason {
                            signal,
                            value: value.clone(),
                            left_link: index,
                            right_link: index,
                        });
                    }
                    owners.insert(token, (index, signal, value));
                }
            }
        }
        let mut components = BTreeMap::<usize, Vec<usize>>::new();
        for index in active {
            components
                .entry(root(&mut parents, index))
                .or_default()
                .push(index);
        }
        let mut clusters = Vec::new();
        for members in components.into_values() {
            let component_reasons: Vec<_> = reasons
                .iter()
                .filter(|r| members.contains(&r.left_link))
                .cloned()
                .collect();
            let unknown = component_reasons
                .iter()
                .any(|r| r.signal == DependencySignal::UnknownIndependence);
            let mut identities: Vec<_> = members
                .iter()
                .map(|&i| {
                    let l = &self.claim_links[i];
                    serde_json::to_string(&(
                        l.source(),
                        l.target_claim_id(),
                        l.kind(),
                        &l.bitemporal,
                        unknown.then_some(self.claim_link_index(i)),
                    ))
                    .expect("link identity")
                })
                .collect();
            identities.sort();
            identities.dedup();
            let bytes = serde_json::to_vec(&identities).expect("link identities");
            let id = format!(
                "independence--{}",
                Sha256::digest(bytes)
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()
            );
            let supporting = members
                .iter()
                .any(|&i| self.claim_links[i].kind() == ClaimLinkKind::Supports);
            for &i in &members {
                self.claim_links[i].independence_cluster = Some(id.clone());
            }
            clusters.push(IndependenceCluster {
                id,
                members: members
                    .iter()
                    .map(|&index| self.claim_link_index(index))
                    .collect(),
                supporting,
                reasons: component_reasons
                    .into_iter()
                    .map(|mut reason| {
                        reason.left_link = self.claim_link_index(reason.left_link);
                        reason.right_link = self.claim_link_index(reason.right_link);
                        reason
                    })
                    .collect(),
            });
        }
        clusters.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(SourceIndependence { clusters })
    }
}

impl IndependenceCluster {
    /// Whether the retained dependency reasons include an evidence-risk finding.
    pub fn has_evidence_risk(&self) -> bool {
        self.reasons
            .iter()
            .any(|reason| reason.signal == DependencySignal::EvidenceRisk)
    }
}

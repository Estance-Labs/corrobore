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
//! Versioned, scoped audit archives and native memory snapshot transport.
use crate::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
fn invalid(error: impl std::fmt::Display) -> GraphError {
    GraphError::InvalidPropertyValue(error.to_string())
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AuditArchive {
    schema: String,
    claim_ids: Vec<ClaimId>,
    audits: BTreeMap<String, Value>,
    snapshot: GraphPersistenceSnapshot,
}
fn records<T: serde::de::DeserializeOwned>(audit: &Value, key: &str) -> Result<Vec<T>, GraphError> {
    serde_json::from_value(audit[key].clone()).map_err(invalid)
}
impl Graph {
    /// Export selected claims and retained dependencies, excluding unrelated claims.
    /// The snapshot is authoritative; included audit views are checked on import.
    pub fn export_claim_audit_archive(&self, roots: &[ClaimId]) -> Result<Value, GraphError> {
        let mut roots = roots.to_vec();
        roots.sort();
        roots.dedup();
        let stores = self.epistemic_stores();
        let mut claims = roots.iter().cloned().collect::<HashSet<_>>();
        let mut observations = HashSet::new();
        let mut sources = HashSet::new();
        let mut evidence = HashSet::new();
        let mut candidates = HashSet::new();
        let mut reconciliations = HashSet::new();
        let mut audits = BTreeMap::new();
        for id in &roots {
            let audit = self.claim_audit_path(id)?;
            claims.extend(
                records::<Claim>(&audit, "related_claims")?
                    .into_iter()
                    .map(|r| r.id().clone()),
            );
            observations.extend(
                records::<Observation>(&audit, "observations")?
                    .into_iter()
                    .map(|r| r.id().clone()),
            );
            sources.extend(
                records::<Source>(&audit, "source_versions")?
                    .into_iter()
                    .map(|r| r.id().clone()),
            );
            evidence.extend(
                records::<EvidenceRecord>(&audit, "evidence")?
                    .into_iter()
                    .map(|r| r.id().clone()),
            );
            candidates.extend(
                records::<CandidateInput>(&audit, "candidates")?
                    .into_iter()
                    .map(|r| r.id().clone()),
            );
            reconciliations.extend(
                records::<ReconciliationRecord>(&audit, "reconciliations")?
                    .into_iter()
                    .map(|r| r.id().clone()),
            );
            audits.insert(id.as_str().to_owned(), audit);
        }
        let merges = stores.merges.audit_subset(&mut reconciliations);
        let reconciliations = stores.reconciliations.audit_subset(&reconciliations);
        // A judgment can cite one feature while still naming two mentions. Keep
        // both original mention bindings and dependencies required by restoration.
        for record in reconciliations.records() {
            for id in [record.left(), record.right()] {
                if let Some(mention) = stores.mentions.mention_by_id(id) {
                    observations.insert(mention.observation_id().clone());
                }
            }
            for citation in record.citations() {
                observations.insert(citation.observation_id.clone());
                sources.insert(citation.source_id.clone());
            }
        }
        for id in &observations {
            if let Some(record) = stores.observations.observation_by_id(id) {
                sources.insert(record.source_id().clone());
            }
        }
        let selected = EpistemicStores {
            claims: stores.claims.audit_subset(&claims),
            observations: stores.observations.audit_subset(&observations),
            sources: stores.sources.audit_subset(&sources),
            mentions: stores.mentions.audit_subset(&observations),
            verifications: stores.verifications.audit_subset(&claims),
            verdicts: stores.verdicts.audit_subset(&claims),
            candidates: stores.candidates.audit_subset(&candidates),
            reconciliations,
            merges,
            analyst_decisions: stores.analyst_decisions.audit_subset(&claims),
            audit_bindings: stores.audit_bindings.audit_subset(&claims),
            ..EpistemicStores::default()
        };
        let archive = AuditArchive {
            schema: "corrobore-claim-audit-v1".into(),
            claim_ids: roots.clone(),
            audits,
            snapshot: self.scoped_audit_snapshot(selected, &evidence, &roots),
        };
        serde_json::to_value(archive).map_err(invalid)
    }
    /// Restore a validated archive into a new graph, verifying every retained audit.
    pub fn from_claim_audit_archive(archive: &Value) -> Result<Self, GraphError> {
        let archive: AuditArchive = serde_json::from_value(archive.clone()).map_err(invalid)?;
        if archive.schema != "corrobore-claim-audit-v1" {
            return Err(invalid("unsupported audit archive schema"));
        }
        let graph = Self::from_persistence_snapshot(archive.snapshot)?;
        let mut expected = BTreeMap::new();
        for id in &archive.claim_ids {
            expected.insert(id.as_str().to_owned(), graph.claim_audit_path(id)?);
        }
        if expected.len() != archive.claim_ids.len() || expected != archive.audits {
            return Err(invalid("archive audit does not match retained records"));
        }
        Ok(graph)
    }
    /// Export the complete native memory snapshot, including governed records.
    pub fn export_memory_json(&self) -> Result<String, GraphError> {
        serde_json::to_string(&self.persistence_snapshot()).map_err(invalid)
    }
    /// Re-import a native memory snapshot with the normal restoration checks.
    pub fn from_memory_json(json: &str) -> Result<Self, GraphError> {
        Self::from_persistence_snapshot(serde_json::from_str(json).map_err(invalid)?)
    }
}

/// Optional export attachment. A malformed audit fails serialization rather than
/// silently producing an incomplete bundle. Existing infallible document builders
/// keep their API while their fallible JSON serialization reports the error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditArchiveAttachment(Option<Result<Value, String>>);
impl AuditArchiveAttachment {
    /// Whether no exported claim has governed provenance to attach.
    pub fn is_none(&self) -> bool {
        self.0.is_none()
    }
}
impl Serialize for AuditArchiveAttachment {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match &self.0 {
            None => serializer.serialize_none(),
            Some(Ok(value)) => value.serialize(serializer),
            Some(Err(error)) => Err(serde::ser::Error::custom(error)),
        }
    }
}
impl Graph {
    /// Attach provenance only for claims on records actually emitted by an exporter.
    pub fn audit_archive_for_export_targets(
        &self,
        targets: &[ClaimTarget],
    ) -> AuditArchiveAttachment {
        let claims = self
            .epistemic_stores()
            .claims
            .claims()
            .into_iter()
            .filter(|claim| targets.contains(claim.target()))
            .map(|claim| claim.id().clone())
            .collect::<Vec<_>>();
        AuditArchiveAttachment((!claims.is_empty()).then(|| {
            self.export_claim_audit_archive(&claims)
                .map_err(|e| e.to_string())
        }))
    }
    /// Re-import the audit extension of a STIX or FIMI document into a fresh graph.
    /// This restores exported claims and their provenance, not unrelated domain data.
    pub fn from_exported_audit_bundle(bundle: &Value) -> Result<Self, GraphError> {
        Self::from_claim_audit_archive(
            bundle
                .get("x_corrobore_audit_archive")
                .ok_or_else(|| invalid("bundle has no Corrobore audit archive"))?,
        )
    }
}

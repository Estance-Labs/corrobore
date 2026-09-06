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
//! Read-only claim audit assembled from exact, retained provenance references.
use crate::*;
use serde::{Deserialize, Serialize};
/// A retained ingestion record that influenced a claim.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClaimAuditReference {
    /// Candidate version; its predecessors are included in the audit.
    Candidate(CandidateId),
    /// Evidence-cited reconciliation decision.
    Reconciliation(ReconciliationRecordId),
}
/// Append-only exact provenance bindings; shared extraction runs are not links.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ClaimAuditBindings {
    links: Vec<(ClaimId, ClaimAuditReference)>,
}
impl ClaimAuditBindings {
    /// Whether there are no explicit provenance associations.
    pub fn is_empty(&self) -> bool {
        self.links.is_empty()
    }
}

fn invalid(message: &str) -> GraphError {
    GraphError::InvalidPropertyValue(message.into())
}
impl ClaimAuditBindings {
    pub(crate) fn validate(&self, stores: &EpistemicStores) -> Result<(), GraphError> {
        let mut seen = std::collections::HashSet::new();
        for (claim, reference) in &self.links {
            stores.claims.claim_by_id(claim)?;
            match reference {
                ClaimAuditReference::Candidate(id) if stores.candidates.get(id).is_none() => {
                    return Err(invalid("audit candidate missing"));
                }
                ClaimAuditReference::Reconciliation(id)
                    if stores.reconciliations.record_by_id(id).is_none() =>
                {
                    return Err(invalid("audit reconciliation missing"));
                }
                _ => {}
            }
            let key =
                serde_json::to_string(&(claim, reference)).map_err(|e| invalid(&e.to_string()))?;
            if !seen.insert(key) {
                return Err(invalid("duplicate audit binding"));
            }
        }
        Ok(())
    }
}
fn array<T: Serialize>(
    items: impl IntoIterator<Item = T>,
) -> Result<serde_json::Value, GraphError> {
    let mut values = items
        .into_iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| invalid(&e.to_string()))?;
    values.sort_by_cached_key(|v| v.to_string());
    Ok(serde_json::Value::Array(values))
}
impl Graph {
    /// Bind a retained ingestion record to a claim without changing either record.
    pub fn link_claim_audit_record(
        &mut self,
        claim: &ClaimId,
        reference: ClaimAuditReference,
    ) -> Result<(), GraphError> {
        let stores = self.epistemic_stores_mut();
        let mut bindings = stores.audit_bindings.clone();
        let link = (claim.clone(), reference);
        if !bindings.links.contains(&link) {
            bindings.links.push(link);
        }
        bindings.validate(stores)?;
        stores.audit_bindings = bindings;
        Ok(())
    }
    /// Assemble exact provenance and stored history without resolving or verifying.
    /// Missing evidence and verification stages are explicit gaps. No clock is read.
    pub fn claim_audit_path(&self, id: &ClaimId) -> Result<serde_json::Value, GraphError> {
        use serde_json::json;
        use std::collections::BTreeSet;
        let stores = self.epistemic_stores();
        self.evidence_store().validate_risk_references()?;
        let claim = stores.claims.claim_by_id(id)?;
        stores.audit_bindings.validate(stores)?;
        stores.claims.validate_link_indices()?;
        stores.analyst_decisions.validate(&stores.claims)?;
        let mut gaps = Vec::new();
        let mut claim_ids = BTreeSet::from([id.as_str().to_owned()]);
        // Follow retained claim-source links to a fixed point; cycles terminate.
        loop {
            let previous = claim_ids.len();
            for link in stores.claims.claim_links() {
                if claim_ids.contains(link.target_claim_id().as_str())
                    && let ClaimLinkSource::Claim(source) = link.source()
                {
                    claim_ids.insert(source.as_str().to_owned());
                }
            }
            if previous == claim_ids.len() {
                break;
            }
        }
        let claims = claim_ids
            .iter()
            .filter_map(|key| {
                match ClaimId::new(key).and_then(|key| stores.claims.claim_by_id(&key)) {
                    Ok(claim) => Some(claim),
                    Err(_) => {
                        gaps.push(json!({"kind":"missing_claim","id":key}));
                        None
                    }
                }
            })
            .collect::<Vec<_>>();
        let links = stores
            .claims
            .claim_links()
            .iter()
            .filter(|link| claim_ids.contains(link.target_claim_id().as_str()))
            .collect::<Vec<_>>();
        let verifications = claims
            .iter()
            .flat_map(|claim| stores.verifications.records_for_claim(claim.id()))
            .collect::<Vec<_>>();
        let mut observations = BTreeSet::new();
        let mut evidence_ids = BTreeSet::new();
        let mut candidate_ids = BTreeSet::new();
        let mut reconciliation_ids = BTreeSet::new();
        for claim in &claims {
            evidence_ids.extend(
                claim
                    .evidence_refs()
                    .iter()
                    .map(|id| id.as_str().to_owned()),
            );
            // An exact promotion target is provenance; a shared run is not.
            for promotion in stores.candidates.promotions() {
                let matches = matches!(promotion.target(), TierRecordRef::Claim(target) if target == claim.id());
                if matches {
                    candidate_ids.insert(promotion.candidate_id().as_str().to_owned());
                }
            }
        }
        for (claim, reference) in &stores.audit_bindings.links {
            if !claim_ids.contains(claim.as_str()) {
                continue;
            }
            match reference {
                ClaimAuditReference::Candidate(id) => {
                    candidate_ids.insert(id.as_str().to_owned());
                }
                ClaimAuditReference::Reconciliation(id) => {
                    reconciliation_ids.insert(id.as_str().to_owned());
                }
            }
        }
        for link in &links {
            match link.source() {
                ClaimLinkSource::Evidence(id) => {
                    evidence_ids.insert(id.as_str().to_owned());
                }
                ClaimLinkSource::Observation(id) => {
                    observations.insert(id.as_str().to_owned());
                }
                _ => {}
            }
        }
        for record in &verifications {
            observations.extend(
                record
                    .inputs()
                    .observation_ids()
                    .iter()
                    .map(|id| id.as_str().to_owned()),
            );
        }
        // Exact observation bindings and retained risk-group references are
        // provenance, unlike mere shared-source or extraction-run membership.
        evidence_ids.extend(
            self.evidence_store()
                .records()
                .iter()
                .filter(|r| {
                    r.observation_id()
                        .is_some_and(|id| observations.contains(id.as_str()))
                })
                .map(|r| r.id().as_str().to_owned()),
        );
        loop {
            let before = evidence_ids.len();
            let mut related = Vec::new();
            for key in &evidence_ids {
                if let Some(record) = self.evidence_by_id(&EvidenceId::new(key)?) {
                    for risk in self.evidence_store().risk_assessments_for(record.id()) {
                        related.extend(
                            risk.finding
                                .evidence_ids
                                .iter()
                                .chain(&risk.quarantined_evidence_ids)
                                .map(|id| id.as_str().to_owned()),
                        );
                    }
                }
            }
            evidence_ids.extend(related);
            if evidence_ids.len() == before {
                break;
            }
        }
        let mut sources = BTreeSet::new();
        let mut evidence = Vec::new();
        for key in evidence_ids {
            if let Some(record) = self.evidence_by_id(&EvidenceId::new(&key)?) {
                if let Some(id) = record.observation_id() {
                    observations.insert(id.as_str().to_owned());
                }
                if let Some(id) = record.source_id() {
                    sources.insert(id.as_str().to_owned());
                }
                evidence.push(record);
            } else {
                gaps.push(json!({"kind":"missing_evidence","id":key}));
            }
        }
        let reconciliations = reconciliation_ids
            .iter()
            .filter_map(|key| {
                stores
                    .reconciliations
                    .record_by_id(&ReconciliationRecordId::new(key).ok()?)
            })
            .collect::<Vec<_>>();
        for record in &reconciliations {
            for citation in record.citations() {
                observations.insert(citation.observation_id.as_str().to_owned());
                sources.insert(citation.source_id.as_str().to_owned());
            }
        }
        let mut observation_records = Vec::new();
        for key in &observations {
            if let Some(record) = stores
                .observations
                .observation_by_id(&ObservationId::new(key)?)
            {
                sources.insert(record.source_id().as_str().to_owned());
                observation_records.push(record);
            } else {
                gaps.push(json!({"kind":"missing_observation","id":key}));
            }
        }
        let mut source_versions = Vec::new();
        for key in sources {
            let versions = stores.sources.source_versions(&SourceId::new(&key)?);
            if versions.is_empty() {
                gaps.push(json!({"kind":"missing_source","id":key}));
            }
            source_versions.extend(versions);
        }
        // Preserve all predecessors without conflating siblings in one run.
        loop {
            let previous = candidate_ids.len();
            let predecessors = candidate_ids
                .iter()
                .filter_map(|key| {
                    stores
                        .candidates
                        .get(&CandidateId::new(key).ok()?)?
                        .repair()
                        .map(|r| r.predecessor.as_str().to_owned())
                })
                .collect::<Vec<_>>();
            candidate_ids.extend(predecessors);
            if previous == candidate_ids.len() {
                break;
            }
        }
        let candidates = candidate_ids
            .iter()
            .filter_map(|key| stores.candidates.get(&CandidateId::new(key).ok()?))
            .collect::<Vec<_>>();
        let promotions = stores
            .candidates
            .promotions()
            .iter()
            .filter(|p| candidate_ids.contains(p.candidate_id().as_str()));
        let mut coverage = Vec::new();
        for claim in &claims {
            let entry = VerificationCoverage::derive(claim, &stores.verifications);
            for (name, deterministic) in [
                ("mechanical_verification", true),
                ("semantic_verification", false),
            ] {
                if !entry
                    .entries()
                    .iter()
                    .any(|e| e.record_id().is_some() && e.deterministic() == deterministic)
                {
                    gaps.push(json!({"kind":name,"claim_id":claim.id().as_str()}));
                }
            }
            coverage.push(entry);
        }
        if candidate_ids.is_empty() {
            gaps.push(json!({"kind":"unrecorded_repair_lineage","claim_id":id.as_str()}));
        }
        if reconciliation_ids.is_empty() {
            gaps.push(json!({"kind":"unrecorded_reconciliation_lineage","claim_id":id.as_str()}));
        }
        let current = stores.verdicts.current_verdict(id);
        if current.is_none() {
            gaps.push(json!({"kind":"no_stored_verdict","claim_id":id.as_str()}));
        }
        let link_membership = stores.claims.claim_links().iter().enumerate()
            .filter(|(_, link)| claim_ids.contains(link.target_claim_id().as_str()))
            .map(|(position, link)| {
                let index = stores.claims.claim_link_index(position);
                let explanation = stores.verdicts.current_verdict(link.target_claim_id()).map(|v| v.explanation());
                let clusters = explanation.as_ref().map(|e| e.clusters().iter().filter(|c| c.members().iter().any(|m| m.link_index() == index && m.reference().is_none_or(|r| r == link.reference_key()))).map(|c| c.cluster_id()).collect::<Vec<_>>()).unwrap_or_default();
                json!({"store_index":index,"reference":link.reference_key(),"claim_id":link.target_claim_id(),"stored_cluster_ids":clusters})
            });
        let contradictions = links.iter().filter(|link| {
            matches!(
                link.kind(),
                ClaimLinkKind::Refutes | ClaimLinkKind::Contradicts
            )
        });
        let risk_ids: BTreeSet<_> = evidence
            .iter()
            .flat_map(|r| r.risk_assessment_ids())
            .collect();
        let mut audit = json!({
            "claim":claim,
            "analyst_decisions":stores.analyst_decisions.records_for_claim(id),
            "related_claims":array(claims.iter().filter(|c|c.id()!=id))?,
            "evidence_links":array(&links)?,
            "link_membership":array(link_membership)?,
            "contradictions":array(contradictions)?,
            "observations":array(observation_records)?,
            "source_versions":array(source_versions)?,
            "evidence":array(evidence)?,
            "mentions":array(stores.mentions.mentions().iter().filter(|m|observations.contains(m.observation_id().as_str())))?,
            "reconciliations":array(reconciliations)?,
            "merge_undos":array(stores.merges.undos().into_iter().filter(|u|reconciliation_ids.contains(u.reconciliation_id().as_str())))?,
            "verifications":array(verifications)?,
            "coverage":array(coverage)?,
            "current_verdict":current,
            "explanation":current.map(|v|v.explanation()),
            "verdict_history":stores.verdicts.verdicts_for_claim(id),
            "state_transitions":stores.verdicts.transitions_for_claim(id),
            "claim_decisions":stores.claims.claim_decisions_for_claim(id)?,
            "verification_disagreements":array(stores.verdicts.verification_disagreements_for_claim(id))?,
            "candidates":array(candidates)?,
            "promotions":array(promotions)?,
            "unverified_steps":array(gaps)?
        });
        if !risk_ids.is_empty() {
            audit["evidence_risk_assessments"] = array(
                self.evidence_store()
                    .risk_assessments()
                    .iter()
                    .filter(|r| risk_ids.contains(&r.id)),
            )?;
        }
        Ok(audit)
    }
}

impl ClaimAuditBindings {
    pub(crate) fn audit_subset(&self, ids: &std::collections::HashSet<ClaimId>) -> Self {
        Self {
            links: self
                .links
                .iter()
                .filter(|(claim, _)| ids.contains(claim))
                .cloned()
                .collect(),
        }
    }
}

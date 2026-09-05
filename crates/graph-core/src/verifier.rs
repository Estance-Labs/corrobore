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
//! Verifier framework (Epic 0029, WS-B item 1), per ADR-0017.
//!
//! Module boundary:
//! this module owns the `Verifier` trait, the typed `VerificationRequest` a
//! verifier reads, the `VerificationOutcome` it reports, and the
//! `VerifierRegistry` that owns provenance and mints the
//! [`VerificationRecord`]. It holds no concrete verifier (items 2 and 3), no
//! precedence policy (item 4, which lives beside the verdict resolution), and
//! no model runtime: non-deterministic verifiers reach the engine through the
//! domain provider ABI or the model adapter, per ADR-0012.
//!
//! Authority targets:
//! - a verifier reports, it does not adjudicate: the outcome carries a result,
//!   a rationale, stated limits, and consumed evidence, and nothing else;
//! - the registry owns the record identifier, the bitemporal stamp, and the
//!   `deterministic` flag, all derived from the registered specification, so a
//!   verifier cannot forge provenance;
//! - versions coexist: registering a new version never replaces an older one,
//!   and records written by earlier versions are never rewritten.
use std::collections::BTreeMap;

use crate::{
    BitemporalStamp, ClaimId, ClaimLink, ClaimStore, EvidenceRecord, EvidenceRecordStore,
    GraphError, Observation, ObservationStore, Source, SourceStore, VerdictAsOf,
    VerificationInputs, VerificationRecord, VerificationRecordId, VerificationRecordStore,
    VerificationResult,
};

/// Cost class of a verifier, mirroring the typed function registry vocabulary
/// so the two registries stay coherent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum VerifierCostClass {
    /// Local, constant-time checks.
    Low,
    /// Local checks that traverse records.
    Medium,
    /// Checks that leave the process or scan broadly.
    High,
}

impl VerifierCostClass {
    /// Canonical lowercase token.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

/// Read-only view of everything a verifier may inspect for one claim at one
/// bitemporal point.
#[derive(Clone, Debug)]
pub struct VerificationRequest<'a> {
    claim: &'a crate::Claim,
    links: Vec<&'a ClaimLink>,
    observations: Vec<&'a Observation>,
    sources: Vec<&'a Source>,
    evidence_records: Vec<&'a EvidenceRecord>,
    as_of: VerdictAsOf,
}

impl<'a> VerificationRequest<'a> {
    /// Build the request for a claim: its links active at the stamp, the
    /// observations those links resolve to, the current source version behind
    /// each observation, and the evidence records the links name.
    ///
    /// # Errors
    ///
    /// [`GraphError::ClaimNotFound`] when the claim does not exist.
    pub fn build(
        claim_id: &ClaimId,
        context: &VerificationContext<'a>,
        stamp: &BitemporalStamp,
    ) -> Result<Self, GraphError> {
        let (claims, observations, sources, evidence) = (
            context.claims(),
            context.observations(),
            context.sources(),
            context.evidence(),
        );
        let claim = claims.claim_by_id(claim_id)?;
        let as_of = VerdictAsOf::new(stamp.valid_from.clone(), stamp.transaction_time.clone());
        let links = claims.links_active_at(claim_id, &as_of);

        let mut seen_observations: Vec<&Observation> = Vec::new();
        let mut seen_evidence: Vec<&EvidenceRecord> = Vec::new();
        let remember_observation =
            |candidate: &'a Observation, collected: &mut Vec<&'a Observation>| {
                if !collected
                    .iter()
                    .any(|existing| existing.id() == candidate.id())
                {
                    collected.push(candidate);
                }
            };

        for link in &links {
            match link.source() {
                crate::ClaimLinkSource::Observation(observation_id) => {
                    if let Some(observation) = observations.observation_by_id(observation_id) {
                        remember_observation(observation, &mut seen_observations);
                    }
                }
                crate::ClaimLinkSource::Evidence(evidence_id) => {
                    if let Some(record) = evidence.evidence_by_id(evidence_id) {
                        if !seen_evidence
                            .iter()
                            .any(|existing| existing.id() == record.id())
                        {
                            seen_evidence.push(record);
                        }
                        if let Some(observation) = record
                            .observation_id()
                            .and_then(|id| observations.observation_by_id(id))
                        {
                            remember_observation(observation, &mut seen_observations);
                        }
                    }
                }
                crate::ClaimLinkSource::Claim(_) => {}
            }
        }

        let mut seen_sources: Vec<&Source> = Vec::new();
        for observation in &seen_observations {
            if let Some(source) = sources.current_source(observation.source_id())
                && !seen_sources
                    .iter()
                    .any(|existing| existing.id() == source.id())
            {
                seen_sources.push(source);
            }
        }

        Ok(Self {
            claim,
            links,
            observations: seen_observations,
            sources: seen_sources,
            evidence_records: seen_evidence,
            as_of,
        })
    }

    /// The claim under verification.
    pub fn claim(&self) -> &crate::Claim {
        self.claim
    }

    /// Evidence links active at the request's bitemporal point.
    pub fn links(&self) -> &[&'a ClaimLink] {
        &self.links
    }

    /// Observations the active links resolve to, deduplicated, in link order.
    pub fn observations(&self) -> &[&'a Observation] {
        &self.observations
    }

    /// Current source versions behind those observations, deduplicated.
    pub fn sources(&self) -> &[&'a Source] {
        &self.sources
    }

    /// Evidence records the active links name.
    pub fn evidence_records(&self) -> &[&'a EvidenceRecord] {
        &self.evidence_records
    }

    /// Bitemporal point the request was built at.
    pub fn as_of(&self) -> &VerdictAsOf {
        &self.as_of
    }

    /// The current source version behind an observation, when registered.
    pub fn source_of(&self, observation: &Observation) -> Option<&'a Source> {
        self.sources
            .iter()
            .find(|source| source.id() == observation.source_id())
            .copied()
    }
}

/// What a verifier reports. Provenance is not part of it: the registry owns
/// the identifier, the stamp, and the determinism flag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerificationOutcome {
    result: VerificationResult,
    rationale: Option<String>,
    limits: Vec<String>,
    evidence_consumed: Vec<String>,
}

impl VerificationOutcome {
    /// Report a result.
    pub fn new(result: VerificationResult) -> Self {
        Self {
            result,
            rationale: None,
            limits: Vec::new(),
            evidence_consumed: Vec::new(),
        }
    }

    /// Explain the result in one human-readable sentence.
    pub fn with_rationale(mut self, rationale: impl Into<String>) -> Self {
        self.rationale = Some(rationale.into());
        self
    }

    /// State what the check does not establish. Every verifier is expected to
    /// carry at least one limit, because a passing mechanical check never
    /// establishes more than its own scope.
    pub fn with_limit(mut self, limit: impl Into<String>) -> Self {
        self.limits.push(limit.into());
        self
    }

    /// Name one evidence reference the verifier read.
    pub fn with_evidence_consumed(mut self, reference: impl Into<String>) -> Self {
        self.evidence_consumed.push(reference.into());
        self
    }

    /// Result.
    pub fn result(&self) -> VerificationResult {
        self.result
    }
}

/// Read-only stores a verification run consults, mirroring the
/// [`crate::ResolutionInputs`] idiom so the two engine entry points read the
/// same way.
#[derive(Clone, Copy, Debug)]
pub struct VerificationContext<'a> {
    claims: &'a ClaimStore,
    observations: &'a ObservationStore,
    sources: &'a SourceStore,
    evidence: &'a EvidenceRecordStore,
}

impl<'a> VerificationContext<'a> {
    /// Bundle the read-only stores.
    pub fn new(
        claims: &'a ClaimStore,
        observations: &'a ObservationStore,
        sources: &'a SourceStore,
        evidence: &'a EvidenceRecordStore,
    ) -> Self {
        Self {
            claims,
            observations,
            sources,
            evidence,
        }
    }

    /// Claim store.
    pub fn claims(&self) -> &'a ClaimStore {
        self.claims
    }

    /// Observation store.
    pub fn observations(&self) -> &'a ObservationStore {
        self.observations
    }

    /// Source store.
    pub fn sources(&self) -> &'a SourceStore {
        self.sources
    }

    /// Evidence store.
    pub fn evidence(&self) -> &'a EvidenceRecordStore {
        self.evidence
    }
}

/// One executable check over a claim and its evidence.
///
/// Implementations live in the core only when deterministic and free of domain
/// vocabulary (ADR-0017). They must not mutate anything: the request is a
/// read-only view and the outcome is a report.
pub trait Verifier: Send + Sync {
    /// Stable `<namespace>.<name>` identifier.
    fn id(&self) -> &str;

    /// Version of this implementation. A logic change requires a new version.
    fn version(&self) -> &str;

    /// Whether the check is mechanically decidable. A non-deterministic
    /// verifier's result is advisory and can never block or force a verdict.
    fn deterministic(&self) -> bool;

    /// Cost class, for scheduling and budget policies.
    fn cost_class(&self) -> VerifierCostClass;

    /// Run the check.
    ///
    /// # Errors
    ///
    /// A typed error means the check could not run. It is not an
    /// `Inconclusive` result: no record is written for a failed run.
    fn verify(&self, request: &VerificationRequest<'_>) -> Result<VerificationOutcome, GraphError>;
}

/// A registered verifier with the provenance the registry will stamp on every
/// record it produces.
pub struct VerifierSpec {
    verifier: Box<dyn Verifier>,
}

impl VerifierSpec {
    /// Register a verifier, taking its declared identity as the specification.
    pub fn new(verifier: Box<dyn Verifier>) -> Self {
        Self { verifier }
    }

    /// Identifier.
    pub fn id(&self) -> &str {
        self.verifier.id()
    }

    /// Version.
    pub fn version(&self) -> &str {
        self.verifier.version()
    }

    /// Determinism, as declared by the implementation and enforced by the
    /// registry on every record.
    pub fn deterministic(&self) -> bool {
        self.verifier.deterministic()
    }

    /// Cost class.
    pub fn cost_class(&self) -> VerifierCostClass {
        self.verifier.cost_class()
    }
}

impl std::fmt::Debug for VerifierSpec {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VerifierSpec")
            .field("id", &self.id())
            .field("version", &self.version())
            .field("deterministic", &self.deterministic())
            .field("cost_class", &self.cost_class())
            .finish()
    }
}

/// Registry of verifiers, keyed by identifier and version so versions coexist.
#[derive(Debug, Default)]
pub struct VerifierRegistry {
    specs: BTreeMap<(String, String), VerifierSpec>,
}

impl VerifierRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a verifier.
    ///
    /// Registering the same identifier and version twice is idempotent when
    /// the declared identity matches. A different implementation under an
    /// existing identifier and version is a conflict, because records already
    /// written under that pair would become unreproducible.
    ///
    /// # Errors
    ///
    /// [`GraphError::InvalidVerifierRegistration`] for a blank identifier or
    /// version, or for a conflicting re-registration.
    pub fn register(&mut self, spec: VerifierSpec) -> Result<(), GraphError> {
        for (field, value) in [("id", spec.id()), ("version", spec.version())] {
            if value.trim().is_empty() {
                return Err(GraphError::InvalidVerifierRegistration(format!(
                    "verifier {field} must not be empty"
                )));
            }
        }

        let key = (spec.id().to_owned(), spec.version().to_owned());
        if self.specs.contains_key(&key) {
            return Err(GraphError::InvalidVerifierRegistration(format!(
                "{} version {} is already registered; the registry cannot prove two \
                 implementations agree, and records already written under this pair must \
                 stay reproducible, so register a new version instead",
                key.0, key.1
            )));
        }

        self.specs.insert(key, spec);
        Ok(())
    }

    /// Every registered version of one verifier, in ascending order.
    pub fn versions_of(&self, id: &str) -> Vec<&str> {
        self.specs
            .keys()
            .filter(|(spec_id, _)| spec_id == id)
            .map(|(_, version)| version.as_str())
            .collect()
    }

    /// The highest registered version of one verifier, by lexical order of the
    /// version string.
    pub fn latest_version(&self, id: &str) -> Option<&str> {
        self.versions_of(id).last().copied()
    }

    /// Number of registered identifier and version pairs.
    pub fn len(&self) -> usize {
        self.specs.len()
    }

    /// Whether no verifier is registered.
    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }

    /// Run one registered verifier against one claim and append its record.
    ///
    /// The registry builds the request, executes the check, and mints the
    /// record: identifier, bitemporal stamp, and `deterministic` flag come
    /// from the registration and the call, never from the verifier.
    ///
    /// # Errors
    ///
    /// [`GraphError::VerifierNotFound`] for an unknown identifier or version,
    /// [`GraphError::ClaimNotFound`] for an unknown claim, and any error the
    /// verifier itself returns. No record is written when the run fails.
    pub fn run(
        &self,
        verifier_id: &str,
        version: &str,
        claim_id: &ClaimId,
        context: &VerificationContext<'_>,
        records: &mut VerificationRecordStore,
        stamp: BitemporalStamp,
    ) -> Result<VerificationRecordId, GraphError> {
        let spec = self
            .specs
            .get(&(verifier_id.to_owned(), version.to_owned()))
            .ok_or_else(|| GraphError::VerifierNotFound {
                id: verifier_id.to_owned(),
                version: version.to_owned(),
            })?;

        let request = VerificationRequest::build(claim_id, context, &stamp)?;
        let outcome = spec.verifier.verify(&request)?;

        let ordinal = records.records_for_claim(claim_id).len() + 1;
        let record_id = VerificationRecordId::new(format!(
            "verification--{}--{}--{}--{ordinal}",
            spec.id(),
            spec.version(),
            claim_id.as_str()
        ))?;

        let mut inputs = VerificationInputs::for_claim(claim_id.clone());
        for link in request.links() {
            inputs = inputs.with_link_ref(link.reference_key());
        }
        for observation in request.observations() {
            inputs = inputs.with_observation(observation.id().clone());
        }

        let mut record = VerificationRecord::new(
            record_id.clone(),
            spec.id(),
            spec.version(),
            spec.deterministic(),
            inputs,
            outcome.result,
            stamp,
        );
        if let Some(rationale) = outcome.rationale {
            record = record.with_rationale(rationale);
        }
        for limit in outcome.limits {
            record = record.with_limit(limit);
        }
        for reference in outcome.evidence_consumed {
            record = record.with_evidence_consumed(reference);
        }

        records.append(record)
    }
}

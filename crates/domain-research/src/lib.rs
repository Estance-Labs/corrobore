// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
#![warn(missing_docs)]

//! MIT-licensed RESEARCH domain pack: scholarly production, method,
//! reproducibility, and epistemic state for any discipline, over Corrobore
//! agentic memory.
//!
//! The pack stores situated claims attributed to a work and an actor rather
//! than free-floating facts, types the stance a citation expresses instead of
//! inferring endorsement from a citation count, and reports reproducibility as
//! observations rather than a score.
//!
//! It performs no bibliometric ranking. There is no authority, prestige, or
//! quality score anywhere in the public surface, and none may be added: a
//! contract test asserts the absence.

mod export;
mod identifier;
mod provider_abi;

pub use export::{
    BibliographyEntry, BibliographyExport, BibliographyExporter, BibliographyRow, ExportError,
    ExportMode, IncompleteEntry,
};
pub use identifier::{
    IdentifierSystem, research_identifier_is_valid, research_identifier_normalize,
};
pub use provider_abi::{corrobore_domain_provider_get_api_v1, research_provider_api_v1};

use domain_common::{
    DomainValidationIssue, DomainValidationResult, DomainValidationSeverity, EvidenceRequirement,
    validate_evidence_requirement,
};
use graph_core::Confidence;
use serde::{Deserialize, Serialize};

/// RESEARCH node type.
///
/// There is no `Author` type. Authorship is a role a person can lose while
/// continuing to exist, so it is carried by a relationship rather than by the
/// identity-bearing node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResearchNodeType {
    /// Research question.
    ResearchQuestion,
    /// Hypothesis.
    Hypothesis,
    /// Study.
    Study,
    /// Method.
    Method,
    /// Instrument.
    Instrument,
    /// Sample.
    Sample,
    /// Dataset.
    Dataset,
    /// Code artifact.
    CodeArtifact,
    /// Experiment.
    Experiment,
    /// Result.
    Result,
    /// Finding.
    Finding,
    /// Claim.
    Claim,
    /// Publication.
    Publication,
    /// Preprint.
    Preprint,
    /// Venue.
    Venue,
    /// Person, who may hold author, reviewer, or investigator roles.
    Person,
    /// Institution.
    Institution,
    /// Grant.
    Grant,
    /// Review.
    Review,
    /// Replication attempt.
    ReplicationAttempt,
    /// Evidence.
    Evidence,
}

impl ResearchNodeType {
    /// Returns the stable label string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ResearchQuestion => "ResearchQuestion",
            Self::Hypothesis => "Hypothesis",
            Self::Study => "Study",
            Self::Method => "Method",
            Self::Instrument => "Instrument",
            Self::Sample => "Sample",
            Self::Dataset => "Dataset",
            Self::CodeArtifact => "CodeArtifact",
            Self::Experiment => "Experiment",
            Self::Result => "Result",
            Self::Finding => "Finding",
            Self::Claim => "Claim",
            Self::Publication => "Publication",
            Self::Preprint => "Preprint",
            Self::Venue => "Venue",
            Self::Person => "Person",
            Self::Institution => "Institution",
            Self::Grant => "Grant",
            Self::Review => "Review",
            Self::ReplicationAttempt => "ReplicationAttempt",
            Self::Evidence => "Evidence",
        }
    }

    /// Resolves a label string to a node type.
    #[must_use]
    pub fn from_label(label: &str) -> Option<Self> {
        let resolved = match label {
            "ResearchQuestion" => Self::ResearchQuestion,
            "Hypothesis" => Self::Hypothesis,
            "Study" => Self::Study,
            "Method" => Self::Method,
            "Instrument" => Self::Instrument,
            "Sample" => Self::Sample,
            "Dataset" => Self::Dataset,
            "CodeArtifact" => Self::CodeArtifact,
            "Experiment" => Self::Experiment,
            "Result" => Self::Result,
            "Finding" => Self::Finding,
            "Claim" => Self::Claim,
            "Publication" => Self::Publication,
            "Preprint" => Self::Preprint,
            "Venue" => Self::Venue,
            "Person" => Self::Person,
            "Institution" => Self::Institution,
            "Grant" => Self::Grant,
            "Review" => Self::Review,
            "ReplicationAttempt" => Self::ReplicationAttempt,
            "Evidence" => Self::Evidence,
            _ => return None,
        };
        Some(resolved)
    }
}

/// RESEARCH relationship type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResearchRelationshipType {
    /// Asks.
    Asks,
    /// Tests.
    Tests,
    /// Uses method.
    UsesMethod,
    /// Uses instrument.
    UsesInstrument,
    /// Uses dataset.
    UsesDataset,
    /// Uses code.
    UsesCode,
    /// Produces result.
    ProducesResult,
    /// Reports finding.
    ReportsFinding,
    /// Asserts claim.
    AssertsClaim,
    /// Supports.
    Supports,
    /// Refutes.
    Refutes,
    /// Qualifies.
    Qualifies,
    /// Replicates.
    Replicates,
    /// Fails to replicate.
    FailsToReplicate,
    /// Cites, the untyped default.
    Cites,
    /// Extends.
    Extends,
    /// Conflicts with.
    ConflictsWith,
    /// Supersedes.
    Supersedes,
    /// Corrects.
    Corrects,
    /// Retracts.
    Retracts,
    /// Authored by.
    AuthoredBy,
    /// Affiliated with.
    AffiliatedWith,
    /// Funded by.
    FundedBy,
    /// Published in.
    PublishedIn,
    /// Reviewed by.
    ReviewedBy,
    /// Derived from.
    DerivedFrom,
    /// Supported by.
    SupportedBy,
}

/// Stance a citing work expresses toward a cited work.
///
/// `Cites` is the untyped default and is never counted as support. The other
/// variants are stance-bearing and carry a higher evidential burden.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CitationStance {
    /// Untyped reference; records that one work referenced another and nothing
    /// more.
    #[default]
    Cites,
    /// The citing work supports the cited work.
    Supports,
    /// The citing work refutes the cited work.
    Refutes,
    /// The citing work qualifies the cited work.
    Qualifies,
    /// The citing work extends the cited work.
    Extends,
}

impl CitationStance {
    /// Returns `true` when the stance asserts a position rather than a bare
    /// reference.
    #[must_use]
    pub const fn is_stance_bearing(self) -> bool {
        !matches!(self, Self::Cites)
    }
}

/// Outcome of a replication attempt, from a closed set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReplicationOutcome {
    /// The result replicated.
    Successful,
    /// The result did not replicate.
    Failed,
    /// The result replicated in part.
    PartiallySuccessful,
    /// The attempt was inconclusive.
    Inconclusive,
}

/// A recorded replication attempt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplicationAttemptRecord {
    /// The work whose result was targeted.
    pub target_work: String,
    /// The work reporting this attempt.
    pub reporting_work: String,
    /// Outcome.
    pub outcome: ReplicationOutcome,
}

impl ReplicationAttemptRecord {
    /// Returns `true` when both the target and the reporting work are named.
    #[must_use]
    pub fn is_attributed(&self) -> bool {
        !self.target_work.trim().is_empty() && !self.reporting_work.trim().is_empty()
    }
}

/// Aggregate replication state derived from recorded attempts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReplicationState {
    /// No attempt is recorded.
    NotAttempted,
    /// Every recorded attempt succeeded.
    Replicated,
    /// Every recorded attempt failed.
    FailedToReplicate,
    /// Attempts disagree.
    Mixed,
    /// Recorded attempts were inconclusive or partial.
    Inconclusive,
}

/// Replication summary returned by the replication built-in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplicationStatus {
    /// Aggregate state.
    pub state: ReplicationState,
    /// Total recorded attempts.
    pub attempts: usize,
    /// Successful attempts.
    pub successful: usize,
    /// Failed attempts.
    pub failed: usize,
}

/// Recorded, auditable override permitting a retracted work to keep supporting
/// a validated claim.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetractionOverride {
    /// Justification.
    pub justification: String,
    /// Actor who recorded the override.
    pub recorded_by: String,
}

impl RetractionOverride {
    /// Returns `true` when the override names both a justification and an
    /// accountable actor.
    #[must_use]
    pub fn is_recorded(&self) -> bool {
        !self.justification.trim().is_empty() && !self.recorded_by.trim().is_empty()
    }
}

/// A work cited in support of a claim, with its retraction state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupportingWork {
    /// Work id.
    pub work_id: String,
    /// Whether the work has been retracted.
    pub retracted: bool,
    /// Override permitting a retracted work to keep supporting a validated
    /// claim.
    pub retraction_override: Option<RetractionOverride>,
}

impl SupportingWork {
    /// Creates a non-retracted supporting work.
    #[must_use]
    pub fn new(work_id: impl Into<String>) -> Self {
        Self {
            work_id: work_id.into(),
            retracted: false,
            retraction_override: None,
        }
    }

    /// Marks the work as retracted.
    #[must_use]
    pub fn retracted(mut self) -> Self {
        self.retracted = true;
        self
    }

    /// Attaches a retraction override.
    #[must_use]
    pub fn with_override(mut self, value: RetractionOverride) -> Self {
        self.retraction_override = Some(value);
        self
    }
}

/// Whether a reproducibility artifact is present or absent.
///
/// Absence is reported as absence, never as a low score.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignalPresence {
    /// The artifact is linked.
    Present,
    /// The artifact is not linked.
    Absent,
}

impl SignalPresence {
    fn from_presence(present: bool) -> Self {
        if present { Self::Present } else { Self::Absent }
    }
}

/// Reproducibility artifacts linked to a record.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReproducibilityArtifacts {
    /// Linked dataset references.
    pub dataset_refs: Vec<String>,
    /// Linked code artifact references.
    pub code_refs: Vec<String>,
    /// Stated method reference.
    pub method_ref: Option<String>,
}

/// Reproducibility observations for a record.
///
/// This is deliberately a set of observations, not a score. Consumers decide
/// what the absence of an artifact means for their purpose.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReproducibilitySignals {
    /// Whether a dataset is linked.
    pub dataset: SignalPresence,
    /// Whether a code artifact is linked.
    pub code: SignalPresence,
    /// Whether a method is stated.
    pub method: SignalPresence,
    /// Recorded replication attempts.
    pub replication_attempts: usize,
    /// Successful replication attempts.
    pub successful_replications: usize,
    /// Failed replication attempts.
    pub failed_replications: usize,
}

/// Retraction state of a record.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RetractionState {
    /// Not retracted.
    NotRetracted,
    /// Retracted.
    Retracted,
}

/// Attribution of a claim to the work asserting it and the actor credited.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimAttribution {
    /// The work asserting the claim.
    pub asserting_work: String,
    /// The actor credited with the claim.
    pub credited_actor: String,
}

/// Intended lifecycle status of a record.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecordStatus {
    /// Candidate, not yet validated.
    #[default]
    Candidate,
    /// Validated.
    Validated,
}

/// A RESEARCH node record for validation, built-ins, and export assembly.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ResearchNodeRecord {
    /// Node type.
    pub node_type: Option<ResearchNodeType>,
    /// External id.
    pub external_id: Option<String>,
    /// Evidence refs.
    pub evidence_refs: Vec<String>,
    /// Confidence.
    pub confidence: Option<Confidence>,
    /// Work asserting a claim.
    pub asserting_work: Option<String>,
    /// Actor credited with a claim.
    pub credited_actor: Option<String>,
    /// Result references, required for a `Finding`.
    pub result_refs: Vec<String>,
    /// Experiment or study references, required for a `Result`.
    pub source_refs: Vec<String>,
    /// Works cited in support, with their retraction state.
    pub supporting_works: Vec<SupportingWork>,
    /// Conflicting record references.
    pub conflict_refs: Vec<String>,
    /// Recorded replication attempts.
    pub replication_attempts: Vec<ReplicationAttemptRecord>,
    /// Linked reproducibility artifacts.
    pub reproducibility: ReproducibilityArtifacts,
    /// Retraction state.
    pub retracted: bool,
    /// Supersession chain, most recent first.
    pub supersedes: Vec<String>,
    /// Intended lifecycle status.
    pub intended_status: RecordStatus,
}

impl ResearchNodeRecord {
    /// Creates a new candidate record of the given type.
    #[must_use]
    pub fn new(node_type: ResearchNodeType) -> Self {
        Self {
            node_type: Some(node_type),
            ..Self::default()
        }
    }

    /// Sets the external id.
    #[must_use]
    pub fn with_external_id(mut self, external_id: impl Into<String>) -> Self {
        self.external_id = Some(external_id.into());
        self
    }

    /// Adds an evidence reference.
    #[must_use]
    pub fn with_evidence_ref(mut self, evidence_ref: impl Into<String>) -> Self {
        self.evidence_refs.push(evidence_ref.into());
        self
    }

    /// Sets the confidence.
    #[must_use]
    pub fn with_confidence(mut self, confidence: Confidence) -> Self {
        self.confidence = Some(confidence);
        self
    }

    /// Sets claim attribution.
    #[must_use]
    pub fn with_attribution(
        mut self,
        asserting_work: impl Into<String>,
        credited_actor: impl Into<String>,
    ) -> Self {
        self.asserting_work = Some(asserting_work.into());
        self.credited_actor = Some(credited_actor.into());
        self
    }

    /// Adds a result reference.
    #[must_use]
    pub fn with_result_ref(mut self, result_ref: impl Into<String>) -> Self {
        self.result_refs.push(result_ref.into());
        self
    }

    /// Adds an experiment or study reference.
    #[must_use]
    pub fn with_source_ref(mut self, source_ref: impl Into<String>) -> Self {
        self.source_refs.push(source_ref.into());
        self
    }

    /// Adds a supporting work.
    #[must_use]
    pub fn with_supporting_work(mut self, work: SupportingWork) -> Self {
        self.supporting_works.push(work);
        self
    }

    /// Adds a conflicting record reference.
    #[must_use]
    pub fn with_conflict_ref(mut self, conflict_ref: impl Into<String>) -> Self {
        self.conflict_refs.push(conflict_ref.into());
        self
    }

    /// Adds a replication attempt.
    #[must_use]
    pub fn with_replication_attempt(mut self, attempt: ReplicationAttemptRecord) -> Self {
        self.replication_attempts.push(attempt);
        self
    }

    /// Sets the reproducibility artifacts.
    #[must_use]
    pub fn with_reproducibility(mut self, artifacts: ReproducibilityArtifacts) -> Self {
        self.reproducibility = artifacts;
        self
    }

    /// Marks the record as retracted.
    #[must_use]
    pub fn retracted(mut self) -> Self {
        self.retracted = true;
        self
    }

    /// Adds a superseded record reference.
    #[must_use]
    pub fn with_supersedes(mut self, superseded: impl Into<String>) -> Self {
        self.supersedes.push(superseded.into());
        self
    }

    /// Marks the record as intended to be validated.
    #[must_use]
    pub fn intended_validated(mut self) -> Self {
        self.intended_status = RecordStatus::Validated;
        self
    }
}

/// A citation relationship record with its stance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CitationRecord {
    /// Citing work.
    pub citing_work: String,
    /// Cited work.
    pub cited_work: String,
    /// Stance.
    pub stance: CitationStance,
    /// Locator identifying where the stance is expressed in the citing work.
    pub locator: Option<String>,
    /// Evidence refs.
    pub evidence_refs: Vec<String>,
    /// Intended lifecycle status.
    pub intended_status: RecordStatus,
}

impl CitationRecord {
    /// Creates an untyped citation between two works.
    #[must_use]
    pub fn new(citing_work: impl Into<String>, cited_work: impl Into<String>) -> Self {
        Self {
            citing_work: citing_work.into(),
            cited_work: cited_work.into(),
            stance: CitationStance::Cites,
            locator: None,
            evidence_refs: Vec::new(),
            intended_status: RecordStatus::Candidate,
        }
    }

    /// Sets the stance.
    #[must_use]
    pub fn with_stance(mut self, stance: CitationStance) -> Self {
        self.stance = stance;
        self
    }

    /// Sets the locator.
    #[must_use]
    pub fn with_locator(mut self, locator: impl Into<String>) -> Self {
        self.locator = Some(locator.into());
        self
    }

    /// Adds an evidence reference.
    #[must_use]
    pub fn with_evidence_ref(mut self, evidence_ref: impl Into<String>) -> Self {
        self.evidence_refs.push(evidence_ref.into());
        self
    }

    /// Marks the citation as intended to be validated.
    #[must_use]
    pub fn intended_validated(mut self) -> Self {
        self.intended_status = RecordStatus::Validated;
        self
    }
}

fn error_issue(code: &str, message: &str, field: &str) -> DomainValidationIssue {
    DomainValidationIssue::new(
        code,
        message,
        Some(field.to_owned()),
        DomainValidationSeverity::Error,
    )
    .expect("static issue payload should be valid")
}

fn is_present(value: Option<&String>) -> bool {
    value.is_some_and(|inner| !inner.trim().is_empty())
}

/// Validates a RESEARCH node record fail-closed.
///
/// Enforced rules:
///
/// - a `Claim` carries the work asserting it and the actor credited with it;
/// - a `Finding` references at least one `Result`;
/// - a `Result` references the experiment or study that produced it;
/// - a `ReplicationAttempt` names its target work, reporting work, and an
///   outcome from the closed set;
/// - a retracted work cannot support a validated claim without a recorded,
///   auditable override.
#[must_use]
pub fn validate_research_node(record: &ResearchNodeRecord) -> DomainValidationResult {
    let mut issues = Vec::new();

    match record.node_type {
        Some(ResearchNodeType::Claim) => {
            // A claim without attribution is invalid regardless of status: a
            // claim is situated by construction.
            if !is_present(record.asserting_work.as_ref())
                || !is_present(record.credited_actor.as_ref())
            {
                issues.push(error_issue(
                    "RESEARCH_CLAIM_ATTRIBUTION_REQUIRED",
                    "a claim requires an asserting work and a credited actor",
                    "asserting_work",
                ));
            }
        }
        Some(ResearchNodeType::Finding) => {
            if record.result_refs.is_empty() {
                issues.push(error_issue(
                    "RESEARCH_FINDING_RESULT_REQUIRED",
                    "a finding must reference at least one result",
                    "result_refs",
                ));
            }
        }
        Some(ResearchNodeType::Result) => {
            if record.source_refs.is_empty() {
                issues.push(error_issue(
                    "RESEARCH_RESULT_SOURCE_REQUIRED",
                    "a result must reference the experiment or study that produced it",
                    "source_refs",
                ));
            }
        }
        Some(ResearchNodeType::ReplicationAttempt) => {
            let attributed = record.replication_attempts.len() == 1
                && record.replication_attempts[0].is_attributed();
            if !attributed {
                issues.push(error_issue(
                    "RESEARCH_REPLICATION_ATTRIBUTION_REQUIRED",
                    "a replication attempt must name one target work and its reporting work",
                    "replication_attempts",
                ));
            }
        }
        Some(_) => {}
        None => {
            issues.push(error_issue(
                "RESEARCH_NODE_TYPE_REQUIRED",
                "a research record requires a known node type",
                "node_type",
            ));
        }
    }

    if record.intended_status == RecordStatus::Validated {
        // Evidence-bearing records need evidence before validation.
        if matches!(
            record.node_type,
            Some(ResearchNodeType::Claim | ResearchNodeType::Finding)
        ) {
            let evidence_result =
                validate_evidence_requirement(EvidenceRequirement::Required, &record.evidence_refs);
            issues.extend(evidence_result.issues().iter().cloned());
        }

        // A retraction must not silently keep propping up a validated claim.
        for work in &record.supporting_works {
            if !work.retracted {
                continue;
            }
            let overridden = work
                .retraction_override
                .as_ref()
                .is_some_and(RetractionOverride::is_recorded);
            if !overridden {
                issues.push(error_issue(
                    "RESEARCH_RETRACTED_SUPPORT_REQUIRES_OVERRIDE",
                    "a retracted work cannot support a validated claim without a recorded override",
                    "supporting_works",
                ));
            }
        }
    }

    if issues.is_empty() {
        DomainValidationResult::pass()
    } else {
        DomainValidationResult::fail(issues)
    }
}

/// Validates a citation relationship fail-closed.
///
/// A stance-bearing citation cannot reach validated status without a locator
/// identifying where the stance is expressed and at least one evidence
/// reference. An untyped `Cites` carries no such burden precisely because it
/// asserts nothing.
#[must_use]
pub fn validate_research_citation(record: &CitationRecord) -> DomainValidationResult {
    let mut issues = Vec::new();

    if record.citing_work.trim().is_empty() || record.cited_work.trim().is_empty() {
        issues.push(error_issue(
            "RESEARCH_CITATION_ENDPOINTS_REQUIRED",
            "a citation requires a citing work and a cited work",
            "citing_work",
        ));
    }

    if record.stance.is_stance_bearing() && record.intended_status == RecordStatus::Validated {
        if !is_present(record.locator.as_ref()) {
            issues.push(error_issue(
                "RESEARCH_CITATION_LOCATOR_REQUIRED",
                "a stance-bearing citation requires a locator into the citing work",
                "locator",
            ));
        }
        let evidence_result =
            validate_evidence_requirement(EvidenceRequirement::Required, &record.evidence_refs);
        issues.extend(evidence_result.issues().iter().cloned());
    }

    if issues.is_empty() {
        DomainValidationResult::pass()
    } else {
        DomainValidationResult::fail(issues)
    }
}

/// Built-in `research.claim_attribution`: returns the work and actor a claim is
/// attributed to.
#[must_use]
pub fn research_claim_attribution(record: &ResearchNodeRecord) -> Option<ClaimAttribution> {
    let asserting_work = record.asserting_work.as_ref()?;
    let credited_actor = record.credited_actor.as_ref()?;
    if asserting_work.trim().is_empty() || credited_actor.trim().is_empty() {
        return None;
    }
    Some(ClaimAttribution {
        asserting_work: asserting_work.clone(),
        credited_actor: credited_actor.clone(),
    })
}

/// Built-in `research.citation_stance`: returns the stored stance.
#[must_use]
pub fn research_citation_stance(record: &CitationRecord) -> CitationStance {
    record.stance
}

/// Built-in `research.support_count`: counts citations that explicitly support.
///
/// Untyped `Cites` relationships are never counted, which is the whole point of
/// typing stance rather than counting references.
#[must_use]
pub fn research_support_count(citations: &[CitationRecord]) -> usize {
    citations
        .iter()
        .filter(|citation| citation.stance == CitationStance::Supports)
        .count()
}

/// Built-in `research.refutation_count`: counts citations that explicitly
/// refute.
#[must_use]
pub fn research_refutation_count(citations: &[CitationRecord]) -> usize {
    citations
        .iter()
        .filter(|citation| citation.stance == CitationStance::Refutes)
        .count()
}

/// Built-in `research.contradiction_count`: counts recorded conflicts.
#[must_use]
pub fn research_contradiction_count(record: &ResearchNodeRecord) -> usize {
    record.conflict_refs.len()
}

/// Built-in `research.replication_status`: summarizes recorded attempts and
/// their outcomes.
#[must_use]
pub fn research_replication_status(record: &ResearchNodeRecord) -> ReplicationStatus {
    let attempts = record.replication_attempts.len();
    let successful = record
        .replication_attempts
        .iter()
        .filter(|attempt| attempt.outcome == ReplicationOutcome::Successful)
        .count();
    let failed = record
        .replication_attempts
        .iter()
        .filter(|attempt| attempt.outcome == ReplicationOutcome::Failed)
        .count();

    let state = if attempts == 0 {
        ReplicationState::NotAttempted
    } else if successful == attempts {
        ReplicationState::Replicated
    } else if failed == attempts {
        ReplicationState::FailedToReplicate
    } else if successful > 0 && failed > 0 {
        ReplicationState::Mixed
    } else {
        ReplicationState::Inconclusive
    };

    ReplicationStatus {
        state,
        attempts,
        successful,
        failed,
    }
}

/// Built-in `research.retraction_status`: reports whether a record is retracted.
#[must_use]
pub fn research_retraction_status(record: &ResearchNodeRecord) -> RetractionState {
    if record.retracted {
        RetractionState::Retracted
    } else {
        RetractionState::NotRetracted
    }
}

/// Built-in `research.supersession_chain`: returns the stored supersession
/// chain.
#[must_use]
pub fn research_supersession_chain(record: &ResearchNodeRecord) -> &[String] {
    record.supersedes.as_slice()
}

/// Built-in `research.reproducibility_signals`: reports which reproducibility
/// artifacts are present.
///
/// This returns observations. It does not return, and must never return, an
/// aggregate quality or reproducibility score.
#[must_use]
pub fn research_reproducibility_signals(record: &ResearchNodeRecord) -> ReproducibilitySignals {
    let status = research_replication_status(record);
    ReproducibilitySignals {
        dataset: SignalPresence::from_presence(!record.reproducibility.dataset_refs.is_empty()),
        code: SignalPresence::from_presence(!record.reproducibility.code_refs.is_empty()),
        method: SignalPresence::from_presence(is_present(
            record.reproducibility.method_ref.as_ref(),
        )),
        replication_attempts: status.attempts,
        successful_replications: status.successful,
        failed_replications: status.failed,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn there_is_no_author_node_type() {
        // Authorship is a role carried by a relationship, not a kind of thing.
        assert!(ResearchNodeType::from_label("Author").is_none());
        assert!(ResearchNodeType::from_label("Reviewer").is_none());
        assert!(ResearchNodeType::from_label("Investigator").is_none());
        assert_eq!(
            ResearchNodeType::from_label("Person"),
            Some(ResearchNodeType::Person)
        );
    }

    #[test]
    fn claim_without_attribution_is_rejected() {
        let record = ResearchNodeRecord::new(ResearchNodeType::Claim);
        let result = validate_research_node(&record);
        assert!(!result.is_valid());
        assert!(
            result
                .issues()
                .iter()
                .any(|issue| issue.code == "RESEARCH_CLAIM_ATTRIBUTION_REQUIRED")
        );

        // Missing only the credited actor is still a rejection.
        let partial = ResearchNodeRecord {
            asserting_work: Some("publication--1".to_owned()),
            ..ResearchNodeRecord::new(ResearchNodeType::Claim)
        };
        assert!(!validate_research_node(&partial).is_valid());
    }

    #[test]
    fn attributed_claim_passes_and_reports_attribution() {
        let record = ResearchNodeRecord::new(ResearchNodeType::Claim)
            .with_attribution("publication--1", "person--1");
        assert!(validate_research_node(&record).is_valid());
        let attribution = research_claim_attribution(&record).unwrap();
        assert_eq!(attribution.asserting_work, "publication--1");
        assert_eq!(attribution.credited_actor, "person--1");
    }

    #[test]
    fn finding_requires_a_result_and_result_requires_a_source() {
        let finding = ResearchNodeRecord::new(ResearchNodeType::Finding);
        assert!(
            validate_research_node(&finding)
                .issues()
                .iter()
                .any(|issue| issue.code == "RESEARCH_FINDING_RESULT_REQUIRED")
        );

        let result = ResearchNodeRecord::new(ResearchNodeType::Result);
        assert!(
            validate_research_node(&result)
                .issues()
                .iter()
                .any(|issue| issue.code == "RESEARCH_RESULT_SOURCE_REQUIRED")
        );

        let ok = ResearchNodeRecord::new(ResearchNodeType::Result).with_source_ref("experiment--1");
        assert!(validate_research_node(&ok).is_valid());
    }

    #[test]
    fn retracted_support_blocks_validation_without_a_recorded_override() {
        let base = ResearchNodeRecord::new(ResearchNodeType::Claim)
            .with_attribution("publication--1", "person--1")
            .with_evidence_ref("evidence--1")
            .intended_validated();

        let blocked = base
            .clone()
            .with_supporting_work(SupportingWork::new("publication--retracted").retracted());
        assert!(
            validate_research_node(&blocked)
                .issues()
                .iter()
                .any(|issue| issue.code == "RESEARCH_RETRACTED_SUPPORT_REQUIRES_OVERRIDE")
        );

        // An override missing its accountable actor is not a recorded override.
        let hollow = base.clone().with_supporting_work(
            SupportingWork::new("publication--retracted")
                .retracted()
                .with_override(RetractionOverride {
                    justification: "still relevant".to_owned(),
                    recorded_by: "   ".to_owned(),
                }),
        );
        assert!(!validate_research_node(&hollow).is_valid());

        let allowed = base.with_supporting_work(
            SupportingWork::new("publication--retracted")
                .retracted()
                .with_override(RetractionOverride {
                    justification: "methods section unaffected by the retraction".to_owned(),
                    recorded_by: "reviewer--1".to_owned(),
                }),
        );
        assert!(validate_research_node(&allowed).is_valid());
    }

    #[test]
    fn untyped_citations_are_not_counted_as_support() {
        let citations = vec![
            CitationRecord::new("a", "z"),
            CitationRecord::new("b", "z"),
            CitationRecord::new("c", "z").with_stance(CitationStance::Supports),
            CitationRecord::new("d", "z").with_stance(CitationStance::Refutes),
        ];

        assert_eq!(research_support_count(&citations), 1);
        assert_eq!(research_refutation_count(&citations), 1);
        assert_eq!(
            research_citation_stance(&citations[0]),
            CitationStance::Cites
        );
        assert!(!CitationStance::Cites.is_stance_bearing());
    }

    #[test]
    fn stance_bearing_citation_requires_locator_and_evidence() {
        let bare = CitationRecord::new("a", "z")
            .with_stance(CitationStance::Refutes)
            .intended_validated();
        let result = validate_research_citation(&bare);
        assert!(!result.is_valid());
        let codes: Vec<&str> = result
            .issues()
            .iter()
            .map(|issue| issue.code.as_str())
            .collect();
        assert!(codes.contains(&"RESEARCH_CITATION_LOCATOR_REQUIRED"));
        assert!(codes.contains(&"EVIDENCE_REQUIRED"));

        let complete = bare
            .clone()
            .with_locator("section 4, paragraph 2")
            .with_evidence_ref("evidence--1");
        assert!(validate_research_citation(&complete).is_valid());

        // An untyped citation carries no such burden.
        let untyped = CitationRecord::new("a", "z").intended_validated();
        assert!(validate_research_citation(&untyped).is_valid());
    }

    #[test]
    fn replication_status_reflects_attempts_and_outcomes() {
        let mut record = ResearchNodeRecord::new(ResearchNodeType::Finding);
        assert_eq!(
            research_replication_status(&record).state,
            ReplicationState::NotAttempted
        );

        record = record.with_replication_attempt(ReplicationAttemptRecord {
            target_work: "publication--1".to_owned(),
            reporting_work: "publication--2".to_owned(),
            outcome: ReplicationOutcome::Failed,
        });
        assert_eq!(
            research_replication_status(&record).state,
            ReplicationState::FailedToReplicate
        );

        record = record.with_replication_attempt(ReplicationAttemptRecord {
            target_work: "publication--1".to_owned(),
            reporting_work: "publication--3".to_owned(),
            outcome: ReplicationOutcome::Successful,
        });
        let status = research_replication_status(&record);
        assert_eq!(status.state, ReplicationState::Mixed);
        assert_eq!(status.attempts, 2);
        assert_eq!(status.successful, 1);
        assert_eq!(status.failed, 1);
    }

    #[test]
    fn reproducibility_signals_report_absence_as_absence() {
        let bare = ResearchNodeRecord::new(ResearchNodeType::Finding);
        let signals = research_reproducibility_signals(&bare);
        assert_eq!(signals.dataset, SignalPresence::Absent);
        assert_eq!(signals.code, SignalPresence::Absent);
        assert_eq!(signals.method, SignalPresence::Absent);
        assert_eq!(signals.replication_attempts, 0);

        let rich = bare.with_reproducibility(ReproducibilityArtifacts {
            dataset_refs: vec!["dataset--1".to_owned()],
            code_refs: vec!["code--1".to_owned()],
            method_ref: Some("method--1".to_owned()),
        });
        let signals = research_reproducibility_signals(&rich);
        assert_eq!(signals.dataset, SignalPresence::Present);
        assert_eq!(signals.code, SignalPresence::Present);
        assert_eq!(signals.method, SignalPresence::Present);
    }

    #[test]
    fn contradiction_count_and_supersession_chain_are_reported() {
        let record = ResearchNodeRecord::new(ResearchNodeType::Claim)
            .with_attribution("publication--1", "person--1")
            .with_conflict_ref("claim--other")
            .with_supersedes("claim--old");
        assert_eq!(research_contradiction_count(&record), 1);
        assert_eq!(research_supersession_chain(&record), ["claim--old"]);
        assert_eq!(
            research_retraction_status(&record),
            RetractionState::NotRetracted
        );
    }
}

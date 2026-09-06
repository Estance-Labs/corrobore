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
//! Verification records, deterministic-first verdicts, and state transitions
//! (Epic 0029, WS-A item 5 and WS-B item 4).
//!
//! Module boundary:
//! this module owns the three append-only governance records that make trusted
//! state a computed, replayable view: the `VerificationRecord` produced by one
//! verifier execution, the `Verdict` computed over active evidence links and
//! verification records, and the `StateTransition` appended on every verdict
//! change. It also owns the ADR-0016 projection and reachability gate plus the
//! ADR-0017 deterministic-first resolution policy. It does not define
//! verifiers, aggregation, or confidence-dimension semantics (WS-D).
//!
//! Validation targets:
//! - stores are append-only: identical re-appends are no-ops, differing
//!   records under an existing identifier are conflicts, nothing is removed;
//! - the verdict is never set by a caller: [`resolve_claim_verdict`] computes
//!   it from the claim store and the verification store, appends verdict and
//!   transition when the state changes, and applies the projected lifecycle
//!   status only when the `ClaimStatus` matrix allows it;
//! - as-of selection is deterministic: valid interval contains the as-of valid
//!   time, latest transaction time not later than the as-of system time, exact
//!   ties broken by the greater record identifier.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    BitemporalStamp, ClaimId, ClaimLink, ClaimLinkKind, ClaimLinkSource, ClaimStatus, ClaimStore,
    Confidence, EvidenceRecordStore, GraphError, ImmutableRecordKind, ObservationId,
    ObservationStore, PropertyMap, PropertyValue, SourceStore, StateTransitionId,
    TemporalTimestamp, ValidationErrorRecord, ValidationErrorSeverity, ValidationTarget, VerdictId,
    VerificationRecordId,
};

/// Validation code recorded when a verdict could not reach `Supported`,
/// `Refuted`, or `Mixed` because no active signal resolved to an observation
/// bound to a source.
pub const CLAIM_UNREACHABLE_EVIDENCE_CODE: &str = "claim.verdict.unreachable_evidence";

/// Validation code emitted when a deterministic verifier and an advisory
/// verifier disagree about the same claim at resolution time.
pub const VERIFICATION_AUTHORITY_DISAGREEMENT_CODE: &str =
    "claim.verdict.verification_authority_disagreement";

/// Computed epistemic state of a claim (ADR-0016).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VerdictState {
    /// Active evidence supports the claim and nothing active refutes it.
    Supported,
    /// Active evidence refutes the claim and nothing active supports it.
    Refuted,
    /// Active support and refutation coexist without policy resolution.
    Mixed,
    /// An explicit dispute or rebuttal workflow is open.
    Contested,
    /// No active evidence link or verification record.
    Unknown,
    /// Evidence exists but is below the sufficiency policy (WS-A item 6).
    InsufficientEvidence,
    /// A newer claim supersedes this one.
    Superseded,
}

impl VerdictState {
    /// Closed vocabulary in canonical order.
    pub const ALL: [Self; 7] = [
        Self::Supported,
        Self::Refuted,
        Self::Mixed,
        Self::Contested,
        Self::Unknown,
        Self::InsufficientEvidence,
        Self::Superseded,
    ];

    /// Canonical lowercase token.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Supported => "supported",
            Self::Refuted => "refuted",
            Self::Mixed => "mixed",
            Self::Contested => "contested",
            Self::Unknown => "unknown",
            Self::InsufficientEvidence => "insufficient_evidence",
            Self::Superseded => "superseded",
        }
    }
}

/// ADR-0016 projection from a computed verdict state to the lifecycle status.
///
/// `actionable` reports whether the actionability gate passed; it only affects
/// `Supported`, which projects to `Validated` in that case. WS-D owns the gate;
/// WS-A callers pass `false`.
pub fn project_verdict_state(state: VerdictState, actionable: bool) -> ClaimStatus {
    match state {
        VerdictState::Supported if actionable => ClaimStatus::Validated,
        VerdictState::Supported => ClaimStatus::Supported,
        VerdictState::Refuted => ClaimStatus::Contradicted,
        VerdictState::Mixed | VerdictState::Contested => ClaimStatus::Disputed,
        VerdictState::Unknown | VerdictState::InsufficientEvidence => ClaimStatus::Unresolved,
        VerdictState::Superseded => ClaimStatus::Superseded,
    }
}

/// Outcome of one verifier execution.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VerificationResult {
    /// The check passed.
    Pass,
    /// The check failed.
    Fail,
    /// The check could not decide.
    Inconclusive,
}

impl VerificationResult {
    /// Canonical lowercase token.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::Inconclusive => "inconclusive",
        }
    }
}

/// Classification of one current verification step in the claim coverage
/// view. A failing result is kept distinct from the authority axis while the
/// entry retains whether its verifier was deterministic.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationCoverageClass {
    /// A deterministic verifier ran without reporting failure.
    MechanicallyChecked,
    /// A non-deterministic verifier supplied a semantic assessment.
    SemanticallyJudged,
    /// No verifier has examined the claim.
    Unchecked,
    /// A current verifier result reports failure.
    Failing,
}

impl VerificationCoverageClass {
    /// Canonical token used by projections and exporters.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MechanicallyChecked => "mechanically_checked",
            Self::SemanticallyJudged => "semantically_judged",
            Self::Unchecked => "unchecked",
            Self::Failing => "failing",
        }
    }
}

/// Claim surface covered by verification records.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationCoverageTarget {
    /// The claim exposes a structured proposition.
    Proposition,
    /// Only the claim's text statement is available.
    Statement,
}

impl VerificationCoverageTarget {
    /// Canonical token used by projections and exporters.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Proposition => "proposition",
            Self::Statement => "statement",
        }
    }
}

/// One current entry in a claim's derived verification coverage.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct VerificationCoverageEntry {
    class: VerificationCoverageClass,
    #[serde(skip_serializing_if = "Option::is_none")]
    record_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    verifier_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    verifier_version: Option<String>,
    deterministic: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<String>,
}

impl VerificationCoverageEntry {
    /// Coverage classification.
    pub fn class(&self) -> VerificationCoverageClass {
        self.class
    }

    /// Verification record behind this entry, absent only when unchecked.
    pub fn record_id(&self) -> Option<&str> {
        self.record_id.as_deref()
    }

    /// Verifier identifier behind this entry, absent only when unchecked.
    pub fn verifier_id(&self) -> Option<&str> {
        self.verifier_id.as_deref()
    }

    /// Verifier version behind this entry, absent only when unchecked.
    pub fn verifier_version(&self) -> Option<&str> {
        self.verifier_version.as_deref()
    }

    /// Whether the backing verifier is deterministic.
    pub fn deterministic(&self) -> bool {
        self.deterministic
    }

    /// Lowercase verification result, absent only when unchecked.
    pub fn result(&self) -> Option<&str> {
        self.result.as_deref()
    }

    fn verifier_reference(&self) -> Option<String> {
        Some(format!(
            "{}@{}",
            self.verifier_id.as_deref()?,
            self.verifier_version.as_deref()?
        ))
    }
}

/// Current verification coverage derived from a claim and its append-only
/// verification records. It is a read view, never a separately persisted
/// report.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct VerificationCoverage {
    claim_id: String,
    target: VerificationCoverageTarget,
    entries: Vec<VerificationCoverageEntry>,
}

impl VerificationCoverage {
    /// Derive the current coverage view.
    ///
    /// Selects the newest logic version per verifier, classifies deterministic
    /// and semantic checks, retains failures, and emits one explicit unchecked
    /// entry when no verifier has run.
    pub fn derive(claim: &crate::Claim, records: &VerificationRecordStore) -> Self {
        let mut current: BTreeMap<&str, &VerificationRecord> = BTreeMap::new();
        for record in records.records_for_claim(claim.id()) {
            current
                .entry(record.verifier_id())
                .and_modify(|selected| {
                    if verification_authority_order(record, selected).is_gt() {
                        *selected = record;
                    }
                })
                .or_insert(record);
        }
        let mut entries: Vec<VerificationCoverageEntry> = current
            .into_values()
            .map(VerificationCoverageEntry::from_record)
            .collect();
        if entries.is_empty() {
            entries.push(VerificationCoverageEntry {
                class: VerificationCoverageClass::Unchecked,
                record_id: None,
                verifier_id: None,
                verifier_version: None,
                deterministic: false,
                result: None,
            });
        }

        Self {
            claim_id: claim.id().as_str().to_owned(),
            target: if claim.proposition().is_some() {
                VerificationCoverageTarget::Proposition
            } else {
                VerificationCoverageTarget::Statement
            },
            entries,
        }
    }

    /// Covered claim identifier.
    pub fn claim_id(&self) -> &str {
        self.claim_id.as_str()
    }

    /// Whether records cover a proposition or a text-only statement.
    pub fn target(&self) -> VerificationCoverageTarget {
        self.target
    }

    /// Current coverage entries in stable verifier order.
    pub fn entries(&self) -> &[VerificationCoverageEntry] {
        &self.entries
    }

    /// Additive graph properties used on projected claim nodes.
    pub fn to_property_map(&self) -> PropertyMap {
        let mut properties = PropertyMap::new();
        properties.insert(
            "verification_coverage".to_owned(),
            PropertyValue::StringList(
                self.entries
                    .iter()
                    .map(|entry| entry.class.as_str().to_owned())
                    .collect(),
            ),
        );
        properties.insert(
            "verification_coverage_target".to_owned(),
            PropertyValue::String(self.target.as_str().to_owned()),
        );
        properties.insert(
            "verification_coverage_unchecked".to_owned(),
            PropertyValue::Bool(
                self.entries
                    .iter()
                    .any(|entry| entry.class == VerificationCoverageClass::Unchecked),
            ),
        );
        for (name, predicate) in [
            (
                "verification_coverage_mechanical",
                VerificationCoverageEntry::is_mechanical as fn(&VerificationCoverageEntry) -> bool,
            ),
            (
                "verification_coverage_semantic",
                VerificationCoverageEntry::is_semantic as fn(&VerificationCoverageEntry) -> bool,
            ),
            (
                "verification_coverage_failing",
                VerificationCoverageEntry::is_failing as fn(&VerificationCoverageEntry) -> bool,
            ),
        ] {
            properties.insert(
                name.to_owned(),
                PropertyValue::StringList(
                    self.entries
                        .iter()
                        .filter(|entry| predicate(entry))
                        .filter_map(VerificationCoverageEntry::verifier_reference)
                        .collect(),
                ),
            );
        }
        properties
    }
}

impl VerificationCoverageEntry {
    fn from_record(record: &VerificationRecord) -> Self {
        Self {
            class: record.coverage_class(),
            record_id: Some(record.id().as_str().to_owned()),
            verifier_id: Some(record.verifier_id().to_owned()),
            verifier_version: Some(record.verifier_version().to_owned()),
            deterministic: record.deterministic(),
            result: Some(record.result().as_str().to_owned()),
        }
    }

    fn is_mechanical(&self) -> bool {
        self.record_id.is_some() && self.deterministic
    }

    fn is_semantic(&self) -> bool {
        self.record_id.is_some() && !self.deterministic
    }

    fn is_failing(&self) -> bool {
        self.class == VerificationCoverageClass::Failing
    }
}

/// What a verification record examined.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationInputs {
    claim_id: ClaimId,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    link_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    observation_ids: Vec<ObservationId>,
}

impl VerificationInputs {
    /// Inputs naming only the claim.
    pub fn for_claim(claim_id: ClaimId) -> Self {
        Self {
            claim_id,
            link_refs: Vec::new(),
            observation_ids: Vec::new(),
        }
    }

    /// Add an evidence link reference (see `ClaimLink::reference_key`).
    pub fn with_link_ref(mut self, link_ref: impl Into<String>) -> Self {
        self.link_refs.push(link_ref.into());
        self
    }

    /// Add an observation.
    pub fn with_observation(mut self, observation_id: ObservationId) -> Self {
        self.observation_ids.push(observation_id);
        self
    }

    /// Claim examined.
    pub fn claim_id(&self) -> &ClaimId {
        &self.claim_id
    }

    /// Evidence link references examined.
    pub fn link_refs(&self) -> &[String] {
        &self.link_refs
    }

    /// Observations examined.
    pub fn observation_ids(&self) -> &[ObservationId] {
        &self.observation_ids
    }
}

/// One verifier execution, append-only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationRecord {
    id: VerificationRecordId,
    verifier_id: String,
    verifier_version: String,
    deterministic: bool,
    inputs: VerificationInputs,
    result: VerificationResult,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rationale: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    limits: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    evidence_consumed: Vec<String>,
    stamp: BitemporalStamp,
}

impl VerificationRecord {
    /// Build a record from its mandatory fields.
    pub fn new(
        id: VerificationRecordId,
        verifier_id: impl Into<String>,
        verifier_version: impl Into<String>,
        deterministic: bool,
        inputs: VerificationInputs,
        result: VerificationResult,
        stamp: BitemporalStamp,
    ) -> Self {
        Self {
            id,
            verifier_id: verifier_id.into(),
            verifier_version: verifier_version.into(),
            deterministic,
            inputs,
            result,
            rationale: None,
            limits: Vec::new(),
            evidence_consumed: Vec::new(),
            stamp,
        }
    }

    /// Set the human-readable rationale.
    pub fn with_rationale(mut self, rationale: impl Into<String>) -> Self {
        self.rationale = Some(rationale.into());
        self
    }

    /// Add a stated limit of the check.
    pub fn with_limit(mut self, limit: impl Into<String>) -> Self {
        self.limits.push(limit.into());
        self
    }

    /// Add an evidence reference the verifier consumed.
    pub fn with_evidence_consumed(mut self, reference: impl Into<String>) -> Self {
        self.evidence_consumed.push(reference.into());
        self
    }

    /// Identifier.
    pub fn id(&self) -> &VerificationRecordId {
        &self.id
    }

    /// Verifier identifier.
    pub fn verifier_id(&self) -> &str {
        self.verifier_id.as_str()
    }

    /// Verifier version.
    pub fn verifier_version(&self) -> &str {
        self.verifier_version.as_str()
    }

    /// Whether the verifier is deterministic (WS-B precedence rule).
    pub fn deterministic(&self) -> bool {
        self.deterministic
    }

    /// Inputs examined.
    pub fn inputs(&self) -> &VerificationInputs {
        &self.inputs
    }

    /// Result.
    pub fn result(&self) -> VerificationResult {
        self.result
    }

    /// Classification contributed by this record to the current coverage
    /// view. Failures remain visibly failing while the deterministic flag
    /// continues to expose their mechanical or semantic authority.
    pub fn coverage_class(&self) -> VerificationCoverageClass {
        if self.result == VerificationResult::Fail {
            VerificationCoverageClass::Failing
        } else if self.deterministic {
            VerificationCoverageClass::MechanicallyChecked
        } else {
            VerificationCoverageClass::SemanticallyJudged
        }
    }

    /// Rationale, when given.
    pub fn rationale(&self) -> Option<&str> {
        self.rationale.as_deref()
    }

    /// Stated limits.
    pub fn limits(&self) -> &[String] {
        &self.limits
    }

    /// Evidence references consumed.
    pub fn evidence_consumed(&self) -> &[String] {
        &self.evidence_consumed
    }

    /// Bitemporal stamp.
    pub fn stamp(&self) -> &BitemporalStamp {
        &self.stamp
    }

    /// Validate mandatory string fields.
    fn validate(&self) -> Result<(), GraphError> {
        if self.verifier_id.trim().is_empty() {
            return Err(GraphError::InvalidPropertyValue(
                "verification record verifier_id must not be empty".to_owned(),
            ));
        }
        if self.verifier_version.trim().is_empty() {
            return Err(GraphError::InvalidPropertyValue(
                "verification record verifier_version must not be empty".to_owned(),
            ));
        }
        Ok(())
    }

    /// Project the record into additive, namespaced `verification_*`
    /// properties.
    pub fn to_property_map(&self) -> PropertyMap {
        let mut properties = PropertyMap::new();
        properties.insert(
            "verification_id".to_owned(),
            PropertyValue::String(self.id.as_str().to_owned()),
        );
        properties.insert(
            "verification_claim".to_owned(),
            PropertyValue::String(self.inputs.claim_id.as_str().to_owned()),
        );
        properties.insert(
            "verification_verifier_id".to_owned(),
            PropertyValue::String(self.verifier_id.clone()),
        );
        properties.insert(
            "verification_verifier_version".to_owned(),
            PropertyValue::String(self.verifier_version.clone()),
        );
        properties.insert(
            "verification_deterministic".to_owned(),
            PropertyValue::Bool(self.deterministic),
        );
        properties.insert(
            "verification_result".to_owned(),
            PropertyValue::String(self.result.as_str().to_owned()),
        );
        if let Some(rationale) = &self.rationale {
            properties.insert(
                "verification_rationale".to_owned(),
                PropertyValue::String(rationale.clone()),
            );
        }
        properties.insert(
            "verification_transaction_time".to_owned(),
            PropertyValue::String(self.stamp.transaction_time.as_str().to_owned()),
        );
        properties
    }
}

/// Append-only store of verification records.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationRecordStore {
    records: Vec<VerificationRecord>,
}

/// Typed, append-only record of an authority-boundary disagreement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationDisagreement {
    claim_id: ClaimId,
    deterministic_record_id: VerificationRecordId,
    advisory_record_id: VerificationRecordId,
    deterministic_result: VerificationResult,
    advisory_result: VerificationResult,
    stamp: BitemporalStamp,
}

impl VerificationDisagreement {
    /// Claim on which the verifier results disagree.
    pub fn claim_id(&self) -> &ClaimId {
        &self.claim_id
    }

    /// Deterministic record carrying authority.
    pub fn deterministic_record_id(&self) -> &VerificationRecordId {
        &self.deterministic_record_id
    }

    /// Non-deterministic advisory record.
    pub fn advisory_record_id(&self) -> &VerificationRecordId {
        &self.advisory_record_id
    }

    /// Result produced by the deterministic record.
    pub fn deterministic_result(&self) -> VerificationResult {
        self.deterministic_result
    }

    /// Opposing result produced by the advisory record.
    pub fn advisory_result(&self) -> VerificationResult {
        self.advisory_result
    }

    /// Resolution stamp at which the disagreement was first observed.
    pub fn stamp(&self) -> &BitemporalStamp {
        &self.stamp
    }

    /// Render this disagreement as a typed, non-blocking validation finding.
    pub fn to_validation_record(&self) -> ValidationErrorRecord {
        ValidationErrorRecord::new(
            VERIFICATION_AUTHORITY_DISAGREEMENT_CODE,
            ValidationErrorSeverity::Warning,
            format!(
                "deterministic verification {} ({}) and advisory verification {} ({}) disagree for claim {}; deterministic precedence applies",
                self.deterministic_record_id.as_str(),
                self.deterministic_result.as_str(),
                self.advisory_record_id.as_str(),
                self.advisory_result.as_str(),
                self.claim_id.as_str(),
            ),
            ValidationTarget::claim(self.claim_id.as_str()),
        )
    }
}

impl VerificationRecordStore {
    /// Creates an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a record. Idempotent for an identical record; a differing record
    /// under an existing identifier is a conflict.
    ///
    /// # Errors
    ///
    /// [`GraphError::InvalidPropertyValue`] for invalid fields or a conflict.
    pub fn append(
        &mut self,
        record: VerificationRecord,
    ) -> Result<VerificationRecordId, GraphError> {
        record.validate()?;

        if let Some(existing) = self.record_by_id(&record.id) {
            if existing == &record {
                return Ok(record.id);
            }
            return Err(GraphError::ImmutableRecordConflict {
                kind: ImmutableRecordKind::VerificationRecord,
                id: record.id.as_str().to_owned(),
            });
        }

        let id = record.id.clone();
        self.records.push(record);
        Ok(id)
    }

    /// One record by identifier.
    pub fn record_by_id(&self, id: &VerificationRecordId) -> Option<&VerificationRecord> {
        self.records.iter().find(|record| &record.id == id)
    }

    /// Records examining a claim, in append order.
    pub fn records_for_claim(&self, claim_id: &ClaimId) -> Vec<&VerificationRecord> {
        self.records
            .iter()
            .filter(|record| &record.inputs.claim_id == claim_id)
            .collect()
    }

    /// Number of records.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether the store is empty.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

/// Computed verdict of a claim at one point in bitemporal space.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Verdict {
    id: VerdictId,
    claim_id: ClaimId,
    state: VerdictState,
    /// Extensible string-keyed dimensions; WS-D names them normatively.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    confidence_dimensions: BTreeMap<String, Confidence>,
    policy_version: String,
    stamp: BitemporalStamp,
}

impl Verdict {
    /// Identifier.
    pub fn id(&self) -> &VerdictId {
        &self.id
    }

    /// Claim the verdict is about.
    pub fn claim_id(&self) -> &ClaimId {
        &self.claim_id
    }

    /// Computed state.
    pub fn state(&self) -> VerdictState {
        self.state
    }

    /// Confidence dimensions.
    pub fn confidence_dimensions(&self) -> &BTreeMap<String, Confidence> {
        &self.confidence_dimensions
    }

    /// Version of the policy that computed the verdict.
    pub fn policy_version(&self) -> &str {
        self.policy_version.as_str()
    }

    /// Bitemporal stamp.
    pub fn stamp(&self) -> &BitemporalStamp {
        &self.stamp
    }

    /// Project the verdict into additive, namespaced `verdict_*` properties.
    pub fn to_property_map(&self) -> PropertyMap {
        let mut properties = PropertyMap::new();
        properties.insert(
            "verdict_id".to_owned(),
            PropertyValue::String(self.id.as_str().to_owned()),
        );
        properties.insert(
            "verdict_claim".to_owned(),
            PropertyValue::String(self.claim_id.as_str().to_owned()),
        );
        properties.insert(
            "verdict_state".to_owned(),
            PropertyValue::String(self.state.as_str().to_owned()),
        );
        properties.insert(
            "verdict_lifecycle_projection".to_owned(),
            PropertyValue::String(lifecycle_token(project_verdict_state(self.state, false))),
        );
        properties.insert(
            "verdict_policy_version".to_owned(),
            PropertyValue::String(self.policy_version.clone()),
        );
        properties.insert(
            "verdict_valid_from".to_owned(),
            PropertyValue::String(self.stamp.valid_from.as_str().to_owned()),
        );
        properties.insert(
            "verdict_transaction_time".to_owned(),
            PropertyValue::String(self.stamp.transaction_time.as_str().to_owned()),
        );
        for (name, value) in &self.confidence_dimensions {
            properties.insert(
                format!("verdict_dimension_{name}"),
                PropertyValue::Float(value.value()),
            );
        }
        properties
    }
}

/// What caused a state transition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransitionTrigger {
    /// A resolution run over links and records.
    ResolutionRun,
    /// A verification record was the newest input driving the change.
    VerificationRecord(VerificationRecordId),
    /// A superseding link closed the claim.
    Supersession,
    /// A lifecycle decision (retraction, rejection) froze the verdict.
    LifecycleDecision,
}

impl TransitionTrigger {
    /// Canonical lowercase token.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ResolutionRun => "resolution_run",
            Self::VerificationRecord(_) => "verification_record",
            Self::Supersession => "supersession",
            Self::LifecycleDecision => "lifecycle_decision",
        }
    }
}

/// Lowercase token of a lifecycle status for projections.
pub fn lifecycle_token(status: ClaimStatus) -> String {
    format!("{status:?}").to_lowercase()
}

/// One appended change of verdict state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateTransition {
    id: StateTransitionId,
    claim_id: ClaimId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    from_state: Option<VerdictState>,
    to_state: VerdictState,
    trigger: TransitionTrigger,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    superseding_verdict_id: Option<VerdictId>,
    stamp: BitemporalStamp,
}

impl StateTransition {
    /// Identifier.
    pub fn id(&self) -> &StateTransitionId {
        &self.id
    }

    /// Claim.
    pub fn claim_id(&self) -> &ClaimId {
        &self.claim_id
    }

    /// State before the change, absent for the first verdict.
    pub fn from_state(&self) -> Option<VerdictState> {
        self.from_state
    }

    /// State after the change.
    pub fn to_state(&self) -> VerdictState {
        self.to_state
    }

    /// Trigger.
    pub fn trigger(&self) -> &TransitionTrigger {
        &self.trigger
    }

    /// Verdict that now carries the state.
    pub fn superseding_verdict_id(&self) -> Option<&VerdictId> {
        self.superseding_verdict_id.as_ref()
    }

    /// Bitemporal stamp.
    pub fn stamp(&self) -> &BitemporalStamp {
        &self.stamp
    }

    /// Project the transition into additive, namespaced `transition_*`
    /// properties.
    pub fn to_property_map(&self) -> PropertyMap {
        let mut properties = PropertyMap::new();
        properties.insert(
            "transition_id".to_owned(),
            PropertyValue::String(self.id.as_str().to_owned()),
        );
        properties.insert(
            "transition_claim".to_owned(),
            PropertyValue::String(self.claim_id.as_str().to_owned()),
        );
        if let Some(from) = self.from_state {
            properties.insert(
                "transition_from_state".to_owned(),
                PropertyValue::String(from.as_str().to_owned()),
            );
        }
        properties.insert(
            "transition_to_state".to_owned(),
            PropertyValue::String(self.to_state.as_str().to_owned()),
        );
        properties.insert(
            "transition_trigger".to_owned(),
            PropertyValue::String(self.trigger.as_str().to_owned()),
        );
        if let TransitionTrigger::VerificationRecord(record) = &self.trigger {
            properties.insert(
                "transition_verification_record".to_owned(),
                PropertyValue::String(record.as_str().to_owned()),
            );
        }
        if let Some(verdict) = &self.superseding_verdict_id {
            properties.insert(
                "transition_verdict".to_owned(),
                PropertyValue::String(verdict.as_str().to_owned()),
            );
        }
        properties.insert(
            "transition_transaction_time".to_owned(),
            PropertyValue::String(self.stamp.transaction_time.as_str().to_owned()),
        );
        properties
    }
}

/// Bitemporal point for as-of queries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerdictAsOf {
    valid_time: TemporalTimestamp,
    system_time: TemporalTimestamp,
}

impl VerdictAsOf {
    /// Build an as-of point.
    pub fn new(valid_time: TemporalTimestamp, system_time: TemporalTimestamp) -> Self {
        Self {
            valid_time,
            system_time,
        }
    }

    /// Valid time.
    pub fn valid_time(&self) -> &TemporalTimestamp {
        &self.valid_time
    }

    /// System time.
    pub fn system_time(&self) -> &TemporalTimestamp {
        &self.system_time
    }

    /// Whether `stamp` is active at this point: valid interval contains the
    /// valid time and the record was known by the system time.
    pub fn covers(&self, stamp: &BitemporalStamp) -> bool {
        let valid_time = self.valid_time.as_str();
        let starts_before = stamp.valid_from.as_str() <= valid_time;
        let ends_after = stamp
            .valid_to
            .as_ref()
            .is_none_or(|valid_to| valid_to.as_str() > valid_time);
        let known = stamp.transaction_time.as_str() <= self.system_time.as_str();

        starts_before && ends_after && known
    }
}

/// Append-only store of verdicts and state transitions.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct VerdictStore {
    verdicts: Vec<Verdict>,
    transitions: Vec<StateTransition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    reachability_gaps: Vec<ReachabilityGap>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    verification_disagreements: Vec<VerificationDisagreement>,
}

impl VerdictStore {
    /// Creates an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a verdict computed by the engine. Not a client-facing surface:
    /// Cypher, HTTP, and memory operations expose no path to it.
    ///
    /// # Errors
    ///
    /// [`GraphError::InvalidVersionState`] when the identifier already exists;
    /// [`GraphError::InvalidPropertyValue`] when `policy_version` is blank.
    pub fn append_verdict(
        &mut self,
        id: VerdictId,
        claim_id: ClaimId,
        state: VerdictState,
        confidence_dimensions: BTreeMap<String, Confidence>,
        policy_version: impl Into<String>,
        stamp: BitemporalStamp,
    ) -> Result<VerdictId, GraphError> {
        let policy_version = policy_version.into();
        if policy_version.trim().is_empty() {
            return Err(GraphError::InvalidPropertyValue(
                "verdict policy_version must not be empty".to_owned(),
            ));
        }
        if self.verdicts.iter().any(|verdict| verdict.id == id) {
            return Err(GraphError::ImmutableRecordConflict {
                kind: ImmutableRecordKind::Verdict,
                id: id.as_str().to_owned(),
            });
        }

        self.verdicts.push(Verdict {
            id: id.clone(),
            claim_id,
            state,
            confidence_dimensions,
            policy_version,
            stamp,
        });
        Ok(id)
    }

    /// Append a state transition.
    ///
    /// # Errors
    ///
    /// [`GraphError::InvalidVersionState`] when the identifier already exists.
    pub fn append_transition(&mut self, transition: StateTransition) -> Result<(), GraphError> {
        if self
            .transitions
            .iter()
            .any(|existing| existing.id == transition.id)
        {
            return Err(GraphError::ImmutableRecordConflict {
                kind: ImmutableRecordKind::StateTransition,
                id: transition.id.as_str().to_owned(),
            });
        }
        self.transitions.push(transition);
        Ok(())
    }

    /// Current verdict: latest transaction time, ties broken by greater id.
    pub fn current_verdict(&self, claim_id: &ClaimId) -> Option<&Verdict> {
        self.verdicts
            .iter()
            .filter(|verdict| &verdict.claim_id == claim_id)
            .max_by(|left, right| Self::verdict_order(left, right))
    }

    /// Verdict active at an as-of point, or `None` when nothing was known.
    pub fn verdict_as_of(&self, claim_id: &ClaimId, as_of: &VerdictAsOf) -> Option<&Verdict> {
        self.verdicts
            .iter()
            .filter(|verdict| &verdict.claim_id == claim_id && as_of.covers(&verdict.stamp))
            .max_by(|left, right| Self::verdict_order(left, right))
    }

    /// Every verdict of a claim, oldest first (transaction time, then id).
    pub fn verdicts_for_claim(&self, claim_id: &ClaimId) -> Vec<&Verdict> {
        let mut verdicts: Vec<&Verdict> = self
            .verdicts
            .iter()
            .filter(|verdict| &verdict.claim_id == claim_id)
            .collect();
        verdicts.sort_by(|left, right| Self::verdict_order(left, right));
        verdicts
    }

    /// Every transition of a claim, oldest first (transaction time, then id).
    pub fn transitions_for_claim(&self, claim_id: &ClaimId) -> Vec<&StateTransition> {
        let mut transitions: Vec<&StateTransition> = self
            .transitions
            .iter()
            .filter(|transition| &transition.claim_id == claim_id)
            .collect();
        transitions.sort_by(|left, right| {
            left.stamp
                .transaction_time
                .as_str()
                .cmp(right.stamp.transaction_time.as_str())
                .then_with(|| left.id.as_str().cmp(right.id.as_str()))
        });
        transitions
    }

    /// Every recorded reachability gap, oldest first.
    pub fn reachability_gaps(&self) -> &[ReachabilityGap] {
        &self.reachability_gaps
    }

    /// Every recorded deterministic/advisory disagreement.
    pub fn verification_disagreements(&self) -> &[VerificationDisagreement] {
        &self.verification_disagreements
    }

    /// Disagreements for one claim, in first-observed order.
    pub fn verification_disagreements_for_claim(
        &self,
        claim_id: &ClaimId,
    ) -> Vec<&VerificationDisagreement> {
        self.verification_disagreements
            .iter()
            .filter(|disagreement| disagreement.claim_id() == claim_id)
            .collect()
    }

    /// Deterministic order: transaction time, then identifier.
    fn verdict_order(left: &Verdict, right: &Verdict) -> std::cmp::Ordering {
        left.stamp
            .transaction_time
            .as_str()
            .cmp(right.stamp.transaction_time.as_str())
            .then_with(|| left.id.as_str().cmp(right.id.as_str()))
    }

    /// Number of verdicts.
    pub fn len(&self) -> usize {
        self.verdicts.len()
    }

    /// Whether no verdict is stored.
    pub fn is_empty(&self) -> bool {
        self.verdicts.is_empty()
    }
}

/// Result of one resolution call.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolutionOutcome {
    claim_id: ClaimId,
    state: VerdictState,
    changed: bool,
    lifecycle_applied: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    verdict_id: Option<VerdictId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reachability_gap: Option<ReachabilityGap>,
}

impl ResolutionOutcome {
    /// Gap recorded when the gate downgraded the verdict to
    /// `InsufficientEvidence`.
    pub fn reachability_gap(&self) -> Option<&ReachabilityGap> {
        self.reachability_gap.as_ref()
    }

    /// Claim resolved.
    pub fn claim_id(&self) -> &ClaimId {
        &self.claim_id
    }

    /// Computed state.
    pub fn state(&self) -> VerdictState {
        self.state
    }

    /// Whether a new verdict and transition were appended.
    pub fn changed(&self) -> bool {
        self.changed
    }

    /// Whether the projected lifecycle status was applied to the claim.
    pub fn lifecycle_applied(&self) -> bool {
        self.lifecycle_applied
    }

    /// Verdict appended by this call, when any.
    pub fn verdict_id(&self) -> Option<&VerdictId> {
        self.verdict_id.as_ref()
    }
}

/// Read-only stores the resolution consults beside the claim and verdict
/// stores.
#[derive(Clone, Copy, Debug)]
pub struct ResolutionInputs<'a> {
    verifications: &'a VerificationRecordStore,
    evidence: &'a EvidenceRecordStore,
    observations: &'a ObservationStore,
    sources: &'a SourceStore,
}

impl<'a> ResolutionInputs<'a> {
    /// Bundle the read-only stores.
    pub fn new(
        verifications: &'a VerificationRecordStore,
        evidence: &'a EvidenceRecordStore,
        observations: &'a ObservationStore,
        sources: &'a SourceStore,
    ) -> Self {
        Self {
            verifications,
            evidence,
            observations,
            sources,
        }
    }

    /// Whether a link source resolves to an observation bound to a registered
    /// source: an observation source directly, or an evidence record naming an
    /// `observation_id`. Claim sources never do.
    pub fn resolves_to_observation(&self, source: &ClaimLinkSource) -> bool {
        match source {
            ClaimLinkSource::Observation(observation_id) => self
                .observations
                .observation_by_id(observation_id)
                .is_some_and(|observation| {
                    self.sources
                        .current_source(observation.source_id())
                        .is_some()
                }),
            ClaimLinkSource::Evidence(evidence_id) => self
                .evidence
                .evidence_by_id(evidence_id)
                .and_then(|record| record.observation_id())
                .is_some_and(|observation_id| {
                    self.resolves_to_observation(&ClaimLinkSource::Observation(
                        observation_id.clone(),
                    ))
                }),
            ClaimLinkSource::Claim(_) => false,
        }
    }

    /// Whether a verification record examined an observation that is still
    /// bound to a registered source.
    pub fn verification_resolves_to_observation(&self, record: &VerificationRecord) -> bool {
        record
            .inputs()
            .observation_ids()
            .iter()
            .any(|observation_id| {
                self.observations
                    .observation_by_id(observation_id)
                    .is_some_and(|observation| {
                        self.sources
                            .current_source(observation.source_id())
                            .is_some()
                    })
            })
    }
}

/// Recorded downgrade of a verdict to `InsufficientEvidence` because no active
/// signal resolved to an observation bound to a source (ADR-0016 invariant).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReachabilityGap {
    claim_id: ClaimId,
    attempted_state: VerdictState,
    unreachable_sources: Vec<String>,
    stamp: BitemporalStamp,
}

impl ReachabilityGap {
    /// Claim whose verdict was downgraded.
    pub fn claim_id(&self) -> &ClaimId {
        &self.claim_id
    }

    /// State the signals would have produced without the gate.
    pub fn attempted_state(&self) -> VerdictState {
        self.attempted_state
    }

    /// Signal sources lacking an observation path, as `<kind>:<id>` tokens.
    pub fn unreachable_sources(&self) -> &[String] {
        &self.unreachable_sources
    }

    /// Bitemporal stamp of the resolution that recorded the gap.
    pub fn stamp(&self) -> &BitemporalStamp {
        &self.stamp
    }

    /// Render the gap as a typed validation finding targeting the claim.
    pub fn to_validation_record(&self) -> ValidationErrorRecord {
        ValidationErrorRecord::new(
            CLAIM_UNREACHABLE_EVIDENCE_CODE,
            ValidationErrorSeverity::Warning,
            format!(
                "claim {} verdict downgraded from {} to {}: no active signal resolves to an \
                 observation bound to a source (unreachable: {})",
                self.claim_id.as_str(),
                self.attempted_state.as_str(),
                VerdictState::InsufficientEvidence.as_str(),
                self.unreachable_sources.join(", ")
            ),
            ValidationTarget::claim(self.claim_id.as_str()),
        )
    }
}

/// Compute and record the verdict of a claim with deterministic-first
/// precedence.
///
/// Active inputs are the links targeting the claim that `stamp` covers (links
/// without a stamp are always active) and the verification records for the
/// claim known by `stamp.transaction_time`. The core policy:
///
/// - any active `Supersedes` link gives `Superseded`;
/// - otherwise support signals (`Supports` links, authoritative deterministic
///   `Pass` records) and refutation signals (`Refutes` or `Contradicts` links,
///   authoritative deterministic `Fail` records) give `Supported`, `Refuted`,
///   or `Mixed`;
/// - only the latest conclusive record of each deterministic verifier carries
///   authority; a newer version wins, then a later transaction time;
/// - non-deterministic records and every `Inconclusive` record are advisory and
///   have no verdict weight; opposing deterministic/advisory results are
///   retained as typed [`VerificationDisagreement`] findings;
/// - no signal gives `Unknown`;
/// - ADR-0016 gate: `Supported`, `Refuted`, and `Mixed` require at least one
///   active signal whose source resolves to an observation bound to a source
///   (see [`ResolutionInputs::resolves_to_observation`]); otherwise the verdict
///   is `InsufficientEvidence` and a [`ReachabilityGap`] is recorded, never a
///   silent promotion.
///
/// When the state differs from the current verdict, a verdict and a transition
/// are appended and the projected lifecycle status is applied to the claim if
/// the `ClaimStatus` matrix allows it. WS-D may add confidence aggregation and
/// actionability, but cannot bypass this deterministic authority boundary.
///
/// # Errors
///
/// [`GraphError::ClaimNotFound`] for an unknown claim; store errors otherwise.
pub fn resolve_claim_verdict(
    claims: &mut ClaimStore,
    verdicts: &mut VerdictStore,
    inputs: &ResolutionInputs<'_>,
    claim_id: &ClaimId,
    stamp: BitemporalStamp,
    policy_version: impl Into<String>,
) -> Result<ResolutionOutcome, GraphError> {
    claims.claim_by_id(claim_id)?;
    let policy_version = policy_version.into();
    let as_of = VerdictAsOf::new(stamp.valid_from.clone(), stamp.transaction_time.clone());

    let mut support = 0_usize;
    let mut refute = 0_usize;
    let mut superseded = false;
    for link in claims.links_active_at(claim_id, &as_of) {
        match link_signal(link.kind()) {
            LinkSignal::Support => support += 1,
            LinkSignal::Refute => refute += 1,
            LinkSignal::Supersede => superseded = true,
            LinkSignal::Neutral => {}
        }
    }

    // Resolve each verifier's append-only history to one current conclusive
    // record. A new version supersedes older logic; within one version, the
    // later transaction (then stable record id) supersedes an earlier run.
    let claim_records = inputs.verifications.records_for_claim(claim_id);
    let deterministic_records =
        authoritative_records(&claim_records, true, stamp.transaction_time.as_str());
    let advisory_records =
        authoritative_records(&claim_records, false, stamp.transaction_time.as_str());

    for record in &deterministic_records {
        match record.result {
            VerificationResult::Pass => support += 1,
            VerificationResult::Fail => refute += 1,
            VerificationResult::Inconclusive => {
                unreachable!("authoritative_records excludes inconclusive verification records")
            }
        }
    }
    let newest_record = deterministic_records
        .iter()
        .copied()
        .max_by(|left, right| verification_signal_order(left, right));

    // Advisory results never change support/refutation counts. When they
    // oppose a deterministic result, record the boundary decision once while
    // preserving both source records in the verification store.
    for deterministic in &deterministic_records {
        for advisory in &advisory_records {
            if !results_disagree(deterministic.result(), advisory.result())
                || verdicts.verification_disagreements.iter().any(|existing| {
                    existing.deterministic_record_id == deterministic.id
                        && existing.advisory_record_id == advisory.id
                })
            {
                continue;
            }
            verdicts
                .verification_disagreements
                .push(VerificationDisagreement {
                    claim_id: claim_id.clone(),
                    deterministic_record_id: deterministic.id.clone(),
                    advisory_record_id: advisory.id.clone(),
                    deterministic_result: deterministic.result,
                    advisory_result: advisory.result,
                    stamp: stamp.clone(),
                });
        }
    }

    let state = if superseded {
        VerdictState::Superseded
    } else {
        match (support > 0, refute > 0) {
            (true, true) => VerdictState::Mixed,
            (true, false) => VerdictState::Supported,
            (false, true) => VerdictState::Refuted,
            (false, false) => VerdictState::Unknown,
        }
    };

    // ADR-0016 gate (WS-A item 6): a state derived from support or refutation
    // signals needs at least one signal whose source resolves to an observation
    // bound to a source. Otherwise the verdict is downgraded to
    // InsufficientEvidence and the gap is recorded, never silently promoted.
    let gate = match state {
        VerdictState::Supported | VerdictState::Refuted | VerdictState::Mixed => {
            let signal_links: Vec<&ClaimLink> = claims
                .links_active_at(claim_id, &as_of)
                .into_iter()
                .filter(|link| {
                    matches!(
                        link_signal(link.kind()),
                        LinkSignal::Support | LinkSignal::Refute
                    )
                })
                .collect();
            if signal_links
                .iter()
                .any(|link| inputs.resolves_to_observation(link.source()))
                || deterministic_records
                    .iter()
                    .any(|record| inputs.verification_resolves_to_observation(record))
            {
                None
            } else {
                let mut unreachable: Vec<String> = signal_links
                    .iter()
                    .map(|link| {
                        format!("{}:{}", link.source().kind_token(), link.source().id_str())
                    })
                    .collect();
                unreachable.extend(
                    deterministic_records
                        .iter()
                        .map(|record| format!("verification:{}", record.id.as_str())),
                );
                unreachable.sort();
                unreachable.dedup();
                Some(ReachabilityGap {
                    claim_id: claim_id.clone(),
                    attempted_state: state,
                    unreachable_sources: unreachable,
                    stamp: stamp.clone(),
                })
            }
        }
        VerdictState::Contested
        | VerdictState::Unknown
        | VerdictState::InsufficientEvidence
        | VerdictState::Superseded => None,
    };
    let state = if gate.is_some() {
        VerdictState::InsufficientEvidence
    } else {
        state
    };

    let previous = verdicts.current_verdict(claim_id).map(Verdict::state);
    if previous == Some(state) {
        return Ok(ResolutionOutcome {
            claim_id: claim_id.clone(),
            state,
            changed: false,
            lifecycle_applied: false,
            verdict_id: None,
            reachability_gap: gate,
        });
    }

    let ordinal = verdicts.verdicts_for_claim(claim_id).len() + 1;
    let verdict_id = VerdictId::new(format!("verdict--{}--{ordinal}", claim_id.as_str()))?;
    let transition_id =
        StateTransitionId::new(format!("transition--{}--{ordinal}", claim_id.as_str()))?;

    let trigger = if superseded {
        TransitionTrigger::Supersession
    } else {
        match newest_record {
            Some(record) if record_is_latest_signal(record, claims, claim_id, &as_of) => {
                TransitionTrigger::VerificationRecord(record.id.clone())
            }
            _ => TransitionTrigger::ResolutionRun,
        }
    };

    verdicts.append_verdict(
        verdict_id.clone(),
        claim_id.clone(),
        state,
        BTreeMap::new(),
        policy_version,
        stamp.clone(),
    )?;
    verdicts.append_transition(StateTransition {
        id: transition_id,
        claim_id: claim_id.clone(),
        from_state: previous,
        to_state: state,
        trigger,
        superseding_verdict_id: Some(verdict_id.clone()),
        stamp,
    })?;

    if let Some(gap) = &gate {
        verdicts.reachability_gaps.push(gap.clone());
    }

    let lifecycle_applied =
        claims.apply_verdict_projection(claim_id, project_verdict_state(state, false))?;

    Ok(ResolutionOutcome {
        claim_id: claim_id.clone(),
        state,
        changed: true,
        lifecycle_applied,
        verdict_id: Some(verdict_id),
        reachability_gap: gate,
    })
}

/// Select one current conclusive result per verifier identifier.
fn authoritative_records<'a>(
    records: &[&'a VerificationRecord],
    deterministic: bool,
    system_time: &str,
) -> Vec<&'a VerificationRecord> {
    let mut selected: BTreeMap<&str, &VerificationRecord> = BTreeMap::new();
    for record in records.iter().copied().filter(|record| {
        record.deterministic == deterministic
            && record.result != VerificationResult::Inconclusive
            && record.stamp.transaction_time.as_str() <= system_time
    }) {
        selected
            .entry(record.verifier_id.as_str())
            .and_modify(|current| {
                if verification_authority_order(record, current).is_gt() {
                    *current = record;
                }
            })
            .or_insert(record);
    }
    selected.into_values().collect()
}

/// Authority order inside one verifier history: implementation version first,
/// then transaction time and stable record identifier.
fn verification_authority_order(
    left: &VerificationRecord,
    right: &VerificationRecord,
) -> std::cmp::Ordering {
    left.verifier_version
        .cmp(&right.verifier_version)
        .then_with(|| {
            left.stamp
                .transaction_time
                .as_str()
                .cmp(right.stamp.transaction_time.as_str())
        })
        .then_with(|| left.id.as_str().cmp(right.id.as_str()))
}

/// Stable recency order for choosing the transition trigger across verifiers.
fn verification_signal_order(
    left: &VerificationRecord,
    right: &VerificationRecord,
) -> std::cmp::Ordering {
    left.stamp
        .transaction_time
        .as_str()
        .cmp(right.stamp.transaction_time.as_str())
        .then_with(|| left.verifier_version.cmp(&right.verifier_version))
        .then_with(|| left.verifier_id.cmp(&right.verifier_id))
        .then_with(|| left.id.as_str().cmp(right.id.as_str()))
}

/// Only opposing conclusive results form an authority disagreement.
fn results_disagree(left: VerificationResult, right: VerificationResult) -> bool {
    matches!(
        (left, right),
        (VerificationResult::Pass, VerificationResult::Fail)
            | (VerificationResult::Fail, VerificationResult::Pass)
    )
}

/// Whether `record` is at least as new as every active link carrying a
/// signal, so the transition can name it as the driving input.
fn record_is_latest_signal(
    record: &VerificationRecord,
    claims: &ClaimStore,
    claim_id: &ClaimId,
    as_of: &VerdictAsOf,
) -> bool {
    claims
        .links_active_at(claim_id, as_of)
        .into_iter()
        .filter(|link| link_signal(link.kind()) != LinkSignal::Neutral)
        .all(|link| {
            link.bitemporal().is_none_or(|stamp| {
                stamp.transaction_time.as_str() <= record.stamp.transaction_time.as_str()
            })
        })
}

/// Whether a link kind counts as support, refutation, supersession, or
/// neither in the core policy.
fn link_signal(kind: ClaimLinkKind) -> LinkSignal {
    match kind {
        ClaimLinkKind::Supports => LinkSignal::Support,
        ClaimLinkKind::Refutes | ClaimLinkKind::Contradicts => LinkSignal::Refute,
        ClaimLinkKind::Supersedes => LinkSignal::Supersede,
        ClaimLinkKind::ContextFor
        | ClaimLinkKind::Duplicates
        | ClaimLinkKind::DerivedFrom
        | ClaimLinkKind::DependsOn => LinkSignal::Neutral,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LinkSignal {
    Support,
    Refute,
    Supersede,
    Neutral,
}

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
//! Verification records, computed verdicts, and state transitions (Epic 0029,
//! WS-A item 5).
//!
//! Module boundary:
//! this module owns the three append-only governance records that make trusted
//! state a computed, replayable view: the `VerificationRecord` produced by one
//! verifier execution, the `Verdict` computed over active evidence links and
//! verification records, and the `StateTransition` appended on every verdict
//! change. It also owns the ADR-0016 projection from `VerdictState` to the
//! lifecycle `ClaimStatus` and the minimal WS-A resolution policy. It does not
//! define verifiers (WS-B), aggregation or confidence-dimension semantics
//! (WS-D), or reachability enforcement (WS-A item 6).
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
    BitemporalStamp, ClaimId, ClaimLinkKind, ClaimStatus, ClaimStore, Confidence, GraphError,
    ObservationId, StateTransitionId, TemporalTimestamp, VerdictId, VerificationRecordId,
};

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
}

/// Append-only store of verification records.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationRecordStore {
    records: Vec<VerificationRecord>,
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
            return Err(GraphError::InvalidPropertyValue(format!(
                "conflicting verification record for {}: records are append-only",
                record.id.as_str()
            )));
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
            return Err(GraphError::InvalidVersionState(format!(
                "verdict {} already exists; verdicts are append-only",
                id.as_str()
            )));
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
            return Err(GraphError::InvalidVersionState(format!(
                "state transition {} already exists; transitions are append-only",
                transition.id.as_str()
            )));
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
}

impl ResolutionOutcome {
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

/// Compute and record the verdict of a claim with the minimal WS-A policy.
///
/// Active inputs are the links targeting the claim that `stamp` covers (links
/// without a stamp are always active) and the verification records for the
/// claim known by `stamp.transaction_time`. The minimal policy:
///
/// - any active `Supersedes` link gives `Superseded`;
/// - otherwise support signals (`Supports` links, deterministic `Pass`
///   records) and refutation signals (`Refutes` or `Contradicts` links,
///   deterministic `Fail` records) give `Supported`, `Refuted`, or `Mixed`;
/// - no signal gives `Unknown`.
///
/// When the state differs from the current verdict, a verdict and a transition
/// are appended and the projected lifecycle status is applied to the claim if
/// the `ClaimStatus` matrix allows it. WS-D replaces the policy; WS-A item 6
/// adds the observation-reachability gate.
///
/// # Errors
///
/// [`GraphError::ClaimNotFound`] for an unknown claim; store errors otherwise.
pub fn resolve_claim_verdict(
    claims: &mut ClaimStore,
    verdicts: &mut VerdictStore,
    verifications: &VerificationRecordStore,
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

    // Deterministic verification records known by the as-of system time. The
    // newest one is remembered so the transition can name it when it drove the
    // change.
    let mut newest_record: Option<&VerificationRecord> = None;
    for record in verifications.records_for_claim(claim_id) {
        if record.stamp.transaction_time.as_str() > stamp.transaction_time.as_str()
            || !record.deterministic
        {
            continue;
        }
        match record.result {
            VerificationResult::Pass => support += 1,
            VerificationResult::Fail => refute += 1,
            VerificationResult::Inconclusive => continue,
        }
        if newest_record.is_none_or(|current| {
            record.stamp.transaction_time.as_str() >= current.stamp.transaction_time.as_str()
        }) {
            newest_record = Some(record);
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

    let previous = verdicts.current_verdict(claim_id).map(Verdict::state);
    if previous == Some(state) {
        return Ok(ResolutionOutcome {
            claim_id: claim_id.clone(),
            state,
            changed: false,
            lifecycle_applied: false,
            verdict_id: None,
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

    let lifecycle_applied =
        claims.apply_verdict_projection(claim_id, project_verdict_state(state, false))?;

    Ok(ResolutionOutcome {
        claim_id: claim_id.clone(),
        state,
        changed: true,
        lifecycle_applied,
        verdict_id: Some(verdict_id),
    })
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
/// neither in the minimal policy.
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

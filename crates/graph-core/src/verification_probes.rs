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
//! Verification probe generation and lifecycle (Epic 0019).
//!
//!
//!
//! - Generate the epic's four typed verification questions deterministically
//!   from validator findings, so every deferred judgment names exactly what
//!   must be checked and on which record.
//! - Keep generation idempotent per probe kind and target: repeated
//!   validation passes never flood the registry with duplicate open probes.
//! - Track an audited, append-only lifecycle — open, answered (supported or
//!   refuted), expired — with typed errors on terminal states; history is
//!   never rewritten.
//! - Link answered probes to the follow-up they justify (an audited
//!   promotion, a quarantine release, a repair), closing the loop with the
//!   immune response.
//!
//! # Finding-to-probe mapping (deterministic, closed)
//!
//! - `immune-epistemic--unsupported-claim` -> `StillSupported`;
//! - `immune-epistemic--stale-evidence` -> `StillSupported`;
//! - `immune-epistemic--source-circularity` -> `CircularDependency`;
//! - `immune-epistemic--open-contradiction` -> `IndependentSource`;
//! - `immune-epistemic--duplicate-suspect` (reserved for the future
//!   deduplication validator) -> `TrulyIdentical`;
//! - every other code generates nothing.

use serde::{Deserialize, Serialize};

use crate::{
    GraphError,
    validation::{ValidationErrorRecord, ValidationTarget},
};

/// One of the epic's four typed verification questions.
///
///
/// name the questions explicitly so probes are typed work items, not prose.
///
///
/// enumerate the four probe kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProbeKind {
    /// Is this relation or claim still supported?
    StillSupported,

    /// Are these two entities truly identical?
    TrulyIdentical,

    /// Does an independent source exist?
    IndependentSource,

    /// Does this path depend circularly on the same source?
    CircularDependency,
}

/// Answer of a verification probe.
///
///
/// keep probe outcomes binary and typed: the checked statement held or not.
///
///
/// enumerate the two answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProbeAnswer {
    /// Verification confirmed the checked statement.
    Supported,

    /// Verification refuted the checked statement.
    Refuted,
}

/// Lifecycle status of a verification probe.
///
///
/// make the probe lifecycle explicit: open probes await work, answered and
/// expired probes are terminal.
///
///
/// enumerate the states; `Answered` carries its answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProbeStatus {
    /// Awaiting verification.
    Open,

    /// Verification completed with an answer.
    Answered(ProbeAnswer),

    /// Verification abandoned without an answer.
    Expired,
}

/// One typed verification probe.
///
///
/// carry everything a verifier needs: the typed question, the record it
/// concerns, and the finding that motivated it — plus the follow-up the
/// answer later justifies.
///
///
/// hold the stable reference, kind, rendered question, originating finding,
/// target, status, justification linkage, and creation order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationProbe {
    /// Stable probe reference.
    pub probe_ref: String,

    /// Typed question kind.
    pub kind: ProbeKind,

    /// Rendered question naming the target.
    pub question: String,

    /// Stable code of the finding that generated the probe.
    pub finding_code: String,

    /// Record the question concerns.
    pub target: ValidationTarget,

    /// Current lifecycle status.
    pub status: ProbeStatus,

    /// Follow-up the answer justifies, when linked.
    pub justifies: Option<String>,

    /// Creation order in the registry.
    pub sequence: u64,
}

/// One audited lifecycle transition of a probe.
///
///
/// keep the lifecycle append-only and reviewable: every status change names
/// the probe, both states, and its order.
///
///
/// carry the transition context.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeTransition {
    /// Probe that transitioned.
    pub probe_ref: String,

    /// Status before the transition.
    pub from: ProbeStatus,

    /// Status after the transition.
    pub to: ProbeStatus,

    /// Monotonic order of the transition.
    pub sequence: u64,
}

/// Registry owning probes and their audited lifecycle.
///
///
/// centralize probe generation and lifecycle so verification work is one
/// ordered, reproducible queue with a complete transition log.
///
///
/// append probes in generation order and transitions in occurrence order.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeRegistry {
    probes: Vec<VerificationProbe>,
    lifecycle: Vec<ProbeTransition>,
    next_probe_sequence: u64,
    next_transition_sequence: u64,
}

impl ProbeRegistry {
    /// Create an empty registry.
    ///
    ///
    /// provide the stable constructor used before any generation.
    ///
    ///
    /// start with no probes and an empty lifecycle log.
    ///
    /// # Errors
    ///
    /// none expected because an empty registry has no external dependency.
    pub fn new() -> Self {
        Self::default()
    }

    /// Generate probes from validator findings.
    ///
    ///
    /// turn findings into typed verification work through the documented
    /// closed mapping, without duplicating open questions.
    ///
    ///
    /// map each finding in order; skip unmapped codes and findings whose kind
    /// and target already have an open probe; return the new probe references
    /// in generation order.
    ///
    /// # Errors
    ///
    /// none expected because generation only appends.
    pub fn generate_from_findings(&mut self, findings: &[ValidationErrorRecord]) -> Vec<String> {
        let mut generated = Vec::new();

        for finding in findings {
            let Some(kind) = probe_kind_for(finding.code()) else {
                continue;
            };
            let target = finding.target().clone();
            let already_open = self.probes.iter().any(|probe| {
                probe.kind == kind && probe.target == target && probe.status == ProbeStatus::Open
            });
            if already_open {
                continue;
            }

            let sequence = self.next_probe_sequence;
            self.next_probe_sequence += 1;
            let probe_ref = format!("probe--{sequence}");
            self.probes.push(VerificationProbe {
                probe_ref: probe_ref.clone(),
                kind,
                question: render_question(kind, &target),
                finding_code: finding.code().to_owned(),
                target,
                status: ProbeStatus::Open,
                justifies: None,
                sequence,
            });
            generated.push(probe_ref);
        }

        generated
    }

    /// Return one probe by reference.
    ///
    ///
    /// let verifiers and audits read a probe without scanning the queue.
    ///
    ///
    /// return the probe when it exists.
    ///
    /// # Errors
    ///
    /// none expected because absence is a deterministic `None`.
    pub fn probe(&self, probe_ref: &str) -> Option<&VerificationProbe> {
        self.probes
            .iter()
            .find(|probe| probe.probe_ref == probe_ref)
    }

    /// Return every probe in generation order.
    ///
    ///
    /// expose the ordered verification queue.
    ///
    ///
    /// return the append-only probe list.
    ///
    /// # Errors
    ///
    /// none expected because reading the queue cannot fail.
    pub fn probes(&self) -> &[VerificationProbe] {
        &self.probes
    }

    /// Return the audited lifecycle log in order.
    ///
    ///
    /// expose the complete transition history for review.
    ///
    ///
    /// return the append-only transition list.
    ///
    /// # Errors
    ///
    /// none expected because reading the log cannot fail.
    pub fn lifecycle(&self) -> &[ProbeTransition] {
        &self.lifecycle
    }

    /// Answer an open probe, optionally linking the follow-up it justifies.
    ///
    ///
    /// close the verification loop: the answer becomes the justification of
    /// the audited follow-up (promotion, quarantine release, repair).
    ///
    ///
    /// transition the probe from open to answered, store the justification
    /// reference, and log the transition.
    ///
    /// # Errors
    ///
    /// return `GraphError::InvalidProbeTransition` for unknown references or
    /// probes not open.
    pub fn answer(
        &mut self,
        probe_ref: &str,
        answer: ProbeAnswer,
        justifies: Option<String>,
    ) -> Result<(), GraphError> {
        let to = ProbeStatus::Answered(answer);
        self.transition(probe_ref, to, justifies)
    }

    /// Expire an open probe.
    ///
    ///
    /// abandon verification work explicitly instead of leaving stale open
    /// questions.
    ///
    ///
    /// transition the probe from open to expired and log the transition.
    ///
    /// # Errors
    ///
    /// return `GraphError::InvalidProbeTransition` for unknown references or
    /// probes not open.
    pub fn expire(&mut self, probe_ref: &str) -> Result<(), GraphError> {
        self.transition(probe_ref, ProbeStatus::Expired, None)
    }

    fn transition(
        &mut self,
        probe_ref: &str,
        to: ProbeStatus,
        justifies: Option<String>,
    ) -> Result<(), GraphError> {
        let sequence = self.next_transition_sequence;
        let probe = self
            .probes
            .iter_mut()
            .find(|probe| probe.probe_ref == probe_ref)
            .ok_or_else(|| {
                GraphError::InvalidProbeTransition(format!("unknown probe {probe_ref}"))
            })?;
        if probe.status != ProbeStatus::Open {
            return Err(GraphError::InvalidProbeTransition(format!(
                "probe {probe_ref} is terminal in status {:?}",
                probe.status
            )));
        }

        let from = probe.status;
        probe.status = to;
        probe.justifies = justifies;
        self.next_transition_sequence += 1;
        self.lifecycle.push(ProbeTransition {
            probe_ref: probe_ref.to_owned(),
            from,
            to,
            sequence,
        });
        Ok(())
    }
}

/// Map a finding code onto its probe kind through the documented closed table.
fn probe_kind_for(finding_code: &str) -> Option<ProbeKind> {
    match finding_code {
        "immune-epistemic--unsupported-claim" | "immune-epistemic--stale-evidence" => {
            Some(ProbeKind::StillSupported)
        }
        "immune-epistemic--source-circularity" => Some(ProbeKind::CircularDependency),
        "immune-epistemic--open-contradiction" => Some(ProbeKind::IndependentSource),
        "immune-epistemic--duplicate-suspect" => Some(ProbeKind::TrulyIdentical),
        _ => None,
    }
}

/// Render the typed question of a probe kind against its target.
fn render_question(kind: ProbeKind, target: &ValidationTarget) -> String {
    let target_ref = match target {
        ValidationTarget::Node(value)
        | ValidationTarget::Relationship(value)
        | ValidationTarget::Claim(value)
        | ValidationTarget::ExportRecord(value)
        | ValidationTarget::Retrieval(value)
        | ValidationTarget::Source(value)
        | ValidationTarget::Evidence(value) => value.as_str(),
    };

    match kind {
        ProbeKind::StillSupported => format!("Is {target_ref} still supported?"),
        ProbeKind::TrulyIdentical => {
            format!("Are the entities merged into {target_ref} truly identical?")
        }
        ProbeKind::IndependentSource => {
            format!("Does an independent source exist for {target_ref}?")
        }
        ProbeKind::CircularDependency => {
            format!("Does {target_ref} depend circularly on the same source?")
        }
    }
}

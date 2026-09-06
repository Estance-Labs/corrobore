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
//! Bundle of the Epic 0029 governed stores carried by a graph (WS-A item 7).
//!
//! Module boundary:
//! this module owns the `EpistemicStores` bundle that moves sources,
//! observations, entity mentions, reconciliation records, claims, verification records,
//! and verdicts together through
//! the graph, its persistence snapshot, and the durable store. It does not
//! define any of the records themselves.
//!
//! Compatibility targets:
//! - the bundle is skipped from serialization when empty, so persistence
//!   snapshots written before WS-A stay byte-identical;
//! - every store keeps its own serialization; the bundle adds no field of its
//!   own.
use serde::{Deserialize, Serialize};

use crate::{ClaimStore, ObservationStore, SourceStore, VerdictStore, VerificationRecordStore};

/// The governed evidence stores of one graph.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EpistemicStores {
    /// Human judgments, separate from machine evidence and verdicts.
    #[serde(default, skip_serializing_if = "crate::AnalystDecisionStore::is_empty")]
    pub analyst_decisions: crate::AnalystDecisionStore,
    /// Exact provenance associations for the claim audit read surface.
    #[serde(default, skip_serializing_if = "crate::ClaimAuditBindings::is_empty")]
    pub audit_bindings: crate::ClaimAuditBindings,
    /// Independent reference evaluations used to measure ingestion quality.
    #[serde(
        default,
        skip_serializing_if = "crate::IngestionEvaluationStore::is_empty"
    )]
    pub ingestion_evaluations: crate::IngestionEvaluationStore,
    /// Append-only reconciliation applications, dependencies and reversals.
    #[serde(default, skip_serializing_if = "crate::MergeStore::is_empty")]
    pub merges: crate::MergeStore,
    /// Evidence-cited reconciliation decisions, including abstentions.
    #[serde(default, skip_serializing_if = "crate::ReconciliationStore::is_empty")]
    pub reconciliations: crate::ReconciliationStore,
    /// Immutable observation-bound surface mentions.
    #[serde(default, skip_serializing_if = "crate::EntityMentionStore::is_empty")]
    pub mentions: crate::EntityMentionStore,
    /// Raw extraction proposals and audited canonical promotions.
    #[serde(default, skip_serializing_if = "crate::CandidateStore::is_empty")]
    pub candidates: crate::CandidateStore,
    /// Stable origin identities.
    #[serde(default, skip_serializing_if = "SourceStore::is_empty")]
    pub sources: SourceStore,
    /// Immutable observed spans, regions, and records.
    #[serde(default, skip_serializing_if = "ObservationStore::is_empty")]
    pub observations: ObservationStore,
    /// Claims, evidence links, stances, workspaces, trust inputs, policies,
    /// and explanations.
    #[serde(default, skip_serializing_if = "ClaimStore::is_empty")]
    pub claims: ClaimStore,
    /// Verifier executions.
    #[serde(default, skip_serializing_if = "VerificationRecordStore::is_empty")]
    pub verifications: VerificationRecordStore,
    /// Computed verdicts, state transitions, and reachability gaps.
    #[serde(default, skip_serializing_if = "VerdictStore::is_empty")]
    pub verdicts: VerdictStore,
}

impl EpistemicStores {
    /// Whether every store is empty.
    pub fn is_empty(&self) -> bool {
        self.analyst_decisions.is_empty()
            && self.audit_bindings.is_empty()
            && self.ingestion_evaluations.is_empty()
            && self.merges.is_empty()
            && self.reconciliations.is_empty()
            && self.mentions.is_empty()
            && self.candidates.is_empty()
            && self.sources.is_empty()
            && self.observations.is_empty()
            && self.claims.is_empty()
            && self.verifications.is_empty()
            && self.verdicts.is_empty()
    }
}

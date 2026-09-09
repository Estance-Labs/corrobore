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
//! verdicts, narratives, and campaigns together through
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

/// Bundle shape that can hold a content handle or a retained storage policy.
pub const EPISTEMIC_SCHEMA_V2: &str = "corrobore-epistemic-v2";

/// The governed evidence stores of one graph.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EpistemicStores {
    /// Investigation artifacts bound to governed records by identity.
    #[serde(default, skip_serializing_if = "crate::ArtifactStore::is_empty")]
    pub artifacts: crate::ArtifactStore,
    /// How each claim type could be re-checked against the world.
    #[serde(
        default,
        skip_serializing_if = "crate::CorrectiveRouteCatalogue::is_empty"
    )]
    pub corrective_routes: crate::CorrectiveRouteCatalogue,
    /// What would change a claim's state, so nothing is permanently closed.
    #[serde(default, skip_serializing_if = "crate::FalsifierStore::is_empty")]
    pub falsifiers: crate::FalsifierStore,
    /// Neutral append-only narrative and campaign collections.
    #[serde(
        default,
        skip_serializing_if = "crate::NarrativeCampaignStore::is_empty"
    )]
    pub narrative_campaigns: crate::NarrativeCampaignStore,
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
    /// Shape this bundle actually holds, when it is newer than the original.
    ///
    /// `None` means every observation is inline text no policy placed, which is
    /// byte for byte what stores written before the content plane contain, so
    /// they must not start announcing a shape they do not hold.
    pub fn declared_schema(&self) -> Option<&'static str> {
        let observation_holds_new = self.observations.observations().iter().any(|observation| {
            observation.content_policy().is_some()
                || matches!(observation.content(), crate::ContentHandle::External(_))
        });
        let source_holds_new = self.sources.source_ids().into_iter().any(|id| {
            self.sources
                .source_versions(id)
                .iter()
                .any(|source| source.artifact_content_policy().is_some())
        });
        (observation_holds_new || source_holds_new).then_some(EPISTEMIC_SCHEMA_V2)
    }

    /// Whether every store is empty.
    pub fn is_empty(&self) -> bool {
        self.artifacts.is_empty()
            && self.corrective_routes.is_empty()
            && self.falsifiers.is_empty()
            && self.narrative_campaigns.is_empty()
            && self.analyst_decisions.is_empty()
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

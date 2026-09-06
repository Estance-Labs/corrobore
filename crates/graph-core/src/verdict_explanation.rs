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
//! Read-only, self-contained explanation of a retained verdict snapshot.
use crate::{ClusterWeight, ConfidenceDimensions, DependencyReason, Verdict};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
/// Distinct uncertainty causes; absence means no classified uncertainty.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UncertaintyKind {
    /// No usable evidence is recorded.
    Ignorance,
    /// Evidence supports more than one reading.
    Ambiguity,
    /// Active support and refutation coexist.
    UnresolvedConflict,
    /// Recorded evidence lies outside temporal validity.
    Staleness,
}
/// A member retains both its store position and stable input reference.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExplainedMember {
    link_index: usize,
    reference: Option<String>,
}
impl ExplainedMember {
    /// Stable link reference; absent on historical records without captured refs.
    pub fn reference(&self) -> Option<&str> {
        self.reference.as_deref()
    }
    /// Position in the append-only claim-link store.
    pub fn link_index(&self) -> usize {
        self.link_index
    }
}
/// Membership, dependency reasons and directional weights from one cluster.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExplainedCluster {
    cluster_id: String,
    members: Vec<ExplainedMember>,
    reasons: Vec<DependencyReason>,
    support: Option<ClusterWeight>,
    refutation: Option<ClusterWeight>,
}
impl ExplainedCluster {
    /// Cluster identifier.
    pub fn cluster_id(&self) -> &str {
        &self.cluster_id
    }
    /// Every member, including unweighted ones.
    pub fn members(&self) -> &[ExplainedMember] {
        &self.members
    }
    /// Dependency reasons used to group the members.
    pub fn reasons(&self) -> &[DependencyReason] {
        &self.reasons
    }
    /// Supporting weight, including strength, authority and increment.
    pub fn support(&self) -> Option<&ClusterWeight> {
        self.support.as_ref()
    }
    /// Refuting weight, absent when unscored.
    pub fn refutation(&self) -> Option<&ClusterWeight> {
        self.refutation.as_ref()
    }
}
/// Explanation derived exclusively from the stored snapshot, never live retrieval.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VerdictExplanation {
    verdict_id: crate::VerdictId,
    claim_id: crate::ClaimId,
    state: crate::VerdictState,
    authority_resolution: Option<crate::SourceAuthorityResolution>,
    policy_version: String,
    stamp: crate::BitemporalStamp,
    dimensions: ConfidenceDimensions,
    uncertainty_kind: Option<UncertaintyKind>,
    clusters: Vec<ExplainedCluster>,
    actionability: Option<crate::ActionabilityAssessment>,
    hypotheses: Option<crate::HypothesisSet>,
}
impl VerdictExplanation {
    /// Primary uncertainty, prioritizing conflict, then staleness and ambiguity.
    pub fn uncertainty_kind(&self) -> Option<UncertaintyKind> {
        self.uncertainty_kind
    }
    /// Exact cluster structure and weights captured by the verdict.
    pub fn clusters(&self) -> &[ExplainedCluster] {
        &self.clusters
    }
    /// Named dimensions, retaining explicit absence.
    pub fn dimensions(&self) -> &ConfidenceDimensions {
        &self.dimensions
    }
    pub(crate) fn from_verdict(verdict: &Verdict, references: &BTreeMap<usize, String>) -> Self {
        // Combine stored membership with stored weights; never rerun aggregation.
        // Distinguish empty evidence, multiple viable readings, directional
        // conflict and stale evidence. Historical missing references stay absent.
        let dimensions = verdict.confidence_dimensions();
        let positive_support = dimensions
            .evidence_sufficiency
            .is_some_and(|v| v.value() > 0.0);
        let conflict = verdict.state() == crate::VerdictState::Mixed
            || (positive_support
                && dimensions
                    .contradiction_load
                    .is_some_and(|v| v.value() > 0.0));
        let viable = verdict.hypothesis_set().map_or(0, |set| {
            set.hypotheses()
                .iter()
                .filter(|h| {
                    matches!(
                        h.state(),
                        crate::VerdictState::Supported | crate::VerdictState::Mixed
                    ) && h.score().is_some_and(|s| s.value() > 0.0)
                })
                .count()
        });
        let uncertainty_kind = if conflict {
            Some(UncertaintyKind::UnresolvedConflict)
        } else if dimensions
            .temporal_validity
            .is_some_and(|v| v.value() == 0.0)
        {
            Some(UncertaintyKind::Staleness)
        } else if viable > 1 {
            Some(UncertaintyKind::Ambiguity)
        } else if verdict.state() == crate::VerdictState::Unknown {
            Some(UncertaintyKind::Ignorance)
        } else {
            None
        };
        let clusters = verdict
            .source_independence()
            .map_or_else(Vec::new, |structure| {
                structure
                    .clusters()
                    .iter()
                    .map(|cluster| {
                        let weight = verdict.cluster_aggregation().and_then(|report| {
                            report
                                .clusters()
                                .iter()
                                .find(|weight| weight.cluster_id() == cluster.id())
                        });
                        ExplainedCluster {
                            cluster_id: cluster.id().to_owned(),
                            members: cluster
                                .members()
                                .iter()
                                .map(|&index| ExplainedMember {
                                    link_index: index,
                                    reference: references.get(&index).cloned(),
                                })
                                .collect(),
                            reasons: cluster.reasons().to_vec(),
                            support: weight.and_then(|w| w.support()).cloned(),
                            refutation: weight.and_then(|w| w.refutation()).cloned(),
                        }
                    })
                    .collect()
            });
        Self {
            verdict_id: verdict.id().clone(),
            claim_id: verdict.claim_id().clone(),
            state: verdict.state(),
            authority_resolution: verdict.authority_resolution().cloned(),
            policy_version: verdict.policy_version().to_owned(),
            stamp: verdict.stamp().clone(),
            dimensions: verdict.confidence_dimensions().clone(),
            uncertainty_kind,
            clusters,
            actionability: verdict.actionability().cloned(),
            hypotheses: verdict.hypothesis_set().cloned(),
        }
    }
}

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
//! Bounded aggregation of independence components, never scalar claim confidence.
use crate::{ClaimStore, Confidence, ConfidenceDimensions, SourceIndependence};
use serde::{Deserialize, Serialize};
/// Current scoring policy. Historical policy strings retain their legacy behavior.
pub const CLUSTER_AGGREGATION_POLICY_VERSION: &str = "ws-d-cluster-v1";
/// Maximum extra strength from repetition inside one dependency component.
pub const WITHIN_CLUSTER_INCREMENT_CAP: f64 = 0.01;
/// Default policy for new verdict resolutions.
pub const DEFAULT_VERDICT_POLICY_VERSION: &str = CLUSTER_AGGREGATION_POLICY_VERSION;
/// One direction's explained contribution, with no implicit inputs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClusterWeight {
    best_strength: Confidence,
    contributing_members: usize,
    within_cluster_increment: Confidence,
    authority: Confidence,
    contribution: Confidence,
}
/// One component's support and refutation contributions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClusterContribution {
    cluster_id: String,
    support: Option<ClusterWeight>,
    refutation: Option<ClusterWeight>,
}
/// Persisted audit of scores obtained from the resolved dependency components.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClusterAggregation {
    clusters: Vec<ClusterContribution>,
    support: Option<Confidence>,
    refutation: Option<Confidence>,
}
impl ClusterAggregation {
    /// Explained contributions, one entry per component.
    pub fn clusters(&self) -> &[ClusterContribution] {
        &self.clusters
    }
}
use crate::{
    ClaimId, ClaimLink, ClaimLinkKind, DependencySignal, GraphError, SourceAuthorityResolution,
    VerdictAsOf,
};
impl ClusterWeight {
    /// Maximum strength among members with explicit positive strength and authority.
    pub fn best_strength(&self) -> Confidence {
        self.best_strength
    }
    /// Number of positive-strength members eligible to corroborate.
    pub fn contributing_members(&self) -> usize {
        self.contributing_members
    }
    /// Headroom-scaled concave increment, always at most the policy cap.
    pub fn within_cluster_increment(&self) -> Confidence {
        self.within_cluster_increment
    }
    /// Maximum explicit authority among the eligible members of this direction.
    pub fn authority(&self) -> Confidence {
        self.authority
    }
    /// `(best_strength + increment) * authority` for this component and direction.
    pub fn contribution(&self) -> Confidence {
        self.contribution
    }
}
impl ClusterContribution {
    /// Identifier shared with the verdict's independence structure.
    pub fn cluster_id(&self) -> &str {
        &self.cluster_id
    }
    /// Supporting contribution, absent without explicit member inputs.
    pub fn support(&self) -> Option<&ClusterWeight> {
        self.support.as_ref()
    }
    /// Refuting contribution, absent without explicit member inputs.
    pub fn refutation(&self) -> Option<&ClusterWeight> {
        self.refutation.as_ref()
    }
}
impl ClusterAggregation {
    /// Bounded supporting mass, combined once per component.
    pub fn support_score(&self) -> Option<Confidence> {
        self.support
    }
    /// Bounded refuting mass, combined once per component.
    pub fn refutation_score(&self) -> Option<Confidence> {
        self.refutation
    }
}
pub(crate) struct VerificationAggregationInput {
    pub has_records: bool,
    pub has_deterministic: bool,
    pub failed: bool,
}
fn bounded(value: f64) -> Confidence {
    Confidence::new(value).expect("bounded aggregation arithmetic")
}
fn channel<'a>(
    links: impl Iterator<Item = &'a ClaimLink>,
) -> Result<Option<ClusterWeight>, GraphError> {
    let mut best = 0.0_f64;
    let mut authority = 0.0_f64;
    let mut positive = 0;
    let mut known = false;
    for link in links {
        // Missing authority or strength cannot acquire a weight from another source.
        let Some((strength, weight)) = link.strength().zip(link.authority()) else {
            continue;
        };
        Confidence::new(strength.value())?;
        Confidence::new(weight.value())?;
        known = true;
        if strength.value() > 0.0 && weight.value() > 0.0 {
            positive += 1;
            best = best.max(strength.value());
            authority = authority.max(weight.value());
        }
    }
    if !known {
        return Ok(None);
    }
    // n -> 1 - 1/n is increasing and concave. The headroom factor keeps the
    // result bounded even at strength one; zero-strength members add nothing.
    let increment = if positive == 0 {
        0.0
    } else {
        WITHIN_CLUSTER_INCREMENT_CAP * (1.0 - best) * (1.0 - 1.0 / positive as f64)
    };
    Ok(Some(ClusterWeight {
        best_strength: bounded(best),
        contributing_members: positive,
        within_cluster_increment: bounded(increment),
        authority: bounded(authority),
        contribution: bounded((best + increment) * authority),
    }))
}
fn combine(values: impl Iterator<Item = Confidence>) -> Option<Confidence> {
    let mut remainder = 1.0;
    let mut known = false;
    for value in values {
        known = true;
        remainder *= 1.0 - value.value();
    }
    known.then(|| bounded(1.0 - remainder))
}
pub(crate) fn aggregate_components(
    claims: &ClaimStore,
    structure: &SourceIndependence,
    claim: &ClaimId,
    as_of: &VerdictAsOf,
    authority: Option<&SourceAuthorityResolution>,
    verification: VerificationAggregationInput,
) -> Result<(ClusterAggregation, ConfidenceDimensions), GraphError> {
    let mut clusters = Vec::new();
    for cluster in structure.clusters() {
        let links = || cluster.members().iter().map(|&i| &claims.claim_links()[i]);
        clusters.push(ClusterContribution {
            cluster_id: cluster.id().to_owned(),
            support: channel(links().filter(|l| l.kind() == ClaimLinkKind::Supports))?,
            refutation: channel(links().filter(|l| {
                matches!(
                    l.kind(),
                    ClaimLinkKind::Refutes | ClaimLinkKind::Contradicts
                )
            }))?,
        });
    }
    let support = combine(
        clusters
            .iter()
            .filter_map(|c| c.support.as_ref().map(|w| w.contribution)),
    );
    let refutation = combine(
        clusters
            .iter()
            .filter_map(|c| c.refutation.as_ref().map(|w| w.contribution)),
    );
    let known_supporting_clusters = structure
        .clusters()
        .iter()
        .filter(|cluster| {
            !cluster
                .reasons()
                .iter()
                .any(|r| r.signal() == DependencySignal::UnknownIndependence)
                && cluster
                    .members()
                    .iter()
                    .any(|&i| claims.claim_links()[i].kind() == ClaimLinkKind::Supports)
        })
        .count();
    let mut has_temporal = false;
    let mut has_active_temporal = false;
    for link in claims
        .claim_links()
        .iter()
        .filter(|l| l.target_claim_id() == claim && crate::source_authority::is_signal(l.kind()))
    {
        if let Some(stamp) = link.bitemporal()
            && stamp.transaction_time.as_str() <= as_of.system_time().as_str()
        {
            has_temporal = true;
            has_active_temporal |= link.is_active_at(as_of);
        }
    }
    let contradiction_load = if verification.failed {
        Some(bounded(1.0))
    } else if support.is_some() || refutation.is_some() {
        let s = support.map_or(0.0, Confidence::value);
        let r = refutation.map_or(0.0, Confidence::value);
        Some(bounded(if s + r == 0.0 { 0.0 } else { r / (s + r) }))
    } else {
        None
    };
    let dimensions = ConfidenceDimensions {
        evidence_sufficiency: support,
        source_authority: authority.and_then(SourceAuthorityResolution::dimension),
        source_independence: (known_supporting_clusters > 0).then(|| {
            bounded(known_supporting_clusters as f64 / (known_supporting_clusters as f64 + 1.0))
        }),
        temporal_validity: has_temporal
            .then(|| bounded(if has_active_temporal { 1.0 } else { 0.0 })),
        contradiction_load,
        verifier_strength: verification.has_records.then(|| {
            bounded(if verification.has_deterministic {
                1.0
            } else {
                0.0
            })
        }),
        ..Default::default()
    };
    Ok((
        ClusterAggregation {
            clusters,
            support,
            refutation,
        },
        dimensions,
    ))
}

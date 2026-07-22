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
//! Advanced loading policy contracts.
//!
//! This module defines prefetch, eviction, protection, and observability models
//! used by memory-aware traversal and working-set streaming policies.

use serde::{Deserialize, Serialize};

/// Typed prefetch decision outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PrefetchDecisionKind {
    /// Prefetched data was used.
    Hit,

    /// Prefetch did not provide needed data.
    Miss,

    /// Prefetch loaded data that was not used.
    Waste,

    /// Prefetch was skipped by policy.
    Skipped,
}

/// One prefetch decision event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PrefetchDecision {
    /// Kind.
    pub kind: PrefetchDecisionKind,
}

impl PrefetchDecision {
    /// Creates a new instance.
    pub fn new(kind: PrefetchDecisionKind) -> Self {
        Self { kind }
    }
}

/// Aggregate prefetch metrics.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrefetchMetrics {
    /// Hit count.
    pub hit_count: u64,
    /// Miss count.
    pub miss_count: u64,
    /// Waste count.
    pub waste_count: u64,
}

impl PrefetchMetrics {
    /// Build aggregate metrics from prefetch decisions.
    pub fn from_decisions(decisions: &[PrefetchDecision]) -> Self {
        let mut metrics = Self {
            hit_count: 0,
            miss_count: 0,
            waste_count: 0,
        };

        for decision in decisions {
            match decision.kind {
                PrefetchDecisionKind::Hit => metrics.hit_count += 1,
                PrefetchDecisionKind::Miss => metrics.miss_count += 1,
                PrefetchDecisionKind::Waste => metrics.waste_count += 1,
                PrefetchDecisionKind::Skipped => {}
            }
        }

        metrics
    }
}

/// Eviction policy family.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EvictionPolicyKind {
    /// Least recently used.
    LeastRecentlyUsed,
    /// First in first out.
    FirstInFirstOut,
    /// Priority score.
    PriorityScore,
}

/// Contract-level eviction policy configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvictionPolicy {
    /// Kind.
    pub kind: EvictionPolicyKind,
    /// Target memory budget bytes.
    pub target_memory_budget_bytes: u64,
}

impl EvictionPolicy {
    /// Creates a new instance.
    pub fn new(kind: EvictionPolicyKind, target_memory_budget_bytes: u64) -> Self {
        Self {
            kind,
            target_memory_budget_bytes,
        }
    }
}

/// Typed protection rules that can block eviction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EvictionProtectionRule {
    /// Pinned record.
    PinnedRecord,
    /// Dirty record.
    DirtyRecord,
}

/// Typed reason for an eviction decision outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EvictionDecisionReason {
    /// Policy accepted.
    PolicyAccepted,
    /// Policy rejected.
    PolicyRejected,
    /// Protected.
    Protected(EvictionProtectionRule),
}

/// One eviction decision outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EvictionDecision {
    /// Evict.
    pub evict: bool,
    /// Reason.
    pub reason: EvictionDecisionReason,
}

impl EvictionDecision {
    /// Build an accepted eviction decision.
    pub fn accept() -> Self {
        Self {
            // Evict.
            evict: true,
            // Reason.
            reason: EvictionDecisionReason::PolicyAccepted,
        }
    }

    /// Build a rejected eviction decision with explicit reason.
    pub fn reject(reason: EvictionDecisionReason) -> Self {
        Self {
            // Evict.
            evict: false,
            reason,
        }
    }
}

/// Working-set observability metrics for advanced loading flows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkingSetObservabilityMetrics {
    /// Memory bytes used.
    pub memory_bytes_used: u64,
    /// Memory budget bytes.
    pub memory_budget_bytes: u64,
    /// Hot node count.
    pub hot_node_count: u64,
    /// Warm node count.
    pub warm_node_count: u64,
    /// Hot relationship count.
    pub hot_relationship_count: u64,
    /// Warm relationship count.
    pub warm_relationship_count: u64,
    /// Page in hit count.
    pub page_in_hit_count: u64,
    /// Page in miss count.
    pub page_in_miss_count: u64,
    /// Prefetch.
    pub prefetch: PrefetchMetrics,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefetch_metrics_ignore_skipped_decisions() {
        let metrics = PrefetchMetrics::from_decisions(&[
            PrefetchDecision::new(PrefetchDecisionKind::Skipped),
            PrefetchDecision::new(PrefetchDecisionKind::Hit),
        ]);

        assert_eq!(metrics.hit_count, 1);
        assert_eq!(metrics.miss_count, 0);
        assert_eq!(metrics.waste_count, 0);
    }

    #[test]
    fn eviction_reject_preserves_reason() {
        let decision = EvictionDecision::reject(EvictionDecisionReason::Protected(
            EvictionProtectionRule::DirtyRecord,
        ));

        assert!(!decision.evict);
        assert_eq!(
            decision.reason,
            EvictionDecisionReason::Protected(EvictionProtectionRule::DirtyRecord)
        );
    }

    #[test]
    fn prefetch_metrics_count_hit_miss_and_waste() {
        let metrics = PrefetchMetrics::from_decisions(&[
            PrefetchDecision::new(PrefetchDecisionKind::Hit),
            PrefetchDecision::new(PrefetchDecisionKind::Miss),
            PrefetchDecision::new(PrefetchDecisionKind::Waste),
            PrefetchDecision::new(PrefetchDecisionKind::Miss),
        ]);

        assert_eq!(metrics.hit_count, 1);
        assert_eq!(metrics.miss_count, 2);
        assert_eq!(metrics.waste_count, 1);
    }

    #[test]
    fn eviction_accept_sets_expected_reason() {
        let decision = EvictionDecision::accept();

        assert!(decision.evict);
        assert_eq!(decision.reason, EvictionDecisionReason::PolicyAccepted);
    }

    #[test]
    fn eviction_policy_constructor_preserves_configuration() {
        let policy = EvictionPolicy::new(EvictionPolicyKind::PriorityScore, 65_536);

        assert_eq!(policy.kind, EvictionPolicyKind::PriorityScore);
        assert_eq!(policy.target_memory_budget_bytes, 65_536);
    }
}

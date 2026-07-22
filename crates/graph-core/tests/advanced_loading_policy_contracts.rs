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
use graph_core::{
    EvictionDecision, EvictionDecisionReason, EvictionPolicy, EvictionPolicyKind,
    EvictionProtectionRule, PrefetchDecision, PrefetchDecisionKind, PrefetchMetrics,
    WorkingSetObservabilityMetrics,
};

//
// Validate prefetch contracts expose explicit decision kinds and aggregate
// hit/miss/waste counters.
#[test]
fn prefetch_contract_exposes_decisions_and_metrics() {
    let decisions = vec![
        PrefetchDecision::new(PrefetchDecisionKind::Hit),
        PrefetchDecision::new(PrefetchDecisionKind::Miss),
        PrefetchDecision::new(PrefetchDecisionKind::Waste),
    ];

    let metrics = PrefetchMetrics::from_decisions(&decisions);

    assert_eq!(metrics.hit_count, 1);
    assert_eq!(metrics.miss_count, 1);
    assert_eq!(metrics.waste_count, 1);
}

//
// Validate eviction policy and decision contracts preserve typed outcomes and
// protection reasons for pinned/dirty records.
#[test]
fn eviction_contract_exposes_policy_and_protection_rules() {
    let policy = EvictionPolicy::new(EvictionPolicyKind::LeastRecentlyUsed, 512 * 1024 * 1024);
    assert_eq!(policy.kind, EvictionPolicyKind::LeastRecentlyUsed);
    assert_eq!(policy.target_memory_budget_bytes, 512 * 1024 * 1024);

    let pinned = EvictionDecision::reject(EvictionDecisionReason::Protected(
        EvictionProtectionRule::PinnedRecord,
    ));
    let dirty = EvictionDecision::reject(EvictionDecisionReason::Protected(
        EvictionProtectionRule::DirtyRecord,
    ));

    assert!(!pinned.evict);
    assert!(!dirty.evict);
}

//
// Validate observability metrics carry working-set and memory counters needed by
// advanced loading dashboards and planner diagnostics.
#[test]
fn observability_metrics_capture_memory_and_working_set_counters() {
    let metrics = WorkingSetObservabilityMetrics {
        memory_bytes_used: 256 * 1024 * 1024,
        memory_budget_bytes: 512 * 1024 * 1024,
        hot_node_count: 120,
        warm_node_count: 340,
        hot_relationship_count: 260,
        warm_relationship_count: 890,
        page_in_hit_count: 45,
        page_in_miss_count: 13,
        prefetch: PrefetchMetrics {
            hit_count: 19,
            miss_count: 7,
            waste_count: 3,
        },
    };

    assert_eq!(metrics.memory_bytes_used, 256 * 1024 * 1024);
    assert_eq!(metrics.memory_budget_bytes, 512 * 1024 * 1024);
    assert_eq!(metrics.hot_node_count, 120);
    assert_eq!(metrics.warm_node_count, 340);
    assert_eq!(metrics.page_in_hit_count, 45);
    assert_eq!(metrics.page_in_miss_count, 13);
    assert_eq!(metrics.prefetch.hit_count, 19);
}

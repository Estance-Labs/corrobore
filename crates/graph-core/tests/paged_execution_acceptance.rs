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
    EvictionDecision, EvictionDecisionReason, EvictionProtectionRule, ExecutionContinuation,
    ExecutionStatusCode, PageIdentity, PageIdentityKind, PageInAwareExecutionStatus, PageInRequest,
    PageInResult, PageInStatus, PrefetchDecision, PrefetchDecisionKind, PrefetchMetrics,
    StorageRef, TraversalBudgetDecision, TraversalCostBreakdown, TraversalCostEstimate,
    TraversalCostRejection, WorkingSetObservabilityMetrics,
};

//
// acceptance: query execution can request cold records through a
// page-in contract and represent continuation-aware partial execution.
#[test]
fn epic_0013_acceptance_page_in_and_partial_execution_flow() {
    let request = PageInRequest {
        page: PageIdentity {
            kind: PageIdentityKind::Adjacency,
            segment: "adjacency/outgoing".to_owned(),
            page_id: "node--incident-42/outgoing/page-2".to_owned(),
            storage_ref: Some(StorageRef::Page {
                segment: "adjacency/outgoing".to_owned(),
                page_id: 2,
            }),
        },
        record_refs: Vec::new(),
    };

    let page_in = PageInResult {
        request: request.clone(),
        status: PageInStatus::Loaded,
        loaded_record_refs: Vec::new(),
    };

    assert_eq!(page_in.status, PageInStatus::Loaded);
    assert_eq!(page_in.request, request);

    let status = PageInAwareExecutionStatus::new(
        ExecutionStatusCode::Partial,
        "Expansion paused at memory budget boundary.",
    )
    .with_page_in_status(PageInStatus::Loaded)
    .with_page(page_in.request.page)
    .with_continuation(ExecutionContinuation {
        token: "continue://ws-incident-42/hop-2".to_owned(),
        resume_from_hop: 2,
    })
    .with_fix_hint("Add relationship filters before resuming traversal.");

    assert_eq!(status.code(), ExecutionStatusCode::Partial);
    assert_eq!(status.page_in_status(), Some(PageInStatus::Loaded));
    assert!(status.continuation().is_some());
    assert!(status.fix_hint().is_some());
}

//
// acceptance: planner cost model can reject overbroad traversals with
// deterministic narrowing hints.
#[test]
fn epic_0013_acceptance_memory_aware_cost_rejection_flow() {
    let estimate = TraversalCostEstimate::new(TraversalCostBreakdown::new(300, 200, 150), 500)
        .with_rejection(TraversalCostRejection::new(
            "Estimated traversal cost exceeds configured memory budget.",
            "Reduce hop limit and add label filters.",
        ));

    assert_eq!(estimate.total_cost(), 650);
    assert_eq!(estimate.budget_limit(), 500);
    assert_eq!(estimate.budget_decision(), TraversalBudgetDecision::Reject);
    assert!(matches!(
    estimate.rejection(),
    Some(rejection)
    if rejection.reason() == "Estimated traversal cost exceeds configured memory budget."
    && rejection.fix_hint() == "Reduce hop limit and add label filters."
    ));
}

//
// acceptance: prefetch metrics, eviction protections, and observability
// counters remain inspectable through typed contract models.
#[test]
fn epic_0013_acceptance_prefetch_eviction_and_observability_flow() {
    let prefetch_decisions = vec![
        PrefetchDecision::new(PrefetchDecisionKind::Hit),
        PrefetchDecision::new(PrefetchDecisionKind::Hit),
        PrefetchDecision::new(PrefetchDecisionKind::Miss),
        PrefetchDecision::new(PrefetchDecisionKind::Waste),
    ];
    let prefetch = PrefetchMetrics::from_decisions(&prefetch_decisions);

    let pinned_evict = EvictionDecision::reject(EvictionDecisionReason::Protected(
        EvictionProtectionRule::PinnedRecord,
    ));
    let dirty_evict = EvictionDecision::reject(EvictionDecisionReason::Protected(
        EvictionProtectionRule::DirtyRecord,
    ));

    assert!(!pinned_evict.evict);
    assert!(!dirty_evict.evict);

    let metrics = WorkingSetObservabilityMetrics {
        memory_bytes_used: 400,
        memory_budget_bytes: 500,
        hot_node_count: 80,
        warm_node_count: 240,
        hot_relationship_count: 120,
        warm_relationship_count: 520,
        page_in_hit_count: 24,
        page_in_miss_count: 6,
        prefetch,
    };

    assert_eq!(metrics.prefetch.hit_count, 2);
    assert_eq!(metrics.prefetch.miss_count, 1);
    assert_eq!(metrics.prefetch.waste_count, 1);
    assert!(metrics.memory_bytes_used <= metrics.memory_budget_bytes);
}

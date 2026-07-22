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
    TraversalBudgetDecision, TraversalCostBreakdown, TraversalCostEstimate, TraversalCostRejection,
};

//
// Validate the traversal cost contract exposes explicit component costs for
// degree expansion, payload loading, and page-in behavior.
#[test]
fn traversal_cost_breakdown_is_explicit_and_matchable() {
    let breakdown = TraversalCostBreakdown::new(120, 48, 16);

    assert_eq!(breakdown.degree_cost(), 120);
    assert_eq!(breakdown.payload_cost(), 48);
    assert_eq!(breakdown.page_in_cost(), 16);
    assert_eq!(breakdown.total_cost(), 184);
}

//
// Validate planners can obtain deterministic total estimates and bounded
// decision outcomes from the traversal cost model.
#[test]
fn traversal_cost_estimate_produces_deterministic_budget_decisions() {
    let estimate = TraversalCostEstimate::new(TraversalCostBreakdown::new(60, 20, 10), 100);

    assert_eq!(estimate.total_cost(), 90);
    assert_eq!(estimate.budget_limit(), 100);
    assert_eq!(estimate.budget_decision(), TraversalBudgetDecision::Accept);

    let rejected = TraversalCostEstimate::new(TraversalCostBreakdown::new(80, 40, 20), 100)
        .with_rejection(TraversalCostRejection::new(
            "Estimated traversal cost exceeds configured budget.",
            "Add relationship filters and lower hop limit.",
        ));

    assert_eq!(rejected.total_cost(), 140);
    assert_eq!(rejected.budget_decision(), TraversalBudgetDecision::Reject);
    assert!(rejected.rejection().is_some());
}

//
// Validate rejection payloads preserve deterministic narrowing hints for safe
// planner feedback when traversal is overbroad.
#[test]
fn traversal_cost_rejection_preserves_narrowing_hints() {
    let rejection = TraversalCostRejection::new(
        "Traversal estimate is over budget.",
        "Add explicit limit and time window constraints.",
    );

    assert_eq!(rejection.reason(), "Traversal estimate is over budget.");
    assert_eq!(
        rejection.fix_hint(),
        "Add explicit limit and time window constraints."
    );
}

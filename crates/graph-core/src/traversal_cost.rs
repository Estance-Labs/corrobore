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
//! Memory-aware traversal cost model contracts.
//!
//! This module defines planner-facing cost model payloads for degree expansion,
//! payload loading, and page-in estimates.

use serde::{Deserialize, Serialize};

/// Budget decision derived from traversal cost and configured limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TraversalBudgetDecision {
    /// Estimated cost is within the configured budget.
    Accept,

    /// Estimated cost exceeds configured budget.
    Reject,
}

/// Explicit component breakdown for traversal cost estimation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraversalCostBreakdown {
    degree_cost: u64,
    payload_cost: u64,
    page_in_cost: u64,
}

impl TraversalCostBreakdown {
    /// Build a traversal cost breakdown from explicit component costs.
    pub fn new(degree_cost: u64, payload_cost: u64, page_in_cost: u64) -> Self {
        Self {
            degree_cost,
            payload_cost,
            page_in_cost,
        }
    }

    /// Return degree-driven traversal expansion cost.
    pub fn degree_cost(&self) -> u64 {
        self.degree_cost
    }

    /// Return payload loading cost.
    pub fn payload_cost(&self) -> u64 {
        self.payload_cost
    }

    /// Return page-in specific cost.
    pub fn page_in_cost(&self) -> u64 {
        self.page_in_cost
    }

    /// Return deterministic total cost from all components.
    pub fn total_cost(&self) -> u64 {
        self.degree_cost
            .saturating_add(self.payload_cost)
            .saturating_add(self.page_in_cost)
    }
}

/// Structured rejection payload for over-budget traversal estimates.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraversalCostRejection {
    reason: String,
    fix_hint: String,
}

impl TraversalCostRejection {
    /// Build a deterministic rejection payload.
    pub fn new(reason: impl Into<String>, fix_hint: impl Into<String>) -> Self {
        Self {
            // Reason.
            reason: reason.into(),
            // Fix hint.
            fix_hint: fix_hint.into(),
        }
    }

    /// Return rejection reason.
    pub fn reason(&self) -> &str {
        self.reason.as_str()
    }

    /// Return deterministic narrowing hint.
    pub fn fix_hint(&self) -> &str {
        self.fix_hint.as_str()
    }
}

/// Planner-facing traversal cost estimate payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraversalCostEstimate {
    breakdown: TraversalCostBreakdown,
    budget_limit: u64,
    rejection: Option<TraversalCostRejection>,
}

impl TraversalCostEstimate {
    /// Build a deterministic traversal cost estimate.
    pub fn new(breakdown: TraversalCostBreakdown, budget_limit: u64) -> Self {
        Self {
            breakdown,
            budget_limit,
            // Rejection.
            rejection: None,
        }
    }

    /// Return a copy with explicit rejection details.
    pub fn with_rejection(mut self, rejection: TraversalCostRejection) -> Self {
        self.rejection = Some(rejection);
        self
    }

    /// Return component breakdown.
    pub fn breakdown(&self) -> &TraversalCostBreakdown {
        &self.breakdown
    }

    /// Return deterministic total traversal cost.
    pub fn total_cost(&self) -> u64 {
        self.breakdown.total_cost()
    }

    /// Return configured budget limit.
    pub fn budget_limit(&self) -> u64 {
        self.budget_limit
    }

    /// Return budget decision based on total cost and configured limit.
    pub fn budget_decision(&self) -> TraversalBudgetDecision {
        if self.total_cost() <= self.budget_limit {
            TraversalBudgetDecision::Accept
        } else {
            TraversalBudgetDecision::Reject
        }
    }

    /// Return optional rejection details.
    pub fn rejection(&self) -> Option<&TraversalCostRejection> {
        self.rejection.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn total_cost_uses_saturating_addition() {
        let breakdown = TraversalCostBreakdown::new(u64::MAX, 100, 100);
        assert_eq!(breakdown.total_cost(), u64::MAX);
    }

    #[test]
    fn budget_decision_rejects_when_total_exceeds_limit() {
        let estimate = TraversalCostEstimate::new(TraversalCostBreakdown::new(100, 50, 25), 150);
        assert_eq!(estimate.budget_decision(), TraversalBudgetDecision::Reject);
    }
}

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
    ExpansionBudget, ExpansionBudgetUsage, ExpansionLimit, ExpansionSafetyErrorCode, GraphError,
    SupernodePolicy,
};

fn acceptance_budget() -> ExpansionBudget {
    ExpansionBudget {
        max_loaded_node_count: 10,
        max_loaded_relationship_count: 20,
        max_hot_node_count: 5,
        max_hot_relationship_count: 8,
        max_warm_adjacency_entry_count: 30,
        max_hop_count: 2,
        max_supernode_expansion_count: 3,
        max_payload_byte_count: 4_096,
        max_execution_time_ms: 250,
    }
}

fn usage_inside_acceptance_budget() -> ExpansionBudgetUsage {
    ExpansionBudgetUsage {
        loaded_node_count: 10,
        loaded_relationship_count: 20,
        hot_node_count: 5,
        hot_relationship_count: 8,
        warm_adjacency_entry_count: 30,
        hop_count: 2,
        supernode_expansion_count: 3,
        payload_byte_count: 4_096,
        execution_time_ms: 250,
    }
}

fn strict_supernode_policy() -> SupernodePolicy {
    SupernodePolicy {
        degree_threshold: 100,
        require_relationship_filter: true,
        require_label_filter: true,
        require_time_window: true,
        require_limit: true,
    }
}

//
// Validate acceptance from the public `graph_core` facade, not from the
// private module layout.
//
// Given a public `ExpansionBudget` and usage at each configured limit,
// when the usage is checked,
// then the model should accept the bounded expansion without requiring traversal,
// storage, Cypher planning, page-in, or prefetch behavior.
#[test]
fn public_expansion_budget_accepts_usage_at_configured_limits() {
    let budget = acceptance_budget();
    let usage = usage_inside_acceptance_budget();

    assert_eq!(budget.check_usage(&usage), Ok(()));
}

//
// Validate that budget-exceeded errors are actionable and machine-readable from
// the public API boundary.
//
// Given a usage snapshot that exceeds the hot relationship limit,
// when the usage is checked,
// then the returned error should carry `EXPANSION_BUDGET_EXCEEDED`, the consumed
// value, the configured limit, and an actionable fix hint.
#[test]
fn public_expansion_budget_reports_consumed_limit_and_fix_hint() {
    let budget = acceptance_budget();
    let usage = ExpansionBudgetUsage {
        hot_relationship_count: 9,
        ..usage_inside_acceptance_budget()
    };

    let error = budget
        .check_usage(&usage)
        .expect_err("hot relationship usage should exceed the acceptance budget");

    assert_eq!(
        error.code,
        ExpansionSafetyErrorCode::ExpansionBudgetExceeded
    );
    assert_eq!(error.limit, ExpansionLimit::HotRelationshipCount);
    assert_eq!(error.allowed, 8);
    assert_eq!(error.consumed, 9);
    assert!(error.fix_hint.contains("LIMIT") || error.fix_hint.contains("filter"));
}

//
// Validate that supernode policy remains separate from generic budget checking.
//
// Given a policy threshold and a usage snapshot inside the generic budget,
// when a high-degree node is evaluated without required guards,
// then only the supernode policy should block expansion with
// `SUPERNODE_EXPANSION_BLOCKED`.
#[test]
fn public_supernode_policy_blocks_unguarded_high_degree_expansion_separately() {
    let budget = acceptance_budget();
    let usage = usage_inside_acceptance_budget();
    let policy = strict_supernode_policy();

    assert_eq!(budget.check_usage(&usage), Ok(()));
    assert!(policy.is_high_degree_node(100));

    let error = policy
        .validate_expansion_guards(100, false, false, false, false)
        .expect_err("unguarded high-degree node should be blocked by supernode policy");

    assert_eq!(
        error.code,
        ExpansionSafetyErrorCode::SupernodeExpansionBlocked
    );
    assert_eq!(error.observed_degree, 100);
    assert_eq!(error.degree_threshold, 100);
    assert!(error.relationship_filter_required);
    assert!(error.label_filter_required);
    assert!(error.time_window_required);
    assert!(error.limit_required);
    assert!(error.fix_hint.contains("LIMIT"));
}

//
// Validate that expansion safety payloads integrate with the public `GraphError`
// boundary without requiring string parsing.
//
// Given public budget and supernode policy failures,
// when they are wrapped in `GraphError`,
// then callers should be able to match the typed variants directly.
#[test]
fn public_graph_error_wraps_expansion_safety_payloads() {
    let budget_error = acceptance_budget()
        .check_usage(&ExpansionBudgetUsage {
            loaded_node_count: 11,
            ..usage_inside_acceptance_budget()
        })
        .expect_err("loaded nodes should exceed the acceptance budget");

    let supernode_error = strict_supernode_policy()
        .validate_expansion_guards(101, false, false, false, false)
        .expect_err("unguarded high-degree expansion should be blocked");

    assert!(matches!(
    GraphError::ExpansionBudgetExceeded(budget_error),
    GraphError::ExpansionBudgetExceeded(payload)
    if payload.code == ExpansionSafetyErrorCode::ExpansionBudgetExceeded
    && payload.limit == ExpansionLimit::LoadedNodeCount
    && payload.consumed == 11
    ));

    assert!(matches!(
    GraphError::SupernodeExpansionBlocked(supernode_error),
    GraphError::SupernodeExpansionBlocked(payload)
    if payload.code == ExpansionSafetyErrorCode::SupernodeExpansionBlocked
    && payload.observed_degree == 101
    && payload.degree_threshold == 100
    ));
}

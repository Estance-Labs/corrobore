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
//! Expansion budget and supernode policy contracts.
//!
//! This module belongs to Graph Working Set Streaming. It defines the
//! typed safety model only: no traversal, no Cypher planning, no page-in, no
//! prefetch, and no storage coupling.
//!
//! The implemented behavior is limited to pure contract checks. Runtime graph
//! expansion remains owned by later working-set traversal issues.

use serde::{Deserialize, Serialize};

/// Explicit limits used to keep graph expansion bounded and explainable.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpansionBudget {
    /// Max loaded node count.
    pub max_loaded_node_count: u64,
    /// Max loaded relationship count.
    pub max_loaded_relationship_count: u64,
    /// Max hot node count.
    pub max_hot_node_count: u64,
    /// Max hot relationship count.
    pub max_hot_relationship_count: u64,
    /// Max warm adjacency entry count.
    pub max_warm_adjacency_entry_count: u64,
    /// Max hop count.
    pub max_hop_count: u64,
    /// Max supernode expansion count.
    pub max_supernode_expansion_count: u64,
    /// Max payload byte count.
    pub max_payload_byte_count: u64,
    /// Max execution time ms.
    pub max_execution_time_ms: u64,
}

/// Observed counters that can later be compared with an `ExpansionBudget`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpansionBudgetUsage {
    /// Loaded node count.
    pub loaded_node_count: u64,
    /// Loaded relationship count.
    pub loaded_relationship_count: u64,
    /// Hot node count.
    pub hot_node_count: u64,
    /// Hot relationship count.
    pub hot_relationship_count: u64,
    /// Warm adjacency entry count.
    pub warm_adjacency_entry_count: u64,
    /// Hop count.
    pub hop_count: u64,
    /// Supernode expansion count.
    pub supernode_expansion_count: u64,
    /// Payload byte count.
    pub payload_byte_count: u64,
    /// Execution time ms.
    pub execution_time_ms: u64,
}

/// Budget dimension that can stop an expansion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExpansionLimit {
    /// Loaded node count.
    LoadedNodeCount,
    /// Loaded relationship count.
    LoadedRelationshipCount,
    /// Hot node count.
    HotNodeCount,
    /// Hot relationship count.
    HotRelationshipCount,
    /// Warm adjacency entry count.
    WarmAdjacencyEntryCount,
    /// Hop count.
    HopCount,
    /// Supernode expansion count.
    SupernodeExpansionCount,
    /// Payload byte count.
    PayloadByteCount,
    /// Execution time ms.
    ExecutionTimeMs,
}

/// Stable machine-readable expansion safety error code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExpansionSafetyErrorCode {
    #[serde(rename = "EXPANSION_BUDGET_EXCEEDED")]
    /// Expansion budget exceeded.
    ExpansionBudgetExceeded,

    #[serde(rename = "SUPERNODE_EXPANSION_BLOCKED")]
    /// Supernode expansion blocked.
    SupernodeExpansionBlocked,
}

/// Error payload for a generic budget limit breach.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpansionBudgetExceeded {
    /// Code.
    pub code: ExpansionSafetyErrorCode,
    /// Limit.
    pub limit: ExpansionLimit,
    /// Allowed.
    pub allowed: u64,
    /// Consumed.
    pub consumed: u64,
    /// Fix hint.
    pub fix_hint: String,
}

/// Policy for identifying and guarding high-degree node expansion.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupernodePolicy {
    /// Degree threshold.
    pub degree_threshold: u64,
    /// Require relationship filter.
    pub require_relationship_filter: bool,
    /// Require label filter.
    pub require_label_filter: bool,
    /// Require time window.
    pub require_time_window: bool,
    /// Require limit.
    pub require_limit: bool,
}

/// Error payload for a high-degree node expansion blocked by policy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupernodeExpansionBlocked {
    /// Code.
    pub code: ExpansionSafetyErrorCode,
    /// Observed degree.
    pub observed_degree: u64,
    /// Degree threshold.
    pub degree_threshold: u64,
    /// Relationship filter required.
    pub relationship_filter_required: bool,
    /// Label filter required.
    pub label_filter_required: bool,
    /// Time window required.
    pub time_window_required: bool,
    /// Limit required.
    pub limit_required: bool,
    /// Fix hint.
    pub fix_hint: String,
}

impl ExpansionBudget {
    /// Compare observed counters with configured limits.
    ///
    /// Return `Ok(())` when usage is inside all limits.
    ///
    ///
    /// # Errors
    ///
    /// Return `EXPANSION_BUDGET_EXCEEDED` with the exhausted limit,
    /// allowed value, consumed value, and fix hint.
    pub fn check_usage(&self, usage: &ExpansionBudgetUsage) -> Result<(), ExpansionBudgetExceeded> {
        Self::check_limit(
            ExpansionLimit::LoadedNodeCount,
            self.max_loaded_node_count,
            usage.loaded_node_count,
        )?;
        Self::check_limit(
            ExpansionLimit::LoadedRelationshipCount,
            self.max_loaded_relationship_count,
            usage.loaded_relationship_count,
        )?;
        Self::check_limit(
            ExpansionLimit::HotNodeCount,
            self.max_hot_node_count,
            usage.hot_node_count,
        )?;
        Self::check_limit(
            ExpansionLimit::HotRelationshipCount,
            self.max_hot_relationship_count,
            usage.hot_relationship_count,
        )?;
        Self::check_limit(
            ExpansionLimit::WarmAdjacencyEntryCount,
            self.max_warm_adjacency_entry_count,
            usage.warm_adjacency_entry_count,
        )?;
        Self::check_limit(
            ExpansionLimit::HopCount,
            self.max_hop_count,
            usage.hop_count,
        )?;
        Self::check_limit(
            ExpansionLimit::SupernodeExpansionCount,
            self.max_supernode_expansion_count,
            usage.supernode_expansion_count,
        )?;
        Self::check_limit(
            ExpansionLimit::PayloadByteCount,
            self.max_payload_byte_count,
            usage.payload_byte_count,
        )?;
        Self::check_limit(
            ExpansionLimit::ExecutionTimeMs,
            self.max_execution_time_ms,
            usage.execution_time_ms,
        )?;

        Ok(())
    }

    fn check_limit(
        limit: ExpansionLimit,
        allowed: u64,
        consumed: u64,
    ) -> Result<(), ExpansionBudgetExceeded> {
        if consumed <= allowed {
            return Ok(());
        }

        Err(ExpansionBudgetExceeded {
            code: ExpansionSafetyErrorCode::ExpansionBudgetExceeded,
            limit,
            allowed,
            consumed,
            fix_hint: budget_fix_hint(limit),
        })
    }
}

impl SupernodePolicy {
    /// Identify whether an observed degree is a high-degree node.
    ///
    /// Compare the degree with `degree_threshold`.
    ///
    ///
    /// # Errors
    ///
    /// This pure check should not fail.
    pub fn is_high_degree_node(&self, observed_degree: u64) -> bool {
        observed_degree >= self.degree_threshold
    }

    /// Ensure required guards exist before high-degree expansion.
    ///
    /// Accept safe expansion and reject missing guards.
    ///
    ///
    /// # Errors
    ///
    /// Return `SUPERNODE_EXPANSION_BLOCKED` with a fix hint.
    pub fn validate_expansion_guards(
        &self,
        observed_degree: u64,
        has_relationship_filter: bool,
        has_label_filter: bool,
        has_time_window: bool,
        has_limit: bool,
    ) -> Result<(), SupernodeExpansionBlocked> {
        if !self.is_high_degree_node(observed_degree) {
            return Ok(());
        }

        let mut missing_guards = Vec::new();

        if self.require_relationship_filter && !has_relationship_filter {
            missing_guards.push("relationship filter");
        }

        if self.require_label_filter && !has_label_filter {
            missing_guards.push("label filter");
        }

        if self.require_time_window && !has_time_window {
            missing_guards.push("time window");
        }

        if self.require_limit && !has_limit {
            missing_guards.push("LIMIT");
        }

        if missing_guards.is_empty() {
            return Ok(());
        }

        Err(SupernodeExpansionBlocked {
            code: ExpansionSafetyErrorCode::SupernodeExpansionBlocked,
            observed_degree,
            degree_threshold: self.degree_threshold,
            relationship_filter_required: self.require_relationship_filter,
            label_filter_required: self.require_label_filter,
            time_window_required: self.require_time_window,
            limit_required: self.require_limit,
            fix_hint: supernode_fix_hint(&missing_guards),
        })
    }
}

fn budget_fix_hint(limit: ExpansionLimit) -> String {
    match limit {
 ExpansionLimit::LoadedNodeCount | ExpansionLimit::LoadedRelationshipCount => {
 "Add a label filter, relationship filter, time window, or LIMIT before expanding more records."
 }
 ExpansionLimit::HotNodeCount | ExpansionLimit::HotRelationshipCount => {
 "Reduce the hot working set with a narrower label, relationship type, time window, or LIMIT."
 }
 ExpansionLimit::WarmAdjacencyEntryCount => {
 "Narrow the warm frontier with relationship filters, label filters, a time window, or LIMIT."
 }
 ExpansionLimit::HopCount => {
 "Reduce traversal depth or add relationship, label, time-window, or LIMIT constraints."
 }
 ExpansionLimit::SupernodeExpansionCount => {
 "Avoid repeated supernode expansion by adding relationship filters, label filters, a time window, or LIMIT."
 }
 ExpansionLimit::PayloadByteCount => {
 "Request less payload data or add relationship, label, time-window, or LIMIT constraints."
 }
 ExpansionLimit::ExecutionTimeMs => {
 "Reduce query scope with relationship filters, label filters, a time window, or LIMIT."
 }
 }
 .to_owned()
}

fn supernode_fix_hint(missing_guards: &[&str]) -> String {
    format!(
        "Add the missing supernode guard(s) before expansion: {}.",
        missing_guards.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn budget() -> ExpansionBudget {
        ExpansionBudget {
            // Max loaded node count.
            max_loaded_node_count: 100,
            // Max loaded relationship count.
            max_loaded_relationship_count: 200,
            // Max hot node count.
            max_hot_node_count: 25,
            // Max hot relationship count.
            max_hot_relationship_count: 50,
            // Max warm adjacency entry count.
            max_warm_adjacency_entry_count: 500,
            // Max hop count.
            max_hop_count: 3,
            // Max supernode expansion count.
            max_supernode_expansion_count: 10,
            // Max payload byte count.
            max_payload_byte_count: 1_048_576,
            // Max execution time ms.
            max_execution_time_ms: 1_500,
        }
    }

    fn usage_inside_budget() -> ExpansionBudgetUsage {
        ExpansionBudgetUsage {
            // Loaded node count.
            loaded_node_count: 99,
            // Loaded relationship count.
            loaded_relationship_count: 199,
            // Hot node count.
            hot_node_count: 24,
            // Hot relationship count.
            hot_relationship_count: 49,
            // Warm adjacency entry count.
            warm_adjacency_entry_count: 499,
            // Hop count.
            hop_count: 3,
            // Supernode expansion count.
            supernode_expansion_count: 9,
            // Payload byte count.
            payload_byte_count: 1_048_575,
            // Execution time ms.
            execution_time_ms: 1_499,
        }
    }

    fn guarded_supernode_policy() -> SupernodePolicy {
        SupernodePolicy {
            // Degree threshold.
            degree_threshold: 1_000,
            // Require relationship filter.
            require_relationship_filter: true,
            // Require label filter.
            require_label_filter: true,
            // Require time window.
            require_time_window: true,
            // Require limit.
            require_limit: true,
        }
    }

    //
    // Verify that the expansion budget can represent every explicit limit named
    // before any traversal code exists.
    //
    // Given an `ExpansionBudget` with representative values for every dimension,
    // when callers read the budget fields,
    // then every configured limit should be preserved as typed configuration.
    #[test]
    fn expansion_budget_represents_all_expected_limits() {
        let budget = budget();

        assert_eq!(budget.max_loaded_node_count, 100);
        assert_eq!(budget.max_loaded_relationship_count, 200);
        assert_eq!(budget.max_hot_node_count, 25);
        assert_eq!(budget.max_hot_relationship_count, 50);
        assert_eq!(budget.max_warm_adjacency_entry_count, 500);
        assert_eq!(budget.max_hop_count, 3);
        assert_eq!(budget.max_supernode_expansion_count, 10);
        assert_eq!(budget.max_payload_byte_count, 1_048_576);
        assert_eq!(budget.max_execution_time_ms, 1_500);
    }

    //
    // Verify that budget usage is represented separately from budget limits so a
    // future exceeded error can report both allowed and consumed values.
    //
    // Given an `ExpansionBudgetUsage` snapshot,
    // when callers inspect observed counters,
    // then the usage shape should preserve every measured dimension.
    #[test]
    fn expansion_budget_usage_represents_observed_counters() {
        let usage = usage_inside_budget();

        assert_eq!(usage.loaded_node_count, 99);
        assert_eq!(usage.loaded_relationship_count, 199);
        assert_eq!(usage.hot_node_count, 24);
        assert_eq!(usage.hot_relationship_count, 49);
        assert_eq!(usage.warm_adjacency_entry_count, 499);
        assert_eq!(usage.hop_count, 3);
        assert_eq!(usage.supernode_expansion_count, 9);
        assert_eq!(usage.payload_byte_count, 1_048_575);
        assert_eq!(usage.execution_time_ms, 1_499);
    }

    //
    // Verify that the generic budget-exceeded payload can express the stable
    // machine-readable `EXPANSION_BUDGET_EXCEEDED` condition.
    //
    // Given an exhausted loaded-node limit,
    // when a budget-exceeded payload is created,
    // then it should carry the code, limit, allowed value, consumed value, and fix hint.
    #[test]
    fn expansion_budget_exceeded_payload_carries_limit_and_fix_hint() {
        let error = ExpansionBudgetExceeded {
            code: ExpansionSafetyErrorCode::ExpansionBudgetExceeded,
            limit: ExpansionLimit::LoadedNodeCount,
            allowed: 100,
            consumed: 101,
            fix_hint: "Add a label filter, relationship filter, time window, or LIMIT.".to_owned(),
        };

        assert_eq!(
            error.code,
            ExpansionSafetyErrorCode::ExpansionBudgetExceeded
        );
        assert_eq!(error.limit, ExpansionLimit::LoadedNodeCount);
        assert_eq!(error.allowed, 100);
        assert_eq!(error.consumed, 101);
        assert!(error.fix_hint.contains("LIMIT"));
    }

    //
    // Verify that supernode policy is separate from the generic expansion budget
    // and can represent each guard requirement independently.
    //
    // Given a guarded `SupernodePolicy`,
    // when callers inspect its configuration,
    // then the degree threshold and guard requirements should be explicit.
    #[test]
    fn supernode_policy_represents_threshold_and_required_guards() {
        let policy = guarded_supernode_policy();

        assert_eq!(policy.degree_threshold, 1_000);
        assert!(policy.require_relationship_filter);
        assert!(policy.require_label_filter);
        assert!(policy.require_time_window);
        assert!(policy.require_limit);
    }

    //
    // Verify that the supernode-blocked payload can express the stable
    // machine-readable `SUPERNODE_EXPANSION_BLOCKED` condition.
    //
    // Given a high-degree node expansion missing required guards,
    // when a supernode-blocked payload is created,
    // then it should carry the code, observed degree, threshold, guard flags, and fix hint.
    #[test]
    fn supernode_blocked_payload_carries_threshold_guards_and_fix_hint() {
        let error = SupernodeExpansionBlocked {
            code: ExpansionSafetyErrorCode::SupernodeExpansionBlocked,
            observed_degree: 1_250,
            degree_threshold: 1_000,
            relationship_filter_required: true,
            label_filter_required: true,
            time_window_required: true,
            limit_required: true,
            fix_hint: "Add relationship, label, time-window, and LIMIT guards.".to_owned(),
        };

        assert_eq!(
            error.code,
            ExpansionSafetyErrorCode::SupernodeExpansionBlocked
        );
        assert_eq!(error.observed_degree, 1_250);
        assert_eq!(error.degree_threshold, 1_000);
        assert!(error.relationship_filter_required);
        assert!(error.label_filter_required);
        assert!(error.time_window_required);
        assert!(error.limit_required);
        assert!(error.fix_hint.contains("LIMIT"));
    }

    //
    // Specify the green path for expansion budget checking before the implementation exists.
    //
    // Given usage where every observed counter is less than or equal to the budget,
    // when `check_usage` is called,
    // then expansion should be accepted.
    #[test]
    fn check_usage_accepts_usage_inside_budget() {
        let budget = budget();
        let usage = usage_inside_budget();

        assert_eq!(budget.check_usage(&usage), Ok(()));
    }

    //
    // Specify the failure path for budget exhaustion before the implementation exists.
    //
    // Given usage that exceeds `max_loaded_node_count`,
    // when `check_usage` is called,
    // then it should return `EXPANSION_BUDGET_EXCEEDED` with consumed and allowed values.
    #[test]
    fn check_usage_reports_loaded_node_limit_breach() {
        let budget = budget();
        let usage = ExpansionBudgetUsage {
            loaded_node_count: 101,
            ..usage_inside_budget()
        };

        let error = budget
            .check_usage(&usage)
            .expect_err("loaded node count should exceed the configured budget");

        assert_eq!(
            error.code,
            ExpansionSafetyErrorCode::ExpansionBudgetExceeded
        );
        assert_eq!(error.limit, ExpansionLimit::LoadedNodeCount);
        assert_eq!(error.allowed, 100);
        assert_eq!(error.consumed, 101);
        assert!(error.fix_hint.contains("filter") || error.fix_hint.contains("LIMIT"));
    }

    //
    // Specify the high-degree classification boundary before the implementation exists.
    //
    // Given a supernode policy with a degree threshold,
    // when the observed degree reaches that threshold,
    // then the node should be treated as high degree.
    #[test]
    fn supernode_policy_treats_degree_at_threshold_as_high_degree() {
        let policy = guarded_supernode_policy();

        assert!(!policy.is_high_degree_node(999));
        assert!(policy.is_high_degree_node(1_000));
        assert!(policy.is_high_degree_node(1_001));
    }

    //
    // Specify the guarded expansion path before the implementation exists.
    //
    // Given a high-degree node and all required guards present,
    // when `validate_expansion_guards` is called,
    // then expansion should be accepted.
    #[test]
    fn validate_expansion_guards_accepts_guarded_high_degree_expansion() {
        let policy = guarded_supernode_policy();

        assert_eq!(
            policy.validate_expansion_guards(1_250, true, true, true, true),
            Ok(())
        );
    }

    //
    // Specify the blocked supernode path before the implementation exists.
    //
    // Given a high-degree node and missing required guards,
    // when `validate_expansion_guards` is called,
    // then it should return `SUPERNODE_EXPANSION_BLOCKED` with actionable context.
    #[test]
    fn validate_expansion_guards_blocks_unguarded_high_degree_expansion() {
        let policy = guarded_supernode_policy();

        let error = policy
            .validate_expansion_guards(1_250, false, false, false, false)
            .expect_err("unguarded high-degree expansion should be blocked");

        assert_eq!(
            error.code,
            ExpansionSafetyErrorCode::SupernodeExpansionBlocked
        );
        assert_eq!(error.observed_degree, 1_250);
        assert_eq!(error.degree_threshold, 1_000);
        assert!(error.relationship_filter_required);
        assert!(error.label_filter_required);
        assert!(error.time_window_required);
        assert!(error.limit_required);
        assert!(error.fix_hint.contains("LIMIT") || error.fix_hint.contains("limit"));
    }
}

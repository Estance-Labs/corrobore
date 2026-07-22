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
    BudgetCounterExplanation, ExpansionBudgetUsage, ExpansionFixHint, ExpansionLimit, FixHintScope,
    HotNodeExplanation, HotNodeLoadReason, HotRelationshipExplanation, HotRelationshipLoadReason,
    LoadingProfileKind, LoadingState, NodeId, RelationshipId, RelationshipType,
    SeedNodeExplanation, SeedSourceKind, SeedSourceMetadata, SkippedExpansionExplanation,
    SkippedExpansionReason, SupernodeBlockExplanation, SupernodeBlockReason, SupernodeGuard,
    WarmAdjacencyExplanation, WarmAdjacencyReason, WorkingSetExplanation,
};
use serde::Serialize;

fn node_id(value: &str) -> NodeId {
    NodeId::new(value).expect("test node ID should be valid")
}

fn relationship_id(value: &str) -> RelationshipId {
    RelationshipId::new(value).expect("test relationship ID should be valid")
}

fn relationship_type(value: &str) -> RelationshipType {
    RelationshipType::new(value).expect("test relationship type should be valid")
}

fn budget_usage() -> ExpansionBudgetUsage {
    ExpansionBudgetUsage {
        loaded_node_count: 12,
        loaded_relationship_count: 24,
        hot_node_count: 4,
        hot_relationship_count: 8,
        warm_adjacency_entry_count: 32,
        hop_count: 2,
        supernode_expansion_count: 1,
        payload_byte_count: 4_096,
        execution_time_ms: 75,
    }
}

fn query_fix_hint(message: &str) -> ExpansionFixHint {
    ExpansionFixHint {
        scope: FixHintScope::Query,
        message: message.to_owned(),
    }
}

fn assert_serializable<T: Serialize>() {}

//
// Verify that a new working-set explanation starts as an empty deterministic
// report before any loading decisions have been recorded.
//
// Given no recorded seed, hot, warm, skipped, supernode, budget, or fix-hint data,
// when a `WorkingSetExplanation` is created,
// then every public accessor should expose an empty or absent explanation section.
#[test]
fn working_set_explanation_starts_empty() {
    let explanation = WorkingSetExplanation::new();

    assert!(explanation.seed_nodes().is_empty());
    assert!(explanation.hot_nodes().is_empty());
    assert!(explanation.hot_relationships().is_empty());
    assert!(explanation.warm_adjacency_entries().is_empty());
    assert!(explanation.skipped_expansions().is_empty());
    assert!(explanation.supernode_blocks().is_empty());
    assert_eq!(explanation.consumed_budget(), None);
    assert!(explanation.remaining_budget_counters().is_empty());
    assert!(explanation.fix_hints().is_empty());
}

//
// Verify that seed node explanations preserve both the seed ID and the source
// metadata that explains where graph loading started.
//
// Given two seed nodes discovered from different source kinds,
// when both seeds are recorded,
// then the explanation should report them in append order with their metadata intact.
#[test]
fn seed_node_explanations_preserve_source_metadata_in_order() {
    let mut explanation = WorkingSetExplanation::new();

    let semantic_seed = SeedNodeExplanation {
        node_id: node_id("campaign--migration-france"),
        source: SeedSourceMetadata {
            kind: SeedSourceKind::SemanticSearch,
            source_id: Some("vector-hit-001".to_owned()),
            source_label: Some("migration narrative targeting France".to_owned()),
            score: Some(0.91),
        },
    };
    let explicit_seed = SeedNodeExplanation {
        node_id: node_id("report--analyst-supplied"),
        source: SeedSourceMetadata {
            kind: SeedSourceKind::ExplicitNodeId,
            source_id: Some("user-request".to_owned()),
            source_label: None,
            score: None,
        },
    };

    explanation.record_seed_node(semantic_seed.clone());
    explanation.record_seed_node(explicit_seed.clone());

    assert_eq!(explanation.seed_nodes(), &[semantic_seed, explicit_seed]);
}

//
// Verify that hot node and hot relationship explanations are reported as separate
// decision streams instead of being inferred from loaded record IDs.
//
// Given a seed node promoted to hot and a prioritized relationship promoted to hot,
// when both decisions are recorded,
// then node-level and relationship-level explanations should remain separate and complete.
#[test]
fn hot_record_explanations_keep_node_and_relationship_reasons_separate() {
    let mut explanation = WorkingSetExplanation::new();

    let hot_node = HotNodeExplanation {
        node_id: node_id("campaign--active"),
        reason: HotNodeLoadReason::SeedNode,
        via_relationship_id: None,
        profile_kind: Some(LoadingProfileKind::FimiInvestigation),
        hop_count: Some(0),
    };
    let hot_relationship = HotRelationshipExplanation {
        relationship_id: relationship_id("relationship--promotes"),
        relationship_type: relationship_type("PROMOTES"),
        source_node_id: node_id("campaign--active"),
        target_node_id: node_id("narrative--targeted"),
        reason: HotRelationshipLoadReason::PrioritizedRelationshipType,
        profile_kind: Some(LoadingProfileKind::FimiInvestigation),
        hop_count: Some(1),
    };

    explanation.record_hot_node(hot_node.clone());
    explanation.record_hot_relationship(hot_relationship.clone());

    assert_eq!(explanation.hot_nodes(), &[hot_node]);
    assert_eq!(explanation.hot_relationships(), &[hot_relationship]);
}

//
// Verify that warm adjacency explanations can report why a neighboring record was
// kept warm without loading the full target payload.
//
// Given a warm adjacency entry retained by a cautious relationship policy,
// when the warm decision is recorded,
// then the explanation should preserve relationship metadata, target loading state,
// profile context, and relevance score.
#[test]
fn warm_adjacency_explanations_preserve_frontier_metadata() {
    let mut explanation = WorkingSetExplanation::new();

    let warm_adjacency = WarmAdjacencyExplanation {
        relationship_id: relationship_id("relationship--targets"),
        relationship_type: relationship_type("TARGETS"),
        source_node_id: node_id("campaign--active"),
        target_node_id: node_id("country--france"),
        target_loading_state: LoadingState::Warm,
        reason: WarmAdjacencyReason::CautiousRelationshipType,
        profile_kind: Some(LoadingProfileKind::FimiInvestigation),
        relevance_score: Some(0.42),
    };

    explanation.record_warm_adjacency(warm_adjacency.clone());

    assert_eq!(explanation.warm_adjacency_entries(), &[warm_adjacency]);
}

//
// Verify that skipped expansion explanations make partial loading decisions
// actionable instead of silently omitting candidate records.
//
// Given a candidate expansion skipped because it would exceed a warm adjacency budget,
// when the skipped decision is recorded,
// then the explanation should include the candidate, budget counter, reason, and fix hint.
#[test]
fn skipped_expansion_explanations_include_budget_counter_and_fix_hint() {
    let mut explanation = WorkingSetExplanation::new();

    let fix_hint =
        query_fix_hint("Add a relationship type filter, label filter, time window, or LIMIT.");
    let skipped = SkippedExpansionExplanation {
        source_node_id: node_id("campaign--active"),
        candidate_node_id: Some(node_id("post--too-far")),
        relationship_id: Some(relationship_id("relationship--mentions")),
        relationship_type: Some(relationship_type("MENTIONS")),
        reason: SkippedExpansionReason::BudgetLimit,
        budget_counter: Some(BudgetCounterExplanation {
            limit: ExpansionLimit::WarmAdjacencyEntryCount,
            allowed: 500,
            consumed: 501,
            remaining: Some(0),
        }),
        fix_hint: Some(fix_hint.clone()),
    };

    explanation.record_skipped_expansion(skipped.clone());

    assert_eq!(explanation.skipped_expansions(), &[skipped]);
    assert_eq!(
        explanation.skipped_expansions()[0].fix_hint.as_ref(),
        Some(&fix_hint)
    );
}

//
// Verify that supernode blocking decisions explain why high-degree expansion was
// stopped and which guards are required for a safer retry.
//
// Given a high-degree node blocked by missing supernode guards,
// when the block is recorded,
// then the explanation should preserve degree data, missing guards, reason, and fix hint.
#[test]
fn supernode_block_explanations_report_reason_guards_and_fix_hint() {
    let mut explanation = WorkingSetExplanation::new();

    let block = SupernodeBlockExplanation {
        node_id: node_id("country--france"),
        observed_degree: 15_000,
        degree_threshold: 1_000,
        reason: SupernodeBlockReason::RequiredGuardsMissing,
        missing_guards: vec![SupernodeGuard::RelationshipFilter, SupernodeGuard::Limit],
        fix_hint: ExpansionFixHint {
            scope: FixHintScope::SupernodeGuard,
            message: "Add relationship filter and LIMIT before expanding this supernode."
                .to_owned(),
        },
    };

    explanation.record_supernode_block(block.clone());

    assert_eq!(explanation.supernode_blocks(), &[block]);
}

//
// Verify that budget usage, remaining counters, and session-level fix hints can be
// represented independently from individual skipped or blocked decisions.
//
// Given consumed budget usage, one remaining-budget counter, and one session-level hint,
// when all three are recorded,
// then the explanation should expose them through their dedicated accessors.
#[test]
fn budget_usage_remaining_counters_and_session_fix_hints_are_exposed() {
    let mut explanation = WorkingSetExplanation::new();

    let usage = budget_usage();
    let remaining_counter = BudgetCounterExplanation {
        limit: ExpansionLimit::PayloadByteCount,
        allowed: 8_192,
        consumed: 4_096,
        remaining: Some(4_096),
    };
    let fix_hint = ExpansionFixHint {
        scope: FixHintScope::Budget,
        message: "Request less payload data or narrow the traversal scope.".to_owned(),
    };

    explanation.record_consumed_budget(usage.clone());
    explanation.record_remaining_budget_counter(remaining_counter.clone());
    explanation.record_fix_hint(fix_hint.clone());

    assert_eq!(explanation.consumed_budget(), Some(&usage));
    assert_eq!(
        explanation.remaining_budget_counters(),
        &[remaining_counter]
    );
    assert_eq!(explanation.fix_hints(), &[fix_hint]);
}

//
// Validate the acceptance path as one public integration scenario, not
// only as isolated unit-style contract checks.
//
// Given a complete FIMI working-set loading session with semantic seeds, hot
// records, warm frontier metadata, skipped expansion, supernode blocking, budget
// usage, remaining counters, and fix hints,
// when the session is recorded through the public `graph_core` facade,
// then the explanation should report every acceptance section deterministically.
#[test]
fn acceptance_working_set_explanation_reports_complete_loading_session() {
    let mut explanation = WorkingSetExplanation::new();

    explanation.record_seed_node(SeedNodeExplanation {
        node_id: node_id("campaign--migration-france"),
        source: SeedSourceMetadata {
            kind: SeedSourceKind::SemanticSearch,
            source_id: Some("semantic-hit--001".to_owned()),
            source_label: Some("migration narrative targeting France".to_owned()),
            score: Some(0.93),
        },
    });
    explanation.record_hot_node(HotNodeExplanation {
        node_id: node_id("campaign--migration-france"),
        reason: HotNodeLoadReason::SeedNode,
        via_relationship_id: None,
        profile_kind: Some(LoadingProfileKind::FimiInvestigation),
        hop_count: Some(0),
    });
    explanation.record_hot_relationship(HotRelationshipExplanation {
        relationship_id: relationship_id("relationship--campaign-promotes-narrative"),
        relationship_type: relationship_type("PROMOTES"),
        source_node_id: node_id("campaign--migration-france"),
        target_node_id: node_id("narrative--border-crisis"),
        reason: HotRelationshipLoadReason::PrioritizedRelationshipType,
        profile_kind: Some(LoadingProfileKind::FimiInvestigation),
        hop_count: Some(1),
    });
    explanation.record_warm_adjacency(WarmAdjacencyExplanation {
        relationship_id: relationship_id("relationship--campaign-targets-france"),
        relationship_type: relationship_type("TARGETS"),
        source_node_id: node_id("campaign--migration-france"),
        target_node_id: node_id("country--france"),
        target_loading_state: LoadingState::Warm,
        reason: WarmAdjacencyReason::CautiousRelationshipType,
        profile_kind: Some(LoadingProfileKind::FimiInvestigation),
        relevance_score: Some(0.37),
    });
    explanation.record_skipped_expansion(SkippedExpansionExplanation {
        source_node_id: node_id("country--france"),
        candidate_node_id: Some(node_id("post--generic-france-mention")),
        relationship_id: Some(relationship_id("relationship--generic-mention")),
        relationship_type: Some(relationship_type("MENTIONS")),
        reason: SkippedExpansionReason::BlockedByProfile,
        budget_counter: None,
        fix_hint: Some(query_fix_hint(
            "Use a more specific relationship type, label filter, time window, or LIMIT.",
        )),
    });
    explanation.record_supernode_block(SupernodeBlockExplanation {
        node_id: node_id("country--france"),
        observed_degree: 15_000,
        degree_threshold: 1_000,
        reason: SupernodeBlockReason::RequiredGuardsMissing,
        missing_guards: vec![SupernodeGuard::RelationshipFilter, SupernodeGuard::Limit],
        fix_hint: ExpansionFixHint {
            scope: FixHintScope::SupernodeGuard,
            message: "Add relationship filter and LIMIT before expanding this supernode."
                .to_owned(),
        },
    });
    explanation.record_consumed_budget(budget_usage());
    explanation.record_remaining_budget_counter(BudgetCounterExplanation {
        limit: ExpansionLimit::PayloadByteCount,
        allowed: 8_192,
        consumed: 4_096,
        remaining: Some(4_096),
    });
    explanation.record_fix_hint(ExpansionFixHint {
        scope: FixHintScope::Query,
        message: "Narrow the query before expanding broad country mentions.".to_owned(),
    });

    assert_eq!(explanation.seed_nodes().len(), 1);
    assert_eq!(explanation.hot_nodes().len(), 1);
    assert_eq!(explanation.hot_relationships().len(), 1);
    assert_eq!(explanation.warm_adjacency_entries().len(), 1);
    assert_eq!(explanation.skipped_expansions().len(), 1);
    assert_eq!(explanation.supernode_blocks().len(), 1);
    assert!(explanation.consumed_budget().is_some());
    assert_eq!(explanation.remaining_budget_counters().len(), 1);
    assert_eq!(explanation.fix_hints().len(), 1);
    assert_eq!(
        explanation.seed_nodes()[0].source.kind,
        SeedSourceKind::SemanticSearch
    );
    assert_eq!(
        explanation.supernode_blocks()[0].missing_guards,
        vec![SupernodeGuard::RelationshipFilter, SupernodeGuard::Limit]
    );
}

//
// Verify that every public explanation contract intended for API or audit output
// keeps its serde serialization bound.
//
// Given the public explanation model and its nested payload types,
// when the crate is compiled,
// then each type should satisfy `serde::Serialize` without depending on LLM output.
#[test]
fn working_set_explanation_contracts_are_serializable() {
    assert_serializable::<WorkingSetExplanation>();
    assert_serializable::<SeedNodeExplanation>();
    assert_serializable::<SeedSourceMetadata>();
    assert_serializable::<HotNodeExplanation>();
    assert_serializable::<HotRelationshipExplanation>();
    assert_serializable::<WarmAdjacencyExplanation>();
    assert_serializable::<SkippedExpansionExplanation>();
    assert_serializable::<SupernodeBlockExplanation>();
    assert_serializable::<BudgetCounterExplanation>();
    assert_serializable::<ExpansionFixHint>();
}

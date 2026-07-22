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
//! Structured explanation model for graph working set loading decisions.
//!
//! Design boundary for issue 41:
//!
//! - Define serializable explanation structures for seed selection, hot loading,
//!   warm frontier retention, skipped expansion, supernode blocking, and budget
//!   accounting.
//! - Keep the model deterministic enough for tests by preserving append order in
//!   vectors instead of using unordered maps for explanation output.
//! - Keep explanation data independent from LLM answer generation. This module
//!   explains graph loading decisions only; response wording, prompting, citation
//!   synthesis, and analyst-facing narrative generation belong outside graph-core.
//! - Do not implement graph expansion, traversal, page-in, prefetch, eviction,
//!   semantic search, audit-log persistence, or storage access here.
//!
//!
//!
//! This file implements the deterministic in-memory recording behavior specified
//! by the phase 2 contract tests. It still does not implement graph expansion,
//! traversal, page-in, prefetch, eviction, semantic search, audit-log persistence,
//! or storage access.

use serde::{Deserialize, Serialize};

use crate::{
    ExpansionBudgetUsage, ExpansionLimit, LoadingProfileKind, LoadingState, NodeId, RelationshipId,
    RelationshipType,
};

/// Structured explanation for one bounded working-set loading session.
///
///
/// - Collect the observable reasons behind the working set that was built.
/// - Preserve seed, hot, warm, skipped, supernode, and budget decisions separately
///   so agents and tests do not need to infer meaning from loaded records alone.
/// - Keep explanation output as serializable data, not generated prose.
///
///
/// A future implementation should append records in the order decisions happen and
/// expose read-only slices for deterministic assertions.
///
/// # Errors
///
///
/// Explanation recording should not fail under normal conditions because it is a
/// diagnostic side channel. Runtime loading failures should be represented by the
/// loading or expansion layer, then recorded here as skipped or blocked decisions.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkingSetExplanation {
    /// Seed nodes that anchored the working-set loading session.
    pub seed_nodes: Vec<SeedNodeExplanation>,

    /// Nodes promoted into the hot working set and the reason for each promotion.
    pub hot_nodes: Vec<HotNodeExplanation>,

    /// Relationships promoted into the hot working set and the reason for each promotion.
    pub hot_relationships: Vec<HotRelationshipExplanation>,

    /// Warm adjacency entries retained near the active hot subgraph.
    pub warm_adjacency_entries: Vec<WarmAdjacencyExplanation>,

    /// Candidate expansions that were evaluated but not loaded.
    pub skipped_expansions: Vec<SkippedExpansionExplanation>,

    /// High-degree node expansion decisions blocked by supernode policy.
    pub supernode_blocks: Vec<SupernodeBlockExplanation>,

    /// Consumed budget snapshot for the loading session when available.
    pub consumed_budget: Option<ExpansionBudgetUsage>,

    /// Remaining budget counters that are useful to explain partial loading.
    pub remaining_budget_counters: Vec<BudgetCounterExplanation>,

    /// Session-level fix hints not tied to a single skipped or blocked decision.
    pub fix_hints: Vec<ExpansionFixHint>,
}

/// Explanation payload for a node selected as a working-set seed.
///
///
/// keep seed identity and seed source metadata together so later loading reports
/// can explain where the active subgraph started.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SeedNodeExplanation {
    /// Node id.
    pub node_id: NodeId,
    /// Source.
    pub source: SeedSourceMetadata,
}

/// Metadata describing how a seed node was discovered.
///
///
/// represent semantic, explicit, query-derived, and runtime-derived seed sources
/// without coupling this crate to a concrete semantic index or query engine.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SeedSourceMetadata {
    /// Kind.
    pub kind: SeedSourceKind,
    /// Source id.
    pub source_id: Option<String>,
    /// Source label.
    pub source_label: Option<String>,
    /// Score.
    pub score: Option<f64>,
}

/// Stable categories for seed-node source metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SeedSourceKind {
    /// The seed came from an explicit node ID supplied by a caller.
    ExplicitNodeId,

    /// The seed came from a semantic retrieval candidate.
    SemanticSearch,

    /// The seed came from a graph query or graph-plan result.
    GraphQuery,

    /// The seed came from a previous traversal or page-in decision.
    Traversal,

    /// The seed source is known by the caller but not modeled by this enum yet.
    External,
}

/// Explanation payload for a node loaded as hot.
///
///
/// record the stable node ID and the reason it became active in the hot working
/// set without storing the node payload itself.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HotNodeExplanation {
    /// Node id.
    pub node_id: NodeId,
    /// Reason.
    pub reason: HotNodeLoadReason,
    /// Via relationship id.
    pub via_relationship_id: Option<RelationshipId>,
    /// Profile kind.
    pub profile_kind: Option<LoadingProfileKind>,
    /// Hop count.
    pub hop_count: Option<u64>,
}

/// Stable reasons for a node becoming hot in a working set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HotNodeLoadReason {
    /// The node was selected as a seed and loaded for active traversal.
    SeedNode,

    /// The node label matched the active loading profile hot-label policy.
    ProfileHotLabel,

    /// The node was required by an accepted traversal expansion.
    TraversalExpansion,

    /// The node was explicitly pinned or requested by a caller.
    ExplicitPin,

    /// The node was loaded on demand from a previously warm or indexed record.
    LazyPageIn,
}

/// Explanation payload for a relationship loaded as hot.
///
///
/// keep relationship identity, relationship type, endpoint IDs, and load reason
/// visible without requiring the relationship payload to be embedded here.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HotRelationshipExplanation {
    /// Relationship id.
    pub relationship_id: RelationshipId,
    /// Relationship type.
    pub relationship_type: RelationshipType,
    /// Source node id.
    pub source_node_id: NodeId,
    /// Target node id.
    pub target_node_id: NodeId,
    /// Reason.
    pub reason: HotRelationshipLoadReason,
    /// Profile kind.
    pub profile_kind: Option<LoadingProfileKind>,
    /// Hop count.
    pub hop_count: Option<u64>,
}

/// Stable reasons for a relationship becoming hot in a working set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HotRelationshipLoadReason {
    /// The relationship type was prioritized by the active loading profile.
    PrioritizedRelationshipType,

    /// The relationship was needed to explain or validate a hot node.
    RequiredForHotNode,

    /// The relationship was accepted by a traversal expansion decision.
    TraversalExpansion,

    /// The relationship was loaded on demand from warm adjacency metadata.
    LazyPageIn,
}

/// Explanation payload for a warm adjacency entry kept near the hot subgraph.
///
///
/// explain why adjacency metadata remains warm even though the target node or
/// relationship payload may not be hot yet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WarmAdjacencyExplanation {
    /// Relationship id.
    pub relationship_id: RelationshipId,
    /// Relationship type.
    pub relationship_type: RelationshipType,
    /// Source node id.
    pub source_node_id: NodeId,
    /// Target node id.
    pub target_node_id: NodeId,
    /// Target loading state.
    pub target_loading_state: LoadingState,
    /// Reason.
    pub reason: WarmAdjacencyReason,
    /// Profile kind.
    pub profile_kind: Option<LoadingProfileKind>,
    /// Relevance score.
    pub relevance_score: Option<f64>,
}

/// Stable reasons for keeping adjacency metadata warm.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WarmAdjacencyReason {
    /// The entry is part of the next likely frontier around a hot record.
    FrontierPrefetch,

    /// The active loading profile treats this path as useful but guarded.
    CautiousRelationshipType,

    /// The entry sits on a configured ring boundary and should not become hot yet.
    RingBoundary,

    /// The entry stayed warm because a budget prevented a full hot load.
    BudgetConstrained,

    /// The entry is retained as lightweight metadata for future page-in.
    NotYetLoaded,
}

/// Explanation payload for a candidate expansion that was not loaded.
///
///
/// make skipped expansion decisions explicit instead of silently omitting records
/// from the working set.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SkippedExpansionExplanation {
    /// Source node id.
    pub source_node_id: NodeId,
    /// Candidate node id.
    pub candidate_node_id: Option<NodeId>,
    /// Relationship id.
    pub relationship_id: Option<RelationshipId>,
    /// Relationship type.
    pub relationship_type: Option<RelationshipType>,
    /// Reason.
    pub reason: SkippedExpansionReason,
    /// Budget counter.
    pub budget_counter: Option<BudgetCounterExplanation>,
    /// Fix hint.
    pub fix_hint: Option<ExpansionFixHint>,
}

/// Stable reasons for not expanding a candidate record.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SkippedExpansionReason {
    /// The candidate would exceed a configured expansion budget.
    BudgetLimit,

    /// The candidate was blocked by high-degree-node safety policy.
    SupernodePolicy,

    /// The active loading profile blocks this path by default.
    BlockedByProfile,

    /// The candidate ranked below the active relevance threshold.
    LowRelevance,

    /// The candidate cannot be page-loaded because required storage metadata is absent.
    MissingStorageRef,

    /// The candidate path is not supported by the current working-set contract.
    UnsupportedPath,

    /// A working-set controller decided not to expand this path (stop or
    /// deferral); deterministic guards were not the cause.
    ControllerDecision,
}

/// Explanation payload for a supernode block.
///
///
/// expose the node, observed degree, threshold, missing guards, and actionable fix
/// hint that prevented uncontrolled high-degree expansion.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SupernodeBlockExplanation {
    /// Node id.
    pub node_id: NodeId,
    /// Observed degree.
    pub observed_degree: u64,
    /// Degree threshold.
    pub degree_threshold: u64,
    /// Reason.
    pub reason: SupernodeBlockReason,
    /// Missing guards.
    pub missing_guards: Vec<SupernodeGuard>,
    /// Fix hint.
    pub fix_hint: ExpansionFixHint,
}

/// Stable reasons for blocking expansion around a high-degree node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SupernodeBlockReason {
    /// The node degree crossed the configured supernode threshold.
    DegreeThresholdExceeded,

    /// The node required narrowing guards before expansion could continue.
    RequiredGuardsMissing,

    /// The active loading profile requires the caller to narrow this node type.
    ProfileRequiresNarrowing,
}

/// Narrowing guard that can make a supernode expansion safer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SupernodeGuard {
    /// Require an explicit relationship-type filter.
    RelationshipFilter,

    /// Require an explicit target or source label filter.
    LabelFilter,

    /// Require a time-window constraint.
    TimeWindow,

    /// Require an explicit result limit.
    Limit,
}

/// Budget counter explained for either consumed or remaining budget.
///
///
/// preserve the budget dimension, allowed value, consumed value, and remaining
/// value when useful for partial or blocked expansion output.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetCounterExplanation {
    /// Limit.
    pub limit: ExpansionLimit,
    /// Allowed.
    pub allowed: u64,
    /// Consumed.
    pub consumed: u64,
    /// Remaining.
    pub remaining: Option<u64>,
}

/// Human-actionable remediation hint tied to a loading decision.
///
///
/// provide concise guidance for agents and API callers without generating final
/// analyst prose in this crate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpansionFixHint {
    /// Scope.
    pub scope: FixHintScope,
    /// Message.
    pub message: String,
}

/// Scope for an actionable fix hint.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FixHintScope {
    /// The caller should narrow the query or graph plan.
    Query,

    /// The caller should adjust loading profile selection or configuration.
    LoadingProfile,

    /// The caller should adjust expansion budget or inspect budget usage.
    Budget,

    /// The caller should add supernode guards before retrying expansion.
    SupernodeGuard,

    /// The caller should verify storage or pager metadata.
    Storage,
}

impl WorkingSetExplanation {
    /// Create an empty explanation for one future working-set loading session.
    ///
    ///
    /// initialize all explanation collections in deterministic append order.
    ///
    ///
    /// return an explanation with no seed, hot, warm, skipped, blocked, budget, or
    /// fix-hint records.
    ///
    /// # Errors
    ///
    /// none expected because empty explanation construction should not validate
    /// graph state or access storage.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a seed node and its source metadata.
    ///
    ///
    /// preserve the entry point that caused the working set to load a local graph
    /// neighborhood.
    ///
    ///
    /// append the seed explanation without deduplicating or sorting so decision
    /// order remains testable.
    ///
    /// # Errors
    ///
    /// none expected; duplicate seed handling belongs to the future caller policy.
    pub fn record_seed_node(&mut self, seed: SeedNodeExplanation) {
        self.seed_nodes.push(seed);
    }

    /// Record why a node was loaded as hot.
    ///
    ///
    /// make hot node presence explainable without requiring tests to infer reasons
    /// from working-set membership alone.
    ///
    ///
    /// append the hot-node explanation in call order.
    ///
    /// # Errors
    ///
    /// none expected; invalid node IDs are rejected by typed ID construction before
    /// reaching this method.
    pub fn record_hot_node(&mut self, hot_node: HotNodeExplanation) {
        self.hot_nodes.push(hot_node);
    }

    /// Record why a relationship was loaded as hot.
    ///
    ///
    /// preserve relationship-level loading rationale separately from node-level
    /// rationale.
    ///
    ///
    /// append the hot-relationship explanation in call order.
    ///
    /// # Errors
    ///
    /// none expected because relationship identity and type are already validated.
    pub fn record_hot_relationship(&mut self, hot_relationship: HotRelationshipExplanation) {
        self.hot_relationships.push(hot_relationship);
    }

    /// Record why an adjacency entry was kept warm.
    ///
    ///
    /// explain the warm frontier without forcing target records to become hot.
    ///
    ///
    /// append the warm-adjacency explanation in call order.
    ///
    /// # Errors
    ///
    /// none expected because this method records a decision already made by loading
    /// policy.
    pub fn record_warm_adjacency(&mut self, warm_adjacency: WarmAdjacencyExplanation) {
        self.warm_adjacency_entries.push(warm_adjacency);
    }

    /// Record a skipped candidate expansion.
    ///
    ///
    /// make partial working-set output explainable when a candidate was omitted by
    /// policy, relevance, missing storage metadata, or budget constraints.
    ///
    ///
    /// append the skipped-expansion explanation in call order.
    ///
    /// # Errors
    ///
    /// none expected because runtime expansion errors are represented as payloads.
    pub fn record_skipped_expansion(&mut self, skipped: SkippedExpansionExplanation) {
        self.skipped_expansions.push(skipped);
    }

    /// Record a supernode blocking decision.
    ///
    ///
    /// preserve why high-degree expansion stopped and what guard would make a retry
    /// safer.
    ///
    ///
    /// append the supernode-block explanation and preserve its fix hint.
    ///
    /// # Errors
    ///
    /// none expected because supernode policy validation occurs before recording.
    pub fn record_supernode_block(&mut self, block: SupernodeBlockExplanation) {
        self.supernode_blocks.push(block);
    }

    /// Record consumed budget counters for the loading session.
    ///
    ///
    /// keep budget usage visible in explanation output so agents and tests can
    /// understand why expansion remained bounded.
    ///
    ///
    /// replace the previous consumed-budget snapshot with the supplied usage.
    ///
    /// # Errors
    ///
    /// none expected; checking whether usage exceeds limits belongs to the budget
    /// module and loading policy.
    pub fn record_consumed_budget(&mut self, usage: ExpansionBudgetUsage) {
        self.consumed_budget = Some(usage);
    }

    /// Record one remaining-budget counter when useful for explanation output.
    ///
    ///
    /// preserve dimensions where remaining capacity helps agents or tests explain
    /// partial expansion.
    ///
    ///
    /// append the counter in call order.
    ///
    /// # Errors
    ///
    /// none expected because arithmetic validation belongs to the producer of the
    /// counter payload.
    pub fn record_remaining_budget_counter(&mut self, counter: BudgetCounterExplanation) {
        self.remaining_budget_counters.push(counter);
    }

    /// Record an actionable fix hint not already attached to a specific decision.
    ///
    ///
    /// allow session-level hints such as narrowing the query or choosing a more
    /// specific loading profile.
    ///
    ///
    /// append the hint in call order.
    ///
    /// # Errors
    ///
    /// none expected because empty or duplicate hint validation belongs to a later
    /// policy decision.
    pub fn record_fix_hint(&mut self, fix_hint: ExpansionFixHint) {
        self.fix_hints.push(fix_hint);
    }

    /// Return seed-node explanations in deterministic record order.
    ///
    ///
    /// expose seed explanations without letting callers mutate internal storage.
    ///
    ///
    /// return a read-only slice preserving append order.
    ///
    /// # Errors
    ///
    /// none expected because absence is represented as an empty slice.
    pub fn seed_nodes(&self) -> &[SeedNodeExplanation] {
        &self.seed_nodes
    }

    /// Return hot-node explanations in deterministic record order.
    pub fn hot_nodes(&self) -> &[HotNodeExplanation] {
        &self.hot_nodes
    }

    /// Return hot-relationship explanations in deterministic record order.
    pub fn hot_relationships(&self) -> &[HotRelationshipExplanation] {
        &self.hot_relationships
    }

    /// Return warm-adjacency explanations in deterministic record order.
    pub fn warm_adjacency_entries(&self) -> &[WarmAdjacencyExplanation] {
        &self.warm_adjacency_entries
    }

    /// Return skipped-expansion explanations in deterministic record order.
    pub fn skipped_expansions(&self) -> &[SkippedExpansionExplanation] {
        &self.skipped_expansions
    }

    /// Return supernode-block explanations in deterministic record order.
    pub fn supernode_blocks(&self) -> &[SupernodeBlockExplanation] {
        &self.supernode_blocks
    }

    /// Return the consumed-budget snapshot when one has been recorded.
    pub fn consumed_budget(&self) -> Option<&ExpansionBudgetUsage> {
        self.consumed_budget.as_ref()
    }

    /// Return remaining-budget counters in deterministic record order.
    pub fn remaining_budget_counters(&self) -> &[BudgetCounterExplanation] {
        &self.remaining_budget_counters
    }

    /// Return session-level fix hints in deterministic record order.
    pub fn fix_hints(&self) -> &[ExpansionFixHint] {
        &self.fix_hints
    }
}

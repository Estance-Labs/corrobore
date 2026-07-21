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
//! Contextual bandit context, action, and reward interfaces (Epic 0017).
//!
//!
//!
//! - Define the controller boundary that will drive working-set decisions
//!   before any full reinforcement learning: a contextual bandit is much
//!   simpler to stabilize than deep RL and is the epic's chosen first
//!   controller.
//! - Keep context, action, and reward types explicit, serializable, and
//!   reproducible from recorded telemetry so policies can be trained and
//!   replayed offline.
//! - Make policies interchangeable behind one trait so baseline heuristics,
//!   the contextual bandit, and the learned pheromone policy plug into the
//!   same call sites.
//! - Keep integration into `GraphWorkingSetManager` out of this module; the
//!   integration issue wires the boundary into the manager under the existing
//!   deterministic budget and supernode guards.
//!
//! # Reward derivation rules (deterministic)
//!
//! From one telemetry retrieval record:
//!
//! - `evidence_found_count` is the outcome's evidence-record count;
//! - `io_count` is the number of `PageIn` decisions;
//! - `memory_cost_bytes` and `latency_ms` come from the caller-supplied
//!   outcome measurements;
//! - a `Prefetch` decision is wasted unless the same record reference appears
//!   later in the stream as an `EdgeExpanded` relationship, a `SeedSelected`
//!   node, or a `PageIn`;
//! - `expected_subgraph_recall` cannot be derived from telemetry: ground
//!   truth belongs to the benchmark harness, which attaches it through the
//!   validated builder.

use serde::{Deserialize, Serialize};

use crate::{
    Confidence,
    expansion_budget::{ExpansionBudget, ExpansionBudgetUsage},
    ids::NodeId,
    properties::LabelSet,
    relationship::RelationshipType,
    temporal::TemporalTimestamp,
    working_set_expansion::ExpansionRequest,
    working_set_telemetry::{
        RetrievalTelemetryRecord, TelemetryQueryDescriptor, WorkingSetDecisionEvent,
    },
};

/// One controller decision over the working set.
///
///
/// name the epic's action space explicitly so policies choose among typed
/// actions instead of engine internals.
///
///
/// model the six actions of the Epic 0017 controller contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WorkingSetAction {
    /// Expand a frontier relationship.
    Expand,

    /// Prefetch a likely-needed record ahead of demand.
    Prefetch,

    /// Page in a required cold record.
    PageIn,

    /// Stop expansion because evidence or budget makes it unjustified.
    Stop,

    /// Verify a claim before trusting a path.
    Verify,

    /// Retrieve from an external source outside the working set.
    RetrieveExternally,
}

impl WorkingSetAction {
    /// The complete, stable action space of the controller.
    ///
    ///
    /// let policies iterate candidate actions without hand-maintained lists
    /// that could drift from the enum.
    pub const ALL: [WorkingSetAction; 6] = [
        WorkingSetAction::Expand,
        WorkingSetAction::Prefetch,
        WorkingSetAction::PageIn,
        WorkingSetAction::Stop,
        WorkingSetAction::Verify,
        WorkingSetAction::RetrieveExternally,
    ];
}

/// Observed degree of one frontier node.
///
///
/// give policies the degree signal used by supernode protection without
/// coupling them to pager internals.
///
///
/// pair a frontier node with its observed adjacency degree.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrontierDegree {
    /// Frontier node.
    pub node_id: NodeId,

    /// Observed adjacency degree of the node.
    pub observed_degree: u64,
}

/// Decision context handed to a controller.
///
///
/// condition action choice on the epic's context features: question type,
/// seeds, filters, remaining budget, working-set history, node degrees,
/// temporality, and confidence scores.
///
///
/// carry the query descriptor, seed and filter state, the budget with its
/// consumed usage, a working-set stats snapshot, frontier degrees, seed
/// confidences, and an optional as-of timestamp.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BanditContext {
    /// Query context (question text, loading profile, task family).
    pub descriptor: TelemetryQueryDescriptor,

    /// Seed nodes of the retrieval.
    pub seed_node_ids: Vec<NodeId>,

    /// Active relationship-type filters.
    pub relationship_type_filters: Vec<RelationshipType>,

    /// Active label filters.
    pub label_filters: LabelSet,

    /// Hard budget of the retrieval.
    pub budget: ExpansionBudget,

    /// Usage consumed so far against the budget.
    pub consumed: ExpansionBudgetUsage,

    /// Snapshot of the working set's record counters.
    pub working_set_stats: crate::GraphWorkingSetStats,

    /// Observed degrees of current frontier nodes.
    pub frontier_degrees: Vec<FrontierDegree>,

    /// Confidence scores of the seeds, when the resolver provided them.
    pub seed_confidences: Vec<Confidence>,

    /// Optional as-of timestamp scoping the retrieval temporally.
    pub as_of: Option<TemporalTimestamp>,
}

impl BanditContext {
    /// Build a context from an expansion request and query descriptor.
    ///
    ///
    /// keep call sites from hand-copying request state into the context and
    /// drifting from the request contract.
    ///
    ///
    /// copy seeds, filters, and budget from the request; start with zero
    /// consumed usage, empty stats, no frontier degrees, no seed confidences,
    /// and no as-of timestamp — callers refine those as execution progresses.
    ///
    /// # Errors
    ///
    /// none expected because the request was already validated at
    /// construction.
    pub fn from_expansion_request(
        request: &ExpansionRequest,
        descriptor: TelemetryQueryDescriptor,
    ) -> Self {
        Self {
            descriptor,
            seed_node_ids: request.seed_node_ids().to_vec(),
            relationship_type_filters: request.relationship_type_filters().to_vec(),
            label_filters: request.label_filters().clone(),
            budget: request.budget().clone(),
            consumed: ExpansionBudgetUsage {
                loaded_node_count: 0,
                loaded_relationship_count: 0,
                hot_node_count: 0,
                hot_relationship_count: 0,
                warm_adjacency_entry_count: 0,
                hop_count: 0,
                supernode_expansion_count: 0,
                payload_byte_count: 0,
                execution_time_ms: 0,
            },
            working_set_stats: crate::GraphWorkingSetStats::default(),
            frontier_degrees: Vec::new(),
            seed_confidences: Vec::new(),
            as_of: None,
        }
    }

    /// Report whether any budget counter is exhausted.
    ///
    ///
    /// give deterministic guards priority: a controller must be able to see
    /// exhaustion, and the integration issue treats exhaustion as a hard stop
    /// regardless of the learned choice.
    ///
    ///
    /// compare each consumed counter with its budget maximum.
    ///
    /// # Errors
    ///
    /// none expected because the comparison is pure.
    pub fn budget_exhausted(&self) -> bool {
        self.consumed.loaded_node_count >= self.budget.max_loaded_node_count
            || self.consumed.loaded_relationship_count >= self.budget.max_loaded_relationship_count
            || self.consumed.hot_node_count >= self.budget.max_hot_node_count
            || self.consumed.hot_relationship_count >= self.budget.max_hot_relationship_count
            || self.consumed.warm_adjacency_entry_count
                >= self.budget.max_warm_adjacency_entry_count
            || self.consumed.hop_count >= self.budget.max_hop_count
            || self.consumed.supernode_expansion_count >= self.budget.max_supernode_expansion_count
            || self.consumed.payload_byte_count >= self.budget.max_payload_byte_count
            || self.consumed.execution_time_ms >= self.budget.max_execution_time_ms
    }
}

/// Reward observed for one retrieval, reproducible from telemetry.
///
///
/// reward the epic's signals — correct evidence found, expected-subgraph
/// recall, memory cost, I/O count, latency, and wasted prefetches — from the
/// recorded stream so training never depends on live sampling.
///
///
/// carry the derived counts and caller-supplied measurements; recall is
/// attached separately by the benchmark harness.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BanditReward {
    /// Evidence records found by the retrieval.
    pub evidence_found_count: u64,

    /// Recall against the expected evidence subgraph, when ground truth exists.
    pub expected_subgraph_recall: Option<Confidence>,

    /// Memory cost of the retrieval, in bytes.
    pub memory_cost_bytes: u64,

    /// Page-in count of the retrieval.
    pub io_count: u64,

    /// End-to-end latency of the retrieval, in milliseconds.
    pub latency_ms: u64,

    /// Prefetches never used later in the retrieval.
    pub wasted_prefetch_count: u64,
}

impl BanditReward {
    /// Derive a reward from one telemetry retrieval record.
    ///
    ///
    /// make rewards reproducible: the same record always yields the same
    /// reward, which offline training and benchmark replays rely on.
    ///
    ///
    /// apply the module-level derivation rules to the record's events and
    /// outcome; records without an outcome yield zero evidence, memory, and
    /// latency.
    ///
    /// # Errors
    ///
    /// none expected because records from the telemetry recorder are already
    /// structurally valid.
    pub fn from_retrieval_record(record: &RetrievalTelemetryRecord) -> Self {
        let mut io_count = 0_u64;
        let mut prefetched: Vec<(usize, &crate::GraphRecordRef)> = Vec::new();

        for (index, event) in record.events.iter().enumerate() {
            match &event.decision {
                WorkingSetDecisionEvent::PageIn { .. } => io_count += 1,
                WorkingSetDecisionEvent::Prefetch { record } => prefetched.push((index, record)),
                _ => {}
            }
        }

        let wasted_prefetch_count = prefetched
            .iter()
            .filter(|(prefetch_index, prefetched_record)| {
                !record.events[prefetch_index + 1..]
                    .iter()
                    .any(|event| prefetch_is_used(&event.decision, prefetched_record))
            })
            .count() as u64;

        let (evidence_found_count, memory_cost_bytes, latency_ms) = match &record.outcome {
            Some(outcome) => (
                outcome.evidence_record_ids.len() as u64,
                outcome.memory_cost_bytes,
                outcome.latency_ms,
            ),
            None => (0, 0, 0),
        };

        Self {
            evidence_found_count,
            expected_subgraph_recall: None,
            memory_cost_bytes,
            io_count,
            latency_ms,
            wasted_prefetch_count,
        }
    }

    /// Attach the expected-subgraph recall measured by the benchmark harness.
    ///
    ///
    /// keep ground truth out of telemetry: recall is a benchmark measurement
    /// against a known expected subgraph, validated as a confidence.
    ///
    ///
    /// return the reward with the recall attached.
    ///
    /// # Errors
    ///
    /// none expected because `Confidence` was validated at construction.
    pub fn with_expected_subgraph_recall(mut self, recall: Confidence) -> Self {
        self.expected_subgraph_recall = Some(recall);
        self
    }

    /// Scalarize the reward with explicit weights.
    ///
    ///
    /// give bandit policies one documented scalar objective:
    ///
    /// ```text
    /// reward = evidence·w_evidence + recall·w_recall
    ///        − memory_bytes·w_memory − io·w_io
    ///        − latency_ms·w_latency − wasted_prefetch·w_wasted
    /// ```
    ///
    ///
    /// apply the combination with a missing recall contributing zero.
    ///
    /// # Errors
    ///
    /// none expected because the combination is pure arithmetic.
    pub fn scalarized(&self, weights: &BanditRewardWeights) -> f64 {
        let recall = self.expected_subgraph_recall.map_or(0.0, Confidence::value);

        self.evidence_found_count as f64 * weights.evidence_weight + recall * weights.recall_weight
            - self.memory_cost_bytes as f64 * weights.memory_weight
            - self.io_count as f64 * weights.io_weight
            - self.latency_ms as f64 * weights.latency_weight
            - self.wasted_prefetch_count as f64 * weights.wasted_prefetch_weight
    }
}

fn prefetch_is_used(
    decision: &WorkingSetDecisionEvent,
    prefetched_record: &crate::GraphRecordRef,
) -> bool {
    match decision {
        WorkingSetDecisionEvent::PageIn { record } => record == prefetched_record,
        WorkingSetDecisionEvent::EdgeExpanded { relationship_id } => matches!(
            prefetched_record,
            crate::GraphRecordRef::Relationship(prefetched) if prefetched == relationship_id
        ),
        WorkingSetDecisionEvent::SeedSelected { node_id, .. } => matches!(
            prefetched_record,
            crate::GraphRecordRef::Node(prefetched) if prefetched == node_id
        ),
        _ => false,
    }
}

/// Explicit weights of the reward scalarization.
///
///
/// keep the trade-off between evidence value and resource costs a visible,
/// auditable configuration instead of constants buried in policy code.
///
///
/// carry one weight per reward term.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BanditRewardWeights {
    /// Weight of each evidence record found.
    pub evidence_weight: f64,

    /// Weight of the expected-subgraph recall.
    pub recall_weight: f64,

    /// Penalty weight per memory byte.
    pub memory_weight: f64,

    /// Penalty weight per page-in.
    pub io_weight: f64,

    /// Penalty weight per millisecond of latency.
    pub latency_weight: f64,

    /// Penalty weight per wasted prefetch.
    pub wasted_prefetch_weight: f64,
}

/// Pluggable policy boundary over working-set actions.
///
///
/// let baseline heuristics, the contextual bandit, and the learned pheromone
/// policy swap behind one interface, so the manager integration issue can wire
/// call sites once.
///
///
/// choose one action for a context and observe the reward that followed a
/// chosen action; implementations must be deterministic for a fixed internal
/// state and context.
pub trait WorkingSetController {
    /// Choose the next working-set action for the given context.
    fn choose_action(&mut self, context: &BanditContext) -> WorkingSetAction;

    /// Observe the reward that followed a previously chosen action.
    fn observe_reward(
        &mut self,
        context: &BanditContext,
        action: WorkingSetAction,
        reward: &BanditReward,
    );
}

/// Deterministic baseline policy: expand until the budget is exhausted.
///
///
/// provide the reference behavior that preserves today's engine semantics and
/// anchors the benchmark suite's baseline comparisons.
///
///
/// choose `Expand` while every budget counter is open and `Stop` once any
/// counter is exhausted; count observed rewards for diagnostics.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GreedyExpandController {
    observed_reward_count: u64,
}

impl GreedyExpandController {
    /// Create the baseline controller.
    ///
    ///
    /// provide the stable constructor used by tests and the benchmark suite.
    ///
    ///
    /// start with zero observed rewards.
    ///
    /// # Errors
    ///
    /// none expected because the baseline holds no external state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Return how many rewards this controller has observed.
    ///
    ///
    /// expose a deterministic diagnostic for tests and benchmark reports.
    ///
    ///
    /// return the observation counter.
    ///
    /// # Errors
    ///
    /// none expected because the counter is plain state.
    pub fn observed_reward_count(&self) -> u64 {
        self.observed_reward_count
    }
}

impl WorkingSetController for GreedyExpandController {
    fn choose_action(&mut self, context: &BanditContext) -> WorkingSetAction {
        if context.budget_exhausted() {
            WorkingSetAction::Stop
        } else {
            WorkingSetAction::Expand
        }
    }

    fn observe_reward(
        &mut self,
        _context: &BanditContext,
        _action: WorkingSetAction,
        _reward: &BanditReward,
    ) {
        self.observed_reward_count += 1;
    }
}

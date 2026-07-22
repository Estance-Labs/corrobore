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
//! Multidimensional pheromone trace model with temporal decay (Epic 0017).
//!
//!
//!
//! - Represent pheromones as context-conditioned future-utility estimates per
//!   edge and per task scope, never as a frequency-only counter.
//! - Feed trace updates exclusively from recorded telemetry retrieval records,
//!   keeping the field strictly observational in this issue: applying records
//!   must never touch working-set state or expansion behavior.
//! - Apply the decay law `τ(e, t+1) = λ·τ(e, t) + reward − penalty` with one
//!   logical tick per applied record in the record's task scope; no wall-clock
//!   time enters the model.
//! - Do not implement anti-pheromone accumulation policies, bandit control, or
//!   any live working-set integration here; later issues consume these traces.
//!
//! # Reward attribution rules (deterministic)
//!
//! For one applied retrieval record in task scope `S`:
//!
//! - each `EdgeExpanded` occurrence rewards `access_frequency` by 1;
//! - the first observation of an edge in `S` rewards `novelty_gain` by 1;
//! - each record an edge participates in rewards `task_affinity` by 1;
//! - `PageIn` count divided by the expanded-edge count rewards
//!   `traversal_cost` for each expanded edge;
//! - outcome evidence count divided by the expanded-edge count rewards
//!   `evidence_gain`; outcome answer quality rewards `downstream_success`;
//! - a `DeadEnd` node penalizes `dead_end_rate` of the edges that admitted it,
//!   using the `EdgeExpanded -> SeedSelected` pairing of the engine stream;
//! - `contradiction_rate`, `staleness`, and `poisoning_risk` are reserved
//!   dimensions: no telemetry signal feeds them yet, but they decay like every
//!   other dimension once populated by future validators.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{
    Confidence, GraphError,
    ids::{NodeId, RelationshipId},
    working_set_telemetry::{RetrievalTelemetryRecord, WorkingSetDecisionEvent},
};

/// Validated temporal decay factor `λ` applied per task-scope tick.
///
///
/// keep trace dynamics stable and reproducible by rejecting factors outside
/// `[0, 1]` and NaN at the boundary instead of letting them corrupt the field.
///
///
/// wrap the validated factor as a copyable primitive.
///
/// # Errors
///
/// construction returns `GraphError::InvalidPheromoneDecay` for NaN or
/// out-of-range values.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PheromoneDecay(f64);

impl PheromoneDecay {
    /// Validate and wrap a decay factor.
    ///
    ///
    /// make invalid decay dynamics unrepresentable past this boundary.
    ///
    ///
    /// accept factors in `[0, 1]`; reject NaN and out-of-range values.
    ///
    /// # Errors
    ///
    /// return `GraphError::InvalidPheromoneDecay` when the factor is NaN or
    /// outside `[0, 1]`.
    pub fn new(value: f64) -> Result<Self, GraphError> {
        if value.is_nan() || !(0.0..=1.0).contains(&value) {
            return Err(GraphError::InvalidPheromoneDecay(value));
        }

        Ok(Self(value))
    }

    /// Return the inner decay factor.
    ///
    ///
    /// expose the validated value for decay computation and reporting.
    ///
    ///
    /// return the wrapped factor.
    ///
    /// # Errors
    ///
    /// none expected because the factor was validated at construction.
    pub fn value(self) -> f64 {
        self.0
    }
}

/// Task scope of a pheromone trace.
///
///
/// make traces multidimensional per task family: an edge that is valuable for
/// a FIMI investigation is not necessarily valuable for malware attribution.
///
///
/// map the optional telemetry task label onto an explicit scope key, with a
/// generic scope for unlabeled retrievals.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PheromoneTaskScope {
    /// Scope for retrievals without a task label.
    Generic,

    /// Scope for retrievals labeled with a task family.
    Task(String),
}

impl PheromoneTaskScope {
    /// Build a task scope from a task-family label.
    ///
    ///
    /// give call sites a readable constructor instead of enum plumbing.
    ///
    ///
    /// wrap the label as a `Task` scope.
    ///
    /// # Errors
    ///
    /// none expected because any label string is a valid scope key.
    pub fn task(label: &str) -> Self {
        Self::Task(label.to_owned())
    }

    /// Build a scope from the optional telemetry task label.
    ///
    ///
    /// centralize the mapping between telemetry descriptors and scope keys so
    /// every consumer buckets records identically.
    ///
    ///
    /// return `Task` when a label is present and `Generic` otherwise.
    ///
    /// # Errors
    ///
    /// none expected because both branches are total.
    pub fn from_label(label: Option<&str>) -> Self {
        match label {
            Some(label) => Self::Task(label.to_owned()),
            None => Self::Generic,
        }
    }
}

/// Multidimensional utility vector of one edge in one task scope.
///
///
/// estimate expected downstream value per edge instead of raw access counts;
/// every dimension is a decaying trace, not a lifetime counter.
///
///
/// carry the ten dimensions of the Epic 0017 `EdgeUtility` vector as decayed
/// trace values at a given scope tick.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EdgeUtility {
    /// Decayed access trace fed by edge expansions.
    pub access_frequency: f64,

    /// Decayed success trace fed by outcome answer quality.
    pub downstream_success: f64,

    /// Decayed evidence trace fed by outcome evidence counts.
    pub evidence_gain: f64,

    /// Decayed novelty trace rewarding first observations in the scope.
    pub novelty_gain: f64,

    /// Decayed cost trace fed by page-in attribution.
    pub traversal_cost: f64,

    /// Decayed dead-end trace fed by dead-end attribution.
    pub dead_end_rate: f64,

    /// Reserved: contradiction signal from future epistemic validators.
    pub contradiction_rate: f64,

    /// Reserved: staleness signal from future temporal validators.
    pub staleness: f64,

    /// Reserved: poisoning signal from future integrity validators.
    pub poisoning_risk: f64,

    /// Decayed task-participation trace.
    pub task_affinity: f64,
}

/// Query-context inputs of the utility combination.
///
///
/// keep query-dependent terms (semantic and temporal relevance) out of stored
/// traces: they belong to the question being asked, not to the edge history.
///
///
/// carry the caller-provided context terms of the utility formula.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct UtilityContext {
    /// Semantic relevance of the edge for the current query.
    pub semantic_relevance: f64,

    /// Temporal relevance of the edge for the current query.
    pub temporal_relevance: f64,
}

/// Combine one edge-utility vector with query context into a utility score.
///
///
/// implement the Epic 0017 combination contract so every consumer ranks edges
/// identically:
///
/// ```text
/// utility(edge, query, agent, task) =
///       semantic_relevance + historical_success + expected_information_gain
///     + evidence_reliability + temporal_relevance
///     - loading_cost - dead_end_probability - integrity_risk
/// ```
///
///
/// map trace dimensions onto the formula terms: `downstream_success` is the
/// historical success; `evidence_gain + novelty_gain` is the expected
/// information gain; evidence reliability enters as the negative contribution
/// of `contradiction_rate + staleness`; `traversal_cost` is the loading cost;
/// `dead_end_rate` is the dead-end probability; `poisoning_risk` is the
/// integrity risk.
///
/// # Errors
///
/// none expected because the combination is a pure arithmetic mapping.
pub fn edge_utility_score(utility: &EdgeUtility, context: &UtilityContext) -> f64 {
    context.semantic_relevance
        + utility.downstream_success
        + (utility.evidence_gain + utility.novelty_gain)
        - (utility.contradiction_rate + utility.staleness)
        + context.temporal_relevance
        - utility.traversal_cost
        - utility.dead_end_rate
        - utility.poisoning_risk
}

/// Task-scoped pheromone field fed from telemetry retrieval records.
///
///
/// own the observational trace store that later issues (anti-pheromone
/// policies, bandit controller) read; this issue keeps it decoupled from the
/// working-set manager so applying records can never alter engine behavior.
///
///
/// maintain per-scope logical ticks and per-edge decaying utility traces,
/// updated by the documented reward attribution rules.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PheromoneField {
    decay: PheromoneDecay,
    scopes: HashMap<PheromoneTaskScope, PheromoneScopeState>,
}

/// Per-scope tick counter and trace store.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct PheromoneScopeState {
    tick: u64,
    traces: HashMap<RelationshipId, PheromoneTrace>,
}

/// Raw trace values of one edge with their last-update tick for lazy decay.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct PheromoneTrace {
    utility: EdgeUtility,
    last_update_tick: u64,
}

impl PheromoneField {
    /// Create an empty pheromone field with the given decay factor.
    ///
    ///
    /// provide the stable constructor used before any record is applied.
    ///
    ///
    /// initialize an empty scope map; no trace exists until a record is
    /// applied.
    ///
    /// # Errors
    ///
    /// none expected because the decay factor was validated at construction.
    pub fn new(decay: PheromoneDecay) -> Self {
        Self {
            decay,
            scopes: HashMap::new(),
        }
    }

    /// Return the current logical tick of a task scope.
    ///
    ///
    /// let consumers reason about decay depth and distinguish "never
    /// observed" scopes from active ones.
    ///
    ///
    /// return the number of records applied in the scope, zero when the scope
    /// has never been touched.
    ///
    /// # Errors
    ///
    /// none expected because unknown scopes deterministically read as zero.
    pub fn scope_tick(&self, scope: &PheromoneTaskScope) -> u64 {
        self.scopes.get(scope).map_or(0, |state| state.tick)
    }

    /// Return the decayed utility of one edge in one task scope.
    ///
    ///
    /// expose trace values at the scope's current tick so readers always see
    /// time-consistent utilities regardless of when the edge was last updated.
    ///
    ///
    /// apply lazy decay `λ^(tick − last_update)` to the raw trace and return
    /// the decayed vector, or `None` when the edge was never observed in the
    /// scope.
    ///
    /// # Errors
    ///
    /// none expected because absence is a deterministic `None`.
    pub fn edge_utility(
        &self,
        relationship_id: &RelationshipId,
        scope: &PheromoneTaskScope,
    ) -> Option<EdgeUtility> {
        let state = self.scopes.get(scope)?;
        let trace = state.traces.get(relationship_id)?;
        let factor = decay_factor(self.decay, state.tick - trace.last_update_tick);
        Some(scale_utility(&trace.utility, factor))
    }

    /// Apply one telemetry retrieval record to the field.
    ///
    ///
    /// advance the record's task scope by one tick and update the traces of
    /// every edge the record touched, following the module-level reward
    /// attribution rules; this is the only mutation path of the field.
    ///
    ///
    /// derive rewards and penalties from the record's events and outcome, then
    /// fold them into decayed traces.
    ///
    /// # Errors
    ///
    /// none expected: records derived from the telemetry recorder are already
    /// structurally valid, and unknown event variants are ignored by design.
    pub fn apply_retrieval_record(&mut self, record: &RetrievalTelemetryRecord) {
        let scope = PheromoneTaskScope::from_label(record.descriptor.task_label.as_deref());
        let state = self.scopes.entry(scope).or_default();
        state.tick += 1;
        let tick = state.tick;

        let observation = RecordObservation::from_record(record);
        if observation.expanded_occurrences.is_empty() && observation.dead_end_edges.is_empty() {
            return;
        }

        let mut deltas: HashMap<RelationshipId, EdgeUtility> = HashMap::new();

        let expanded_count = observation.expanded_occurrences.len() as f64;
        let page_in_share = observation.page_in_count as f64 / expanded_count.max(1.0);
        let (evidence_share, answer_quality) = match &record.outcome {
            Some(outcome) => (
                outcome.evidence_record_ids.len() as f64 / expanded_count.max(1.0),
                outcome.answer_quality.map_or(0.0, Confidence::value),
            ),
            None => (0.0, 0.0),
        };

        for relationship_id in &observation.expanded_occurrences {
            let delta = deltas.entry(relationship_id.clone()).or_default();
            delta.access_frequency += 1.0;
            delta.traversal_cost += page_in_share;
            delta.evidence_gain += evidence_share;
            delta.downstream_success += answer_quality;
        }

        for relationship_id in observation.unique_expanded_edges() {
            let is_novel = !state.traces.contains_key(&relationship_id);
            let delta = deltas.entry(relationship_id).or_default();
            delta.task_affinity += 1.0;
            if is_novel {
                delta.novelty_gain += 1.0;
            }
        }

        for relationship_id in &observation.dead_end_edges {
            deltas
                .entry(relationship_id.clone())
                .or_default()
                .dead_end_rate += 1.0;
        }

        for (relationship_id, delta) in deltas {
            let trace = state
                .traces
                .entry(relationship_id)
                .or_insert(PheromoneTrace {
                    utility: EdgeUtility::default(),
                    last_update_tick: tick,
                });
            let factor = decay_factor(self.decay, tick - trace.last_update_tick);
            trace.utility = add_utility(&scale_utility(&trace.utility, factor), &delta);
            trace.last_update_tick = tick;
        }
    }
}

/// Deterministic per-record observation extracted from the telemetry stream.
///
///
/// separate stream parsing from reward arithmetic so the attribution rules in
/// the module documentation stay auditable in one place.
///
///
/// collect expanded-edge occurrences in order, page-in counts, and the edges
/// that admitted later dead-end nodes via the `EdgeExpanded -> SeedSelected`
/// pairing.
struct RecordObservation {
    expanded_occurrences: Vec<RelationshipId>,
    page_in_count: u64,
    dead_end_edges: Vec<RelationshipId>,
}

impl RecordObservation {
    fn from_record(record: &RetrievalTelemetryRecord) -> Self {
        let mut expanded_occurrences = Vec::new();
        let mut page_in_count = 0_u64;
        let mut admitted_by: HashMap<NodeId, RelationshipId> = HashMap::new();
        let mut pending_admission: Option<RelationshipId> = None;
        let mut dead_end_edges = Vec::new();

        for event in &record.events {
            match &event.decision {
                WorkingSetDecisionEvent::EdgeExpanded { relationship_id } => {
                    expanded_occurrences.push(relationship_id.clone());
                    pending_admission = Some(relationship_id.clone());
                }
                WorkingSetDecisionEvent::SeedSelected { node_id, .. } => {
                    if let Some(relationship_id) = pending_admission.take() {
                        admitted_by.insert(node_id.clone(), relationship_id);
                    }
                }
                WorkingSetDecisionEvent::PageIn { .. } => {
                    page_in_count += 1;
                }
                WorkingSetDecisionEvent::DeadEnd { node_id } => {
                    if let Some(relationship_id) = admitted_by.get(node_id) {
                        dead_end_edges.push(relationship_id.clone());
                    }
                }
                _ => {}
            }
        }

        Self {
            expanded_occurrences,
            page_in_count,
            dead_end_edges,
        }
    }

    fn unique_expanded_edges(&self) -> Vec<RelationshipId> {
        let mut unique = Vec::new();
        for relationship_id in &self.expanded_occurrences {
            if !unique.contains(relationship_id) {
                unique.push(relationship_id.clone());
            }
        }
        unique
    }
}

fn decay_factor(decay: PheromoneDecay, elapsed_ticks: u64) -> f64 {
    decay
        .value()
        .powi(elapsed_ticks.min(i32::MAX as u64) as i32)
}

fn scale_utility(utility: &EdgeUtility, factor: f64) -> EdgeUtility {
    EdgeUtility {
        access_frequency: utility.access_frequency * factor,
        downstream_success: utility.downstream_success * factor,
        evidence_gain: utility.evidence_gain * factor,
        novelty_gain: utility.novelty_gain * factor,
        traversal_cost: utility.traversal_cost * factor,
        dead_end_rate: utility.dead_end_rate * factor,
        contradiction_rate: utility.contradiction_rate * factor,
        staleness: utility.staleness * factor,
        poisoning_risk: utility.poisoning_risk * factor,
        task_affinity: utility.task_affinity * factor,
    }
}

fn add_utility(base: &EdgeUtility, delta: &EdgeUtility) -> EdgeUtility {
    EdgeUtility {
        access_frequency: base.access_frequency + delta.access_frequency,
        downstream_success: base.downstream_success + delta.downstream_success,
        evidence_gain: base.evidence_gain + delta.evidence_gain,
        novelty_gain: base.novelty_gain + delta.novelty_gain,
        traversal_cost: base.traversal_cost + delta.traversal_cost,
        dead_end_rate: base.dead_end_rate + delta.dead_end_rate,
        contradiction_rate: base.contradiction_rate + delta.contradiction_rate,
        staleness: base.staleness + delta.staleness,
        poisoning_risk: base.poisoning_risk + delta.poisoning_risk,
        task_affinity: base.task_affinity + delta.task_affinity,
    }
}

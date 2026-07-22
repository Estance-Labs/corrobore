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
//! Anti-pheromone negative navigation field (Epic 0017).
//!
//!
//!
//! - Learn zones to avoid: turn navigation guidance into a positive and
//!   negative field instead of an attraction-only mechanism.
//! - Feed negative traces from recorded telemetry (dead ends, profile-blocked
//!   expansions, supernode blocks) and from explicit validator reports (stale
//!   evidence, contradictory paths, suspected poisoning) for the epistemic and
//!   immune-system epics.
//! - Reuse the pheromone decay law and task scoping: one logical tick per
//!   applied record per scope, lazy decay on read, no wall-clock time.
//! - Keep deterministic supernode protection authoritative: this field is a
//!   passive observer that complements, and never replaces, the expansion
//!   budget and supernode guards.
//!
//! # Negative attribution rules (deterministic)
//!
//! For one applied retrieval record in task scope `S`:
//!
//! - the field remembers, per scope, which edge admitted each node through the
//!   `EdgeExpanded -> SeedSelected` pairing, across records;
//! - a `DeadEnd` node penalizes `dead_end` on its remembered admitting edge;
//! - an `EdgeSkipped` decision with reason `BlockedByProfile` or
//!   `LowRelevance` penalizes `irrelevant_expansion` on the skipped
//!   relationship;
//! - a `SupernodeBlocked` node penalizes `supernode_explosion` on its
//!   remembered admitting edge; blocks on nodes with no admission history
//!   (plain seeds) are not attributed to any edge;
//! - `stale_evidence`, `contradictory_path`, and `suspected_poisoning`
//!   accumulate only through `report_negative_observation`, applied at the
//!   scope's current tick without advancing it.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{
    ids::{NodeId, RelationshipId},
    pheromone_trace::{
        EdgeUtility, PheromoneDecay, PheromoneTaskScope, UtilityContext, edge_utility_score,
    },
    working_set_explanation::SkippedExpansionReason,
    working_set_telemetry::{RetrievalTelemetryRecord, WorkingSetDecisionEvent},
};

/// Negative utility vector of one edge in one task scope.
///
///
/// represent the epic's anti-pheromone contribution model as explicit decaying
/// dimensions instead of one opaque penalty number, so consumers can explain
/// why an edge is avoided.
///
///
/// carry the six negative dimensions; `total()` is the combined field value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AntiPheromoneVector {
    /// Decayed dead-end trace fed by dead-end attribution.
    pub dead_end: f64,

    /// Decayed irrelevance trace fed by profile and relevance skips.
    pub irrelevant_expansion: f64,

    /// Decayed supernode trace fed by supernode-block attribution.
    pub supernode_explosion: f64,

    /// Reserved: staleness reports from future temporal validators.
    pub stale_evidence: f64,

    /// Reserved: contradiction reports from future epistemic validators.
    pub contradictory_path: f64,

    /// Reserved: poisoning reports from future integrity validators.
    pub suspected_poisoning: f64,
}

impl AntiPheromoneVector {
    /// Return the combined anti-pheromone value of the edge.
    ///
    ///
    /// implement the epic's contribution model
    /// `anti_pheromone(edge) = dead_end + irrelevant_expansion +
    /// supernode_explosion + stale_evidence + contradictory_path +
    /// suspected_poisoning` so every consumer combines dimensions identically.
    ///
    ///
    /// sum the six dimensions.
    ///
    /// # Errors
    ///
    /// none expected because the total is a pure arithmetic sum.
    pub fn total(&self) -> f64 {
        self.dead_end
            + self.irrelevant_expansion
            + self.supernode_explosion
            + self.stale_evidence
            + self.contradictory_path
            + self.suspected_poisoning
    }
}

/// Externally reported negative signal on one edge.
///
///
/// give the epistemic graph (Epic 0018) and the graph immune system (Epic
/// 0019) a typed reporting path into the negative field before those
/// validators exist, keeping the reserved dimensions well-defined.
///
///
/// name the three validator-fed dimensions; telemetry-fed dimensions have no
/// external variant on purpose.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AntiPheromoneSignal {
    /// The edge is supported by stale evidence.
    StaleEvidence,

    /// The edge lies on a path with contradictory evidence.
    ContradictoryPath,

    /// The edge is suspected to result from graph poisoning.
    SuspectedPoisoning,
}

/// Combine positive utility and negative field into one navigation score.
///
///
/// make the navigation field explicitly two-sided: the epic's utility
/// combination attracts, the anti-pheromone total repels, and ranking uses
/// their difference.
///
///
/// return `edge_utility_score(utility, context) − anti.total()`.
///
/// # Errors
///
/// none expected because the combination is a pure arithmetic mapping.
pub fn navigation_field_score(
    utility: &EdgeUtility,
    anti: &AntiPheromoneVector,
    context: &UtilityContext,
) -> f64 {
    edge_utility_score(utility, context) - anti.total()
}

/// Task-scoped anti-pheromone field fed from telemetry and validator reports.
///
///
/// own the negative navigation field consumed by the future bandit controller;
/// this issue keeps it decoupled from the working-set manager so applying
/// records can never alter engine behavior or weaken deterministic guards.
///
///
/// maintain per-scope ticks, per-edge negative traces with lazy decay, and a
/// per-scope admission memory pairing nodes with the edge that admitted them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AntiPheromoneField {
    decay: PheromoneDecay,
    scopes: HashMap<PheromoneTaskScope, AntiPheromoneScopeState>,
}

/// Per-scope tick counter, negative traces, and admission memory.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct AntiPheromoneScopeState {
    tick: u64,
    traces: HashMap<RelationshipId, AntiPheromoneTrace>,
    admitted_by: HashMap<NodeId, RelationshipId>,
}

/// Raw negative trace values with their last-update tick for lazy decay.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct AntiPheromoneTrace {
    vector: AntiPheromoneVector,
    last_update_tick: u64,
}

impl AntiPheromoneField {
    /// Create an empty anti-pheromone field with the given decay factor.
    ///
    ///
    /// provide the stable constructor used before any record or report is
    /// applied.
    ///
    ///
    /// initialize an empty scope map; no trace exists until a negative signal
    /// is observed.
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
    /// let consumers reason about penalty decay depth per scope.
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

    /// Return the decayed anti-pheromone vector of one edge in one scope.
    ///
    ///
    /// expose penalties at the scope's current tick so old penalties fade
    /// instead of permanently blacklisting an edge.
    ///
    ///
    /// apply lazy decay to the stored trace and return the decayed vector, or
    /// `None` when the edge was never penalized in the scope.
    ///
    /// # Errors
    ///
    /// none expected because absence is a deterministic `None`.
    pub fn edge_anti_pheromone(
        &self,
        relationship_id: &RelationshipId,
        scope: &PheromoneTaskScope,
    ) -> Option<AntiPheromoneVector> {
        let state = self.scopes.get(scope)?;
        let trace = state.traces.get(relationship_id)?;
        let factor = decay_factor(self.decay, state.tick - trace.last_update_tick);
        Some(scale_vector(&trace.vector, factor))
    }

    /// Apply one telemetry retrieval record to the negative field.
    ///
    ///
    /// advance the record's task scope by one tick, refresh the admission
    /// memory, and accumulate the telemetry-fed negative signals following the
    /// module-level attribution rules; this and `report_negative_observation`
    /// are the only mutation paths of the field.
    ///
    ///
    /// derive dead-end, irrelevance, and supernode penalties from the record's
    /// events and fold them into decayed traces.
    ///
    /// # Errors
    ///
    /// none expected: records derived from the telemetry recorder are already
    /// structurally valid, and unattributable signals are ignored by design.
    pub fn apply_retrieval_record(&mut self, record: &RetrievalTelemetryRecord) {
        let scope = PheromoneTaskScope::from_label(record.descriptor.task_label.as_deref());
        let state = self.scopes.entry(scope).or_default();
        state.tick += 1;
        let tick = state.tick;

        let mut deltas: HashMap<RelationshipId, AntiPheromoneVector> = HashMap::new();
        let mut pending_admission: Option<RelationshipId> = None;

        for event in &record.events {
            match &event.decision {
                WorkingSetDecisionEvent::EdgeExpanded { relationship_id } => {
                    pending_admission = Some(relationship_id.clone());
                }
                WorkingSetDecisionEvent::SeedSelected { node_id, .. } => {
                    if let Some(relationship_id) = pending_admission.take() {
                        state.admitted_by.insert(node_id.clone(), relationship_id);
                    }
                }
                WorkingSetDecisionEvent::DeadEnd { node_id } => {
                    if let Some(relationship_id) = state.admitted_by.get(node_id) {
                        deltas.entry(relationship_id.clone()).or_default().dead_end += 1.0;
                    }
                }
                WorkingSetDecisionEvent::SupernodeBlocked { node_id } => {
                    if let Some(relationship_id) = state.admitted_by.get(node_id) {
                        deltas
                            .entry(relationship_id.clone())
                            .or_default()
                            .supernode_explosion += 1.0;
                    }
                }
                WorkingSetDecisionEvent::EdgeSkipped {
                    relationship_id: Some(relationship_id),
                    reason:
                        SkippedExpansionReason::BlockedByProfile | SkippedExpansionReason::LowRelevance,
                    ..
                } => {
                    deltas
                        .entry(relationship_id.clone())
                        .or_default()
                        .irrelevant_expansion += 1.0;
                }
                _ => {}
            }
        }

        for (relationship_id, delta) in deltas {
            let trace = state
                .traces
                .entry(relationship_id)
                .or_insert(AntiPheromoneTrace {
                    vector: AntiPheromoneVector::default(),
                    last_update_tick: tick,
                });
            let factor = decay_factor(self.decay, tick - trace.last_update_tick);
            trace.vector = add_vectors(&scale_vector(&trace.vector, factor), &delta);
            trace.last_update_tick = tick;
        }
    }

    /// Report one validator-fed negative observation on an edge.
    ///
    ///
    /// let epistemic and immune-system validators populate the reserved
    /// dimensions before those epics land, at the scope's current tick.
    ///
    ///
    /// decay the edge trace to the current tick and add one unit to the
    /// reported dimension without advancing the scope tick.
    ///
    /// # Errors
    ///
    /// none expected because any edge may legitimately receive its first
    /// report before any telemetry observation.
    pub fn report_negative_observation(
        &mut self,
        scope: &PheromoneTaskScope,
        relationship_id: &RelationshipId,
        signal: AntiPheromoneSignal,
    ) {
        let state = self.scopes.entry(scope.clone()).or_default();
        let tick = state.tick;
        let trace = state
            .traces
            .entry(relationship_id.clone())
            .or_insert(AntiPheromoneTrace {
                vector: AntiPheromoneVector::default(),
                last_update_tick: tick,
            });
        let factor = decay_factor(self.decay, tick - trace.last_update_tick);
        let mut vector = scale_vector(&trace.vector, factor);
        match signal {
            AntiPheromoneSignal::StaleEvidence => vector.stale_evidence += 1.0,
            AntiPheromoneSignal::ContradictoryPath => vector.contradictory_path += 1.0,
            AntiPheromoneSignal::SuspectedPoisoning => vector.suspected_poisoning += 1.0,
        }
        trace.vector = vector;
        trace.last_update_tick = tick;
    }
}

fn decay_factor(decay: PheromoneDecay, elapsed_ticks: u64) -> f64 {
    decay
        .value()
        .powi(elapsed_ticks.min(i32::MAX as u64) as i32)
}

fn scale_vector(vector: &AntiPheromoneVector, factor: f64) -> AntiPheromoneVector {
    AntiPheromoneVector {
        dead_end: vector.dead_end * factor,
        irrelevant_expansion: vector.irrelevant_expansion * factor,
        supernode_explosion: vector.supernode_explosion * factor,
        stale_evidence: vector.stale_evidence * factor,
        contradictory_path: vector.contradictory_path * factor,
        suspected_poisoning: vector.suspected_poisoning * factor,
    }
}

fn add_vectors(base: &AntiPheromoneVector, delta: &AntiPheromoneVector) -> AntiPheromoneVector {
    AntiPheromoneVector {
        dead_end: base.dead_end + delta.dead_end,
        irrelevant_expansion: base.irrelevant_expansion + delta.irrelevant_expansion,
        supernode_explosion: base.supernode_explosion + delta.supernode_explosion,
        stale_evidence: base.stale_evidence + delta.stale_evidence,
        contradictory_path: base.contradictory_path + delta.contradictory_path,
        suspected_poisoning: base.suspected_poisoning + delta.suspected_poisoning,
    }
}

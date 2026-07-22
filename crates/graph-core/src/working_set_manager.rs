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
//! In-memory working set manager contract.
//!
//!
//!
//! - Implement deterministic in-memory working-set lifecycle and record tracking.
//! - Keep manager behavior limited to creation, lookup, seed loading, hot
//!   relationship tracking, warm adjacency tracking, pinning, dirty tracking,
//!   stats retrieval, and explanation retrieval.
//! - Do not implement expansion, lazy page-in, prefetch, eviction, semantic search,
//!   persistent storage, graph traversal, or query execution here.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::{
    GraphError, GraphWorkingSet, GraphWorkingSetStats, NodeId, RelationshipId, RequestId,
    RetrievalOutcome, TelemetryQueryDescriptor, WarmAdjacencyEntry, WorkingSetDecisionEvent,
    WorkingSetExplanation, WorkingSetId, WorkingSetTelemetryRecorder,
};

/// Request object used to create an in-memory working set through the manager.
///
///
/// keep manager creation inputs explicit and extensible before future implementations add
/// budget, profile, seed-source, or caller metadata.
///
///
/// create a request that carries the target working-set ID into the manager
/// without exposing raw identifier strings.
///
/// # Errors
///
/// request construction should not fail because `WorkingSetId` validation already
/// happens before this type is built.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GraphWorkingSetCreateRequest {
    working_set_id: WorkingSetId,
}

/// Deterministic hot-record budget applied to one working set.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkingSetHotBudget {
    /// Maximum hot nodes to retain.
    pub max_hot_node_count: u64,
    /// Maximum hot relationships to retain.
    pub max_hot_relationship_count: u64,
}

impl WorkingSetHotBudget {
    /// Create a deterministic hot-record budget.
    pub fn new(max_hot_node_count: u64, max_hot_relationship_count: u64) -> Self {
        Self {
            max_hot_node_count,
            max_hot_relationship_count,
        }
    }
}

/// Deterministic eviction outcome after enforcing a hot-record budget.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkingSetEvictionOutcome {
    /// Hot nodes evicted in deterministic order.
    pub evicted_hot_node_ids: Vec<NodeId>,
    /// Hot relationships evicted in deterministic order.
    pub evicted_hot_relationship_ids: Vec<RelationshipId>,
}

impl GraphWorkingSetCreateRequest {
    /// Build a request for creating one in-memory working set.
    ///
    ///
    /// make the creation path readable at call sites and avoid passing raw
    /// identifiers directly into manager lifecycle methods.
    ///
    ///
    /// preserve the validated working-set ID supplied by the caller.
    ///
    /// # Errors
    ///
    /// none expected because ID validation belongs to `WorkingSetId::new`.
    pub fn new(working_set_id: WorkingSetId) -> Self {
        Self { working_set_id }
    }

    /// Return the working-set ID requested by the caller.
    ///
    ///
    /// expose request metadata without letting callers mutate the request after
    /// creation.
    ///
    ///
    /// return the same typed ID supplied at construction time.
    ///
    /// # Errors
    ///
    /// none expected because the request always owns a validated ID.
    pub fn working_set_id(&self) -> &WorkingSetId {
        &self.working_set_id
    }
}

/// In-memory owner for bounded graph working sets.
///
///
/// centralize lifecycle and record-tracking operations that should not live on
/// the bare `GraphWorkingSet` data model once agents start creating and querying
/// multiple working sets.
///
///
/// store working sets by `WorkingSetId`, provide deterministic lookups, update
/// hot, warm, pinned, and dirty record state through manager methods, and expose
/// stats plus explanation data for callers.
///
/// # Errors
///
/// missing working-set lookups return `GraphError::WorkingSetNotFound`. Duplicate
/// creation and broken manager-owned explanation state return typed invariant
/// errors rather than panicking.
#[derive(Clone, Debug, Default)]
pub struct GraphWorkingSetManager {
    working_sets: HashMap<WorkingSetId, GraphWorkingSet>,
    explanations: HashMap<WorkingSetId, WorkingSetExplanation>,
    telemetry: HashMap<WorkingSetId, WorkingSetTelemetryRecorder>,
}

impl GraphWorkingSetManager {
    /// Create an empty in-memory working set manager.
    ///
    ///
    /// provide the stable constructor used before creating or retrieving any
    /// bounded working set.
    ///
    ///
    /// initialize manager-owned maps for working sets and explanation data without
    /// opening storage, loading graph records, or starting background work.
    ///
    /// # Errors
    ///
    /// none expected because an empty in-memory manager has no external dependency.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create and register a working set from a typed request object.
    ///
    ///
    /// make working-set lifecycle explicit at the manager boundary instead of
    /// letting callers allocate unmanaged `GraphWorkingSet` values.
    ///
    ///
    /// store the new working set by ID, initialize explanation data for that ID,
    /// and return a read-only view of the stored set.
    ///
    /// # Errors
    ///
    /// return `GraphError::InternalInvariantViolation` when the working-set ID is
    /// already registered because this issue does not define replacement semantics.
    pub fn create_working_set(
        &mut self,
        request: GraphWorkingSetCreateRequest,
    ) -> Result<&GraphWorkingSet, GraphError> {
        let working_set_id = request.working_set_id;

        if self.working_sets.contains_key(&working_set_id) {
            return Err(GraphError::InternalInvariantViolation(format!(
                "working set already exists: {}",
                working_set_id.as_str()
            )));
        }

        self.working_sets.insert(
            working_set_id.clone(),
            GraphWorkingSet::new(working_set_id.clone()),
        );
        self.explanations
            .insert(working_set_id.clone(), WorkingSetExplanation::new());
        self.telemetry.insert(
            working_set_id.clone(),
            WorkingSetTelemetryRecorder::new(working_set_id.clone()),
        );

        self.get_working_set(&working_set_id)
    }

    /// Retrieve an existing working set by ID.
    ///
    ///
    /// provide the main read path for agents, query planners, and tests that need
    /// to inspect a bounded working set without taking ownership of it.
    ///
    ///
    /// return the stored working set when present.
    ///
    /// # Errors
    ///
    /// return `GraphError::WorkingSetNotFound` when no working set exists for the
    /// requested ID.
    pub fn get_working_set(
        &self,
        working_set_id: &WorkingSetId,
    ) -> Result<&GraphWorkingSet, GraphError> {
        self.working_sets
            .get(working_set_id)
            .ok_or_else(|| GraphError::WorkingSetNotFound(working_set_id.clone()))
    }

    /// Load seed node IDs into an existing working set.
    ///
    ///
    /// let semantic search, explicit caller input, or query planning define the
    /// ring-0 entry points for a working set while the manager owns the mutation.
    ///
    ///
    /// record each seed node on the target working set and optionally mark those
    /// seed nodes as hot when requested by the caller.
    ///
    /// # Errors
    ///
    /// return `GraphError::WorkingSetNotFound` when the target working set is
    /// missing. Invalid node IDs should already be rejected by `NodeId::new`.
    pub fn load_seed_node_ids(
        &mut self,
        working_set_id: &WorkingSetId,
        seed_node_ids: impl IntoIterator<Item = NodeId>,
        mark_as_hot: bool,
    ) -> Result<&GraphWorkingSet, GraphError> {
        let working_set = self.get_working_set_mut(working_set_id)?;
        let mut loaded_node_ids = Vec::new();

        for node_id in seed_node_ids {
            working_set.record_seed_node(node_id.clone());
            loaded_node_ids.push(node_id.clone());

            if mark_as_hot {
                working_set.track_hot_node(node_id);
            }
        }

        for node_id in loaded_node_ids {
            self.record_decision_for_existing(
                working_set_id,
                WorkingSetDecisionEvent::SeedSelected {
                    node_id,
                    marked_hot: mark_as_hot,
                },
            )?;
        }

        self.get_working_set(working_set_id)
    }

    /// Add one hot relationship to an existing working set.
    ///
    ///
    /// keep relationship-level hot tracking under the manager once the working set
    /// is created, so future implementations can attach explanation and budget metadata.
    ///
    ///
    /// mark the relationship as hot in the target working set and keep stats
    /// consistent.
    ///
    /// # Errors
    ///
    /// return `GraphError::WorkingSetNotFound` when the target working set is
    /// missing. Invalid relationship IDs should already be rejected by construction.
    pub fn add_hot_relationship(
        &mut self,
        working_set_id: &WorkingSetId,
        relationship_id: RelationshipId,
    ) -> Result<&GraphWorkingSet, GraphError> {
        let working_set = self.get_working_set_mut(working_set_id)?;
        working_set.track_hot_relationship(relationship_id.clone());

        self.record_decision_for_existing(
            working_set_id,
            WorkingSetDecisionEvent::EdgeExpanded { relationship_id },
        )?;

        self.get_working_set(working_set_id)
    }

    /// Add one warm adjacency entry to an existing working set.
    ///
    ///
    /// let the manager attach lightweight frontier metadata without requiring full
    /// node or relationship payloads to become hot.
    ///
    ///
    /// attach the entry under the provided source node, preserve insertion order,
    /// and update warm stats through the working set.
    ///
    /// # Errors
    ///
    /// return `GraphError::WorkingSetNotFound` when the target working set is
    /// missing. Return the working-set warm-adjacency validation error if the entry
    /// source and provided source do not match.
    pub fn add_warm_adjacency(
        &mut self,
        working_set_id: &WorkingSetId,
        source_node_id: NodeId,
        entry: WarmAdjacencyEntry,
    ) -> Result<&GraphWorkingSet, GraphError> {
        let relationship_id = entry.relationship_id().clone();
        let target_node_id = entry.target_node_id().clone();

        let working_set = self.get_working_set_mut(working_set_id)?;
        working_set.attach_warm_adjacency(source_node_id.clone(), entry)?;

        self.record_decision_for_existing(
            working_set_id,
            WorkingSetDecisionEvent::WarmAdjacencyAttached {
                source_node_id,
                relationship_id,
                target_node_id,
            },
        )?;

        self.get_working_set(working_set_id)
    }

    /// Pin a node in an existing working set.
    ///
    ///
    /// represent records that future eviction logic must preserve without
    /// implementing eviction in this issue.
    ///
    ///
    /// add the node ID to the working set's pinned node collection and leave
    /// loading state unchanged.
    ///
    /// # Errors
    ///
    /// return `GraphError::WorkingSetNotFound` when the target working set is
    /// missing.
    pub fn pin_node(
        &mut self,
        working_set_id: &WorkingSetId,
        node_id: NodeId,
    ) -> Result<&GraphWorkingSet, GraphError> {
        let working_set = self.get_working_set_mut(working_set_id)?;
        working_set.pin_node(node_id);
        Ok(working_set)
    }

    /// Unpin a node in an existing working set.
    ///
    ///
    /// allow callers to release a node from the protected set while keeping the
    /// operation deterministic and manager-owned.
    ///
    ///
    /// remove the node ID from the pinned set if it is present and leave all other
    /// record state untouched.
    ///
    /// # Errors
    ///
    /// return `GraphError::WorkingSetNotFound` when the target working set is
    /// missing. Unpinning an absent node is a deterministic no-op.
    pub fn unpin_node(
        &mut self,
        working_set_id: &WorkingSetId,
        node_id: &NodeId,
    ) -> Result<&GraphWorkingSet, GraphError> {
        self.rebuild_working_set_with_filtered_pins(working_set_id, Some(node_id), None)
    }

    /// Pin a relationship in an existing working set.
    ///
    ///
    /// represent relationship records that future eviction logic must preserve
    /// without implementing eviction in this issue.
    ///
    ///
    /// add the relationship ID to the working set's pinned relationship collection
    /// and leave loading state unchanged.
    ///
    /// # Errors
    ///
    /// return `GraphError::WorkingSetNotFound` when the target working set is
    /// missing.
    pub fn pin_relationship(
        &mut self,
        working_set_id: &WorkingSetId,
        relationship_id: RelationshipId,
    ) -> Result<&GraphWorkingSet, GraphError> {
        let working_set = self.get_working_set_mut(working_set_id)?;
        working_set.pin_relationship(relationship_id);
        Ok(working_set)
    }

    /// Unpin a relationship in an existing working set.
    ///
    ///
    /// allow callers to release a relationship from the protected set while keeping
    /// the operation deterministic and manager-owned.
    ///
    ///
    /// remove the relationship ID from the pinned relationship set if it is present
    /// and leave all other record state untouched.
    ///
    /// # Errors
    ///
    /// return `GraphError::WorkingSetNotFound` when the target working set is
    /// missing. Unpinning an absent relationship is a deterministic no-op.
    pub fn unpin_relationship(
        &mut self,
        working_set_id: &WorkingSetId,
        relationship_id: &RelationshipId,
    ) -> Result<&GraphWorkingSet, GraphError> {
        self.rebuild_working_set_with_filtered_pins(working_set_id, None, Some(relationship_id))
    }

    /// Mark a node record as dirty in an existing working set.
    ///
    ///
    /// keep pending mutation tracking visible at the manager boundary before any
    /// storage flush or transaction integration exists.
    ///
    ///
    /// add the node ID to the dirty node collection of the target working set and
    /// leave payload persistence to later issues.
    ///
    /// # Errors
    ///
    /// return `GraphError::WorkingSetNotFound` when the target working set is
    /// missing.
    pub fn mark_dirty_node(
        &mut self,
        working_set_id: &WorkingSetId,
        node_id: NodeId,
    ) -> Result<&GraphWorkingSet, GraphError> {
        let working_set = self.get_working_set_mut(working_set_id)?;
        working_set.mark_dirty_node(node_id);
        Ok(working_set)
    }

    /// Mark a relationship record as dirty in an existing working set.
    ///
    ///
    /// keep pending relationship mutation tracking visible at the manager boundary
    /// before any storage flush or transaction integration exists.
    ///
    ///
    /// add the relationship ID to the dirty relationship collection of the target
    /// working set and leave persistence to later issues.
    ///
    /// # Errors
    ///
    /// return `GraphError::WorkingSetNotFound` when the target working set is
    /// missing.
    pub fn mark_dirty_relationship(
        &mut self,
        working_set_id: &WorkingSetId,
        relationship_id: RelationshipId,
    ) -> Result<&GraphWorkingSet, GraphError> {
        let working_set = self.get_working_set_mut(working_set_id)?;
        working_set.mark_dirty_relationship(relationship_id);
        Ok(working_set)
    }

    /// Return stats for an existing working set.
    ///
    ///
    /// expose manager-owned summary counters without letting callers mutate the
    /// working set just to inspect load state.
    ///
    ///
    /// return the target working set's current stats as a read-only reference.
    ///
    /// # Errors
    ///
    /// return `GraphError::WorkingSetNotFound` when the target working set is
    /// missing.
    pub fn stats(
        &self,
        working_set_id: &WorkingSetId,
    ) -> Result<&GraphWorkingSetStats, GraphError> {
        let working_set = self.get_working_set(working_set_id)?;
        Ok(working_set.stats())
    }

    /// Enforce deterministic hot-record budgets for an existing working set.
    pub fn enforce_hot_budget_deterministic(
        &mut self,
        working_set_id: &WorkingSetId,
        budget: &WorkingSetHotBudget,
    ) -> Result<WorkingSetEvictionOutcome, GraphError> {
        let working_set = self.get_working_set_mut(working_set_id)?;
        let mut outcome = WorkingSetEvictionOutcome::default();

        let mut warm_node_ids = HashSet::new();
        let mut warm_relationship_ids = HashSet::new();
        for (source_node_id, entries) in working_set.warm_adjacency_by_source() {
            warm_node_ids.insert(source_node_id.clone());
            for entry in entries {
                warm_node_ids.insert(entry.target_node_id().clone());
                warm_relationship_ids.insert(entry.relationship_id().clone());
            }
        }

        let mut evictable_hot_nodes: Vec<NodeId> = working_set
            .hot_node_ids()
            .iter()
            .filter(|node_id| {
                !working_set.pinned_node_ids().contains(*node_id)
                    && !working_set.dirty_node_ids().contains(*node_id)
            })
            .cloned()
            .collect();
        evictable_hot_nodes.sort_by(|left, right| left.as_str().cmp(right.as_str()));

        while working_set.stats().hot_node_count() > budget.max_hot_node_count {
            let Some(node_id) = evictable_hot_nodes.pop() else {
                break;
            };
            let downgraded_state = if warm_node_ids.contains(&node_id) {
                crate::LoadingState::Warm
            } else if working_set.seed_node_ids().contains(&node_id) {
                crate::LoadingState::Indexed
            } else {
                crate::LoadingState::Cold
            };
            working_set.set_node_loading_state(node_id.clone(), downgraded_state);
            outcome.evicted_hot_node_ids.push(node_id);
        }

        let mut evictable_hot_relationships: Vec<RelationshipId> = working_set
            .hot_relationship_ids()
            .iter()
            .filter(|relationship_id| {
                !working_set
                    .pinned_relationship_ids()
                    .contains(*relationship_id)
                    && !working_set
                        .dirty_relationship_ids()
                        .contains(*relationship_id)
            })
            .cloned()
            .collect();
        evictable_hot_relationships.sort_by(|left, right| left.as_str().cmp(right.as_str()));

        while working_set.stats().hot_relationship_count() > budget.max_hot_relationship_count {
            let Some(relationship_id) = evictable_hot_relationships.pop() else {
                break;
            };
            let downgraded_state = if warm_relationship_ids.contains(&relationship_id) {
                crate::LoadingState::Warm
            } else {
                crate::LoadingState::Indexed
            };
            working_set.set_relationship_loading_state(relationship_id.clone(), downgraded_state);
            outcome.evicted_hot_relationship_ids.push(relationship_id);
        }

        Ok(outcome)
    }

    /// Return explanation data for an existing working set.
    ///
    ///
    /// let agents and diagnostics inspect why records are present in the working
    /// set without generating analyst prose inside graph-core.
    ///
    ///
    /// return the explanation container associated with the target working set ID.
    ///
    /// # Errors
    ///
    /// return `GraphError::WorkingSetNotFound` when no working set exists for the
    /// requested ID. Return an invariant violation if the working set exists but
    /// its manager-owned explanation container is missing.
    pub fn explanation(
        &self,
        working_set_id: &WorkingSetId,
    ) -> Result<&WorkingSetExplanation, GraphError> {
        self.get_working_set(working_set_id)?;

        self.explanations.get(working_set_id).ok_or_else(|| {
            GraphError::InternalInvariantViolation(format!(
                "missing explanation for working set {}",
                working_set_id.as_str()
            ))
        })
    }

    /// Return the telemetry recorder for an existing working set.
    ///
    ///
    /// expose the passive decision stream so learning, benchmarks, and
    /// diagnostics can replay every navigation decision without touching
    /// working-set state.
    ///
    ///
    /// return the recorder associated with the target working set ID.
    ///
    /// # Errors
    ///
    /// return `GraphError::WorkingSetNotFound` when no working set exists for
    /// the requested ID. Return an invariant violation if the working set
    /// exists but its manager-owned recorder is missing.
    pub fn telemetry(
        &self,
        working_set_id: &WorkingSetId,
    ) -> Result<&WorkingSetTelemetryRecorder, GraphError> {
        self.get_working_set(working_set_id)?;

        self.telemetry.get(working_set_id).ok_or_else(|| {
            GraphError::InternalInvariantViolation(format!(
                "missing telemetry recorder for working set {}",
                working_set_id.as_str()
            ))
        })
    }

    /// Record one non-marker telemetry decision for an existing working set.
    ///
    ///
    /// give the expansion engine and future controllers a manager-owned path
    /// for traversal decisions the manager cannot observe itself (skips,
    /// page-ins, dead ends, supernode blocks).
    ///
    ///
    /// append the decision to the working set's recorder and return its
    /// sequence number.
    ///
    /// # Errors
    ///
    /// return `GraphError::WorkingSetNotFound` when the target working set is
    /// missing, and `GraphError::InternalInvariantViolation` when the decision
    /// is a retrieval marker; markers must go through begin/complete.
    pub fn record_telemetry_decision(
        &mut self,
        working_set_id: &WorkingSetId,
        decision: WorkingSetDecisionEvent,
    ) -> Result<u64, GraphError> {
        self.get_working_set(working_set_id)?;
        self.recorder_mut(working_set_id)?.record_decision(decision)
    }

    /// Open retrieval telemetry on an existing working set.
    ///
    ///
    /// bound the decision stream so every navigation decision can be
    /// attributed to one query context and, later, one reward.
    ///
    ///
    /// record a `RetrievalStarted` marker carrying the query descriptor.
    ///
    /// # Errors
    ///
    /// return `GraphError::WorkingSetNotFound` when the target working set is
    /// missing, and `GraphError::InternalInvariantViolation` when another
    /// retrieval is already open.
    pub fn begin_retrieval_telemetry(
        &mut self,
        working_set_id: &WorkingSetId,
        retrieval_id: RequestId,
        descriptor: TelemetryQueryDescriptor,
    ) -> Result<(), GraphError> {
        self.get_working_set(working_set_id)?;
        self.recorder_mut(working_set_id)?
            .begin_retrieval(retrieval_id, descriptor)?;
        Ok(())
    }

    /// Complete the open retrieval telemetry on an existing working set.
    ///
    ///
    /// close the attribution window with caller-supplied outcome measurements
    /// instead of engine-sampled clocks, keeping the stream reproducible.
    ///
    ///
    /// record a `RetrievalCompleted` marker carrying the outcome.
    ///
    /// # Errors
    ///
    /// return `GraphError::WorkingSetNotFound` when the target working set is
    /// missing, and `GraphError::InternalInvariantViolation` when no retrieval
    /// is open or the provided ID does not match the open retrieval.
    pub fn complete_retrieval_telemetry(
        &mut self,
        working_set_id: &WorkingSetId,
        retrieval_id: &RequestId,
        outcome: RetrievalOutcome,
    ) -> Result<(), GraphError> {
        self.get_working_set(working_set_id)?;
        self.recorder_mut(working_set_id)?
            .complete_retrieval(retrieval_id, outcome)?;
        Ok(())
    }

    fn record_decision_for_existing(
        &mut self,
        working_set_id: &WorkingSetId,
        decision: WorkingSetDecisionEvent,
    ) -> Result<u64, GraphError> {
        self.recorder_mut(working_set_id)?.record_decision(decision)
    }

    fn recorder_mut(
        &mut self,
        working_set_id: &WorkingSetId,
    ) -> Result<&mut WorkingSetTelemetryRecorder, GraphError> {
        self.telemetry.get_mut(working_set_id).ok_or_else(|| {
            GraphError::InternalInvariantViolation(format!(
                "missing telemetry recorder for working set {}",
                working_set_id.as_str()
            ))
        })
    }

    fn get_working_set_mut(
        &mut self,
        working_set_id: &WorkingSetId,
    ) -> Result<&mut GraphWorkingSet, GraphError> {
        self.working_sets
            .get_mut(working_set_id)
            .ok_or_else(|| GraphError::WorkingSetNotFound(working_set_id.clone()))
    }

    fn rebuild_working_set_with_filtered_pins(
        &mut self,
        working_set_id: &WorkingSetId,
        omitted_node_id: Option<&NodeId>,
        omitted_relationship_id: Option<&RelationshipId>,
    ) -> Result<&GraphWorkingSet, GraphError> {
        let existing = self.get_working_set(working_set_id)?.clone();
        let rebuilt = Self::rebuild_filtered_working_set(
            &existing,
            omitted_node_id,
            omitted_relationship_id,
        )?;

        self.working_sets.insert(working_set_id.clone(), rebuilt);
        self.get_working_set(working_set_id)
    }

    fn rebuild_filtered_working_set(
        existing: &GraphWorkingSet,
        omitted_node_id: Option<&NodeId>,
        omitted_relationship_id: Option<&RelationshipId>,
    ) -> Result<GraphWorkingSet, GraphError> {
        let mut rebuilt = GraphWorkingSet::new(existing.id().clone());

        for node_id in existing.seed_node_ids() {
            rebuilt.record_seed_node(node_id.clone());
        }

        for (source_node_id, entries) in existing.warm_adjacency_by_source() {
            for entry in entries {
                rebuilt.attach_warm_adjacency(source_node_id.clone(), entry.clone())?;
            }
        }

        for node_id in existing.hot_node_ids() {
            rebuilt.track_hot_node(node_id.clone());
        }

        for relationship_id in existing.hot_relationship_ids() {
            rebuilt.track_hot_relationship(relationship_id.clone());
        }

        for node_id in existing.pinned_node_ids() {
            if omitted_node_id != Some(node_id) {
                rebuilt.pin_node(node_id.clone());
            }
        }

        for relationship_id in existing.pinned_relationship_ids() {
            if omitted_relationship_id != Some(relationship_id) {
                rebuilt.pin_relationship(relationship_id.clone());
            }
        }

        for node_id in existing.dirty_node_ids() {
            rebuilt.mark_dirty_node(node_id.clone());
        }

        for relationship_id in existing.dirty_relationship_ids() {
            rebuilt.mark_dirty_relationship(relationship_id.clone());
        }

        Ok(rebuilt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AdjacencyDirection, LoadingState, RelationshipType, WarmAdjacencyEntryInput};

    fn working_set_id(value: &str) -> WorkingSetId {
        WorkingSetId::new(value).expect("test working set ID should be valid")
    }

    fn node_id(value: &str) -> NodeId {
        NodeId::new(value).expect("test node ID should be valid")
    }

    fn relationship_id(value: &str) -> RelationshipId {
        RelationshipId::new(value).expect("test relationship ID should be valid")
    }

    fn relationship_type(value: &str) -> RelationshipType {
        RelationshipType::new(value).expect("test relationship type should be valid")
    }

    fn create_request(id: &WorkingSetId) -> GraphWorkingSetCreateRequest {
        GraphWorkingSetCreateRequest::new(id.clone())
    }

    fn create_manager_with_working_set(id: &WorkingSetId) -> GraphWorkingSetManager {
        let mut manager = GraphWorkingSetManager::new();
        manager
            .create_working_set(create_request(id))
            .expect("working set should be created");
        manager
    }

    fn warm_entry(
        relationship_id: RelationshipId,
        source_node_id: NodeId,
        target_node_id: NodeId,
    ) -> WarmAdjacencyEntry {
        WarmAdjacencyEntry::new(WarmAdjacencyEntryInput::new(
            relationship_id,
            relationship_type("PROMOTES"),
            source_node_id,
            target_node_id,
            vec!["Narrative".to_owned()],
            AdjacencyDirection::Outgoing,
        ))
        .expect("warm adjacency entry should be valid")
    }

    //
    // Verify that the manager owns working-set lifecycle instead of forcing
    // callers to allocate unmanaged `GraphWorkingSet` values.
    //
    // Given an empty `GraphWorkingSetManager` and a create request,
    // when the manager creates a working set and it is retrieved by ID,
    // then both views should expose the same working-set ID.
    #[test]
    fn manager_creates_and_retrieves_working_set_by_id() {
        let mut manager = GraphWorkingSetManager::new();
        let id = working_set_id("working-set--42");

        let created = manager
            .create_working_set(create_request(&id))
            .expect("working set should be created");
        assert_eq!(created.id(), &id);

        let retrieved = manager
            .get_working_set(&id)
            .expect("working set should be retrievable");
        assert_eq!(retrieved.id(), &id);
    }

    //
    // Verify that missing working-set lookup is represented as a typed domain
    // error, not a panic, string-only error, or generic not-implemented branch.
    //
    // Given an empty manager,
    // when a caller retrieves an unknown working-set ID,
    // then the manager should return `GraphError::WorkingSetNotFound` with that ID.
    #[test]
    fn missing_working_set_lookup_returns_typed_error() {
        let manager = GraphWorkingSetManager::new();
        let missing_id = working_set_id("working-set--missing");

        let error = manager
            .get_working_set(&missing_id)
            .expect_err("missing working set should return a typed error");

        assert!(matches!(
        error,
        GraphError::WorkingSetNotFound(id) if id == missing_id
        ));
    }

    //
    // Verify that seed loading records the semantic or explicit entry points of a
    // bounded working set without promoting them to hot unless requested.
    //
    // Given an existing working set,
    // when seed node IDs are loaded with `mark_as_hot = false`,
    // then the nodes should be present as seeds and remain indexed rather than hot.
    #[test]
    fn manager_loads_seed_nodes_without_marking_them_hot() {
        let id = working_set_id("working-set--seeds");
        let mut manager = create_manager_with_working_set(&id);
        let campaign = node_id("campaign--1");
        let narrative = node_id("narrative--1");

        let working_set = manager
            .load_seed_node_ids(&id, [campaign.clone(), narrative.clone()], false)
            .expect("seed nodes should be loaded");

        assert!(working_set.seed_node_ids().contains(&campaign));
        assert!(working_set.seed_node_ids().contains(&narrative));
        assert_eq!(
            working_set.node_loading_state(&campaign),
            Some(LoadingState::Indexed)
        );
        assert!(!working_set.hot_node_ids().contains(&campaign));
    }

    //
    // Verify that callers can choose to make loaded seed nodes immediately active
    // in the hot working set.
    //
    // Given an existing working set,
    // when seed node IDs are loaded with `mark_as_hot = true`,
    // then the seed nodes should also be tracked as hot records.
    #[test]
    fn manager_can_mark_seed_nodes_as_hot_when_loaded() {
        let id = working_set_id("working-set--hot-seeds");
        let mut manager = create_manager_with_working_set(&id);
        let campaign = node_id("campaign--hot");

        let working_set = manager
            .load_seed_node_ids(&id, [campaign.clone()], true)
            .expect("seed node should be loaded as hot");

        assert!(working_set.seed_node_ids().contains(&campaign));
        assert!(working_set.hot_node_ids().contains(&campaign));
        assert_eq!(
            working_set.node_loading_state(&campaign),
            Some(LoadingState::Hot)
        );
    }

    //
    // Verify that the manager can track hot relationships as first-class working
    // set records, independently from seed-node loading.
    //
    // Given an existing working set,
    // when a relationship is added as hot through the manager,
    // then the working set should expose that relationship as hot and count it in stats.
    #[test]
    fn manager_tracks_hot_relationships() {
        let id = working_set_id("working-set--hot-relationship");
        let mut manager = create_manager_with_working_set(&id);
        let relationship = relationship_id("relationship--hot");

        let working_set = manager
            .add_hot_relationship(&id, relationship.clone())
            .expect("hot relationship should be tracked");

        assert!(working_set.hot_relationship_ids().contains(&relationship));
        assert_eq!(
            working_set.relationship_loading_state(&relationship),
            Some(LoadingState::Hot)
        );
        assert_eq!(working_set.stats().hot_relationship_count(), 1);
    }

    //
    // Verify that the manager can retain warm adjacency metadata around a frontier
    // node without requiring target node or relationship payloads to be hot.
    //
    // Given an existing working set and a warm adjacency entry,
    // when the entry is added through the manager,
    // then it should be grouped under the source node and represented in warm stats.
    #[test]
    fn manager_tracks_warm_adjacency_entries() {
        let id = working_set_id("working-set--warm-adjacency");
        let mut manager = create_manager_with_working_set(&id);
        let source = node_id("campaign--warm-source");
        let target = node_id("narrative--warm-target");
        let relationship = relationship_id("relationship--warm");
        let entry = warm_entry(relationship.clone(), source.clone(), target.clone());

        let working_set = manager
            .add_warm_adjacency(&id, source.clone(), entry)
            .expect("warm adjacency should be tracked");

        let entries = working_set
            .warm_adjacency_for_source(&source)
            .expect("source should have warm adjacency entries");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].target_node_id(), &target);
        assert_eq!(
            working_set.relationship_loading_state(&relationship),
            Some(LoadingState::Warm)
        );
        assert_eq!(working_set.stats().warm_relationship_count(), 1);
    }

    //
    // Verify that pinned node state is represented through manager operations and
    // can be released deterministically.
    //
    // Given an existing working set,
    // when a node is pinned and then unpinned through the manager,
    // then the pinned node collection should first contain and then omit that node.
    #[test]
    fn manager_pins_and_unpins_nodes() {
        let id = working_set_id("working-set--pin-node");
        let mut manager = create_manager_with_working_set(&id);
        let node = node_id("node--pinned");

        let working_set = manager
            .pin_node(&id, node.clone())
            .expect("node should be pinned");
        assert!(working_set.pinned_node_ids().contains(&node));

        let working_set = manager
            .unpin_node(&id, &node)
            .expect("node should be unpinned");
        assert!(!working_set.pinned_node_ids().contains(&node));
    }

    //
    // Verify that pinned relationship state is represented through manager
    // operations and can be released deterministically.
    //
    // Given an existing working set,
    // when a relationship is pinned and then unpinned through the manager,
    // then the pinned relationship collection should first contain and then omit it.
    #[test]
    fn manager_pins_and_unpins_relationships() {
        let id = working_set_id("working-set--pin-relationship");
        let mut manager = create_manager_with_working_set(&id);
        let relationship = relationship_id("relationship--pinned");

        let working_set = manager
            .pin_relationship(&id, relationship.clone())
            .expect("relationship should be pinned");
        assert!(
            working_set
                .pinned_relationship_ids()
                .contains(&relationship)
        );

        let working_set = manager
            .unpin_relationship(&id, &relationship)
            .expect("relationship should be unpinned");
        assert!(
            !working_set
                .pinned_relationship_ids()
                .contains(&relationship)
        );
    }

    //
    // Verify that dirty node and relationship records can be represented through
    // manager-owned operations before any persistence or flush behavior exists.
    //
    // Given an existing working set,
    // when a node and relationship are marked dirty through the manager,
    // then both dirty collections should expose the corresponding IDs.
    #[test]
    fn manager_tracks_dirty_records() {
        let id = working_set_id("working-set--dirty");
        let mut manager = create_manager_with_working_set(&id);
        let node = node_id("node--dirty");
        let relationship = relationship_id("relationship--dirty");

        manager
            .mark_dirty_node(&id, node.clone())
            .expect("dirty node should be tracked");
        let working_set = manager
            .mark_dirty_relationship(&id, relationship.clone())
            .expect("dirty relationship should be tracked");

        assert!(working_set.dirty_node_ids().contains(&node));
        assert!(working_set.dirty_relationship_ids().contains(&relationship));
    }

    //
    // Verify that stats are available through the manager as the read boundary for
    // summary counters.
    //
    // Given an existing working set with hot seed nodes and a hot relationship,
    // when stats are requested from the manager,
    // then the stats should reflect hot node and relationship counts.
    #[test]
    fn manager_returns_basic_working_set_stats() {
        let id = working_set_id("working-set--stats");
        let mut manager = create_manager_with_working_set(&id);
        let campaign = node_id("campaign--stats");
        let relationship = relationship_id("relationship--stats");

        manager
            .load_seed_node_ids(&id, [campaign], true)
            .expect("hot seed should be tracked");
        manager
            .add_hot_relationship(&id, relationship)
            .expect("hot relationship should be tracked");

        let stats = manager.stats(&id).expect("stats should be available");

        assert_eq!(stats.hot_node_count(), 1);
        assert_eq!(stats.hot_relationship_count(), 1);
    }

    //
    // Verify that explanation data is owned and retrievable through the manager,
    // even before richer explanation recording is implemented.
    //
    // Given an existing working set,
    // when explanation data is requested from the manager,
    // then an empty deterministic explanation container should be returned.
    #[test]
    fn manager_returns_working_set_explanation_data() {
        let id = working_set_id("working-set--explanation");
        let manager = create_manager_with_working_set(&id);

        let explanation = manager
            .explanation(&id)
            .expect("explanation should be available");

        assert!(explanation.seed_nodes().is_empty());
        assert!(explanation.hot_nodes().is_empty());
        assert!(explanation.hot_relationships().is_empty());
        assert!(explanation.warm_adjacency_entries().is_empty());
    }
}

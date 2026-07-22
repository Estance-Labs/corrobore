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
//! Full-trajectory provenance capture (Epic 0018).
//!
//!
//!
//! - Make an answer's path as auditable as its sources: provenance covers the
//!   full navigation trajectory — seeds, page-ins, expansions, skips, warm
//!   adjacency, controller choices, dead ends — not only the documents
//!   finally cited.
//! - Stay strictly derived from the recorded telemetry of Epic 0017: capture
//!   is a pure projection of the retrieval records, with no second
//!   bookkeeping path, so equal records yield equal provenance.
//! - Link cited evidence back to navigation: every node or relationship in a
//!   supporting or counter-evidence subgraph resolves to the ordered steps
//!   that surfaced it.
//! - Project into the proof-carrying answer envelope's `source_provenance`
//!   reference.

use serde::{Deserialize, Serialize};

use crate::{
    graph_pager::GraphRecordRef,
    ids::{NodeId, RelationshipId, RequestId},
    proof_carrying_answer::SourceProvenanceRef,
    working_set_telemetry::{
        RetrievalTelemetryRecord, TelemetryQueryDescriptor, WorkingSetDecisionEvent,
        WorkingSetTelemetryEvent,
    },
};

/// One recorded navigation step of a trajectory.
///
///
/// keep steps identical to the recorded decisions: provenance never invents
/// or rewrites navigation history.
///
///
/// carry the telemetry sequence number and the typed decision.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrajectoryStep {
    /// Sequence number of the decision in the working set's stream.
    pub sequence: u64,

    /// Recorded decision.
    pub decision: WorkingSetDecisionEvent,
}

/// The recorded trajectory of one retrieval.
///
///
/// preserve retrieval boundaries: each retrieval keeps its identifier, its
/// query descriptor, and its ordered decisions.
///
///
/// carry the retrieval identifier, the descriptor recorded at its start, and
/// the ordered steps between its markers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RetrievalTrajectory {
    /// Identifier of the retrieval.
    pub retrieval_id: RequestId,

    /// Query context recorded when the retrieval began.
    pub descriptor: TelemetryQueryDescriptor,

    /// Ordered navigation steps of the retrieval.
    pub steps: Vec<TrajectoryStep>,
}

/// Reference from a cited record back to one navigation step.
///
///
/// let consumers jump from any evidence item to the exact decisions that
/// surfaced it during navigation.
///
///
/// carry the retrieval identifier and the step's sequence number.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfacingStep {
    /// Retrieval the step belongs to.
    pub retrieval_id: RequestId,

    /// Sequence number of the step in the working set's stream.
    pub sequence: u64,
}

/// Full-trajectory provenance captured from recorded retrievals.
///
///
/// own the epic's trajectory-provenance contract: the complete, ordered
/// navigation history behind an answer, queryable per cited record.
///
///
/// carry one trajectory per recorded retrieval in recording order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrajectoryProvenance {
    /// One trajectory per recorded retrieval, in recording order.
    pub retrievals: Vec<RetrievalTrajectory>,
}

impl TrajectoryProvenance {
    /// Return the ordered steps that surfaced one node.
    ///
    ///
    /// make every cited node navigable to the decisions that touched it:
    /// page-ins, seed selections, warm adjacency, skips, dead ends, blocks,
    /// and controller choices.
    ///
    ///
    /// scan the trajectories in order and collect the steps whose decision
    /// references the node.
    ///
    /// # Errors
    ///
    /// none expected because absence is a deterministic empty result.
    pub fn surfacing_steps_for_node(&self, node_id: &NodeId) -> Vec<SurfacingStep> {
        self.collect_steps(|decision| decision_touches_node(decision, node_id))
    }

    /// Return the ordered steps that surfaced one relationship.
    ///
    ///
    /// make every cited relationship navigable to the decisions that touched
    /// it: page-ins, expansions, warm adjacency, and skips.
    ///
    ///
    /// scan the trajectories in order and collect the steps whose decision
    /// references the relationship.
    ///
    /// # Errors
    ///
    /// none expected because absence is a deterministic empty result.
    pub fn surfacing_steps_for_relationship(
        &self,
        relationship_id: &RelationshipId,
    ) -> Vec<SurfacingStep> {
        self.collect_steps(|decision| decision_touches_relationship(decision, relationship_id))
    }

    /// Project this provenance into an envelope source-provenance reference.
    ///
    ///
    /// give the proof-carrying answer its `source_provenance` without
    /// duplicating the trajectory inside the envelope: the reference names
    /// the retrievals whose records hold it.
    ///
    ///
    /// list the retrieval identifiers in order and attach the caller's cited
    /// source references.
    ///
    /// # Errors
    ///
    /// none expected because the projection is pure.
    pub fn to_source_provenance_ref(&self, source_refs: Vec<String>) -> SourceProvenanceRef {
        SourceProvenanceRef {
            retrieval_ids: self
                .retrievals
                .iter()
                .map(|trajectory| trajectory.retrieval_id.clone())
                .collect(),
            source_refs,
        }
    }

    fn collect_steps(
        &self,
        mut matches: impl FnMut(&WorkingSetDecisionEvent) -> bool,
    ) -> Vec<SurfacingStep> {
        let mut steps = Vec::new();
        for trajectory in &self.retrievals {
            for step in &trajectory.steps {
                if matches(&step.decision) {
                    steps.push(SurfacingStep {
                        retrieval_id: trajectory.retrieval_id.clone(),
                        sequence: step.sequence,
                    });
                }
            }
        }
        steps
    }
}

/// Capture full-trajectory provenance from recorded retrievals.
///
///
/// implement the pure projection: provenance is exactly the recorded
/// decisions, grouped by retrieval, in order.
///
///
/// map each retrieval record onto a trajectory carrying its identifier,
/// descriptor, and ordered steps.
///
/// # Errors
///
/// none expected because records from the telemetry recorder are already
/// structurally valid.
pub fn capture_trajectory_provenance(records: &[RetrievalTelemetryRecord]) -> TrajectoryProvenance {
    TrajectoryProvenance {
        retrievals: records
            .iter()
            .map(|record| RetrievalTrajectory {
                retrieval_id: record.retrieval_id.clone(),
                descriptor: record.descriptor.clone(),
                steps: record.events.iter().map(step_from_event).collect(),
            })
            .collect(),
    }
}

fn step_from_event(event: &WorkingSetTelemetryEvent) -> TrajectoryStep {
    TrajectoryStep {
        sequence: event.sequence,
        decision: event.decision.clone(),
    }
}

fn decision_touches_node(decision: &WorkingSetDecisionEvent, node_id: &NodeId) -> bool {
    match decision {
        WorkingSetDecisionEvent::SeedCandidateObserved { node_id: candidate }
        | WorkingSetDecisionEvent::SeedSelected {
            node_id: candidate, ..
        }
        | WorkingSetDecisionEvent::DeadEnd { node_id: candidate }
        | WorkingSetDecisionEvent::SupernodeBlocked { node_id: candidate } => candidate == node_id,
        WorkingSetDecisionEvent::EdgeSkipped {
            source_node_id,
            candidate_node_id,
            ..
        } => source_node_id == node_id || candidate_node_id.as_ref() == Some(node_id),
        WorkingSetDecisionEvent::WarmAdjacencyAttached {
            source_node_id,
            target_node_id,
            ..
        } => source_node_id == node_id || target_node_id == node_id,
        WorkingSetDecisionEvent::PageIn { record }
        | WorkingSetDecisionEvent::Prefetch { record }
        | WorkingSetDecisionEvent::Eviction { record } => {
            matches!(record, GraphRecordRef::Node(candidate) if candidate == node_id)
        }
        WorkingSetDecisionEvent::ControllerActionChosen { source_node_id, .. } => {
            source_node_id.as_ref() == Some(node_id)
        }
        WorkingSetDecisionEvent::EdgeExpanded { .. }
        | WorkingSetDecisionEvent::RetrievalStarted { .. }
        | WorkingSetDecisionEvent::RetrievalCompleted { .. } => false,
    }
}

fn decision_touches_relationship(
    decision: &WorkingSetDecisionEvent,
    relationship_id: &RelationshipId,
) -> bool {
    match decision {
        WorkingSetDecisionEvent::EdgeExpanded {
            relationship_id: candidate,
        }
        | WorkingSetDecisionEvent::WarmAdjacencyAttached {
            relationship_id: candidate,
            ..
        } => candidate == relationship_id,
        WorkingSetDecisionEvent::EdgeSkipped {
            relationship_id: candidate,
            ..
        } => candidate.as_ref() == Some(relationship_id),
        WorkingSetDecisionEvent::PageIn { record }
        | WorkingSetDecisionEvent::Prefetch { record }
        | WorkingSetDecisionEvent::Eviction { record } => matches!(
            record,
            GraphRecordRef::Relationship(candidate) if candidate == relationship_id
        ),
        _ => false,
    }
}

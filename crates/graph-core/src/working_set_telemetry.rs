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
//! Working-set decision telemetry contracts (Epic 0017, Phase 1: instrument
//! before learning).
//!
//!
//!
//! - Record every working-set navigation decision as a passive, deterministic
//!   observation stream owned by the working-set manager.
//! - Group decisions into per-retrieval records through explicit retrieval
//!   markers carrying a query descriptor and a caller-supplied outcome.
//! - Keep instrumentation strictly observational: recording must never change
//!   working-set state, expansion behavior, budgets, or supernode protection.
//! - Do not implement pheromone traces, anti-pheromones, bandit control, or
//!   benchmark reporting here; those consume this telemetry in later issues.

use serde::{Deserialize, Serialize};

use crate::{
    Confidence, GraphError,
    graph_pager::GraphRecordRef,
    ids::{EvidenceId, NodeId, RelationshipId, RequestId},
    loading_profile::LoadingProfileKind,
    working_set::WorkingSetId,
    working_set_explanation::SkippedExpansionReason,
};

/// Caller-facing description of the retrieval that produced a decision stream.
///
///
/// let pheromone and bandit learning condition on query context (question text,
/// loading profile, task family) without parsing engine internals.
///
///
/// carry optional query text, the active loading-profile kind, and a free-form
/// task label; all fields are optional so partial context is still recordable.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TelemetryQueryDescriptor {
    /// Raw query or question text, when the caller can share it.
    pub query_text: Option<String>,

    /// Loading-profile kind active during the retrieval.
    pub profile_kind: Option<LoadingProfileKind>,

    /// Task-family label used to scope future pheromone traces.
    pub task_label: Option<String>,
}

/// Caller-supplied measurements describing how a retrieval ended.
///
///
/// keep telemetry deterministic: the engine never samples wall-clock time or
/// process memory itself; callers provide the measurements they trust.
///
///
/// carry evidence identifiers, optional answer quality, and cost measurements
/// for the completed retrieval.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RetrievalOutcome {
    /// Evidence records found during the retrieval.
    pub evidence_record_ids: Vec<EvidenceId>,

    /// Answer quality assessment, when the caller evaluated one.
    pub answer_quality: Option<Confidence>,

    /// Memory cost attributed to the retrieval, in bytes.
    pub memory_cost_bytes: u64,

    /// End-to-end retrieval latency, in milliseconds.
    pub latency_ms: u64,
}

/// One recorded working-set navigation decision.
///
///
/// give every engine decision a stable, typed observation shape so learned
/// policies train on decisions, not on log text.
///
///
/// model retrieval boundaries, seed decisions, edge decisions, paging
/// decisions, and negative-signal observations as explicit variants.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum WorkingSetDecisionEvent {
    /// A retrieval opened on the working set. Recorded only through
    /// `begin_retrieval_telemetry`, never through generic decision recording.
    RetrievalStarted {
        /// Identifier correlating all decisions of this retrieval.
        retrieval_id: RequestId,
        /// Query context for the retrieval.
        descriptor: TelemetryQueryDescriptor,
    },

    /// The open retrieval completed. Recorded only through
    /// `complete_retrieval_telemetry`, never through generic decision recording.
    RetrievalCompleted {
        /// Identifier of the completed retrieval.
        retrieval_id: RequestId,
        /// Caller-supplied outcome measurements.
        outcome: RetrievalOutcome,
    },

    /// A seed candidate was observed before selection.
    SeedCandidateObserved {
        /// Candidate node.
        node_id: NodeId,
    },

    /// A node was loaded into the working set as a seed.
    SeedSelected {
        /// Selected node.
        node_id: NodeId,
        /// Whether the node was marked hot at load time.
        marked_hot: bool,
    },

    /// A relationship became hot in the working set.
    EdgeExpanded {
        /// Expanded relationship.
        relationship_id: RelationshipId,
    },

    /// A candidate expansion was evaluated and not taken.
    EdgeSkipped {
        /// Frontier node whose expansion was skipped.
        source_node_id: NodeId,
        /// Candidate target node, when known.
        candidate_node_id: Option<NodeId>,
        /// Candidate relationship, when known.
        relationship_id: Option<RelationshipId>,
        /// Stable skip reason shared with working-set explanations.
        reason: SkippedExpansionReason,
    },

    /// A warm adjacency entry was attached at the ring boundary.
    WarmAdjacencyAttached {
        /// Frontier node owning the warm entry.
        source_node_id: NodeId,
        /// Warm relationship.
        relationship_id: RelationshipId,
        /// Warm target node.
        target_node_id: NodeId,
    },

    /// A record payload was paged in from storage.
    PageIn {
        /// Paged-in record.
        record: GraphRecordRef,
    },

    /// A record was prefetched ahead of demand.
    Prefetch {
        /// Prefetched record.
        record: GraphRecordRef,
    },

    /// A record was evicted from the working set.
    Eviction {
        /// Evicted record.
        record: GraphRecordRef,
    },

    /// A frontier node produced no admitted expansion.
    DeadEnd {
        /// Dead-end frontier node.
        node_id: NodeId,
    },

    /// A high-degree frontier node was blocked by supernode policy.
    SupernodeBlocked {
        /// Blocked frontier node.
        node_id: NodeId,
    },

    /// A working-set controller chose an action for a decision point.
    ControllerActionChosen {
        /// Frontier source the choice applied to, when the decision point is
        /// source-scoped.
        source_node_id: Option<NodeId>,
        /// Chosen controller action.
        action: crate::bandit_controller::WorkingSetAction,
    },
}

impl WorkingSetDecisionEvent {
    /// Report whether this decision is a retrieval boundary marker.
    ///
    ///
    /// markers own retrieval-grouping integrity, so they must only enter the
    /// stream through the dedicated begin/complete operations.
    ///
    ///
    /// return true for `RetrievalStarted` and `RetrievalCompleted`.
    ///
    /// # Errors
    ///
    /// none expected because this is a pure classification.
    pub fn is_retrieval_marker(&self) -> bool {
        matches!(
            self,
            Self::RetrievalStarted { .. } | Self::RetrievalCompleted { .. }
        )
    }
}

/// One sequenced telemetry event in a working set's decision stream.
///
///
/// give consumers a deterministic total order without relying on wall-clock
/// timestamps that would break reproducibility.
///
///
/// pair a monotonically increasing per-working-set sequence number with the
/// recorded decision.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkingSetTelemetryEvent {
    /// Monotonic per-working-set sequence number.
    pub sequence: u64,

    /// Recorded decision.
    pub decision: WorkingSetDecisionEvent,
}

/// Per-retrieval telemetry envelope derived from the decision stream.
///
///
/// expose the schema demanded by Epic 0017 acceptance: query, decisions, and
/// outcome per retrieval, without making the stream itself retrieval-scoped.
///
///
/// carry the retrieval identifier, working-set identifier, query descriptor,
/// the events recorded between the retrieval markers, and the outcome when the
/// retrieval completed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RetrievalTelemetryRecord {
    /// Identifier of the retrieval.
    pub retrieval_id: RequestId,

    /// Working set the retrieval ran against.
    pub working_set_id: WorkingSetId,

    /// Query context recorded when the retrieval began.
    pub descriptor: TelemetryQueryDescriptor,

    /// Decisions recorded strictly between the retrieval markers.
    pub events: Vec<WorkingSetTelemetryEvent>,

    /// Outcome recorded at completion; absent for retrievals still open.
    pub outcome: Option<RetrievalOutcome>,
}

/// Deterministic, manager-owned recorder for one working set's decisions.
///
///
/// centralize decision capture behind the working-set manager (mirroring the
/// explanation ownership model) so every mutation path records observations
/// without caller opt-in and without behavior changes.
///
///
/// append sequenced events, enforce retrieval-marker integrity, and derive
/// per-retrieval records from the stream on demand.
///
/// # Errors
///
/// marker misuse (nested retrievals, completion without an open retrieval,
/// mismatched retrieval IDs, forged markers) returns
/// `GraphError::InternalInvariantViolation`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkingSetTelemetryRecorder {
    working_set_id: WorkingSetId,
    next_sequence: u64,
    events: Vec<WorkingSetTelemetryEvent>,
    open_retrieval_id: Option<RequestId>,
}

impl WorkingSetTelemetryRecorder {
    /// Create an empty recorder bound to one working set.
    ///
    ///
    /// bind the stream to its working set so derived retrieval records carry
    /// the correct ownership without consulting the manager again.
    ///
    ///
    /// initialize an empty event stream with sequence numbering starting at 0
    /// and no open retrieval.
    ///
    /// # Errors
    ///
    /// none expected because an empty recorder has no external dependency.
    pub fn new(working_set_id: WorkingSetId) -> Self {
        Self {
            working_set_id,
            next_sequence: 0,
            events: Vec::new(),
            open_retrieval_id: None,
        }
    }

    /// Return the working set this recorder observes.
    ///
    ///
    /// let consumers attribute exported streams without side channels.
    ///
    ///
    /// return the bound working-set ID.
    ///
    /// # Errors
    ///
    /// none expected because the recorder always owns a validated ID.
    pub fn working_set_id(&self) -> &WorkingSetId {
        &self.working_set_id
    }

    /// Return the full recorded decision stream in sequence order.
    ///
    ///
    /// expose the raw observations that pheromone updates and benchmark
    /// replays consume.
    ///
    ///
    /// return all recorded events, including retrieval markers.
    ///
    /// # Errors
    ///
    /// none expected because reading the stream cannot fail.
    pub fn events(&self) -> &[WorkingSetTelemetryEvent] {
        &self.events
    }

    /// Derive per-retrieval telemetry records from the decision stream.
    ///
    ///
    /// provide the retrieval-scoped schema required by Epic 0017 acceptance
    /// while keeping the append-only stream as the single source of truth.
    ///
    ///
    /// group events strictly between `RetrievalStarted` and
    /// `RetrievalCompleted` markers into records; a still-open retrieval yields
    /// a record without an outcome.
    ///
    /// # Errors
    ///
    /// none expected because marker integrity is enforced at recording time.
    pub fn retrieval_records(&self) -> Vec<RetrievalTelemetryRecord> {
        let mut records = Vec::new();
        let mut open: Option<RetrievalTelemetryRecord> = None;

        for event in &self.events {
            match &event.decision {
                WorkingSetDecisionEvent::RetrievalStarted {
                    retrieval_id,
                    descriptor,
                } => {
                    open = Some(RetrievalTelemetryRecord {
                        retrieval_id: retrieval_id.clone(),
                        working_set_id: self.working_set_id.clone(),
                        descriptor: descriptor.clone(),
                        events: Vec::new(),
                        outcome: None,
                    });
                }
                WorkingSetDecisionEvent::RetrievalCompleted { outcome, .. } => {
                    if let Some(mut record) = open.take() {
                        record.outcome = Some(outcome.clone());
                        records.push(record);
                    }
                }
                _ => {
                    if let Some(record) = open.as_mut() {
                        record.events.push(event.clone());
                    }
                }
            }
        }

        if let Some(record) = open {
            records.push(record);
        }

        records
    }

    /// Record one non-marker decision and return its sequence number.
    ///
    ///
    /// keep the generic recording path open to the manager, the expansion
    /// engine, and future controllers while protecting marker integrity.
    ///
    ///
    /// append the decision with the next sequence number.
    ///
    /// # Errors
    ///
    /// return `GraphError::InternalInvariantViolation` when the decision is a
    /// retrieval marker; markers must go through begin/complete.
    pub fn record_decision(
        &mut self,
        decision: WorkingSetDecisionEvent,
    ) -> Result<u64, GraphError> {
        if decision.is_retrieval_marker() {
            return Err(GraphError::InternalInvariantViolation(format!(
                "retrieval markers must be recorded through begin/complete for working set {}",
                self.working_set_id.as_str()
            )));
        }

        Ok(self.append(decision))
    }

    /// Open a retrieval and record its start marker.
    ///
    ///
    /// give every retrieval an explicit boundary so downstream learning can
    /// attribute decisions and rewards to one query context.
    ///
    ///
    /// record `RetrievalStarted` and remember the open retrieval ID.
    ///
    /// # Errors
    ///
    /// return `GraphError::InternalInvariantViolation` when another retrieval
    /// is already open on this working set.
    pub fn begin_retrieval(
        &mut self,
        retrieval_id: RequestId,
        descriptor: TelemetryQueryDescriptor,
    ) -> Result<u64, GraphError> {
        if let Some(open_retrieval_id) = &self.open_retrieval_id {
            return Err(GraphError::InternalInvariantViolation(format!(
                "retrieval {} is already open for working set {}",
                open_retrieval_id.as_str(),
                self.working_set_id.as_str()
            )));
        }

        self.open_retrieval_id = Some(retrieval_id.clone());
        Ok(self.append(WorkingSetDecisionEvent::RetrievalStarted {
            retrieval_id,
            descriptor,
        }))
    }

    /// Complete the open retrieval and record its outcome marker.
    ///
    ///
    /// close the attribution window with the caller-supplied measurements the
    /// reward interfaces will consume.
    ///
    ///
    /// record `RetrievalCompleted` with the outcome and clear the open state.
    ///
    /// # Errors
    ///
    /// return `GraphError::InternalInvariantViolation` when no retrieval is
    /// open or when the provided ID does not match the open retrieval.
    pub fn complete_retrieval(
        &mut self,
        retrieval_id: &RequestId,
        outcome: RetrievalOutcome,
    ) -> Result<u64, GraphError> {
        match &self.open_retrieval_id {
            None => {
                return Err(GraphError::InternalInvariantViolation(format!(
                    "no retrieval is open for working set {}",
                    self.working_set_id.as_str()
                )));
            }
            Some(open_retrieval_id) if open_retrieval_id != retrieval_id => {
                return Err(GraphError::InternalInvariantViolation(format!(
                    "retrieval {} does not match open retrieval {} for working set {}",
                    retrieval_id.as_str(),
                    open_retrieval_id.as_str(),
                    self.working_set_id.as_str()
                )));
            }
            Some(_) => {}
        }

        self.open_retrieval_id = None;
        Ok(self.append(WorkingSetDecisionEvent::RetrievalCompleted {
            retrieval_id: retrieval_id.clone(),
            outcome,
        }))
    }

    fn append(&mut self, decision: WorkingSetDecisionEvent) -> u64 {
        let sequence = self.next_sequence;
        self.next_sequence += 1;
        self.events
            .push(WorkingSetTelemetryEvent { sequence, decision });
        sequence
    }
}

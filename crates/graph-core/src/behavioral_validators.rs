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
//! Behavioral anomaly validators of the immune system (Epic 0019).
//!
//!
//!
//! - Detect the epic's behavioral defect classes from recorded engine state
//!   alone — pheromone fields and telemetry retrieval records — with no live
//!   sampling, so identical inputs yield identical findings.
//! - Stay a pure read: findings are typed Epic 0007 validation records, and
//!   no field or record is ever mutated.
//! - Report in a documented deterministic order — anomalous pheromone growth,
//!   retrieval drift, then centrality shifts — each pass in caller or listing
//!   order.
//!
//! # Detection rules (deterministic)
//!
//! - **Anomalous pheromone growth**: a caller-listed edge whose decayed
//!   access trace in the analyzed task scope exceeds
//!   `max_access_frequency`; decay normalizes history, so only rapid recent
//!   growth can exceed a bound expressed on the decayed trace.
//! - **Retrieval drift**: with at least two records, the latest retrieval's
//!   expansion ratio (`expanded / (expanded + skipped)`, vacuously 1.0)
//!   diverging from the aggregated ratio of all earlier records by more than
//!   `max_drift_ratio`.
//! - **Centrality shift**: a node whose observed frontier degree — the
//!   warm-adjacency attachments recorded for it as expansion source — jumps
//!   in the latest record by more than `max_degree_jump` over its maximum in
//!   any earlier record; nodes are evaluated in identifier order.

use std::collections::HashMap;

use crate::{
    GraphError,
    ids::{NodeId, RelationshipId},
    pheromone_trace::{PheromoneField, PheromoneTaskScope},
    validation::{ValidationErrorRecord, ValidationErrorSeverity, ValidationTarget},
    working_set_telemetry::{RetrievalTelemetryRecord, WorkingSetDecisionEvent},
};

/// Stable finding code of the pheromone-growth validator.
const PHEROMONE_GROWTH_CODE: &str = "immune-behavioral--pheromone-growth";

/// Stable finding code of the retrieval-drift validator.
const RETRIEVAL_DRIFT_CODE: &str = "immune-behavioral--retrieval-drift";

/// Stable finding code of the centrality-shift validator.
const CENTRALITY_SHIFT_CODE: &str = "immune-behavioral--centrality-shift";

/// Declared bounds separating normal from anomalous recorded behavior.
///
///
/// keep the anomaly thresholds a visible, auditable configuration instead of
/// constants buried in detection code.
///
///
/// carry one bound per behavioral rule.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BehavioralBounds {
    /// Maximum decayed access trace an edge may reach before flagging.
    pub max_access_frequency: f64,

    /// Maximum divergence of the latest expansion ratio from history.
    pub max_drift_ratio: f64,

    /// Maximum jump of a node's observed frontier degree over its history.
    pub max_degree_jump: u64,
}

/// Inputs of one behavioral validation pass.
///
///
/// make every input recorded state: the pheromone field, the task scope under
/// analysis, the caller-listed edges to audit, and the retrieval records in
/// recording order.
///
///
/// carry the recorded state and the declared bounds.
pub struct BehavioralValidationInputs<'a> {
    /// Pheromone field built from recorded telemetry; only read.
    pub pheromones: &'a PheromoneField,

    /// Task scope under analysis.
    pub scope: &'a PheromoneTaskScope,

    /// Edges audited for anomalous growth, in evaluation order.
    pub edges: &'a [RelationshipId],

    /// Retrieval records in recording order; the last one is the latest.
    pub records: &'a [RetrievalTelemetryRecord],

    /// Declared anomaly bounds.
    pub bounds: &'a BehavioralBounds,
}

/// Validate recorded engine behavior.
///
///
/// give the immune system its behavioral detection pass over the Epic 0017
/// observability surfaces, feeding tier routing and probe generation.
///
///
/// run the three detection rules of the module documentation and return the
/// typed findings in the documented order.
///
/// # Errors
///
/// none expected today; the result type reserves the typed-error boundary
/// used by every immune validator.
pub fn validate_graph_behavior(
    inputs: &BehavioralValidationInputs<'_>,
) -> Result<Vec<ValidationErrorRecord>, GraphError> {
    let mut findings = Vec::new();

    // Pass 1: anomalous pheromone growth over the caller-listed edges.
    for edge in inputs.edges {
        let Some(utility) = inputs.pheromones.edge_utility(edge, inputs.scope) else {
            continue;
        };
        if utility.access_frequency > inputs.bounds.max_access_frequency {
            findings.push(ValidationErrorRecord::new(
                PHEROMONE_GROWTH_CODE,
                ValidationErrorSeverity::Warning,
                format!(
                    "edge {} access trace {} exceeds the declared bound {}",
                    edge.as_str(),
                    utility.access_frequency,
                    inputs.bounds.max_access_frequency
                ),
                ValidationTarget::relationship(edge.as_str()),
            ));
        }
    }

    // Pass 2: retrieval drift of the latest record against its history.
    if let Some((latest, history)) = inputs.records.split_last()
        && !history.is_empty()
    {
        let history_ratio =
            expansion_ratio(history.iter().map(decision_counts).fold((0, 0), sum_counts));
        let latest_ratio = expansion_ratio(decision_counts(latest));
        let drift = (latest_ratio - history_ratio).abs();
        if drift > inputs.bounds.max_drift_ratio {
            findings.push(ValidationErrorRecord::new(
                RETRIEVAL_DRIFT_CODE,
                ValidationErrorSeverity::Warning,
                format!(
                    "retrieval {} expansion ratio {latest_ratio} drifts {drift} from the \
                     recorded history ratio {history_ratio}",
                    latest.retrieval_id.as_str()
                ),
                ValidationTarget::retrieval(latest.retrieval_id.as_str()),
            ));
        }
    }

    // Pass 3: centrality shifts of the latest record against its history.
    if let Some((latest, history)) = inputs.records.split_last()
        && !history.is_empty()
    {
        let latest_degrees = frontier_degrees(latest);
        let mut prior_max: HashMap<&NodeId, u64> = HashMap::new();
        for record in history {
            for (node, degree) in frontier_degrees(record) {
                let entry = prior_max.entry(node).or_default();
                *entry = (*entry).max(degree);
            }
        }

        let mut nodes: Vec<(&NodeId, u64)> = latest_degrees.into_iter().collect();
        nodes.sort_by(|left, right| left.0.as_str().cmp(right.0.as_str()));
        for (node, degree) in nodes {
            let prior = prior_max.get(node).copied().unwrap_or(0);
            if degree.saturating_sub(prior) > inputs.bounds.max_degree_jump {
                findings.push(ValidationErrorRecord::new(
                    CENTRALITY_SHIFT_CODE,
                    ValidationErrorSeverity::Warning,
                    format!(
                        "node {} frontier degree jumped from {prior} to {degree}",
                        node.as_str()
                    ),
                    ValidationTarget::node(node.as_str()),
                ));
            }
        }
    }

    Ok(findings)
}

/// Count the expanded and skipped decisions of one record.
fn decision_counts(record: &RetrievalTelemetryRecord) -> (u64, u64) {
    let mut expanded = 0;
    let mut skipped = 0;
    for event in &record.events {
        match &event.decision {
            WorkingSetDecisionEvent::EdgeExpanded { .. } => expanded += 1,
            WorkingSetDecisionEvent::EdgeSkipped { .. } => skipped += 1,
            _ => {}
        }
    }
    (expanded, skipped)
}

fn sum_counts(acc: (u64, u64), item: (u64, u64)) -> (u64, u64) {
    (acc.0 + item.0, acc.1 + item.1)
}

/// Expansion ratio of one decision count, vacuously full without decisions.
fn expansion_ratio((expanded, skipped): (u64, u64)) -> f64 {
    let total = expanded + skipped;
    if total == 0 {
        1.0
    } else {
        expanded as f64 / total as f64
    }
}

/// Observed frontier degree per node: warm-adjacency attachments recorded for
/// the node as expansion source.
fn frontier_degrees(record: &RetrievalTelemetryRecord) -> HashMap<&NodeId, u64> {
    let mut degrees: HashMap<&NodeId, u64> = HashMap::new();
    for event in &record.events {
        if let WorkingSetDecisionEvent::WarmAdjacencyAttached { source_node_id, .. } =
            &event.decision
        {
            *degrees.entry(source_node_id).or_default() += 1;
        }
    }
    degrees
}

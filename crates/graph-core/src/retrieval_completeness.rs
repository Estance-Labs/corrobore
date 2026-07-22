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
//! Retrieval-completeness signal and its computation (Epic 0018).
//!
//!
//!
//! - Make retrieval-state uncertainty measurable: a response can be correct on
//!   the loaded elements yet misleading because the working set was
//!   incomplete, so completeness is computed and reported even when the answer
//!   is confident.
//! - Compute deterministically from recorded engine state alone: the input is
//!   the retrieval telemetry of Epic 0017, never live sampling, so equal
//!   records yield equal reports.
//! - Surface the reasons behind reduced completeness as typed, ordered
//!   reductions for explainability.
//! - The validated `RetrievalCompleteness` carrier lives in the
//!   proof-carrying answer envelope module; this module owns its computation.
//!
//! # Coverage weights (deterministic)
//!
//! Over one or more recorded retrievals:
//!
//! - each `EdgeExpanded` decision counts fully covered (weight 1.0);
//! - each `WarmAdjacencyAttached` decision counts half-covered: its metadata
//!   is known but its payload was not loaded (0.5 covered, 0.5 uncovered);
//! - each `EdgeSkipped` decision counts uncovered (weight 1.0);
//! - each `ControllerActionChosen` decision with `Stop` counts uncovered: the
//!   frontier beyond the stop is unknown (weight 1.0);
//! - each `ControllerActionChosen` decision with `Verify` or
//!   `RetrieveExternally` counts uncovered as a deferred source (weight 1.0);
//! - each `SupernodeBlocked` decision counts uncovered (weight 1.0);
//! - page-ins, seed selections, dead ends, prefetches, and retrieval markers
//!   never change coverage.
//!
//! `completeness = covered / (covered + uncovered)`, and a retrieval with no
//! coverage-relevant decisions is vacuously complete.

use serde::{Deserialize, Serialize};

use crate::{
    bandit_controller::WorkingSetAction,
    proof_carrying_answer::RetrievalCompleteness,
    working_set_telemetry::{RetrievalTelemetryRecord, WorkingSetDecisionEvent},
};

/// One typed reason completeness was reduced.
///
///
/// keep reduced completeness explainable: every uncovered contribution names
/// its cause and its count.
///
///
/// pair a reduction kind with the number of contributing decisions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletenessReduction {
    /// Cause of the reduction.
    pub kind: CompletenessReductionKind,

    /// Number of decisions contributing this reduction.
    pub count: u64,
}

/// Causes of reduced retrieval completeness, in stable report order.
///
///
/// name the uncovered categories of the coverage model so consumers can act
/// on them (raise budgets, expand warm frontiers, resume deferred sources).
///
///
/// enumerate the five reduction causes; report order follows this order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CompletenessReductionKind {
    /// Candidates skipped by profile, budget, relevance, or controller.
    SkippedEdges,

    /// Warm frontier entries whose payloads were never loaded.
    WarmFrontier,

    /// Controller stop decisions cutting the frontier.
    ControllerStops,

    /// Sources deferred to verification or external retrieval.
    DeferredSources,

    /// High-degree frontiers blocked by supernode protection.
    SupernodeBlocks,
}

/// Deterministic completeness report for a set of recorded retrievals.
///
///
/// give the proof-carrying answer envelope its `retrieval_completeness` value
/// together with the explainable reduction breakdown.
///
///
/// carry the validated completeness ratio and the non-zero reductions in
/// stable order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RetrievalCompletenessReport {
    /// Validated completeness ratio in `[0, 1]`.
    pub completeness: RetrievalCompleteness,

    /// Non-zero reductions in `CompletenessReductionKind` order.
    pub reductions: Vec<CompletenessReduction>,
}

/// Compute retrieval completeness from recorded retrievals.
///
///
/// implement the module-level coverage weights as a pure function of the
/// telemetry records so completeness is reproducible from recorded state
/// alone.
///
///
/// sum covered and uncovered weights across the records, derive the ratio
/// (vacuously 1.0 when nothing coverage-relevant was recorded), and list the
/// non-zero reductions in stable order.
///
/// # Errors
///
/// none expected: the computed ratio is bounded by construction, so the
/// validated carrier always accepts it.
pub fn compute_retrieval_completeness(
    records: &[RetrievalTelemetryRecord],
) -> RetrievalCompletenessReport {
    let mut expanded_count: u64 = 0;
    let mut warm_count: u64 = 0;
    let mut skipped_count: u64 = 0;
    let mut stop_count: u64 = 0;
    let mut deferred_count: u64 = 0;
    let mut supernode_count: u64 = 0;

    for record in records {
        for event in &record.events {
            match &event.decision {
                WorkingSetDecisionEvent::EdgeExpanded { .. } => expanded_count += 1,
                WorkingSetDecisionEvent::WarmAdjacencyAttached { .. } => warm_count += 1,
                WorkingSetDecisionEvent::EdgeSkipped { .. } => skipped_count += 1,
                WorkingSetDecisionEvent::ControllerActionChosen { action, .. } => match action {
                    WorkingSetAction::Stop => stop_count += 1,
                    WorkingSetAction::Verify | WorkingSetAction::RetrieveExternally => {
                        deferred_count += 1;
                    }
                    WorkingSetAction::Expand
                    | WorkingSetAction::Prefetch
                    | WorkingSetAction::PageIn => {}
                },
                WorkingSetDecisionEvent::SupernodeBlocked { .. } => supernode_count += 1,
                _ => {}
            }
        }
    }

    let covered = expanded_count as f64 + 0.5 * warm_count as f64;
    let uncovered = skipped_count as f64
        + 0.5 * warm_count as f64
        + stop_count as f64
        + deferred_count as f64
        + supernode_count as f64;
    let total = covered + uncovered;
    let ratio = if total == 0.0 { 1.0 } else { covered / total };

    let mut reductions = Vec::new();
    for (kind, count) in [
        (CompletenessReductionKind::SkippedEdges, skipped_count),
        (CompletenessReductionKind::WarmFrontier, warm_count),
        (CompletenessReductionKind::ControllerStops, stop_count),
        (CompletenessReductionKind::DeferredSources, deferred_count),
        (CompletenessReductionKind::SupernodeBlocks, supernode_count),
    ] {
        if count > 0 {
            reductions.push(CompletenessReduction { kind, count });
        }
    }

    RetrievalCompletenessReport {
        completeness: RetrievalCompleteness::new(ratio)
            .expect("coverage ratio should be bounded by construction"),
        reductions,
    }
}

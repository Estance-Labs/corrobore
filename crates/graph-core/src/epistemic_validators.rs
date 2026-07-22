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
//! Epistemic validators of the immune system (Epic 0019).
//!
//!
//!
//! - Detect the epic's epistemic defect classes deterministically over the
//!   claim, evidence, vocabulary, and bitemporal primitives: unsupported
//!   claims, source circularity, open contradictions, and stale evidence.
//! - Stay a pure read: findings are typed Epic 0007 validation records, no
//!   store is ever mutated, and identical inputs yield identical findings.
//! - Stale-evidence detection is driven by the bitemporal as-of semantics
//!   over caller-linked facts, never by wall clocks.
//! - Report in a documented deterministic order — unsupported claims, source
//!   circularity, open contradictions, then stale evidence — each pass in
//!   listing order.
//!
//! # Detection rules (deterministic)
//!
//! - **Unsupported claim**: a Claim-kind node with no incoming `SUPPORTS`
//!   relation whose source classifies as Observation or Evidence.
//! - **Source circularity**: a Claim-kind node with at least two supporting
//!   observations, where every supporting observation is reported by at
//!   least one source and the distinct reporting sources collapse to one —
//!   pseudo-independent corroboration.
//! - **Open contradiction**: a `CONTRADICTS` relation not declared resolved
//!   by the caller.
//! - **Stale evidence**: a support relation linked by the caller to a
//!   bitemporal fact that has no state valid at the as-of time (unknown
//!   facts are conservatively stale).

use std::collections::HashSet;

use crate::{
    GraphError,
    bitemporal::BitemporalFactStore,
    epistemic_vocabulary::{
        EpistemicNodeKind, EpistemicRelationKind, classify_epistemic_node, epistemic_nodes_of_kind,
    },
    graph::Graph,
    ids::{FactId, NodeId, RelationshipId},
    temporal::TemporalTimestamp,
    validation::{ValidationErrorRecord, ValidationErrorSeverity, ValidationTarget},
};

/// Stable finding code of the unsupported-claim validator.
const UNSUPPORTED_CLAIM_CODE: &str = "immune-epistemic--unsupported-claim";

/// Stable finding code of the source-circularity validator.
const SOURCE_CIRCULARITY_CODE: &str = "immune-epistemic--source-circularity";

/// Stable finding code of the open-contradiction validator.
const OPEN_CONTRADICTION_CODE: &str = "immune-epistemic--open-contradiction";

/// Stable finding code of the stale-evidence validator.
const STALE_EVIDENCE_CODE: &str = "immune-epistemic--stale-evidence";

/// Inputs of one epistemic validation pass.
///
///
/// keep the linkage the domain owns explicit: which support relations are
/// backed by which bitemporal facts, and which contradictions were resolved,
/// come from the caller instead of a hidden registry.
///
///
/// carry the graph, the fact store, the as-of time, and the caller-declared
/// linkages.
pub struct EpistemicValidationInputs<'a> {
    /// Graph under validation; only read.
    pub graph: &'a Graph,

    /// Bitemporal facts backing support relations.
    pub facts: &'a BitemporalFactStore,

    /// As-of time for staleness, in canonical comparable form.
    pub as_of: &'a TemporalTimestamp,

    /// Support relations backed by bitemporal facts, in evaluation order.
    pub evidence_facts: &'a [(RelationshipId, FactId)],

    /// Contradiction relations the caller declares resolved.
    pub resolved_contradictions: &'a [RelationshipId],
}

/// Validate the epistemic integrity of a graph.
///
///
/// give the immune system its epistemic detection pass over the primitives of
/// Epics 0005, 0018, and the bitemporal model, feeding tier routing and
/// probe generation.
///
///
/// run the four detection rules of the module documentation and return the
/// typed findings in the documented order.
///
/// # Errors
///
/// propagate the graph's typed listing and lookup errors; detection itself
/// cannot fail.
pub fn validate_graph_epistemics(
    inputs: &EpistemicValidationInputs<'_>,
) -> Result<Vec<ValidationErrorRecord>, GraphError> {
    let mut findings = Vec::new();

    let claims = epistemic_nodes_of_kind(inputs.graph, EpistemicNodeKind::Claim)?;

    // Pass 1: unsupported claims.
    for claim in &claims {
        if epistemic_supporters(inputs.graph, claim)?.is_empty() {
            findings.push(ValidationErrorRecord::new(
                UNSUPPORTED_CLAIM_CODE,
                ValidationErrorSeverity::Warning,
                format!("claim {} has no epistemic support", claim.as_str()),
                ValidationTarget::node(claim.as_str()),
            ));
        }
    }

    // Pass 2: source circularity.
    for claim in &claims {
        let supporters = epistemic_supporters(inputs.graph, claim)?;
        let observations: Vec<&NodeId> = supporters
            .iter()
            .filter(|(_, kind)| *kind == EpistemicNodeKind::Observation)
            .map(|(node_id, _)| node_id)
            .collect();
        if observations.len() < 2 {
            continue;
        }

        let mut distinct_sources: HashSet<NodeId> = HashSet::new();
        let mut every_observation_reported = true;
        for observation in &observations {
            let reporters = reporting_sources(inputs.graph, observation)?;
            if reporters.is_empty() {
                every_observation_reported = false;
                break;
            }
            distinct_sources.extend(reporters);
        }

        if every_observation_reported && distinct_sources.len() == 1 {
            let lone_source = distinct_sources
                .iter()
                .next()
                .expect("one distinct source should exist");
            findings.push(ValidationErrorRecord::new(
                SOURCE_CIRCULARITY_CODE,
                ValidationErrorSeverity::Warning,
                format!(
                    "claim {} corroboration depends circularly on source {}",
                    claim.as_str(),
                    lone_source.as_str()
                ),
                ValidationTarget::node(claim.as_str()),
            ));
        }
    }

    // Pass 3: open contradictions.
    let contradicts_type = EpistemicRelationKind::Contradicts.canonical_relationship_type();
    for relationship in inputs.graph.list_relationships()? {
        if relationship.rel_type() == &contradicts_type
            && !inputs.resolved_contradictions.contains(relationship.id())
        {
            findings.push(ValidationErrorRecord::new(
                OPEN_CONTRADICTION_CODE,
                ValidationErrorSeverity::Warning,
                format!(
                    "contradiction {} has no declared resolution",
                    relationship.id().as_str()
                ),
                ValidationTarget::relationship(relationship.id().as_str()),
            ));
        }
    }

    // Pass 4: stale evidence.
    for (relationship_id, fact) in inputs.evidence_facts {
        let valid_states = inputs.facts.states_as_of(fact, inputs.as_of, None);
        if valid_states.is_empty() {
            findings.push(ValidationErrorRecord::new(
                STALE_EVIDENCE_CODE,
                ValidationErrorSeverity::Warning,
                format!(
                    "support {} is backed by fact {} with no state valid at {}",
                    relationship_id.as_str(),
                    fact.as_str(),
                    inputs.as_of.as_str()
                ),
                ValidationTarget::relationship(relationship_id.as_str()),
            ));
        }
    }

    Ok(findings)
}

/// Return the epistemic supporters of a claim: sources of incoming `SUPPORTS`
/// relations that classify as Observation or Evidence, with their kinds.
fn epistemic_supporters(
    graph: &Graph,
    claim: &NodeId,
) -> Result<Vec<(NodeId, EpistemicNodeKind)>, GraphError> {
    let supports_type = EpistemicRelationKind::Supports.canonical_relationship_type();
    let mut supporters = Vec::new();

    for relationship in graph.incoming(claim)? {
        if relationship.rel_type() != &supports_type {
            continue;
        }
        let Some(node) = graph.get_node(relationship.source())? else {
            continue;
        };
        if let Some(kind @ (EpistemicNodeKind::Observation | EpistemicNodeKind::Evidence)) =
            classify_epistemic_node(&node)
        {
            supporters.push((relationship.source().clone(), kind));
        }
    }

    Ok(supporters)
}

/// Return the Source-kind nodes reporting one observation.
fn reporting_sources(graph: &Graph, observation: &NodeId) -> Result<Vec<NodeId>, GraphError> {
    let reports_type = EpistemicRelationKind::Reports.canonical_relationship_type();
    let mut sources = Vec::new();

    for relationship in graph.incoming(observation)? {
        if relationship.rel_type() != &reports_type {
            continue;
        }
        let Some(node) = graph.get_node(relationship.source())? else {
            continue;
        };
        if classify_epistemic_node(&node) == Some(EpistemicNodeKind::Source) {
            sources.push(relationship.source().clone());
        }
    }

    Ok(sources)
}

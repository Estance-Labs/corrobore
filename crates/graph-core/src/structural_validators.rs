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
//! Structural validators of the immune system (Epic 0019).
//!
//!
//!
//! - Detect the epic's structural defect classes deterministically over the
//!   graph read surface: dangling links, impossible cycles, and schema
//!   violations.
//! - Never mutate the graph: validation is a pure read, and findings are
//!   typed validation records aligned with the Epic 0007 vocabulary (code,
//!   severity, target, message).
//! - Report in a documented deterministic order — dangling links, impossible
//!   cycles, then schema violations, each in graph listing order — so runs
//!   over identical graphs are equal.
//! - Keep responses, tier movements, and probes out of this module; they
//!   consume these findings.
//!
//! # Epistemic endpoint schema (deterministic)
//!
//! Relations whose type classifies in the epistemic vocabulary must connect
//! the declared endpoint kinds; endpoints outside the vocabulary violate the
//! schema:
//!
//! - `REPORTS`: Source -> Observation;
//! - `SUPPORTS` / `REFUTES` / `CONTRADICTS`: Observation | Evidence | Claim
//!   -> Claim | Hypothesis;
//! - `SUPERSEDES`: Claim -> Claim;
//! - `ASSESSES`: Assessment -> Claim | Hypothesis;
//! - `INFERS`: Inference -> Claim | Hypothesis;
//! - `DECIDES`: Decision -> Assessment | Claim | Hypothesis.

use std::collections::{HashMap, HashSet};

use crate::{
    GraphError,
    epistemic_vocabulary::{EpistemicNodeKind, EpistemicRelationKind, classify_epistemic_node},
    graph::Graph,
    ids::NodeId,
    relationship::{Relationship, RelationshipType},
    validation::{ValidationErrorRecord, ValidationErrorSeverity, ValidationTarget},
};

/// Stable finding code of the dangling-link validator.
const DANGLING_LINK_CODE: &str = "immune-structural--dangling-link";

/// Stable finding code of the impossible-cycle validator.
const IMPOSSIBLE_CYCLE_CODE: &str = "immune-structural--impossible-cycle";

/// Stable finding code of the schema-violation validator.
const SCHEMA_VIOLATION_CODE: &str = "immune-structural--schema-violation";

/// Validate the structural integrity of a graph.
///
///
/// give the immune system its structural detection pass: every finding names
/// a concrete defect on a concrete record, ready for tier routing and repair
/// proposals.
///
///
/// run the dangling-link, impossible-cycle (for the declared acyclic
/// relationship types), and epistemic schema validators over the graph
/// listings, and return the typed findings in the documented order.
///
/// # Errors
///
/// propagate the graph's typed listing and lookup errors; detection itself
/// cannot fail.
pub fn validate_graph_structure(
    graph: &Graph,
    acyclic_relationship_types: &[RelationshipType],
) -> Result<Vec<ValidationErrorRecord>, GraphError> {
    let relationships = graph.list_relationships()?;
    let mut findings = Vec::new();

    findings.extend(dangling_link_findings(graph, &relationships)?);
    findings.extend(impossible_cycle_findings(
        &relationships,
        acyclic_relationship_types,
    ));
    findings.extend(schema_violation_findings(graph, &relationships)?);

    Ok(findings)
}

fn dangling_link_findings(
    graph: &Graph,
    relationships: &[Relationship],
) -> Result<Vec<ValidationErrorRecord>, GraphError> {
    let mut findings = Vec::new();

    for relationship in relationships {
        for endpoint in [relationship.source(), relationship.target()] {
            if graph.get_node(endpoint)?.is_none() {
                findings.push(ValidationErrorRecord::new(
                    DANGLING_LINK_CODE,
                    ValidationErrorSeverity::Error,
                    format!(
                        "relationship {} references missing or tombstoned node {}",
                        relationship.id().as_str(),
                        endpoint.as_str()
                    ),
                    ValidationTarget::relationship(relationship.id().as_str()),
                ));
            }
        }
    }

    Ok(findings)
}

fn impossible_cycle_findings(
    relationships: &[Relationship],
    acyclic_relationship_types: &[RelationshipType],
) -> Vec<ValidationErrorRecord> {
    let mut findings = Vec::new();

    for relationship_type in acyclic_relationship_types {
        let edges: Vec<&Relationship> = relationships
            .iter()
            .filter(|relationship| relationship.rel_type() == relationship_type)
            .collect();

        let mut adjacency: HashMap<&NodeId, Vec<&Relationship>> = HashMap::new();
        for edge in &edges {
            adjacency.entry(edge.source()).or_default().push(edge);
        }

        // Depth-first search in listing order; an edge reaching a node on the
        // current path closes an impossible cycle.
        let mut settled: HashSet<&NodeId> = HashSet::new();
        for start in edges.iter().map(|edge| edge.source()) {
            if settled.contains(start) {
                continue;
            }
            let mut on_path: Vec<&NodeId> = Vec::new();
            let mut stack: Vec<(&NodeId, usize)> = vec![(start, 0)];
            on_path.push(start);

            while let Some((node, next_edge)) = stack.pop() {
                let neighbors = adjacency.get(node).map_or(&[][..], Vec::as_slice);
                if next_edge >= neighbors.len() {
                    settled.insert(node);
                    on_path.pop();
                    continue;
                }
                stack.push((node, next_edge + 1));

                let edge = neighbors[next_edge];
                let target = edge.target();
                if on_path.contains(&target) {
                    findings.push(ValidationErrorRecord::new(
                        IMPOSSIBLE_CYCLE_CODE,
                        ValidationErrorSeverity::Error,
                        format!(
                            "relationship {} closes a cycle over acyclic type {}",
                            edge.id().as_str(),
                            relationship_type.as_str()
                        ),
                        ValidationTarget::relationship(edge.id().as_str()),
                    ));
                } else if !settled.contains(target) {
                    on_path.push(target);
                    stack.push((target, 0));
                }
            }
        }
    }

    findings
}

fn schema_violation_findings(
    graph: &Graph,
    relationships: &[Relationship],
) -> Result<Vec<ValidationErrorRecord>, GraphError> {
    let mut findings = Vec::new();

    for relationship in relationships {
        let Some(relation_kind) =
            EpistemicRelationKind::from_relationship_type(relationship.rel_type())
        else {
            continue;
        };
        let (expected_sources, expected_targets) = expected_endpoints(relation_kind);

        let source_kind = endpoint_kind(graph, relationship.source())?;
        let target_kind = endpoint_kind(graph, relationship.target())?;
        let source_ok = source_kind.is_some_and(|kind| expected_sources.contains(&kind));
        let target_ok = target_kind.is_some_and(|kind| expected_targets.contains(&kind));

        if !source_ok || !target_ok {
            findings.push(ValidationErrorRecord::new(
                SCHEMA_VIOLATION_CODE,
                ValidationErrorSeverity::Error,
                format!(
                    "relationship {} violates the {relation_kind:?} endpoint schema \
                     (source {source_kind:?}, target {target_kind:?})",
                    relationship.id().as_str()
                ),
                ValidationTarget::relationship(relationship.id().as_str()),
            ));
        }
    }

    Ok(findings)
}

fn endpoint_kind(graph: &Graph, node_id: &NodeId) -> Result<Option<EpistemicNodeKind>, GraphError> {
    Ok(graph
        .get_node(node_id)?
        .as_ref()
        .and_then(classify_epistemic_node))
}

fn expected_endpoints(
    kind: EpistemicRelationKind,
) -> (&'static [EpistemicNodeKind], &'static [EpistemicNodeKind]) {
    const CLAIM_LIKE: &[EpistemicNodeKind] =
        &[EpistemicNodeKind::Claim, EpistemicNodeKind::Hypothesis];
    const SUPPORTERS: &[EpistemicNodeKind] = &[
        EpistemicNodeKind::Observation,
        EpistemicNodeKind::Evidence,
        EpistemicNodeKind::Claim,
    ];

    match kind {
        EpistemicRelationKind::Reports => (
            &[EpistemicNodeKind::Source],
            &[EpistemicNodeKind::Observation],
        ),
        EpistemicRelationKind::Supports
        | EpistemicRelationKind::Refutes
        | EpistemicRelationKind::Contradicts => (SUPPORTERS, CLAIM_LIKE),
        EpistemicRelationKind::Supersedes => {
            (&[EpistemicNodeKind::Claim], &[EpistemicNodeKind::Claim])
        }
        // Epic 0029 evidence-link kinds: context and derivation come from any
        // supporter; duplication and dependency are claim-to-claim.
        EpistemicRelationKind::ContextFor | EpistemicRelationKind::DerivedFrom => {
            (SUPPORTERS, CLAIM_LIKE)
        }
        EpistemicRelationKind::Duplicates | EpistemicRelationKind::DependsOn => {
            (CLAIM_LIKE, CLAIM_LIKE)
        }
        EpistemicRelationKind::Assesses => (&[EpistemicNodeKind::Assessment], CLAIM_LIKE),
        EpistemicRelationKind::Infers => (&[EpistemicNodeKind::Inference], CLAIM_LIKE),
        EpistemicRelationKind::Decides => (
            &[EpistemicNodeKind::Decision],
            &[
                EpistemicNodeKind::Assessment,
                EpistemicNodeKind::Claim,
                EpistemicNodeKind::Hypothesis,
            ],
        ),
    }
}

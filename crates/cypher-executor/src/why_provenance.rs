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
//! Why-provenance: what a query read, and separately, what supports it.
//!
//! Module boundary: this module records the substructure an answer was computed
//! from. It decides nothing epistemic.
//!
//! The separation is the point. Computational provenance is causal — the query
//! bound this node, read that property — and says nothing about whether the
//! answer is true. Semantic support is evidential: this claim has these
//! supporting and refuting links. Folding them into one list would let a reader
//! take "the query touched it" for "the evidence backs it", so they stay
//! distinct fields with distinct names, and a report says which one it is.

use cypher_planner::ProvenancePlan;
use graph_core::{EpistemicRelationKind, Graph, Node, PropertyValue, Relationship};

/// A graph element an answer was computed from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProvenanceElement {
    /// Node identity.
    Node(String),
    /// Relationship identity.
    Relationship(String),
}

/// One binding that contributed to an answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContributingElement {
    variable: String,
    element: ProvenanceElement,
    projected: bool,
}

/// The causal read set behind one returned record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowProvenance {
    row: usize,
    contributing: Vec<ContributingElement>,
}

/// Evidence that semantically supports a claim the answer read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SemanticSupport {
    claim_id: String,
    node_id: String,
    verdict_state: Option<String>,
    supporting: Vec<String>,
    refuting: Vec<String>,
}

/// What an answer was computed from, and what supports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WhyProvenance {
    declared: ProvenancePlan,
    computational: Vec<RowProvenance>,
    semantic_support: Vec<SemanticSupport>,
}

impl ContributingElement {
    /// Pattern variable the element was bound to.
    pub fn variable(&self) -> &str {
        &self.variable
    }

    /// The element itself.
    pub fn element(&self) -> &ProvenanceElement {
        &self.element
    }

    /// Whether the answer reads this binding.
    pub fn projected(&self) -> bool {
        self.projected
    }
}

impl RowProvenance {
    /// Zero-based index of the record this read set produced.
    pub fn row(&self) -> usize {
        self.row
    }

    /// Elements the row was computed from, in variable order.
    pub fn contributing(&self) -> &[ContributingElement] {
        &self.contributing
    }
}

impl SemanticSupport {
    /// Claim identity carried by the projected node.
    pub fn claim_id(&self) -> &str {
        &self.claim_id
    }

    /// Projected node the query read.
    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    /// Projected verdict state, when the claim has one.
    pub fn verdict_state(&self) -> Option<&str> {
        self.verdict_state.as_deref()
    }

    /// Elements whose links support the claim.
    pub fn supporting(&self) -> &[String] {
        &self.supporting
    }

    /// Elements whose links refute or contradict the claim.
    pub fn refuting(&self) -> &[String] {
        &self.refuting
    }
}

impl WhyProvenance {
    /// The read set the plan declared before execution.
    pub fn declared(&self) -> &ProvenancePlan {
        &self.declared
    }

    /// What each returned record was actually computed from.
    pub fn computational(&self) -> &[RowProvenance] {
        &self.computational
    }

    /// Evidence supporting the claims the answer read. Empty when the answer
    /// read no claim, which is the ordinary case for a structural query.
    pub fn semantic_support(&self) -> &[SemanticSupport] {
        &self.semantic_support
    }

    /// Whether this answer carries a causal read set and no evidential claim.
    /// A caller that reports provenance must say which kind it is showing.
    pub fn is_computational_only(&self) -> bool {
        self.semantic_support.is_empty()
    }
}

/// Assemble a contributing element from a bound node or relationship.
pub(crate) fn node_element(variable: &str, node: &Node, projected: bool) -> ContributingElement {
    ContributingElement {
        variable: variable.to_owned(),
        element: ProvenanceElement::Node(node.id().as_str().to_owned()),
        projected,
    }
}

pub(crate) fn relationship_element(
    variable: &str,
    relationship: &Relationship,
    projected: bool,
) -> ContributingElement {
    ContributingElement {
        variable: variable.to_owned(),
        element: ProvenanceElement::Relationship(relationship.id().as_str().to_owned()),
        projected,
    }
}

// One variable can contribute several distinct elements when an aggregate
// unions the rows that fed it, so identity is the variable and the element
// together: deduplicating on the variable alone would hide rows.
pub(crate) fn row_provenance(
    row: usize,
    mut contributing: Vec<ContributingElement>,
) -> RowProvenance {
    let key = |element: &ContributingElement| {
        (
            element.variable.clone(),
            match &element.element {
                ProvenanceElement::Node(id) => format!("node:{id}"),
                ProvenanceElement::Relationship(id) => format!("relationship:{id}"),
            },
        )
    };
    contributing.sort_by_key(key);
    contributing.dedup_by_key(|element| key(element));
    RowProvenance { row, contributing }
}

/// Derive the semantic support of every claim the answer read.
///
/// Support is read from the graph the query ran against: the epistemic
/// projection carries claims as nodes and their evidence links as typed
/// relationships, so the evidential answer comes from retained records rather
/// than from a second interpretation of the query.
pub(crate) fn semantic_support(graph: &Graph, rows: &[RowProvenance]) -> Vec<SemanticSupport> {
    let mut read_nodes: Vec<String> = rows
        .iter()
        .flat_map(|row| row.contributing.iter())
        .filter_map(|element| match &element.element {
            ProvenanceElement::Node(id) => Some(id.clone()),
            ProvenanceElement::Relationship(_) => None,
        })
        .collect();
    read_nodes.sort();
    read_nodes.dedup();

    let relationships = graph.list_relationships().unwrap_or_default();
    let mut support = Vec::new();
    for node_id in read_nodes {
        let Some(node) = graph_core::NodeId::new(&node_id)
            .ok()
            .and_then(|id| graph.get_node(&id).ok().flatten())
        else {
            continue;
        };
        let Some(claim_id) = string_property(&node, "claim_id") else {
            continue;
        };
        let mut supporting = Vec::new();
        let mut refuting = Vec::new();
        for relationship in &relationships {
            if relationship.target().as_str() != node_id {
                continue;
            }
            let kind = relationship.rel_type().as_str();
            let source = relationship.source().as_str().to_owned();
            if kind
                == EpistemicRelationKind::Supports
                    .canonical_relationship_type()
                    .as_str()
            {
                supporting.push(source);
            } else if kind
                == EpistemicRelationKind::Refutes
                    .canonical_relationship_type()
                    .as_str()
                || kind
                    == EpistemicRelationKind::Contradicts
                        .canonical_relationship_type()
                        .as_str()
            {
                refuting.push(source);
            }
        }
        supporting.sort();
        refuting.sort();
        support.push(SemanticSupport {
            claim_id,
            node_id,
            verdict_state: string_property(&node, "verdict_state"),
            supporting,
            refuting,
        });
    }
    support
}

fn string_property(node: &Node, key: &str) -> Option<String> {
    match node.property(key) {
        Some(PropertyValue::String(value)) => Some(value.clone()),
        _ => None,
    }
}

pub(crate) fn assemble(
    declared: ProvenancePlan,
    computational: Vec<RowProvenance>,
    semantic_support: Vec<SemanticSupport>,
) -> WhyProvenance {
    WhyProvenance {
        declared,
        computational,
        semantic_support,
    }
}

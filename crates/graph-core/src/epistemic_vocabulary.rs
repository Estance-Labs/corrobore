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
//! Explicit epistemic node and relation vocabulary (Epic 0018).
//!
//!
//!
//! - Stop conflating facts, claims, observations, hypotheses, and
//!   interpretations: every epistemic kind is an explicit, typed member of a
//!   closed vocabulary, never a free-text label convention.
//! - Reuse the Epic 0005 claim primitives: the relation vocabulary embeds the
//!   existing `ClaimLinkKind` semantics instead of duplicating them, and the
//!   claim, evidence, stance, and hypothesis types keep their contracts while
//!   gaining explicit kinds.
//! - Make each kind independently queryable at the graph-core boundary with
//!   deterministic, insertion-ordered results.
//! - Do not implement the proof-carrying answer envelope, completeness,
//!   bitemporality, or provenance here; later issues consume this vocabulary.
//!
//! # Classification precedence (deterministic)
//!
//! A node classifies as the first kind in `EpistemicNodeKind::ALL` order whose
//! canonical label it carries; nodes without any canonical label are outside
//! the epistemic vocabulary.

use serde::{Deserialize, Serialize};

use crate::{
    GraphError,
    claim::{AgentStance, Claim, ClaimLinkKind, HypothesisWorkspace},
    evidence::EvidenceRecord,
    graph::Graph,
    ids::NodeId,
    node::Node,
    relationship::RelationshipType,
};

/// One kind of the closed epistemic node vocabulary.
///
///
/// name the eleven epistemic kinds of the epic explicitly so intelligence
/// reasoning can distinguish what is known, observed, claimed, hypothesized,
/// and decided.
///
///
/// enumerate the kinds; `ALL` fixes classification precedence and report
/// order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EpistemicNodeKind {
    /// A real-world entity (actor, infrastructure, account).
    Entity,

    /// A real-world event.
    Event,

    /// A source-grounded observation.
    Observation,

    /// A claim whose truth is under epistemic management.
    Claim,

    /// A hypothesis under investigation.
    Hypothesis,

    /// An evidence record backing or refuting claims.
    Evidence,

    /// A source that reports observations.
    Source,

    /// An analyst or agent assessment.
    Assessment,

    /// An explicit contradiction between claims or observations.
    Contradiction,

    /// An inferred statement derived from other epistemic records.
    Inference,

    /// A decision taken on the basis of assessments.
    Decision,
}

impl EpistemicNodeKind {
    /// The complete, ordered epistemic node vocabulary.
    ///
    ///
    /// fix classification precedence: the first matching kind in this order
    /// wins when a node carries several canonical labels.
    pub const ALL: [EpistemicNodeKind; 11] = [
        EpistemicNodeKind::Entity,
        EpistemicNodeKind::Event,
        EpistemicNodeKind::Observation,
        EpistemicNodeKind::Claim,
        EpistemicNodeKind::Hypothesis,
        EpistemicNodeKind::Evidence,
        EpistemicNodeKind::Source,
        EpistemicNodeKind::Assessment,
        EpistemicNodeKind::Contradiction,
        EpistemicNodeKind::Inference,
        EpistemicNodeKind::Decision,
    ];

    /// Return the canonical node label of this kind.
    ///
    ///
    /// keep the label vocabulary closed and typo-proof: classification only
    /// ever compares against these constants.
    ///
    ///
    /// return the stable label string.
    ///
    /// # Errors
    ///
    /// none expected because the mapping is total.
    pub fn canonical_label(self) -> &'static str {
        match self {
            EpistemicNodeKind::Entity => "Entity",
            EpistemicNodeKind::Event => "Event",
            EpistemicNodeKind::Observation => "Observation",
            EpistemicNodeKind::Claim => "Claim",
            EpistemicNodeKind::Hypothesis => "Hypothesis",
            EpistemicNodeKind::Evidence => "Evidence",
            EpistemicNodeKind::Source => "Source",
            EpistemicNodeKind::Assessment => "Assessment",
            EpistemicNodeKind::Contradiction => "Contradiction",
            EpistemicNodeKind::Inference => "Inference",
            EpistemicNodeKind::Decision => "Decision",
        }
    }

    /// Classify a label against the canonical vocabulary.
    ///
    ///
    /// centralize label interpretation so every consumer classifies
    /// identically.
    ///
    ///
    /// return the kind whose canonical label matches, or `None` for labels
    /// outside the vocabulary.
    ///
    /// # Errors
    ///
    /// none expected because absence is a deterministic `None`.
    pub fn from_label(label: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.canonical_label() == label)
    }
}

/// One kind of the closed epistemic relation vocabulary.
///
///
/// name the epistemic relations explicitly, embedding the Epic 0005
/// claim-link semantics (supports, refutes, contradicts, supersedes) instead
/// of duplicating them.
///
///
/// enumerate the kinds; `ALL` fixes the report order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EpistemicRelationKind {
    /// A source reports an observation.
    Reports,

    /// Supporting context for a target claim (claim-link aligned).
    Supports,

    /// Refuting context for a target claim (claim-link aligned).
    Refutes,

    /// Explicit conflict between two claims (claim-link aligned).
    Contradicts,

    /// One claim replaces another with audit history (claim-link aligned).
    Supersedes,

    /// An assessment evaluates a claim or hypothesis.
    Assesses,

    /// An inference derives from epistemic records.
    Infers,

    /// A decision follows from assessments.
    Decides,
}

impl EpistemicRelationKind {
    /// The complete, ordered epistemic relation vocabulary.
    ///
    ///
    /// fix the report order so vocabularies diff cleanly across runs.
    pub const ALL: [EpistemicRelationKind; 8] = [
        EpistemicRelationKind::Reports,
        EpistemicRelationKind::Supports,
        EpistemicRelationKind::Refutes,
        EpistemicRelationKind::Contradicts,
        EpistemicRelationKind::Supersedes,
        EpistemicRelationKind::Assesses,
        EpistemicRelationKind::Infers,
        EpistemicRelationKind::Decides,
    ];

    /// Return the canonical relationship type of this kind.
    ///
    ///
    /// keep the relation vocabulary closed: graph relationships expressing
    /// epistemic structure use exactly these types.
    ///
    ///
    /// return the validated canonical relationship type.
    ///
    /// # Errors
    ///
    /// none expected because the canonical names are statically valid.
    pub fn canonical_relationship_type(self) -> RelationshipType {
        let name = match self {
            EpistemicRelationKind::Reports => "REPORTS",
            EpistemicRelationKind::Supports => "SUPPORTS",
            EpistemicRelationKind::Refutes => "REFUTES",
            EpistemicRelationKind::Contradicts => "CONTRADICTS",
            EpistemicRelationKind::Supersedes => "SUPERSEDES",
            EpistemicRelationKind::Assesses => "ASSESSES",
            EpistemicRelationKind::Infers => "INFERS",
            EpistemicRelationKind::Decides => "DECIDES",
        };
        RelationshipType::new(name).expect("canonical relationship type should be valid")
    }

    /// Classify a relationship type against the canonical vocabulary.
    ///
    ///
    /// centralize relationship-type interpretation so every consumer
    /// classifies identically.
    ///
    ///
    /// return the kind whose canonical type matches, or `None` for types
    /// outside the vocabulary.
    ///
    /// # Errors
    ///
    /// none expected because absence is a deterministic `None`.
    pub fn from_relationship_type(relationship_type: &RelationshipType) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| &kind.canonical_relationship_type() == relationship_type)
    }

    /// Return the Epic 0005 claim-link kind this relation embeds, when one
    /// exists.
    ///
    ///
    /// keep the claim graph authoritative for claim-to-claim semantics: only
    /// the four claim-link kinds map back, the others are vocabulary-only.
    ///
    ///
    /// return the aligned `ClaimLinkKind` for supports, refutes, contradicts,
    /// and supersedes; `None` otherwise.
    ///
    /// # Errors
    ///
    /// none expected because the partial mapping is explicit.
    pub fn claim_link_kind(self) -> Option<ClaimLinkKind> {
        match self {
            EpistemicRelationKind::Supports => Some(ClaimLinkKind::Supports),
            EpistemicRelationKind::Refutes => Some(ClaimLinkKind::Refutes),
            EpistemicRelationKind::Contradicts => Some(ClaimLinkKind::Contradicts),
            EpistemicRelationKind::Supersedes => Some(ClaimLinkKind::Supersedes),
            EpistemicRelationKind::Reports
            | EpistemicRelationKind::Assesses
            | EpistemicRelationKind::Infers
            | EpistemicRelationKind::Decides => None,
        }
    }
}

impl From<ClaimLinkKind> for EpistemicRelationKind {
    fn from(link_kind: ClaimLinkKind) -> Self {
        match link_kind {
            ClaimLinkKind::Supports => EpistemicRelationKind::Supports,
            ClaimLinkKind::Refutes => EpistemicRelationKind::Refutes,
            ClaimLinkKind::Contradicts => EpistemicRelationKind::Contradicts,
            ClaimLinkKind::Supersedes => EpistemicRelationKind::Supersedes,
        }
    }
}

/// Compile-time epistemic kind of an existing graph-core primitive.
///
///
/// let the Epic 0005 primitives keep their contracts while making their place
/// in the vocabulary explicit and queryable at compile time.
///
///
/// expose one associated kind per implementing type.
pub trait EpistemicPrimitive {
    /// Epistemic kind of this primitive.
    const KIND: EpistemicNodeKind;
}

impl EpistemicPrimitive for Claim {
    const KIND: EpistemicNodeKind = EpistemicNodeKind::Claim;
}

impl EpistemicPrimitive for EvidenceRecord {
    const KIND: EpistemicNodeKind = EpistemicNodeKind::Evidence;
}

impl EpistemicPrimitive for AgentStance {
    const KIND: EpistemicNodeKind = EpistemicNodeKind::Assessment;
}

impl EpistemicPrimitive for HypothesisWorkspace {
    const KIND: EpistemicNodeKind = EpistemicNodeKind::Hypothesis;
}

/// Classify one node against the epistemic vocabulary.
///
///
/// give every consumer the same deterministic node classification with the
/// documented precedence.
///
///
/// return the first kind in `ALL` order whose canonical label the node
/// carries, or `None` when the node is outside the vocabulary.
///
/// # Errors
///
/// none expected because absence is a deterministic `None`.
pub fn classify_epistemic_node(node: &Node) -> Option<EpistemicNodeKind> {
    EpistemicNodeKind::ALL
        .into_iter()
        .find(|kind| node.has_label(kind.canonical_label()))
}

/// Query all current nodes of one epistemic kind.
///
///
/// make each kind independently queryable at the graph boundary without
/// label-string dispatch at call sites.
///
///
/// return the IDs of current nodes classifying as the kind, in insertion
/// order.
///
/// # Errors
///
/// propagate the graph's typed listing errors.
pub fn epistemic_nodes_of_kind(
    graph: &Graph,
    kind: EpistemicNodeKind,
) -> Result<Vec<NodeId>, GraphError> {
    Ok(graph
        .list_nodes()?
        .iter()
        .filter(|node| classify_epistemic_node(node) == Some(kind))
        .map(|node| node.id().clone())
        .collect())
}

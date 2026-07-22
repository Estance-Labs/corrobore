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
//! Proof-carrying answer envelope (Epic 0018).
//!
//!
//!
//! - Never return a bare answer: every retrieval response carries its proof —
//!   supporting subgraph, counter-evidence, provenance, confidence,
//!   retrieval completeness, and unresolved unknowns.
//! - Separate the three uncertainties the epic distinguishes: answer
//!   uncertainty (`confidence`), evidence uncertainty (the supporting and
//!   counter-evidence subgraphs), and retrieval-state uncertainty
//!   (`retrieval_completeness`); a confident-but-incomplete answer is a
//!   first-class state.
//! - Type proof content as graph record references and epistemic identifiers,
//!   never prose.
//! - Keep completeness computation (issue on the completeness signal) and
//!   full-trajectory provenance capture (issue on provenance) out of this
//!   module; the envelope only carries their typed results.

use serde::{Deserialize, Serialize};

use crate::{
    Confidence, GraphError,
    ids::{ClaimId, EvidenceId, NodeId, RelationshipId, RequestId},
};

/// The answered statement of a proof-carrying response.
///
///
/// keep the human-readable answer text attached to its epistemic anchor: the
/// claim whose assessment the answer expresses, when one exists.
///
///
/// carry the answer text and the optional primary claim identifier.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnswerStatement {
    /// Human-readable answer text.
    pub text: String,

    /// Claim the answer primarily assesses, when the answer is claim-backed.
    pub primary_claim_id: Option<ClaimId>,
}

/// Typed graph references backing or opposing an answer.
///
///
/// make proof content auditable record references — nodes, relationships,
/// claims, and evidence records — instead of prose that cannot be traversed.
///
///
/// carry the identifier collections; emptiness is an explicit, queryable
/// state.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EvidenceSubgraph {
    /// Nodes of the proof subgraph.
    pub node_ids: Vec<NodeId>,

    /// Relationships of the proof subgraph.
    pub relationship_ids: Vec<RelationshipId>,

    /// Claims participating in the proof.
    pub claim_ids: Vec<ClaimId>,

    /// Evidence records participating in the proof.
    pub evidence_ids: Vec<EvidenceId>,
}

impl EvidenceSubgraph {
    /// Report whether the subgraph references nothing.
    ///
    ///
    /// let consumers distinguish "no counter-evidence found" from "counter-
    /// evidence present" without inspecting four collections.
    ///
    ///
    /// return true when all identifier collections are empty.
    ///
    /// # Errors
    ///
    /// none expected because the check is pure.
    pub fn is_empty(&self) -> bool {
        self.node_ids.is_empty()
            && self.relationship_ids.is_empty()
            && self.claim_ids.is_empty()
            && self.evidence_ids.is_empty()
    }
}

/// Reference from an answer to its recorded provenance.
///
///
/// link the envelope to the retrievals whose recorded telemetry holds the
/// full navigation trajectory — captured by the dedicated provenance issue —
/// and to the cited source references, without duplicating either here.
///
///
/// carry the retrieval identifiers and cited source references in order.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SourceProvenanceRef {
    /// Retrievals whose telemetry records the navigation trajectory.
    pub retrieval_ids: Vec<RequestId>,

    /// Cited source references.
    pub source_refs: Vec<String>,
}

/// Validated retrieval-completeness ratio in `[0, 1]`.
///
///
/// make retrieval-state uncertainty a first-class bounded signal, distinct
/// from answer confidence: a response can be correct on the loaded elements
/// yet misleading because the working set was incomplete.
///
///
/// wrap the validated ratio as a copyable primitive; its computation from
/// working-set state belongs to the dedicated completeness issue.
///
/// # Errors
///
/// construction returns `GraphError::InvalidRetrievalCompleteness` for NaN or
/// out-of-range values.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RetrievalCompleteness(f64);

impl RetrievalCompleteness {
    /// Validate and wrap a completeness ratio.
    ///
    ///
    /// make invalid retrieval-state signals unrepresentable past this
    /// boundary, mirroring the confidence primitive.
    ///
    ///
    /// accept ratios in `[0, 1]`; reject NaN and out-of-range values.
    ///
    /// # Errors
    ///
    /// return `GraphError::InvalidRetrievalCompleteness` when the ratio is
    /// NaN or outside `[0, 1]`.
    pub fn new(value: f64) -> Result<Self, GraphError> {
        if value.is_nan() || !(0.0..=1.0).contains(&value) {
            return Err(GraphError::InvalidRetrievalCompleteness(value));
        }

        Ok(Self(value))
    }

    /// Return the inner completeness ratio.
    ///
    ///
    /// expose the validated value for reporting and thresholds.
    ///
    ///
    /// return the wrapped ratio.
    ///
    /// # Errors
    ///
    /// none expected because the ratio was validated at construction.
    pub fn value(self) -> f64 {
        self.0
    }
}

/// One typed open question left by a retrieval.
///
///
/// make "what we do not know" explicit and actionable instead of an omission:
/// unknowns are the seed of the next-best-evidence epic.
///
///
/// model the three unknown cases: claims lacking evidence, contradictions
/// left unresolved, and frontiers never expanded.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum UnresolvedUnknown {
    /// A claim in the answer path lacks supporting evidence.
    MissingEvidence {
        /// Claim needing evidence.
        claim_id: ClaimId,
    },

    /// Two claims contradict each other and no resolution was applied.
    UnresolvedContradiction {
        /// Claim under contradiction.
        claim_id: ClaimId,
        /// Claim contradicting it.
        contradicting_claim_id: ClaimId,
    },

    /// A frontier node was never expanded (budget, controller, or deferral).
    UnexpandedFrontier {
        /// Frontier node left unexpanded.
        node_id: NodeId,
    },
}

/// Proof-carrying answer envelope.
///
///
/// give every retrieval response the epic's seven components as one typed
/// value so downstream consumers can audit the answer, weigh its proof, and
/// see what remains unknown.
///
///
/// carry the answer statement, supporting and counter-evidence subgraphs, the
/// provenance reference, the confidence, the retrieval completeness, and the
/// unresolved unknowns.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProofCarryingAnswer {
    /// The answered statement.
    pub answer: AnswerStatement,

    /// Typed references supporting the answer.
    pub supporting_subgraph: EvidenceSubgraph,

    /// Typed references opposing the answer.
    pub counter_evidence: EvidenceSubgraph,

    /// Reference to the recorded provenance of the answer.
    pub source_provenance: SourceProvenanceRef,

    /// Answer uncertainty.
    pub confidence: Confidence,

    /// Retrieval-state uncertainty, independent of confidence.
    pub retrieval_completeness: RetrievalCompleteness,

    /// Typed open questions left by the retrieval.
    pub unresolved_unknowns: Vec<UnresolvedUnknown>,
}

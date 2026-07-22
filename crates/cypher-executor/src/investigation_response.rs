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
use cypher_parser::{NormalizedThreshold, ReturnProjection};
use graph_core::{
    CalibratedAssessment, EvidenceSubgraph, NextBestEvidenceRanking, SourceProvenanceRef,
    UnresolvedUnknown,
};
use serde::Serialize;

use crate::{InvestigationContractOutcome, InvestigationStopReason};

/// Available source values from which RETURN fields can be projected.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InvestigationProjectionSource {
    assessment: Option<CalibratedAssessment>,
    proof_graph: Option<EvidenceSubgraph>,
    counter_evidence: Option<EvidenceSubgraph>,
    unknowns: Option<Vec<UnresolvedUnknown>>,
    next_best_evidence: Option<NextBestEvidenceRanking>,
    provenance: Option<SourceProvenanceRef>,
}

impl InvestigationProjectionSource {
    /// Extracts all projection values from one calibrated assessment.
    #[must_use]
    pub fn from_assessment(assessment: CalibratedAssessment) -> Self {
        Self {
            proof_graph: Some(assessment.supporting_evidence().clone()),
            counter_evidence: Some(assessment.counter_evidence().clone()),
            unknowns: Some(assessment.unresolved_unknowns().to_vec()),
            next_best_evidence: Some(assessment.next_best_evidence().clone()),
            provenance: Some(assessment.source_provenance().clone()),
            assessment: Some(assessment),
        }
    }
}

/// Availability of one requested projection.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum InvestigationProjectionValue<T> {
    /// Requested value was produced and is attached.
    Available(T),
    /// Requested value was not produced by execution.
    Unavailable {
        /// Typed reason no successful value is attached.
        reason: InvestigationProjectionUnavailableReason,
    },
}

/// Explicit reason a requested projection has no value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum InvestigationProjectionUnavailableReason {
    /// The execution source did not produce the requested value.
    NotProduced,
}

/// One typed field selected by the RETURN contract.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum InvestigationProjectedField {
    /// Complete calibrated assessment.
    Assessment(InvestigationProjectionValue<Box<CalibratedAssessment>>),
    /// Supporting proof graph.
    ProofGraph(InvestigationProjectionValue<EvidenceSubgraph>),
    /// Counter-evidence graph.
    CounterEvidence(InvestigationProjectionValue<EvidenceSubgraph>),
    /// Explicit unresolved unknowns.
    Unknowns(InvestigationProjectionValue<Vec<UnresolvedUnknown>>),
    /// Ranked next-best-evidence proposals.
    NextBestEvidence(InvestigationProjectionValue<NextBestEvidenceRanking>),
}

impl InvestigationProjectedField {
    /// Reports whether the requested field is explicitly unavailable.
    #[must_use]
    pub fn is_unavailable(&self) -> bool {
        match self {
            Self::Assessment(value) => unavailable(value),
            Self::ProofGraph(value) | Self::CounterEvidence(value) => unavailable(value),
            Self::Unknowns(value) => unavailable(value),
            Self::NextBestEvidence(value) => unavailable(value),
        }
    }
}

/// Audit metadata attached to an investigation response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct InvestigationResponseMetadata {
    /// Outcomes proving enforcement of applicable contracts.
    pub contract_outcomes: Vec<InvestigationContractOutcome>,
    /// Measured evidence completeness, when available.
    pub completeness: Option<NormalizedThreshold>,
    /// Temporal snapshot used to produce the response.
    pub temporal_context: Option<String>,
    /// Deterministic execution stop reason, when execution did not complete.
    pub stop_reason: Option<InvestigationStopReason>,
}

/// Gateway-safe typed investigation response.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct InvestigationResponse {
    /// Fields selected by RETURN in deterministic normalized order.
    pub fields: Vec<InvestigationProjectedField>,
    /// Shared source provenance, when produced.
    pub provenance: Option<SourceProvenanceRef>,
    /// Contract and execution audit metadata.
    pub metadata: InvestigationResponseMetadata,
}

/// Projects exactly the fields requested by a normalized RETURN contract.
///
/// Missing source values remain explicit unavailable entries; the projector
/// never fabricates a successful value or silently drops a requested field.
pub fn project_investigation_response(
    requested: &[ReturnProjection],
    source: &InvestigationProjectionSource,
    metadata: InvestigationResponseMetadata,
) -> InvestigationResponse {
    let fields = requested
        .iter()
        .map(|projection| match projection {
            ReturnProjection::Assessment => InvestigationProjectedField::Assessment(project(
                source.assessment.clone().map(Box::new),
            )),
            ReturnProjection::ProofGraph => {
                InvestigationProjectedField::ProofGraph(project(source.proof_graph.clone()))
            }
            ReturnProjection::CounterEvidence => InvestigationProjectedField::CounterEvidence(
                project(source.counter_evidence.clone()),
            ),
            ReturnProjection::Unknowns => {
                InvestigationProjectedField::Unknowns(project(source.unknowns.clone()))
            }
            ReturnProjection::NextBestEvidence => InvestigationProjectedField::NextBestEvidence(
                project(source.next_best_evidence.clone()),
            ),
        })
        .collect();

    InvestigationResponse {
        fields,
        provenance: source.provenance.clone(),
        metadata,
    }
}

fn project<T>(value: Option<T>) -> InvestigationProjectionValue<T> {
    value.map_or(
        InvestigationProjectionValue::Unavailable {
            reason: InvestigationProjectionUnavailableReason::NotProduced,
        },
        InvestigationProjectionValue::Available,
    )
}

fn unavailable<T>(value: &InvestigationProjectionValue<T>) -> bool {
    matches!(value, InvestigationProjectionValue::Unavailable { .. })
}

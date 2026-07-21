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
//! Calibrated active-investigation assessment output (Epic 0020).
//!
//! The envelope composes existing proof-carrying retrieval references,
//! calibrated uncertainty signals, information-gain estimates, and ranked
//! Next Best Evidence proposals. It does not duplicate provenance or scoring
//! models and does not execute any proposed action.

use serde::{Deserialize, Serialize};

use crate::{
    Confidence, EvidenceSubgraph, GraphError, InformationGainEstimate, NextBestEvidenceRanking,
    RetrievalCompleteness, SourceProvenanceRef, UnresolvedUnknown,
    stop_condition::InvestigationStopCondition,
};

/// Auditable answer state for one active investigation question.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "CalibratedAssessmentWire")]
pub struct CalibratedAssessment {
    question: String,
    current_confidence: Confidence,
    supporting_evidence: EvidenceSubgraph,
    counter_evidence: EvidenceSubgraph,
    source_provenance: SourceProvenanceRef,
    retrieval_completeness: RetrievalCompleteness,
    unresolved_unknowns: Vec<UnresolvedUnknown>,
    expected_information_gain: InformationGainEstimate,
    next_best_evidence: NextBestEvidenceRanking,
    stop_condition: InvestigationStopCondition,
}

#[derive(Deserialize)]
struct CalibratedAssessmentWire {
    question: String,
    current_confidence: Confidence,
    supporting_evidence: EvidenceSubgraph,
    counter_evidence: EvidenceSubgraph,
    source_provenance: SourceProvenanceRef,
    retrieval_completeness: RetrievalCompleteness,
    unresolved_unknowns: Vec<UnresolvedUnknown>,
    expected_information_gain: InformationGainEstimate,
    next_best_evidence: NextBestEvidenceRanking,
    stop_condition: InvestigationStopCondition,
}

impl CalibratedAssessment {
    /// Creates a complete calibrated assessment.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::InvalidCalibratedAssessment`] when `question` is
    /// blank.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        question: impl Into<String>,
        current_confidence: Confidence,
        supporting_evidence: EvidenceSubgraph,
        counter_evidence: EvidenceSubgraph,
        source_provenance: SourceProvenanceRef,
        retrieval_completeness: RetrievalCompleteness,
        unresolved_unknowns: Vec<UnresolvedUnknown>,
        expected_information_gain: InformationGainEstimate,
        next_best_evidence: NextBestEvidenceRanking,
        stop_condition: InvestigationStopCondition,
    ) -> Result<Self, GraphError> {
        let question = question.into();
        if question.trim().is_empty() {
            return Err(GraphError::InvalidCalibratedAssessment(
                "investigation question must not be blank".to_owned(),
            ));
        }

        Ok(Self {
            question,
            current_confidence,
            supporting_evidence,
            counter_evidence,
            source_provenance,
            retrieval_completeness,
            unresolved_unknowns,
            expected_information_gain,
            next_best_evidence,
            stop_condition,
        })
    }

    /// Returns the active investigation question.
    #[must_use]
    pub fn question(&self) -> &str {
        &self.question
    }

    /// Returns the current answer confidence.
    #[must_use]
    pub const fn current_confidence(&self) -> Confidence {
        self.current_confidence
    }

    /// Returns the proof-carrying supporting record references.
    #[must_use]
    pub const fn supporting_evidence(&self) -> &EvidenceSubgraph {
        &self.supporting_evidence
    }

    /// Returns the proof-carrying counter-evidence record references.
    #[must_use]
    pub const fn counter_evidence(&self) -> &EvidenceSubgraph {
        &self.counter_evidence
    }

    /// Returns the shared retrieval and source provenance references.
    #[must_use]
    pub const fn source_provenance(&self) -> &SourceProvenanceRef {
        &self.source_provenance
    }

    /// Returns retrieval completeness independently of confidence.
    #[must_use]
    pub const fn retrieval_completeness(&self) -> RetrievalCompleteness {
        self.retrieval_completeness
    }

    /// Returns the unresolved epistemic unknowns.
    #[must_use]
    pub fn unresolved_unknowns(&self) -> &[UnresolvedUnknown] {
        &self.unresolved_unknowns
    }

    /// Returns the expected information-gain estimate for the assessment.
    #[must_use]
    pub const fn expected_information_gain(&self) -> InformationGainEstimate {
        self.expected_information_gain
    }

    /// Returns ranked and fully explained Next Best Evidence proposals.
    #[must_use]
    pub const fn next_best_evidence(&self) -> &NextBestEvidenceRanking {
        &self.next_best_evidence
    }

    /// Returns the typed stop-condition audit decision and thresholds.
    #[must_use]
    pub const fn stop_condition(&self) -> &InvestigationStopCondition {
        &self.stop_condition
    }
}

impl TryFrom<CalibratedAssessmentWire> for CalibratedAssessment {
    type Error = GraphError;

    fn try_from(wire: CalibratedAssessmentWire) -> Result<Self, Self::Error> {
        Self::new(
            wire.question,
            wire.current_confidence,
            wire.supporting_evidence,
            wire.counter_evidence,
            wire.source_provenance,
            wire.retrieval_completeness,
            wire.unresolved_unknowns,
            wire.expected_information_gain,
            wire.next_best_evidence,
            wire.stop_condition,
        )
    }
}

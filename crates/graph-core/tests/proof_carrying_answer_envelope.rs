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
use graph_core::{
    AnswerStatement, ClaimId, Confidence, EvidenceId, EvidenceSubgraph, GraphError, NodeId,
    ProofCarryingAnswer, RelationshipId, RequestId, RetrievalCompleteness, SourceProvenanceRef,
    UnresolvedUnknown,
};

fn node_id(value: &str) -> NodeId {
    NodeId::new(value).expect("envelope node ID should be valid")
}

fn relationship_id(value: &str) -> RelationshipId {
    RelationshipId::new(value).expect("envelope relationship ID should be valid")
}

fn claim_id(value: &str) -> ClaimId {
    ClaimId::new(value).expect("envelope claim ID should be valid")
}

fn evidence_id(value: &str) -> EvidenceId {
    EvidenceId::new(value).expect("envelope evidence ID should be valid")
}

fn retrieval_id(value: &str) -> RequestId {
    RequestId::new(value).expect("envelope retrieval ID should be valid")
}

fn confidence(value: f64) -> Confidence {
    Confidence::new(value).expect("envelope confidence should be valid")
}

fn completeness(value: f64) -> RetrievalCompleteness {
    RetrievalCompleteness::new(value).expect("envelope completeness should be valid")
}

fn supporting_subgraph() -> EvidenceSubgraph {
    EvidenceSubgraph {
        node_ids: vec![node_id("node--campaign"), node_id("node--narrative")],
        relationship_ids: vec![relationship_id("relationship--promotes")],
        claim_ids: vec![claim_id("claim--attribution")],
        evidence_ids: vec![evidence_id("evidence--shared-infrastructure")],
    }
}

fn envelope() -> ProofCarryingAnswer {
    ProofCarryingAnswer {
        answer: AnswerStatement {
            text: "Actor A likely operates Campaign B".to_owned(),
            primary_claim_id: Some(claim_id("claim--attribution")),
        },
        supporting_subgraph: supporting_subgraph(),
        counter_evidence: EvidenceSubgraph {
            node_ids: Vec::new(),
            relationship_ids: Vec::new(),
            claim_ids: Vec::new(),
            evidence_ids: vec![evidence_id("evidence--commercial-hosting")],
        },
        source_provenance: SourceProvenanceRef {
            retrieval_ids: vec![retrieval_id("request--attribution-1")],
            source_refs: vec!["source--vendor-report".to_owned()],
        },
        confidence: confidence(0.68),
        retrieval_completeness: completeness(0.8),
        unresolved_unknowns: vec![UnresolvedUnknown::MissingEvidence {
            claim_id: claim_id("claim--attribution"),
        }],
    }
}

//
// Verify that the envelope carries the epic's seven components as one typed
// value: answer, supporting subgraph, counter-evidence, provenance,
// confidence, completeness, and unknowns.
//
// Given a fully populated envelope,
// when each component is read,
// then all seven should be present with their typed content.
#[test]
fn envelope_carries_all_seven_components() {
    let answer = envelope();

    assert_eq!(answer.answer.text, "Actor A likely operates Campaign B");
    assert_eq!(
        answer.answer.primary_claim_id,
        Some(claim_id("claim--attribution"))
    );
    assert!(!answer.supporting_subgraph.is_empty());
    assert!(!answer.counter_evidence.is_empty());
    assert_eq!(
        answer.source_provenance.retrieval_ids,
        vec![retrieval_id("request--attribution-1")]
    );
    assert_eq!(answer.confidence.value(), 0.68);
    assert_eq!(answer.retrieval_completeness.value(), 0.8);
    assert_eq!(answer.unresolved_unknowns.len(), 1);
}

//
// Verify that the completeness carrier validates its range as a typed error,
// mirroring the confidence primitive: retrieval-state uncertainty must be a
// bounded ratio.
//
// Given completeness values outside [0, 1] or NaN,
// when the carrier is constructed,
// then construction should fail with `GraphError::InvalidRetrievalCompleteness`.
#[test]
fn completeness_carrier_validates_its_range() {
    for invalid in [-0.1, 1.1, f64::NAN] {
        let error = RetrievalCompleteness::new(invalid)
            .expect_err("out-of-range completeness should return a typed error");
        assert!(matches!(error, GraphError::InvalidRetrievalCompleteness(_)));
    }

    assert!(RetrievalCompleteness::new(0.0).is_ok());
    assert!(RetrievalCompleteness::new(1.0).is_ok());
}

//
// Verify that answer uncertainty and retrieval-state uncertainty are
// independent: the epic's confident-but-incomplete case must be representable.
//
// Given an envelope with high confidence and low completeness,
// when both signals are read,
// then each should keep its own value without influencing the other.
#[test]
fn confidence_and_completeness_are_independent_signals() {
    let mut answer = envelope();
    answer.confidence = confidence(0.95);
    answer.retrieval_completeness = completeness(0.25);

    assert_eq!(answer.confidence.value(), 0.95);
    assert_eq!(answer.retrieval_completeness.value(), 0.25);
}

//
// Verify that supporting and counter-evidence are typed graph record
// references, never prose: nodes, relationships, claims, and evidence records
// by identifier.
//
// Given an empty and a populated evidence subgraph,
// when emptiness and contents are inspected,
// then the typed identifier collections should drive both.
#[test]
fn evidence_subgraphs_are_typed_record_references() {
    let empty = EvidenceSubgraph {
        node_ids: Vec::new(),
        relationship_ids: Vec::new(),
        claim_ids: Vec::new(),
        evidence_ids: Vec::new(),
    };
    assert!(empty.is_empty());

    let populated = supporting_subgraph();
    assert!(!populated.is_empty());
    assert_eq!(populated.node_ids.len(), 2);
    assert_eq!(populated.relationship_ids.len(), 1);
    assert_eq!(populated.claim_ids.len(), 1);
    assert_eq!(populated.evidence_ids.len(), 1);
}

//
// Verify that unresolved unknowns are typed open questions covering the
// epic's three cases: missing evidence, unresolved contradictions, and
// unexpanded frontiers.
//
// Given one unknown of each variant,
// when they are matched,
// then each should expose its typed context.
#[test]
fn unknowns_are_typed_open_questions() {
    let unknowns = [
        UnresolvedUnknown::MissingEvidence {
            claim_id: claim_id("claim--needs-evidence"),
        },
        UnresolvedUnknown::UnresolvedContradiction {
            claim_id: claim_id("claim--disputed"),
            contradicting_claim_id: claim_id("claim--counter"),
        },
        UnresolvedUnknown::UnexpandedFrontier {
            node_id: node_id("node--never-expanded"),
        },
    ];

    assert!(matches!(
        &unknowns[0],
        UnresolvedUnknown::MissingEvidence { claim_id } if claim_id.as_str() == "claim--needs-evidence"
    ));
    assert!(matches!(
        &unknowns[1],
        UnresolvedUnknown::UnresolvedContradiction { claim_id, contradicting_claim_id }
            if claim_id.as_str() == "claim--disputed"
                && contradicting_claim_id.as_str() == "claim--counter"
    ));
    assert!(matches!(
        &unknowns[2],
        UnresolvedUnknown::UnexpandedFrontier { node_id } if node_id.as_str() == "node--never-expanded"
    ));
}

//
// Verify that the provenance reference links the envelope to the recorded
// retrievals whose telemetry holds the full trajectory (captured by the
// dedicated provenance issue), plus the cited source references.
//
// Given a provenance reference with retrievals and sources,
// when its collections are read,
// then both should be preserved in order.
#[test]
fn provenance_reference_points_at_recorded_retrievals() {
    let provenance = SourceProvenanceRef {
        retrieval_ids: vec![
            retrieval_id("request--attribution-1"),
            retrieval_id("request--attribution-2"),
        ],
        source_refs: vec!["source--vendor-report".to_owned()],
    };

    assert_eq!(provenance.retrieval_ids.len(), 2);
    assert_eq!(
        provenance.source_refs,
        vec!["source--vendor-report".to_owned()]
    );
}

//
// Verify reproducibility: identically built envelopes are equal, so proof
// payloads can be compared and replayed deterministically.
//
// Given two envelopes built by the same construction,
// when they are compared,
// then they should be exactly equal.
#[test]
fn identically_built_envelopes_are_equal() {
    assert_eq!(envelope(), envelope());
}

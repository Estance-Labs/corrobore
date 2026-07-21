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
    ClaimAnalyticalTarget, ClaimId, ClaimInput, ClaimStatement, ClaimStatus, ClaimStore,
    ClaimTarget, EvidenceId, GraphError,
};

fn claim_id(value: &str) -> ClaimId {
    ClaimId::new(value).expect("test claim ID should be valid")
}

fn evidence_id(value: &str) -> EvidenceId {
    EvidenceId::new(value).expect("test evidence ID should be valid")
}

fn create_asserted_claim_with_evidence(
    store: &mut ClaimStore,
    id: &ClaimId,
    statement: &str,
    evidence: EvidenceId,
) {
    let input = ClaimInput::new(
        id.clone(),
        ClaimStatement::new(statement).expect("statement should be valid"),
        ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new(statement, None)),
    )
    .with_evidence_ref(evidence);

    store
        .create_asserted_claim(input)
        .expect("asserted claim creation should succeed");
}

//
// Verify a claim can be retracted with explicit reason metadata, remains
// readable, and preserves prior evidence links.
#[test]
fn retract_claim_requires_reason_and_preserves_claim_history_and_evidence() {
    let mut store = ClaimStore::new();
    let claim = claim_id("claim--retract-1");
    let evidence = evidence_id("evidence--retract-1");

    create_asserted_claim_with_evidence(
        &mut store,
        &claim,
        "Claim that will be retracted",
        evidence.clone(),
    );

    let retracted = store
        .retract_claim(
            claim.clone(),
            "Source was determined to be fabricated".to_owned(),
            Some("actor://analyst/1".to_owned()),
            Some("session://triage/42".to_owned()),
        )
        .expect("retraction with reason should succeed");

    assert_eq!(retracted.status(), ClaimStatus::Retracted);
    assert_eq!(retracted.evidence_refs(), &[evidence]);
    assert_eq!(retracted.version(), 2);

    let read_back = store
        .claim_by_id(&claim)
        .expect("retracted claim should remain readable");
    assert_eq!(read_back.status(), ClaimStatus::Retracted);

    let reasons = store
        .claim_decisions_for_claim(&claim)
        .expect("decision metadata should be readable for retracted claim");
    assert_eq!(reasons.len(), 1);
    assert_eq!(
        reasons[0].reason(),
        "Source was determined to be fabricated"
    );
}

//
// Verify a claim can be rejected with explicit reason metadata, remains
// readable, and preserves prior evidence links.
#[test]
fn reject_claim_requires_reason_and_preserves_claim_history_and_evidence() {
    let mut store = ClaimStore::new();
    let claim = claim_id("claim--reject-1");
    let evidence = evidence_id("evidence--reject-1");

    create_asserted_claim_with_evidence(
        &mut store,
        &claim,
        "Claim that will be rejected",
        evidence.clone(),
    );

    let rejected = store
        .reject_claim(
            claim.clone(),
            "Independent validation disproved this claim".to_owned(),
            Some("actor://reviewer/7".to_owned()),
            Some("session://review/13".to_owned()),
        )
        .expect("rejection with reason should succeed");

    assert_eq!(rejected.status(), ClaimStatus::Rejected);
    assert_eq!(rejected.evidence_refs(), &[evidence]);
    assert_eq!(rejected.version(), 2);

    let read_back = store
        .claim_by_id(&claim)
        .expect("rejected claim should remain readable");
    assert_eq!(read_back.status(), ClaimStatus::Rejected);

    let reasons = store
        .claim_decisions_for_claim(&claim)
        .expect("decision metadata should be readable for rejected claim");
    assert_eq!(reasons.len(), 1);
    assert_eq!(
        reasons[0].reason(),
        "Independent validation disproved this claim"
    );
}

//
// Verify missing reason metadata is rejected with explicit typed errors for both
// retraction and rejection workflows.
#[test]
fn retract_and_reject_require_non_empty_reason_metadata() {
    let mut store = ClaimStore::new();
    let claim = claim_id("claim--reason-required");
    create_asserted_claim_with_evidence(
        &mut store,
        &claim,
        "Claim used for missing-reason checks",
        evidence_id("evidence--reason-required"),
    );

    let retract_error = store
        .retract_claim(claim.clone(), " ".to_owned(), None, None)
        .expect_err("retraction should require a non-empty reason");
    assert!(matches!(retract_error, GraphError::MissingRetractionReason));

    let reject_error = store
        .reject_claim(claim, "".to_owned(), None, None)
        .expect_err("rejection should require a non-empty reason");
    assert!(matches!(reject_error, GraphError::MissingRejectionReason));
}

//
// Verify invalid lifecycle transitions are still rejected for retraction and
// rejection operations.
#[test]
fn retract_and_reject_reuse_typed_invalid_transition_errors() {
    let mut store = ClaimStore::new();
    let claim = claim_id("claim--invalid-transition");
    create_asserted_claim_with_evidence(
        &mut store,
        &claim,
        "Claim used for invalid transition checks",
        evidence_id("evidence--invalid-transition"),
    );

    store
        .reject_claim(
            claim.clone(),
            "Initial rejection reason".to_owned(),
            None,
            None,
        )
        .expect("initial rejection should succeed");

    let error = store
        .retract_claim(
            claim,
            "Cannot retract rejected claim".to_owned(),
            None,
            None,
        )
        .expect_err("invalid transition should be rejected");

    assert!(matches!(
        error,
        GraphError::InvalidClaimStatusTransition {
            from: ClaimStatus::Rejected,
            to: ClaimStatus::Retracted
        }
    ));
}

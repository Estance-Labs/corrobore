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
    ClaimAnalyticalTarget, ClaimId, ClaimInput, ClaimLinkKind, ClaimLinkSource, ClaimStatement,
    ClaimStatus, ClaimStore, ClaimTarget, EvidenceId, GraphError,
};

fn claim_id(value: &str) -> ClaimId {
    ClaimId::new(value).expect("test claim ID should be valid")
}

fn evidence_id(value: &str) -> EvidenceId {
    EvidenceId::new(value).expect("test evidence ID should be valid")
}

fn create_asserted_claim(store: &mut ClaimStore, id: &ClaimId, statement: &str) {
    let input = ClaimInput::new(
        id.clone(),
        ClaimStatement::new(statement).expect("statement should be valid"),
        ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new(statement, None)),
    );

    store
        .create_asserted_claim(input)
        .expect("asserted claim creation should succeed");
}

//
// Verify that evidence can explicitly support a claim while preserving typed
// link semantics without mutating lifecycle status.
#[test]
fn attaching_supporting_evidence_creates_support_link_without_validating_claim() {
    let mut store = ClaimStore::new();
    let target_claim = claim_id("claim--target-1");
    let evidence = evidence_id("evidence--support-1");

    create_asserted_claim(
        &mut store,
        &target_claim,
        "Target claim with explicit supporting evidence",
    );
    store.register_evidence(evidence.clone());

    let link = store
        .attach_supporting_evidence_to_claim(evidence.clone(), target_claim.clone())
        .expect("supporting evidence link should be accepted");

    assert_eq!(link.kind(), ClaimLinkKind::Supports);
    assert_eq!(link.target_claim_id(), &target_claim);
    assert!(matches!(
    link.source(),
    ClaimLinkSource::Evidence(source) if source == &evidence
    ));

    let claim = store
        .claim_by_id(&target_claim)
        .expect("target claim should still exist");
    assert_eq!(claim.status(), ClaimStatus::Asserted);
}

//
// Verify that evidence can explicitly refute a claim while preserving typed
// link semantics without forcing immediate rejection.
#[test]
fn attaching_refuting_evidence_creates_refutation_link_without_rejecting_claim() {
    let mut store = ClaimStore::new();
    let target_claim = claim_id("claim--target-2");
    let evidence = evidence_id("evidence--refute-1");

    create_asserted_claim(
        &mut store,
        &target_claim,
        "Target claim with explicit refuting evidence",
    );
    store.register_evidence(evidence.clone());

    let link = store
        .attach_refuting_evidence_to_claim(evidence.clone(), target_claim.clone())
        .expect("refuting evidence link should be accepted");

    assert_eq!(link.kind(), ClaimLinkKind::Refutes);
    assert_eq!(link.target_claim_id(), &target_claim);
    assert!(matches!(
    link.source(),
    ClaimLinkSource::Evidence(source) if source == &evidence
    ));

    let claim = store
        .claim_by_id(&target_claim)
        .expect("target claim should still exist");
    assert_eq!(claim.status(), ClaimStatus::Asserted);
}

//
// Verify that one claim can explicitly support another claim using a typed
// claim-to-claim support link.
#[test]
fn attaching_supporting_claim_creates_claim_to_claim_support_link() {
    let mut store = ClaimStore::new();
    let source_claim = claim_id("claim--supports-source");
    let target_claim = claim_id("claim--supports-target");

    create_asserted_claim(&mut store, &source_claim, "Source support claim");
    create_asserted_claim(&mut store, &target_claim, "Target supported claim");

    let link = store
        .attach_supporting_claim_to_claim(source_claim.clone(), target_claim.clone())
        .expect("supporting claim link should be accepted");

    assert_eq!(link.kind(), ClaimLinkKind::Supports);
    assert_eq!(link.target_claim_id(), &target_claim);
    assert!(matches!(
    link.source(),
    ClaimLinkSource::Claim(source) if source == &source_claim
    ));
}

//
// Verify that one claim can explicitly refute another claim using a typed
// claim-to-claim refutation link.
#[test]
fn attaching_refuting_claim_creates_claim_to_claim_refutation_link() {
    let mut store = ClaimStore::new();
    let source_claim = claim_id("claim--refutes-source");
    let target_claim = claim_id("claim--refutes-target");

    create_asserted_claim(&mut store, &source_claim, "Source refutation claim");
    create_asserted_claim(&mut store, &target_claim, "Target refuted claim");

    let link = store
        .attach_refuting_claim_to_claim(source_claim.clone(), target_claim.clone())
        .expect("refuting claim link should be accepted");

    assert_eq!(link.kind(), ClaimLinkKind::Refutes);
    assert_eq!(link.target_claim_id(), &target_claim);
    assert!(matches!(
    link.source(),
    ClaimLinkSource::Claim(source) if source == &source_claim
    ));
}

//
// Verify that missing evidence references produce an explicit typed error.
#[test]
fn attaching_evidence_with_unknown_evidence_id_returns_evidence_not_found() {
    let mut store = ClaimStore::new();
    let target_claim = claim_id("claim--target-unknown-evidence");
    let missing_evidence = evidence_id("evidence--missing");

    create_asserted_claim(
        &mut store,
        &target_claim,
        "Target claim for missing evidence validation",
    );

    let error = store
        .attach_supporting_evidence_to_claim(missing_evidence.clone(), target_claim)
        .expect_err("missing evidence should fail with explicit error");

    assert!(matches!(
    error,
    GraphError::EvidenceNotFound(id) if id == missing_evidence
    ));
}

//
// Verify that missing claim references produce explicit typed claim-not-found
// errors for claim-to-claim linking.
#[test]
fn attaching_claim_link_with_unknown_claim_returns_claim_not_found() {
    let mut store = ClaimStore::new();
    let source_claim = claim_id("claim--known-source");
    let missing_target = claim_id("claim--missing-target");

    create_asserted_claim(&mut store, &source_claim, "Known source claim");

    let error = store
        .attach_supporting_claim_to_claim(source_claim, missing_target.clone())
        .expect_err("missing target claim should fail with explicit error");

    assert!(matches!(error, GraphError::ClaimNotFound(id) if id == missing_target));
}

//
// Verify invalid claim links are rejected with a dedicated typed error instead
// of being silently accepted.
#[test]
fn attaching_claim_link_rejects_self_link_as_invalid_link() {
    let mut store = ClaimStore::new();
    let claim = claim_id("claim--self-link");
    create_asserted_claim(&mut store, &claim, "Claim that cannot self-link");

    let error = store
        .attach_refuting_claim_to_claim(claim.clone(), claim)
        .expect_err("self-refuting link should be rejected");

    assert!(
        matches!(error, GraphError::InvalidClaimLink(message) if message.contains("self-link"))
    );
}

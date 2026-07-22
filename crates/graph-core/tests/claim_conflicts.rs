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
    ClaimStatus, ClaimStore, ClaimTarget, GraphError,
};

fn claim_id(value: &str) -> ClaimId {
    ClaimId::new(value).expect("test claim ID should be valid")
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
// Verify that one claim can explicitly contradict another while preserving both
// claims and treating contradiction as conflict context, not final resolution.
#[test]
fn attaching_contradiction_link_preserves_both_claims_without_forcing_rejection() {
    let mut store = ClaimStore::new();
    let source_claim = claim_id("claim--contradicts-source");
    let target_claim = claim_id("claim--contradicts-target");

    create_asserted_claim(&mut store, &source_claim, "Source contradiction claim");
    create_asserted_claim(&mut store, &target_claim, "Target contradicted claim");

    let link = store
        .attach_contradicting_claim_to_claim(
            source_claim.clone(),
            target_claim.clone(),
            Some("analysis://conflicts/42".to_owned()),
        )
        .expect("contradiction link should be accepted");

    assert_eq!(link.kind(), ClaimLinkKind::Contradicts);
    assert_eq!(link.target_claim_id(), &target_claim);
    assert_eq!(link.explanation_ref(), Some("analysis://conflicts/42"));
    assert_eq!(link.source(), &ClaimLinkSource::Claim(source_claim.clone()));

    let source = store
        .claim_by_id(&source_claim)
        .expect("source claim should remain readable");
    let target = store
        .claim_by_id(&target_claim)
        .expect("target claim should remain readable");
    assert_eq!(source.status(), ClaimStatus::Asserted);
    assert_eq!(target.status(), ClaimStatus::Asserted);
}

//
// Verify that one claim can supersede another while preserving historical
// records for both older and newer claim versions.
#[test]
fn attaching_supersession_link_preserves_older_and_newer_claims() {
    let mut store = ClaimStore::new();
    let newer_claim = claim_id("claim--supersedes-source");
    let older_claim = claim_id("claim--supersedes-target");

    create_asserted_claim(&mut store, &newer_claim, "Newer superseding claim");
    create_asserted_claim(&mut store, &older_claim, "Older superseded claim");

    let link = store
        .attach_superseding_claim_to_claim(newer_claim.clone(), older_claim.clone(), None)
        .expect("supersession link should be accepted");

    assert_eq!(link.kind(), ClaimLinkKind::Supersedes);
    assert_eq!(link.target_claim_id(), &older_claim);
    assert_eq!(link.explanation_ref(), None);
    assert_eq!(link.source(), &ClaimLinkSource::Claim(newer_claim.clone()));

    let newer = store
        .claim_by_id(&newer_claim)
        .expect("newer claim should remain readable");
    let older = store
        .claim_by_id(&older_claim)
        .expect("older claim should remain readable");
    assert_eq!(newer.status(), ClaimStatus::Asserted);
    assert_eq!(older.status(), ClaimStatus::Asserted);
}

//
// Verify explicit self-contradiction errors are returned and not collapsed into
// generic missing-claim or string-only failures.
#[test]
fn attach_contradiction_rejects_self_contradiction() {
    let mut store = ClaimStore::new();
    let claim = claim_id("claim--self-contradiction");
    create_asserted_claim(&mut store, &claim, "Claim that cannot contradict itself");

    let error = store
        .attach_contradicting_claim_to_claim(claim.clone(), claim.clone(), None)
        .expect_err("self-contradiction must be rejected");

    assert!(matches!(
    error,
    GraphError::SelfContradictionNotAllowed(id) if id == claim
    ));
}

//
// Verify explicit self-supersession errors are returned and not collapsed into
// generic missing-claim or string-only failures.
#[test]
fn attach_supersession_rejects_self_supersession() {
    let mut store = ClaimStore::new();
    let claim = claim_id("claim--self-supersession");
    create_asserted_claim(&mut store, &claim, "Claim that cannot supersede itself");

    let error = store
        .attach_superseding_claim_to_claim(claim.clone(), claim.clone(), None)
        .expect_err("self-supersession must be rejected");

    assert!(matches!(
    error,
    GraphError::SelfSupersessionNotAllowed(id) if id == claim
    ));
}

//
// Verify missing source or target claim references produce explicit typed
// claim-not-found errors for contradiction/supersession linking.
#[test]
fn contradiction_and_supersession_reject_missing_claim_references() {
    let mut store = ClaimStore::new();
    let known = claim_id("claim--known-for-missing");
    let missing_source = claim_id("claim--missing-source");
    let missing_target = claim_id("claim--missing-target");

    create_asserted_claim(&mut store, &known, "Known claim for missing checks");

    let missing_source_error = store
        .attach_contradicting_claim_to_claim(missing_source.clone(), known.clone(), None)
        .expect_err("missing source should be rejected");
    assert!(matches!(
    missing_source_error,
    GraphError::ClaimNotFound(id) if id == missing_source
    ));

    let missing_target_error = store
        .attach_superseding_claim_to_claim(known, missing_target.clone(), None)
        .expect_err("missing target should be rejected");
    assert!(matches!(
    missing_target_error,
    GraphError::ClaimNotFound(id) if id == missing_target
    ));
}

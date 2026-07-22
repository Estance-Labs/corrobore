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
    ClaimAnalyticalTarget, ClaimId, ClaimInput, ClaimLink, ClaimLinkKind, ClaimLinkSource,
    ClaimStatement, ClaimStore, ClaimTarget, EpistemicExplanationKind, GraphError, WorkspaceId,
};

fn claim_id(value: &str) -> ClaimId {
    ClaimId::new(value).expect("test claim ID should be valid")
}

fn workspace_id(value: &str) -> WorkspaceId {
    WorkspaceId::new(value).expect("test workspace ID should be valid")
}

fn create_asserted_claim_with_workspace(
    store: &mut ClaimStore,
    id: &ClaimId,
    statement: &str,
    workspace: WorkspaceId,
) {
    let input = ClaimInput::new(
        id.clone(),
        ClaimStatement::new(statement).expect("statement should be valid"),
        ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new(statement, None)),
    )
    .with_workspace_id(workspace);

    store
        .create_asserted_claim(input)
        .expect("asserted claim creation should succeed");
}

//
// Verify claim lifecycle updates carry explanation metadata including actor,
// session, workspace, and reason references.
#[test]
fn claim_state_changes_carry_explanation_metadata() {
    let mut store = ClaimStore::new();
    let claim = claim_id("claim--explain-lifecycle");
    let ws = workspace_id("workspace--audit-1");
    create_asserted_claim_with_workspace(
        &mut store,
        &claim,
        "Claim with lifecycle explanation metadata",
        ws,
    );

    store
        .retract_claim(
            claim.clone(),
            "reason://evidence-invalidated".to_owned(),
            Some("actor://analyst/42".to_owned()),
            Some("session://triage/7".to_owned()),
        )
        .expect("retraction should succeed");

    let explanations = store
        .explain_claim(&claim)
        .expect("claim explanation lookup should succeed");

    assert!(explanations.iter().any(|entry| {
        entry.kind() == EpistemicExplanationKind::Retraction
            && entry.actor_ref() == Some("actor://analyst/42")
            && entry.session_ref() == Some("session://triage/7")
            && entry.workspace_ref() == Some("workspace--audit-1")
            && entry.reason_ref() == Some("reason://evidence-invalidated")
    }));
}

//
// Verify support and refutation links can be explained with explicit entry kinds.
#[test]
fn support_and_refutation_links_have_explanation_entries() {
    let mut store = ClaimStore::new();
    let target = claim_id("claim--target-links");
    let support = claim_id("claim--support-links");
    let refute = claim_id("claim--refute-links");

    let ws = workspace_id("workspace--links-1");
    create_asserted_claim_with_workspace(&mut store, &target, "Target claim", ws.clone());
    create_asserted_claim_with_workspace(&mut store, &support, "Support claim", ws.clone());
    create_asserted_claim_with_workspace(&mut store, &refute, "Refute claim", ws);

    let support_link = store
        .attach_supporting_claim_to_claim(support, target.clone())
        .expect("support link should attach");
    let refute_link = store
        .attach_refuting_claim_to_claim(refute, target)
        .expect("refute link should attach");

    let support_explanation = store
        .explain_claim_link(&support_link)
        .expect("support link explanation should exist");
    assert_eq!(
        support_explanation.kind(),
        EpistemicExplanationKind::SupportLink
    );

    let refute_explanation = store
        .explain_claim_link(&refute_link)
        .expect("refute link explanation should exist");
    assert_eq!(
        refute_explanation.kind(),
        EpistemicExplanationKind::RefutationLink
    );
}

//
// Verify contradiction and supersession links carry explanation metadata.
#[test]
fn contradiction_and_supersession_links_carry_explanation_metadata() {
    let mut store = ClaimStore::new();
    let older = claim_id("claim--older-version");
    let newer = claim_id("claim--newer-version");
    let contradictory = claim_id("claim--contradictory-version");

    let ws = workspace_id("workspace--links-2");
    create_asserted_claim_with_workspace(&mut store, &older, "Older claim", ws.clone());
    create_asserted_claim_with_workspace(&mut store, &newer, "Newer claim", ws.clone());
    create_asserted_claim_with_workspace(&mut store, &contradictory, "Contradictory claim", ws);

    let contradiction_link = store
        .attach_contradicting_claim_to_claim(
            contradictory,
            older.clone(),
            Some("reason://conflicting-observation".to_owned()),
        )
        .expect("contradiction link should attach");

    let supersession_link = store
        .attach_superseding_claim_to_claim(
            newer,
            older,
            Some("reason://higher-confidence-update".to_owned()),
        )
        .expect("supersession link should attach");

    let contradiction_explanation = store
        .explain_claim_link(&contradiction_link)
        .expect("contradiction explanation should exist");
    assert_eq!(
        contradiction_explanation.kind(),
        EpistemicExplanationKind::ContradictionLink
    );
    assert_eq!(
        contradiction_explanation.reason_ref(),
        Some("reason://conflicting-observation")
    );

    let supersession_explanation = store
        .explain_claim_link(&supersession_link)
        .expect("supersession explanation should exist");
    assert_eq!(
        supersession_explanation.kind(),
        EpistemicExplanationKind::SupersessionLink
    );
    assert_eq!(
        supersession_explanation.reason_ref(),
        Some("reason://higher-confidence-update")
    );
}

//
// Verify resolution-output explanations can preserve consumed inputs and
// metadata references.
#[test]
fn resolution_output_explanations_list_consumed_inputs() {
    let mut store = ClaimStore::new();

    store
        .record_resolution_explanation(
            "resolution://claim-42/run-1".to_owned(),
            vec![
                "claim:claim--42".to_owned(),
                "link:support:claim--42".to_owned(),
                "stance:stance--7".to_owned(),
            ],
            Some("actor://resolver/service".to_owned()),
            Some("session://resolver/run-1".to_owned()),
            Some(workspace_id("workspace--resolution-1")),
            Some("reason://conservative-policy".to_owned()),
        )
        .expect("recording resolution explanation should succeed");

    let resolution_explanation = store
        .explain_resolution_output("resolution://claim-42/run-1")
        .expect("resolution explanation should exist");

    assert_eq!(
        resolution_explanation.kind(),
        EpistemicExplanationKind::ResolutionOutput
    );
    assert_eq!(
        resolution_explanation.consumed_inputs(),
        &[
            "claim:claim--42".to_owned(),
            "link:support:claim--42".to_owned(),
            "stance:stance--7".to_owned(),
        ]
    );
}

//
// Verify missing explanation targets return explicit typed errors.
#[test]
fn missing_explanation_targets_return_typed_errors() {
    let store = ClaimStore::new();
    let missing_claim = claim_id("claim--missing-explanation");

    let missing_claim_error = store
        .explain_claim(&missing_claim)
        .expect_err("missing claim explanation should fail");
    assert!(matches!(
    missing_claim_error,
    GraphError::ClaimExplanationNotFound(id) if id == missing_claim
    ));

    let missing_link = ClaimLink::new(
        ClaimLinkSource::Claim(claim_id("claim--missing-link-source")),
        claim_id("claim--missing-link-target"),
        ClaimLinkKind::Supports,
    );
    let missing_link_error = store
        .explain_claim_link(&missing_link)
        .expect_err("missing link explanation should fail");
    assert!(matches!(
    missing_link_error,
    GraphError::ClaimLinkExplanationNotFound(key) if !key.is_empty()
    ));

    let missing_resolution_error = store
        .explain_resolution_output("resolution://missing")
        .expect_err("missing resolution explanation should fail");
    assert!(matches!(
    missing_resolution_error,
    GraphError::ResolutionExplanationNotFound(id) if id == "resolution://missing"
    ));
}

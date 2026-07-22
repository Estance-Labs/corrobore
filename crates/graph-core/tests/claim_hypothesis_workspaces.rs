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
    ActorId, AgentStanceInput, ClaimAnalyticalTarget, ClaimId, ClaimInput, ClaimStatement,
    ClaimStore, ClaimTarget, GraphError, HypothesisWorkspaceId, HypothesisWorkspaceInput,
    HypothesisWorkspaceStatus, StanceKind,
};

fn claim_id(value: &str) -> ClaimId {
    ClaimId::new(value).expect("test claim ID should be valid")
}

fn actor_id(value: &str) -> ActorId {
    ActorId::new(value).expect("test actor ID should be valid")
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
// Verify a hypothesis workspace can be represented with explicit metadata and
// does not overlap the working-set loading model.
#[test]
fn hypothesis_workspace_can_be_created_with_explicit_metadata() {
    let mut store = ClaimStore::new();
    let owner = actor_id("actor--hypothesis-owner");

    let workspace_id = store
        .create_hypothesis_workspace(
            HypothesisWorkspaceInput::new(
                HypothesisWorkspaceId::new("hypothesis-workspace--alpha")
                    .expect("workspace ID should be valid"),
                "Competing attribution hypothesis A".to_owned(),
                "Bounded analytical context for actor attribution".to_owned(),
                owner.clone(),
            )
            .with_created_at("2026-07-06T10:00:00Z".to_owned())
            .with_parent_context_ref("analysis://incident/42".to_owned()),
        )
        .expect("workspace creation should succeed");

    let workspace = store
        .hypothesis_workspace_by_id(&workspace_id)
        .expect("workspace should be readable");

    assert_eq!(workspace.id(), &workspace_id);
    assert_eq!(workspace.status(), HypothesisWorkspaceStatus::Active);
    assert_eq!(workspace.owner_actor(), &owner);
    assert_eq!(workspace.created_at(), Some("2026-07-06T10:00:00Z"));
    assert_eq!(
        workspace.parent_context_ref(),
        Some("analysis://incident/42")
    );
}

//
// Verify claims can be attached and listed by workspace, and multiple
// workspaces can coexist with independent membership.
#[test]
fn workspaces_can_coexist_with_independent_claim_membership() {
    let mut store = ClaimStore::new();
    let claim_a = claim_id("claim--workspace-a");
    let claim_b = claim_id("claim--workspace-b");

    create_asserted_claim(&mut store, &claim_a, "Claim for workspace alpha");
    create_asserted_claim(&mut store, &claim_b, "Claim for workspace beta");

    let alpha = store
        .create_hypothesis_workspace(HypothesisWorkspaceInput::new(
            HypothesisWorkspaceId::new("hypothesis-workspace--alpha")
                .expect("workspace ID should be valid"),
            "Alpha".to_owned(),
            "Alpha context".to_owned(),
            actor_id("actor--alpha"),
        ))
        .expect("alpha workspace should be created");
    let beta = store
        .create_hypothesis_workspace(HypothesisWorkspaceInput::new(
            HypothesisWorkspaceId::new("hypothesis-workspace--beta")
                .expect("workspace ID should be valid"),
            "Beta".to_owned(),
            "Beta context".to_owned(),
            actor_id("actor--beta"),
        ))
        .expect("beta workspace should be created");

    store
        .attach_claim_to_hypothesis_workspace(alpha.clone(), claim_a.clone())
        .expect("claim A should attach to alpha workspace");
    store
        .attach_claim_to_hypothesis_workspace(beta.clone(), claim_b.clone())
        .expect("claim B should attach to beta workspace");

    let alpha_claims = store
        .list_claims_in_hypothesis_workspace(&alpha)
        .expect("alpha claim listing should succeed");
    let beta_claims = store
        .list_claims_in_hypothesis_workspace(&beta)
        .expect("beta claim listing should succeed");

    assert_eq!(alpha_claims, vec![claim_a]);
    assert_eq!(beta_claims, vec![claim_b]);
}

//
// Verify workspace status transitions remain explicit and can be set to active,
// deferred, rejected, and merged-later states.
#[test]
fn workspace_status_can_be_marked_explicitly() {
    let mut store = ClaimStore::new();
    let workspace_id = store
        .create_hypothesis_workspace(HypothesisWorkspaceInput::new(
            HypothesisWorkspaceId::new("hypothesis-workspace--status")
                .expect("workspace ID should be valid"),
            "Status Workspace".to_owned(),
            "Status transitions".to_owned(),
            actor_id("actor--status"),
        ))
        .expect("workspace creation should succeed");

    let deferred = store
        .set_hypothesis_workspace_status(workspace_id.clone(), HypothesisWorkspaceStatus::Deferred)
        .expect("deferred status update should succeed");
    assert_eq!(deferred.status(), HypothesisWorkspaceStatus::Deferred);

    let rejected = store
        .set_hypothesis_workspace_status(workspace_id.clone(), HypothesisWorkspaceStatus::Rejected)
        .expect("rejected status update should succeed");
    assert_eq!(rejected.status(), HypothesisWorkspaceStatus::Rejected);

    let merged_later = store
        .set_hypothesis_workspace_status(workspace_id, HypothesisWorkspaceStatus::MergedLater)
        .expect("merged-later status update should succeed");
    assert_eq!(
        merged_later.status(),
        HypothesisWorkspaceStatus::MergedLater
    );
}

//
// Verify missing workspace lookups and claim attachments return explicit typed
// workspace-not-found errors.
#[test]
fn missing_workspace_lookup_returns_typed_error() {
    let mut store = ClaimStore::new();
    let missing = HypothesisWorkspaceId::new("hypothesis-workspace--missing")
        .expect("workspace ID should be valid");

    let lookup_error = store
        .hypothesis_workspace_by_id(&missing)
        .expect_err("missing workspace lookup should fail");
    assert!(matches!(
    lookup_error,
    GraphError::HypothesisWorkspaceNotFound(id) if id == missing
    ));

    let attach_error = store
        .attach_claim_to_hypothesis_workspace(missing.clone(), claim_id("claim--x"))
        .expect_err("missing workspace attach should fail");
    assert!(matches!(
    attach_error,
    GraphError::HypothesisWorkspaceNotFound(id) if id == missing
    ));
}

//
// Verify stance membership can be associated with a hypothesis workspace.
#[test]
fn stances_can_be_attached_to_hypothesis_workspace() {
    let mut store = ClaimStore::new();
    let claim = claim_id("claim--stance-membership");
    create_asserted_claim(&mut store, &claim, "Claim for workspace stance membership");

    let stance_id = store
        .create_agent_stance(AgentStanceInput::new(
            "agent://workspace-member".to_owned(),
            claim,
            StanceKind::Disputes,
        ))
        .expect("stance creation should succeed");

    let workspace_id = store
        .create_hypothesis_workspace(HypothesisWorkspaceInput::new(
            HypothesisWorkspaceId::new("hypothesis-workspace--stances")
                .expect("workspace ID should be valid"),
            "Workspace Stances".to_owned(),
            "Stance membership context".to_owned(),
            actor_id("actor--stance-owner"),
        ))
        .expect("workspace creation should succeed");

    store
        .attach_stance_to_hypothesis_workspace(workspace_id.clone(), stance_id.clone())
        .expect("stance should attach to hypothesis workspace");

    let stance_members = store
        .list_stances_in_hypothesis_workspace(&workspace_id)
        .expect("workspace stance list should succeed");

    assert_eq!(stance_members, vec![stance_id]);
}

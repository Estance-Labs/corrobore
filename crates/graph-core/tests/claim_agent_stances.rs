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
    AgentStanceInput, AgentStancePatch, ClaimAnalyticalTarget, ClaimId, ClaimInput, ClaimStatement,
    ClaimStatus, ClaimStore, ClaimTarget, GraphError, StanceKind, WorkspaceId,
};

fn claim_id(value: &str) -> ClaimId {
    ClaimId::new(value).expect("test claim ID should be valid")
}

fn workspace_id(value: &str) -> WorkspaceId {
    WorkspaceId::new(value).expect("test workspace ID should be valid")
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
// Verify stance can be represented independently from global claim status.
#[test]
fn agent_stance_is_independent_from_global_claim_status() {
    let mut store = ClaimStore::new();
    let claim = claim_id("claim--stance-independent");
    create_asserted_claim(&mut store, &claim, "Claim for stance independence");

    let stance_id = store
        .create_agent_stance(AgentStanceInput::new(
            "agent://alpha".to_owned(),
            claim.clone(),
            StanceKind::Supports,
        ))
        .expect("stance creation should succeed");

    let stance = store
        .stance_by_id(&stance_id)
        .expect("created stance should be readable by id");
    let claim_record = store
        .claim_by_id(&claim)
        .expect("claim should remain readable");

    assert_eq!(stance.stance(), StanceKind::Supports);
    assert_eq!(claim_record.status(), ClaimStatus::Asserted);
}

//
// Verify multiple agents can hold different stances on the same claim.
#[test]
fn multiple_agents_can_hold_different_stances_for_same_claim() {
    let mut store = ClaimStore::new();
    let claim = claim_id("claim--multi-agent-stance");
    create_asserted_claim(&mut store, &claim, "Claim with divergent agent stances");

    store
        .create_agent_stance(AgentStanceInput::new(
            "agent://alpha".to_owned(),
            claim.clone(),
            StanceKind::Supports,
        ))
        .expect("first stance creation should succeed");

    store
        .create_agent_stance(AgentStanceInput::new(
            "agent://bravo".to_owned(),
            claim.clone(),
            StanceKind::Refutes,
        ))
        .expect("second stance creation should succeed");

    let stances = store
        .stances_by_claim(&claim)
        .expect("stances should be readable by claim");

    assert_eq!(stances.len(), 2);
    assert!(
        stances
            .iter()
            .any(|s| s.agent_ref() == "agent://alpha" && s.stance() == StanceKind::Supports)
    );
    assert!(
        stances
            .iter()
            .any(|s| s.agent_ref() == "agent://bravo" && s.stance() == StanceKind::Refutes)
    );
}

//
// Verify stance records support optional workspace scoping and preserve
// confidence + reason references.
#[test]
fn stance_supports_workspace_scope_confidence_and_reason_references() {
    let mut store = ClaimStore::new();
    let claim = claim_id("claim--workspace-stance");
    create_asserted_claim(&mut store, &claim, "Claim scoped by workspace stance");

    let ws = workspace_id("workspace--case-77");
    let stance_id = store
        .create_agent_stance(
            AgentStanceInput::new(
                "agent://analyst/9".to_owned(),
                claim.clone(),
                StanceKind::Disputes,
            )
            .with_workspace_id(ws.clone())
            .with_confidence(0.61)
            .with_reason_ref("analysis://reason/99".to_owned()),
        )
        .expect("workspace-scoped stance creation should succeed");

    let stance = store
        .stance_by_id(&stance_id)
        .expect("workspace-scoped stance should be readable");

    assert_eq!(stance.workspace_id(), Some(&ws));
    assert_eq!(stance.confidence(), Some(0.61));
    assert_eq!(stance.reason_refs(), &["analysis://reason/99".to_owned()]);
}

//
// Verify stance updates do not erase claim history or mutate claim status.
#[test]
fn updating_stance_does_not_modify_claim_record() {
    let mut store = ClaimStore::new();
    let claim = claim_id("claim--stance-update");
    create_asserted_claim(&mut store, &claim, "Claim with mutable stance");

    let initial_claim_version = store
        .claim_by_id(&claim)
        .expect("claim should exist")
        .version();

    let stance_id = store
        .create_agent_stance(AgentStanceInput::new(
            "agent://delta".to_owned(),
            claim.clone(),
            StanceKind::WithholdsJudgment,
        ))
        .expect("initial stance creation should succeed");

    let updated = store
        .update_agent_stance(
            &stance_id,
            AgentStancePatch::new(StanceKind::Accepts)
                .with_confidence(0.88)
                .with_reason_ref("analysis://reason/accept-1".to_owned()),
        )
        .expect("stance update should succeed");

    let claim_after = store
        .claim_by_id(&claim)
        .expect("claim should still exist after stance update");

    assert_eq!(updated.stance(), StanceKind::Accepts);
    assert_eq!(claim_after.status(), ClaimStatus::Asserted);
    assert_eq!(claim_after.version(), initial_claim_version);
}

//
// Verify explicit missing claim and missing stance errors are exposed.
#[test]
fn missing_claim_or_stance_lookups_return_explicit_errors() {
    let store = ClaimStore::new();
    let missing_claim = claim_id("claim--missing-for-stance");

    let missing_claim_error = store
        .stances_by_claim(&missing_claim)
        .expect_err("missing claim stance lookup should fail");
    assert!(matches!(missing_claim_error, GraphError::ClaimNotFound(id) if id == missing_claim));

    let missing_stance_error = store
        .stance_by_id("stance--missing")
        .expect_err("missing stance lookup should fail");
    assert!(matches!(
    missing_stance_error,
    GraphError::StanceNotFound(id) if id == "stance--missing"
    ));
}

//
// Verify belief-state view can be generated for an agent without changing claim
// lifecycle state.
#[test]
fn belief_state_summarizes_agent_stances() {
    let mut store = ClaimStore::new();
    let claim = claim_id("claim--belief-state");
    create_asserted_claim(&mut store, &claim, "Claim used for belief-state view");

    store
        .create_agent_stance(AgentStanceInput::new(
            "agent://echo".to_owned(),
            claim.clone(),
            StanceKind::Rejects,
        ))
        .expect("agent stance creation should succeed");

    let belief_state = store
        .belief_state_for_agent("agent://echo")
        .expect("belief state should be materialized");

    assert_eq!(belief_state.agent_ref(), "agent://echo");
    assert_eq!(belief_state.stances().len(), 1);
    assert_eq!(belief_state.stances()[0].claim_id(), &claim);
    assert_eq!(belief_state.stances()[0].stance(), StanceKind::Rejects);
}

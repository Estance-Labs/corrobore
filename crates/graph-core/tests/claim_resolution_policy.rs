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
    AgentStanceInput, ClaimAnalyticalTarget, ClaimId, ClaimInput, ClaimLink, ClaimLinkKind,
    ClaimLinkSource, ClaimStatement, ClaimStatus, ClaimStore, ClaimTarget,
    EpistemicResolutionContext, EpistemicResolutionPolicyKind,
    EpistemicResolutionPolicyRegistration, GraphError, ResolutionTrustInput, StanceKind,
    TemporalMetadata,
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
// Verify the deterministic resolution boundary can represent links, stances,
// trust inputs, confidence context, and temporal metadata without hidden logic.
#[test]
fn resolution_context_represents_required_inputs() {
    let mut store = ClaimStore::new();
    let claim = claim_id("claim--resolution-context");
    let support_claim = claim_id("claim--resolution-support");
    create_asserted_claim(&mut store, &claim, "Primary claim for resolution context");
    create_asserted_claim(&mut store, &support_claim, "Supporting claim for context");

    let stance_id = store
        .create_agent_stance(AgentStanceInput::new(
            "agent://resolver/tester".to_owned(),
            claim.clone(),
            StanceKind::Supports,
        ))
        .expect("stance creation should succeed");

    let stance = store
        .stance_by_id(&stance_id)
        .expect("stance should be readable")
        .clone();

    let link = ClaimLink::new(
        ClaimLinkSource::Claim(support_claim.clone()),
        claim.clone(),
        ClaimLinkKind::Supports,
    )
    .with_explanation_ref(Some("analysis://support/link-1".to_owned()));

    let context = EpistemicResolutionContext::new(
        store
            .claim_by_id(&claim)
            .expect("claim should be readable")
            .clone(),
    )
    .with_link(link)
    .with_stance(stance)
    .with_trust_input(ResolutionTrustInput::new(
        "source://intel/feed-alpha".to_owned(),
        0.77,
    ))
    .with_temporal(TemporalMetadata {
        observed_at: Some("2026-07-06T10:00:00Z".to_owned()),
        ..Default::default()
    })
    .with_policy_metadata("policy-version".to_owned(), "deterministic-v1".to_owned());

    assert_eq!(context.links().len(), 1);
    assert_eq!(context.stances().len(), 1);
    assert_eq!(context.trust_inputs().len(), 1);
    assert_eq!(
        context.temporal().observed_at.as_deref(),
        Some("2026-07-06T10:00:00Z")
    );
}

//
// Verify unknown policy names return an explicit typed error.
#[test]
fn unknown_policy_name_returns_typed_error() {
    let mut store = ClaimStore::new();
    let claim = claim_id("claim--missing-policy");
    create_asserted_claim(&mut store, &claim, "Claim requiring policy resolution");

    let error = store
        .resolve_claim_with_policy(&claim, "policy://missing")
        .expect_err("missing policy should return explicit error");

    assert!(matches!(
    error,
    GraphError::ResolutionPolicyNotFound(name) if name == "policy://missing"
    ));
}

//
// Verify deterministic conservative policy produces stable outputs for the same
// claim state and context.
#[test]
fn conservative_policy_resolution_is_deterministic() {
    let mut store = ClaimStore::new();
    let claim = claim_id("claim--deterministic-policy");
    let support = claim_id("claim--deterministic-support");
    create_asserted_claim(&mut store, &claim, "Primary deterministic claim");
    create_asserted_claim(&mut store, &support, "Support for deterministic claim");

    store
        .attach_supporting_claim_to_claim(support.clone(), claim.clone())
        .expect("supporting link should attach");

    store
        .register_resolution_policy(EpistemicResolutionPolicyRegistration::new(
            "policy://conservative-default".to_owned(),
            EpistemicResolutionPolicyKind::ConservativeDeterministic,
        ))
        .expect("policy registration should succeed");

    let first = store
        .resolve_claim_with_policy(&claim, "policy://conservative-default")
        .expect("first policy evaluation should succeed");
    let second = store
        .resolve_claim_with_policy(&claim, "policy://conservative-default")
        .expect("second policy evaluation should succeed");

    assert_eq!(first.recommended_status(), second.recommended_status());
    assert_eq!(first.confidence(), second.confidence());
    assert_eq!(first.explanation(), second.explanation());
    assert_eq!(first.consumed_input_refs(), second.consumed_input_refs());
}

//
// Verify policy output carries explanation and consumed inputs, while remaining
// compatible with unresolved outcomes.
#[test]
fn resolution_output_explains_unresolved_or_changed_state() {
    let mut store = ClaimStore::new();
    let claim = claim_id("claim--resolution-output");
    let support = claim_id("claim--resolution-output-support");
    let refute = claim_id("claim--resolution-output-refute");

    create_asserted_claim(&mut store, &claim, "Claim used for resolution output");
    create_asserted_claim(&mut store, &support, "Supporting evidence claim");
    create_asserted_claim(&mut store, &refute, "Refuting evidence claim");

    store
        .attach_supporting_claim_to_claim(support, claim.clone())
        .expect("supporting link should attach");
    store
        .attach_refuting_claim_to_claim(refute, claim.clone())
        .expect("refuting link should attach");

    store
        .register_resolution_policy(EpistemicResolutionPolicyRegistration::new(
            "policy://output-check".to_owned(),
            EpistemicResolutionPolicyKind::ConservativeDeterministic,
        ))
        .expect("policy registration should succeed");

    let resolution = store
        .resolve_claim_with_policy(&claim, "policy://output-check")
        .expect("policy evaluation should succeed");

    assert!(
        !resolution.explanation().trim().is_empty(),
        "resolution explanation should be non-empty"
    );
    assert!(
        !resolution.consumed_input_refs().is_empty(),
        "resolution should list consumed inputs"
    );
    assert!(
        matches!(
            resolution.recommended_status(),
            ClaimStatus::Supported | ClaimStatus::Disputed | ClaimStatus::Unresolved
        ),
        "resolution should remain within deterministic conservative outcomes"
    );
}

//
// Verify registered policy entries can be selected by stable policy name.
#[test]
fn registered_policy_can_be_selected_by_name() {
    let mut store = ClaimStore::new();

    store
        .register_resolution_policy(
            EpistemicResolutionPolicyRegistration::new(
                "policy://selector".to_owned(),
                EpistemicResolutionPolicyKind::ConservativeDeterministic,
            )
            .with_metadata("owner".to_owned(), "epic-0005".to_owned()),
        )
        .expect("policy registration should succeed");

    let selected = store
        .resolution_policy_by_name("policy://selector")
        .expect("policy lookup should succeed");

    assert_eq!(selected.name(), "policy://selector");
    assert_eq!(
        selected.kind(),
        EpistemicResolutionPolicyKind::ConservativeDeterministic
    );
    assert!(
        selected
            .metadata()
            .iter()
            .any(|(k, v)| k == "owner" && v == "epic-0005")
    );
}

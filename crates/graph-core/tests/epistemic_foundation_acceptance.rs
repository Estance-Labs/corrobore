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
//! epistemic foundation acceptance suite.
//!
//! This suite is the source of truth for the integrated epistemic behavior
//! expected by issue #98. It exercises the public `graph_core` API and keeps
//! production probabilistic adjudication out of scope.

use graph_core::{
    AgentStanceInput, ClaimAnalyticalTarget, ClaimId, ClaimInput, ClaimStatement, ClaimStatus,
    ClaimStore, ClaimTarget, ClaimTargetValidationContext, EpistemicExplanationKind,
    EpistemicResolutionPolicyKind, EpistemicResolutionPolicyRegistration, Graph,
    GraphWorkingSetCreateRequest, GraphWorkingSetManager, HypothesisWorkspaceId,
    HypothesisWorkspaceInput, HypothesisWorkspaceStatus, NodeInput, RelationshipInput, StanceKind,
    TrustInputInput, TrustInputKind, WorkingSetId, default_generic_loading_profile,
};

fn claim_id(value: &str) -> ClaimId {
    ClaimId::new(value).expect("acceptance claim ID should be valid")
}

fn create_asserted_claim(store: &mut ClaimStore, id: &ClaimId, statement: &str) {
    let input = ClaimInput::new(
        id.clone(),
        ClaimStatement::new(statement).expect("acceptance statement should be valid"),
        ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new(statement, None)),
    );

    store
        .create_asserted_claim(input)
        .expect("asserted claim should be created");
}

//
// Validate the integrated epistemic contract in one deterministic
// acceptance scenario spanning lifecycle, links, stances, hypothesis workspaces,
// trust inputs, deterministic resolution, and explanation metadata.
#[test]
fn epic_0005_epistemic_foundation_happy_path_contract() {
    let mut store = ClaimStore::new();

    let candidate = claim_id("claim--epic-0005-candidate");
    let primary = claim_id("claim--epic-0005-primary");
    let support = claim_id("claim--epic-0005-support");
    let refute = claim_id("claim--epic-0005-refute");
    let contradictory = claim_id("claim--epic-0005-contradictory");
    let superseding = claim_id("claim--epic-0005-superseding");
    let retractable = claim_id("claim--epic-0005-retractable");
    let rejectable = claim_id("claim--epic-0005-rejectable");

    // Candidate + asserted creation boundaries.
    let candidate_created = store
        .create_candidate_claim(ClaimInput::new(
            candidate.clone(),
            ClaimStatement::new("Candidate claim for acceptance")
                .expect("candidate statement should be valid"),
            ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new(
                "Candidate claim for acceptance",
                None,
            )),
        ))
        .expect("candidate claim should be created");
    assert_eq!(candidate_created, candidate);
    assert_eq!(
        store
            .claim_by_id(&candidate)
            .expect("candidate claim should be readable")
            .status(),
        ClaimStatus::Candidate
    );

    for (id, statement) in [
        (&primary, "Primary asserted claim"),
        (&support, "Supporting claim"),
        (&refute, "Refuting claim"),
        (&contradictory, "Contradictory claim"),
        (&superseding, "Superseding claim"),
        (&retractable, "Retractable claim"),
        (&rejectable, "Rejectable claim"),
    ] {
        create_asserted_claim(&mut store, id, statement);
    }

    // Target reference validation remains explicit and deterministic.
    let mut target_context = ClaimTargetValidationContext::new();
    target_context.register_source("source://epic-0005/report");
    ClaimTarget::Source(graph_core::ClaimSourceTargetRef::new(
        "source://epic-0005/report",
    ))
    .validate_references(&target_context)
    .expect("source target should validate in explicit context");

    // Support/refutation plus contradiction/supersession remain visible links.
    let support_link = store
        .attach_supporting_claim_to_claim(support.clone(), primary.clone())
        .expect("support link should be created");
    let _refute_link = store
        .attach_refuting_claim_to_claim(refute.clone(), primary.clone())
        .expect("refute link should be created");
    let _contradiction_link = store
        .attach_contradicting_claim_to_claim(
            contradictory.clone(),
            primary.clone(),
            Some("reason://epic-0005/contradiction".to_owned()),
        )
        .expect("contradiction link should be created");
    let _supersession_link = store
        .attach_superseding_claim_to_claim(
            superseding.clone(),
            primary.clone(),
            Some("reason://epic-0005/supersession".to_owned()),
        )
        .expect("supersession link should be created");

    // Retraction/rejection keep reason metadata as auditable decisions.
    let retracted = store
        .retract_claim(
            retractable.clone(),
            "reason://epic-0005/retracted".to_owned(),
            Some("actor://analyst/epic-0005".to_owned()),
            Some("session://epic-0005/retract".to_owned()),
        )
        .expect("retraction should succeed");
    assert_eq!(retracted.status(), ClaimStatus::Retracted);

    let rejected = store
        .reject_claim(
            rejectable.clone(),
            "reason://epic-0005/rejected".to_owned(),
            Some("actor://reviewer/epic-0005".to_owned()),
            Some("session://epic-0005/reject".to_owned()),
        )
        .expect("rejection should succeed");
    assert_eq!(rejected.status(), ClaimStatus::Rejected);

    // Agent disagreement remains first-class and auditable.
    store
        .create_agent_stance(AgentStanceInput::new(
            "agent://alpha".to_owned(),
            primary.clone(),
            StanceKind::Supports,
        ))
        .expect("support stance should be created");
    store
        .create_agent_stance(AgentStanceInput::new(
            "agent://bravo".to_owned(),
            primary.clone(),
            StanceKind::Refutes,
        ))
        .expect("refute stance should be created");

    let belief_alpha = store
        .belief_state_for_agent("agent://alpha")
        .expect("belief state should be readable");
    assert_eq!(belief_alpha.stances().len(), 1);

    // Hypothesis workspace claim membership remains explicit.
    let workspace_id = store
        .create_hypothesis_workspace(HypothesisWorkspaceInput::new(
            HypothesisWorkspaceId::new("hypothesis-workspace--epic-0005")
                .expect("workspace ID should be valid"),
            "hypothesis".to_owned(),
            "Competing interpretation scope".to_owned(),
            graph_core::ActorId::new("actor--epic-0005-owner")
                .expect("owner actor ID should be valid"),
        ))
        .expect("hypothesis workspace should be created");
    store
        .attach_claim_to_hypothesis_workspace(workspace_id.clone(), primary.clone())
        .expect("primary claim should attach to workspace");
    store
        .set_hypothesis_workspace_status(workspace_id.clone(), HypothesisWorkspaceStatus::Deferred)
        .expect("workspace status should update");
    let workspace_claims = store
        .list_claims_in_hypothesis_workspace(&workspace_id)
        .expect("workspace claim list should be readable");
    assert_eq!(workspace_claims, vec![primary.clone()]);

    // Trust inputs and deterministic resolution remain explicit and reproducible.
    store.register_trust_subject("source://epic-0005/feed".to_owned());
    store
        .create_trust_input(
            TrustInputInput::new(
                TrustInputKind::SourceReliability,
                "source://epic-0005/feed".to_owned(),
                0.78,
            )
            .with_claim_ref(primary.clone()),
        )
        .expect("trust input should be created");

    store
        .register_resolution_policy(EpistemicResolutionPolicyRegistration::new(
            "policy://epic-0005/conservative".to_owned(),
            EpistemicResolutionPolicyKind::ConservativeDeterministic,
        ))
        .expect("resolution policy should be registered");

    let resolution = store
        .resolve_claim_with_policy(&primary, "policy://epic-0005/conservative")
        .expect("deterministic resolution should succeed");
    assert!(matches!(
        resolution.recommended_status(),
        ClaimStatus::Supported | ClaimStatus::Disputed | ClaimStatus::Unresolved
    ));
    assert!(!resolution.explanation().trim().is_empty());
    assert!(!resolution.consumed_input_refs().is_empty());

    // Explanation metadata remains queryable for claim and link operations.
    let claim_explanations = store
        .explain_claim(&retractable)
        .expect("retractable claim explanations should exist");
    assert!(
        claim_explanations
            .iter()
            .any(|entry| entry.kind() == EpistemicExplanationKind::Retraction)
    );

    let support_link_explanation = store
        .explain_claim_link(&support_link)
        .expect("support link explanation should exist");
    assert_eq!(
        support_link_explanation.kind(),
        EpistemicExplanationKind::SupportLink
    );

    store
        .record_resolution_explanation(
            "resolution://epic-0005/run-1".to_owned(),
            resolution.consumed_input_refs().to_vec(),
            Some("actor://resolver/epic-0005".to_owned()),
            Some("session://resolver/epic-0005".to_owned()),
            None,
            Some("reason://epic-0005/policy".to_owned()),
        )
        .expect("resolution explanation should be recorded");

    let resolution_explanation = store
        .explain_resolution_output("resolution://epic-0005/run-1")
        .expect("resolution explanation should be queryable");
    assert_eq!(
        resolution_explanation.kind(),
        EpistemicExplanationKind::ResolutionOutput
    );
}

//
// Validate compatibility with graph-core + working-set public contracts so Epic
// 0005 acceptance does not regress previously established /0002/0003
// behavior surfaces.
#[test]
fn epic_0005_acceptance_keeps_public_contract_compatibility() {
    let mut graph = Graph::new();
    let source = graph
        .create_node(NodeInput::new(["Source"]))
        .expect("source node should be created");
    let target = graph
        .create_node(NodeInput::new(["Target"]))
        .expect("target node should be created");

    graph
        .create_relationship(
            RelationshipInput::new(source.clone(), "LINKS_TO", target.clone())
                .expect("relationship input should be valid"),
        )
        .expect("relationship should be created");

    let profile = default_generic_loading_profile();
    assert!(!profile.prioritized_relationship_types.is_empty());

    let mut manager = GraphWorkingSetManager::new();
    let working_set_id =
        WorkingSetId::new("working-set--epic-0005").expect("working-set ID should be valid");
    manager
        .create_working_set(GraphWorkingSetCreateRequest::new(working_set_id.clone()))
        .expect("working set should be created");

    let working_set = manager
        .load_seed_node_ids(&working_set_id, [source, target], false)
        .expect("seed node loading should succeed");
    assert_eq!(working_set.seed_node_ids().len(), 2);
}

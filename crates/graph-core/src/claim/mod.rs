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
//! Claim record model and lifecycle primitives for epistemic graph behavior.
//!
//! Module boundary:
//! this module owns first-class claim records, lifecycle states, the optional
//! structured proposition beside the text statement, and a minimal in-memory
//! claim store boundary. It does not implement contradiction graph traversal,
//! supersession graphs, trust scoring, verdict computation, or automated
//! epistemic reasoning policies.

pub(crate) use std::collections::{HashMap, HashSet};

pub(crate) use serde::{Deserialize, Serialize};

pub(crate) use crate::{
    ActorId, BitemporalStamp, ClaimId, ClaimVersionId, Confidence, EvidenceId, ExtractionRunId,
    GraphError, HypothesisWorkspaceId, NodeId, ObservationId, PropertyMap, PropertyValue,
    RelationshipId, TemporalMetadata, TemporalTimestamp, VerdictAsOf, WorkspaceId,
};

mod epistemic;
mod hypothesis;
mod link;
mod proposition;
mod stance;
mod status;
mod store;
mod target;

pub use epistemic::*;
pub use hypothesis::*;
pub use link::*;
pub use proposition::*;
pub use stance::*;
pub use status::*;
pub use store::*;
pub use target::*;

pub(crate) fn validate_stance_confidence(confidence: f64) -> Result<(), GraphError> {
    if !(0.0..=1.0).contains(&confidence) {
        return Err(GraphError::InvalidConfidence(confidence));
    }

    Ok(())
}

pub(crate) fn validate_trust_input_value(value: f64) -> Result<(), GraphError> {
    if !(0.0..=1.0).contains(&value) {
        return Err(GraphError::InvalidTrustInputValue(value));
    }

    Ok(())
}

pub(crate) fn claim_link_explanation_key(link: &ClaimLink) -> String {
    let source_ref = format!("{}:{}", link.source().kind_token(), link.source().id_str());

    format!(
        "{}:{}:{}",
        source_ref,
        link.target_claim_id().as_str(),
        claim_link_kind_token(link.kind())
    )
}

pub(crate) fn claim_link_kind_to_explanation_kind(kind: ClaimLinkKind) -> EpistemicExplanationKind {
    match kind {
        ClaimLinkKind::Supports => EpistemicExplanationKind::SupportLink,
        ClaimLinkKind::Refutes => EpistemicExplanationKind::RefutationLink,
        ClaimLinkKind::Contradicts => EpistemicExplanationKind::ContradictionLink,
        ClaimLinkKind::Supersedes => EpistemicExplanationKind::SupersessionLink,
        ClaimLinkKind::ContextFor => EpistemicExplanationKind::ContextLink,
        ClaimLinkKind::Duplicates => EpistemicExplanationKind::DuplicateLink,
        ClaimLinkKind::DerivedFrom => EpistemicExplanationKind::DerivationLink,
        ClaimLinkKind::DependsOn => EpistemicExplanationKind::DependencyLink,
    }
}

pub(crate) fn claim_link_kind_token(kind: ClaimLinkKind) -> &'static str {
    match kind {
        ClaimLinkKind::Supports => "supports",
        ClaimLinkKind::Refutes => "refutes",
        ClaimLinkKind::Contradicts => "contradicts",
        ClaimLinkKind::Supersedes => "supersedes",
        ClaimLinkKind::ContextFor => "context_for",
        ClaimLinkKind::Duplicates => "duplicates",
        ClaimLinkKind::DerivedFrom => "derived_from",
        ClaimLinkKind::DependsOn => "depends_on",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn claim_status_allows_candidate_to_asserted_transition() {
        assert_eq!(
            ClaimStatus::ensure_valid_transition(ClaimStatus::Candidate, ClaimStatus::Asserted),
            Ok(())
        );
    }

    #[test]
    fn claim_status_rejects_rejected_to_supported_transition() {
        let error =
            ClaimStatus::ensure_valid_transition(ClaimStatus::Rejected, ClaimStatus::Supported)
                .expect_err("rejected claim should not transition to supported");

        assert!(matches!(
        error,
        GraphError::InvalidClaimStatusTransition { from, to }
        if from == ClaimStatus::Rejected && to == ClaimStatus::Supported
        ));
    }

    #[test]
    fn claim_status_allows_noop_transition_for_same_state() {
        assert_eq!(
            ClaimStatus::ensure_valid_transition(ClaimStatus::Supported, ClaimStatus::Supported),
            Ok(())
        );
    }

    #[test]
    fn claim_status_disallows_transition_from_terminal_superseded() {
        let error =
            ClaimStatus::ensure_valid_transition(ClaimStatus::Superseded, ClaimStatus::Validated)
                .expect_err("superseded claim should not transition to validated");

        assert!(matches!(
        error,
        GraphError::InvalidClaimStatusTransition { from, to }
        if from == ClaimStatus::Superseded && to == ClaimStatus::Validated
        ));
    }

    #[test]
    fn stance_and_trust_value_validators_enforce_unit_interval() {
        validate_stance_confidence(0.0).expect("lower stance bound should be accepted");
        validate_stance_confidence(1.0).expect("upper stance bound should be accepted");
        validate_trust_input_value(0.0).expect("lower trust bound should be accepted");
        validate_trust_input_value(1.0).expect("upper trust bound should be accepted");

        let stance_error = validate_stance_confidence(1.2)
            .expect_err("stance confidence above 1.0 should be rejected");
        let trust_error =
            validate_trust_input_value(-0.2).expect_err("trust input below 0.0 should be rejected");

        assert!(matches!(
        stance_error,
        GraphError::InvalidConfidence(value) if value == 1.2
        ));
        assert!(matches!(
        trust_error,
        GraphError::InvalidTrustInputValue(value) if value == -0.2
        ));
    }

    #[test]
    fn claim_link_helpers_generate_expected_tokens_and_explanation_kinds() {
        let target_claim = ClaimId::new("claim--target").expect("target claim ID should be valid");
        let evidence_id = EvidenceId::new("evidence--1").expect("evidence ID should be valid");
        let source_claim = ClaimId::new("claim--source").expect("source claim ID should be valid");

        let evidence_support_link = ClaimLink::new(
            ClaimLinkSource::Evidence(evidence_id),
            target_claim.clone(),
            ClaimLinkKind::Supports,
        );
        let claim_refute_link = ClaimLink::new(
            ClaimLinkSource::Claim(source_claim),
            target_claim.clone(),
            ClaimLinkKind::Refutes,
        );

        assert_eq!(claim_link_kind_token(ClaimLinkKind::Supports), "supports");
        assert_eq!(claim_link_kind_token(ClaimLinkKind::Refutes), "refutes");
        assert_eq!(
            claim_link_kind_token(ClaimLinkKind::Contradicts),
            "contradicts"
        );
        assert_eq!(
            claim_link_kind_token(ClaimLinkKind::Supersedes),
            "supersedes"
        );

        assert_eq!(
            claim_link_kind_to_explanation_kind(ClaimLinkKind::Supports),
            EpistemicExplanationKind::SupportLink
        );
        assert_eq!(
            claim_link_kind_to_explanation_kind(ClaimLinkKind::Refutes),
            EpistemicExplanationKind::RefutationLink
        );
        assert_eq!(
            claim_link_kind_to_explanation_kind(ClaimLinkKind::Contradicts),
            EpistemicExplanationKind::ContradictionLink
        );
        assert_eq!(
            claim_link_kind_to_explanation_kind(ClaimLinkKind::Supersedes),
            EpistemicExplanationKind::SupersessionLink
        );

        assert_eq!(
            claim_link_explanation_key(&evidence_support_link),
            "evidence:evidence--1:claim--target:supports"
        );
        assert_eq!(
            claim_link_explanation_key(&claim_refute_link),
            "claim:claim--source:claim--target:refutes"
        );
    }

    #[test]
    fn claim_statement_rejects_empty_values() {
        let error = ClaimStatement::new(" ").expect_err("blank claim statement should be rejected");

        assert!(matches!(
        error,
        GraphError::InvalidPropertyValue(message)
        if message == "claim statement must not be empty"
        ));
    }

    #[test]
    fn claim_target_validation_rejects_empty_or_invalid_fields() {
        let context = ClaimTargetValidationContext::new();

        let evidence_error = ClaimTarget::Evidence(ClaimEvidenceTargetRef::new(" "))
            .validate_references(&context)
            .expect_err("blank evidence reference should be rejected");
        assert!(matches!(
        evidence_error,
        GraphError::InvalidPropertyValue(message)
        if message == "evidence target reference must not be empty"
        ));

        let source_error = ClaimTarget::Source(ClaimSourceTargetRef::new(" "))
            .validate_references(&context)
            .expect_err("blank source reference should be rejected");
        assert!(matches!(
        source_error,
        GraphError::InvalidPropertyValue(message)
        if message == "source target reference must not be empty"
        ));

        let temporal_error = ClaimTarget::TemporalAssertion(ClaimTemporalTarget::new(" ", "x"))
            .validate_references(&context)
            .expect_err("blank temporal field should be rejected");
        assert!(matches!(
        temporal_error,
        GraphError::InvalidPropertyValue(message)
        if message == "temporal target requires non-empty field and value"
        ));

        let confidence_error =
            ClaimTarget::ConfidenceAssertion(ClaimConfidenceTarget::new("entity", 1.2))
                .validate_references(&context)
                .expect_err("confidence above 1.0 should be rejected");
        assert!(matches!(
        confidence_error,
        GraphError::InvalidConfidence(value) if value == 1.2
        ));

        let analytical_error =
            ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new(" ", None))
                .validate_references(&context)
                .expect_err("blank analytical summary should be rejected");
        assert!(matches!(
        analytical_error,
        GraphError::InvalidPropertyValue(message)
        if message == "analytical target summary must not be empty"
        ));
    }

    #[test]
    fn hypothesis_workspace_creation_rejects_invalid_inputs_and_duplicates() {
        let mut store = ClaimStore::new();

        let missing_title = store
            .create_hypothesis_workspace(HypothesisWorkspaceInput::new(
                HypothesisWorkspaceId::new("hypothesis-workspace--missing-title")
                    .expect("workspace ID should be valid"),
                " ".to_owned(),
                "description".to_owned(),
                ActorId::new("actor--owner").expect("actor ID should be valid"),
            ))
            .expect_err("blank title should be rejected");
        assert!(matches!(
        missing_title,
        GraphError::InvalidPropertyValue(message)
        if message == "hypothesis workspace title must not be empty"
        ));

        let workspace_id = store
            .create_hypothesis_workspace(HypothesisWorkspaceInput::new(
                HypothesisWorkspaceId::new("hypothesis-workspace--duplicate")
                    .expect("workspace ID should be valid"),
                "Title".to_owned(),
                "Description".to_owned(),
                ActorId::new("actor--owner").expect("actor ID should be valid"),
            ))
            .expect("workspace should be created");

        let duplicate = store
            .create_hypothesis_workspace(HypothesisWorkspaceInput::new(
                workspace_id,
                "Title".to_owned(),
                "Description".to_owned(),
                ActorId::new("actor--owner").expect("actor ID should be valid"),
            ))
            .expect_err("duplicate workspace ID should be rejected");
        assert!(matches!(duplicate, GraphError::InvalidVersionState(_)));
    }

    #[test]
    fn trust_input_paths_cover_blank_unknown_and_missing_claim() {
        let mut store = ClaimStore::new();

        let invalid_value = store
            .create_trust_input(TrustInputInput::new(
                TrustInputKind::SourceReliability,
                "subject://x".to_owned(),
                1.3,
            ))
            .expect_err("invalid trust value should fail first");
        assert!(matches!(
        invalid_value,
        GraphError::InvalidTrustInputValue(value) if value == 1.3
        ));

        let blank_subject = store
            .create_trust_input(TrustInputInput::new(
                TrustInputKind::SourceReliability,
                " ".to_owned(),
                0.7,
            ))
            .expect_err("blank trust subject should be rejected");
        assert!(matches!(blank_subject, GraphError::TrustSubjectNotFound(_)));

        let unknown_subject = store
            .create_trust_input(TrustInputInput::new(
                TrustInputKind::SourceReliability,
                "subject://unknown".to_owned(),
                0.7,
            ))
            .expect_err("unknown trust subject should be rejected");
        assert!(matches!(
            unknown_subject,
            GraphError::TrustSubjectNotFound(_)
        ));

        store.register_trust_subject("subject://known".to_owned());
        let missing_claim_error = store
            .create_trust_input(
                TrustInputInput::new(
                    TrustInputKind::SourceReliability,
                    "subject://known".to_owned(),
                    0.7,
                )
                .with_claim_ref(claim_id("claim--missing")),
            )
            .expect_err("missing claim reference should be rejected");
        assert!(matches!(missing_claim_error, GraphError::ClaimNotFound(_)));
    }

    #[test]
    fn trust_input_and_explanation_queries_report_typed_not_found_errors() {
        let store = ClaimStore::new();

        let subject_error = store
            .trust_inputs_by_subject("subject://missing")
            .expect_err("unknown subject should return typed not-found error");
        assert!(matches!(subject_error, GraphError::TrustSubjectNotFound(_)));

        let claim_error = store
            .explain_claim(&claim_id("claim--missing"))
            .expect_err("missing claim explanation should return typed not-found error");
        assert!(matches!(
            claim_error,
            GraphError::ClaimExplanationNotFound(_)
        ));

        let resolution_error = store
            .explain_resolution_output("resolution://missing")
            .expect_err("missing resolution explanation should return typed not-found error");
        assert!(matches!(
        resolution_error,
        GraphError::ResolutionExplanationNotFound(value) if value == "resolution://missing"
        ));
    }

    #[test]
    fn record_and_lookup_resolution_explanation_cover_success_and_validation_error() {
        let mut store = ClaimStore::new();

        let empty_error = store
            .record_resolution_explanation(" ".to_owned(), vec![], None, None, None, None)
            .expect_err("blank resolution reference should be rejected");
        assert!(matches!(
        empty_error,
        GraphError::InvalidPropertyValue(message)
        if message == "resolution reference must not be empty"
        ));

        store
            .record_resolution_explanation(
                "resolution://ok".to_owned(),
                vec!["claim:1".to_owned(), "link:2".to_owned()],
                Some("actor://resolver".to_owned()),
                Some("session://s".to_owned()),
                Some(WorkspaceId::new("workspace--r").expect("workspace ID should be valid")),
                Some("reason://r".to_owned()),
            )
            .expect("resolution explanation should be recorded");

        let explanation = store
            .explain_resolution_output("resolution://ok")
            .expect("stored resolution explanation should be readable");
        assert_eq!(
            explanation.kind(),
            EpistemicExplanationKind::ResolutionOutput
        );
        assert_eq!(explanation.consumed_inputs().len(), 2);
        assert_eq!(explanation.actor_ref(), Some("actor://resolver"));
        assert_eq!(explanation.workspace_ref(), Some("workspace--r"));
    }

    #[test]
    fn agent_stance_validation_and_link_explanation_paths_cover_errors_and_success() {
        let mut store = ClaimStore::new();
        let source_claim = claim_id("claim--source");
        let target_claim = claim_id("claim--target");
        create_asserted_claim(&mut store, &source_claim, "source claim");
        create_asserted_claim(&mut store, &target_claim, "target claim");

        let invalid_agent = store
            .create_agent_stance(
                AgentStanceInput::new(" ".to_owned(), target_claim.clone(), StanceKind::Supports)
                    .with_confidence(0.7),
            )
            .expect_err("blank agent ref should be rejected");
        assert!(matches!(
        invalid_agent,
        GraphError::InvalidPropertyValue(message)
        if message == "agent stance agent_ref must not be empty"
        ));

        let invalid_confidence = store
            .create_agent_stance(
                AgentStanceInput::new(
                    "agent://a".to_owned(),
                    target_claim.clone(),
                    StanceKind::Supports,
                )
                .with_confidence(1.2),
            )
            .expect_err("invalid stance confidence should be rejected");
        assert!(matches!(
        invalid_confidence,
        GraphError::InvalidConfidence(value) if value == 1.2
        ));

        let link = store
            .attach_supporting_claim_to_claim(source_claim, target_claim)
            .expect("support link should be created");
        let explanation = store
            .explain_claim_link(&link)
            .expect("link explanation should exist");
        assert_eq!(explanation.kind(), EpistemicExplanationKind::SupportLink);

        let missing_link_error = store
            .explain_claim_link(&ClaimLink::new(
                ClaimLinkSource::Claim(claim_id("claim--other")),
                claim_id("claim--target-missing-link"),
                ClaimLinkKind::Refutes,
            ))
            .expect_err("missing link explanation should return typed not-found error");
        assert!(matches!(
            missing_link_error,
            GraphError::ClaimLinkExplanationNotFound(_)
        ));
    }

    #[test]
    fn claim_status_transition_matrix_covers_additional_allowed_and_rejected_paths() {
        let allowed = vec![
            (ClaimStatus::Candidate, ClaimStatus::Rejected),
            (ClaimStatus::Asserted, ClaimStatus::Supported),
            (ClaimStatus::Supported, ClaimStatus::Disputed),
            (ClaimStatus::Disputed, ClaimStatus::Validated),
            (ClaimStatus::Contradicted, ClaimStatus::Unresolved),
            (ClaimStatus::Unresolved, ClaimStatus::Supported),
            (ClaimStatus::Validated, ClaimStatus::Disputed),
            (ClaimStatus::Validated, ClaimStatus::Contradicted),
            (ClaimStatus::Validated, ClaimStatus::Retracted),
        ];

        for (from, to) in allowed {
            ClaimStatus::ensure_valid_transition(from, to)
                .expect("transition should be accepted by lifecycle policy");
        }

        let rejected = vec![
            (ClaimStatus::Candidate, ClaimStatus::Supported),
            (ClaimStatus::Validated, ClaimStatus::Supported),
            (ClaimStatus::Retracted, ClaimStatus::Asserted),
        ];

        for (from, to) in rejected {
            let error = ClaimStatus::ensure_valid_transition(from, to)
                .expect_err("invalid transition should return typed rejection");
            assert!(matches!(
            error,
            GraphError::InvalidClaimStatusTransition {
            from: actual_from,
            to: actual_to,
            } if actual_from == from && actual_to == to
            ));
        }
    }

    #[test]
    fn stance_and_workspace_input_builders_preserve_all_fields() {
        let claim = claim_id("claim--builder-test");
        let workspace =
            WorkspaceId::new("workspace--builder-test").expect("workspace id should be valid");

        let stance_input = AgentStanceInput::new(
            "agent://builder".to_owned(),
            claim.clone(),
            StanceKind::WithholdsJudgment,
        )
        .with_workspace_id(workspace.clone())
        .with_confidence(0.55)
        .with_reason_ref("reason://one".to_owned())
        .with_reason_ref("reason://two".to_owned());
        assert_eq!(stance_input.agent_ref, "agent://builder");
        assert_eq!(stance_input.claim_id, claim);
        assert_eq!(stance_input.workspace_id, Some(workspace));
        assert_eq!(stance_input.stance, StanceKind::WithholdsJudgment);
        assert_eq!(stance_input.confidence, Some(0.55));
        assert_eq!(stance_input.reason_refs.len(), 2);

        let patch = AgentStancePatch::new(StanceKind::Disputes)
            .with_confidence(0.33)
            .with_reason_ref("reason://patch".to_owned());
        assert_eq!(patch.stance, StanceKind::Disputes);
        assert_eq!(patch.confidence, Some(0.33));
        assert_eq!(patch.reason_refs, vec!["reason://patch".to_owned()]);

        let workspace_input = HypothesisWorkspaceInput::new(
            HypothesisWorkspaceId::new("hypothesis-workspace--builder")
                .expect("workspace id should be valid"),
            "Hypothesis Title".to_owned(),
            "Hypothesis Description".to_owned(),
            ActorId::new("actor--builder").expect("actor id should be valid"),
        )
        .with_created_at("2026-07-07T00:00:00Z".to_owned())
        .with_parent_context_ref("context://parent".to_owned());
        assert_eq!(workspace_input.title, "Hypothesis Title");
        assert_eq!(workspace_input.description, "Hypothesis Description");
        assert_eq!(
            workspace_input.created_at.as_deref(),
            Some("2026-07-07T00:00:00Z")
        );
        assert_eq!(
            workspace_input.parent_context_ref.as_deref(),
            Some("context://parent")
        );
    }

    #[test]
    fn trust_input_and_belief_state_accessors_expose_stored_values() {
        let claim = claim_id("claim--trust-builder");
        let temporal = TemporalMetadata::default();

        let input = TrustInputInput::new(
            TrustInputKind::ModelReliability,
            "subject://model".to_owned(),
            0.8,
        )
        .with_provenance_ref("prov://model".to_owned())
        .with_reason_ref("reason://model".to_owned())
        .with_temporal(temporal.clone())
        .with_claim_ref(claim.clone());
        assert_eq!(input.kind, TrustInputKind::ModelReliability);
        assert_eq!(input.subject_ref, "subject://model");
        assert_eq!(input.value, 0.8);
        assert_eq!(input.provenance_ref.as_deref(), Some("prov://model"));
        assert_eq!(input.reason_ref.as_deref(), Some("reason://model"));
        assert_eq!(input.temporal, temporal);
        assert_eq!(input.claim_refs, vec![claim]);

        let stance = AgentStance {
            stance_id: "stance--belief".to_owned(),
            agent_ref: "agent://belief".to_owned(),
            claim_id: claim_id("claim--belief"),
            workspace_id: None,
            stance: StanceKind::Supports,
            confidence: Some(0.9),
            reason_refs: vec!["reason://belief".to_owned()],
        };
        let belief = BeliefState::new("agent://belief".to_owned(), vec![stance.clone()]);
        assert_eq!(belief.agent_ref(), "agent://belief");
        assert_eq!(belief.stances(), &[stance]);
    }

    #[test]
    fn claim_target_and_metadata_paths_cover_unknown_refs_and_unsupported_kind() {
        let context = ClaimTargetValidationContext::new();

        let relationship =
            RelationshipId::new("relationship--unknown").expect("relationship id should be valid");
        let relationship_error = ClaimTarget::Relationship(relationship)
            .validate_references(&context)
            .expect_err("unknown relationship target should be rejected");
        assert!(matches!(
            relationship_error,
            GraphError::ClaimTargetNotFound(_)
        ));

        let evidence_error =
            ClaimTarget::Evidence(ClaimEvidenceTargetRef::new("evidence://unknown"))
                .validate_references(&context)
                .expect_err("unknown non-empty evidence ref should be rejected");
        assert!(matches!(evidence_error, GraphError::ClaimTargetNotFound(_)));

        let source_error = ClaimTarget::Source(ClaimSourceTargetRef::new("source://unknown"))
            .validate_references(&context)
            .expect_err("unknown non-empty source ref should be rejected");
        assert!(matches!(source_error, GraphError::ClaimTargetNotFound(_)));

        let confidence_kind_error =
            ClaimTarget::ConfidenceAssertion(ClaimConfidenceTarget::new(" ", 0.5))
                .validate_references(&context)
                .expect_err("blank confidence kind should be rejected");
        assert!(matches!(
        confidence_kind_error,
        GraphError::InvalidPropertyValue(message)
        if message == "confidence target kind must not be empty"
        ));

        let unsupported = ClaimTarget::Unsupported {
            kind: "legacy".to_owned(),
            raw_reference: "raw://target".to_owned(),
        };
        assert_eq!(unsupported.kind(), ClaimTargetKind::Unsupported);
        let unsupported_error = unsupported
            .resolve_target_metadata(&context)
            .expect_err("unsupported target kind should fail metadata resolution");
        assert!(matches!(
        unsupported_error,
        GraphError::UnsupportedClaimTargetKind(kind) if kind == "legacy"
        ));
    }

    #[test]
    fn claim_and_statement_accessors_expose_expected_values() {
        let mut store = ClaimStore::new();
        let id = claim_id("claim--accessors");
        let statement =
            ClaimStatement::new("claim accessor statement").expect("statement should be valid");
        assert_eq!(statement.as_str(), "claim accessor statement");

        let input = ClaimInput::new(
            id.clone(),
            statement,
            ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new(
                "summary",
                Some("workspace://h".to_owned()),
            )),
        )
        .with_source_ref("source://one")
        .with_temporal(TemporalMetadata::default());

        store
            .create_candidate_claim(input)
            .expect("candidate claim should be created");
        let claim = store.claim_by_id(&id).expect("claim should be retrievable");

        assert_eq!(claim.id(), &id);
        assert_eq!(claim.version(), 1);
        assert_eq!(claim.status(), ClaimStatus::Candidate);
        assert_eq!(claim.statement().as_str(), "claim accessor statement");
        assert!(matches!(
            claim.target(),
            ClaimTarget::AnalyticalAssertion(_)
        ));
        assert_eq!(claim.source_refs(), &["source://one".to_owned()]);
        assert!(claim.created_by().is_none());
        assert!(claim.confidence().is_none());
    }

    #[test]
    fn policy_and_workspace_validations_cover_blank_values_and_not_found() {
        let mut store = ClaimStore::new();

        let missing_workspace_description = store
            .create_hypothesis_workspace(HypothesisWorkspaceInput::new(
                HypothesisWorkspaceId::new("hypothesis-workspace--blank-description")
                    .expect("workspace id should be valid"),
                "Title".to_owned(),
                " ".to_owned(),
                ActorId::new("actor--owner").expect("actor id should be valid"),
            ))
            .expect_err("blank workspace description should be rejected");
        assert!(matches!(
        missing_workspace_description,
        GraphError::InvalidPropertyValue(message)
        if message == "hypothesis workspace description must not be empty"
        ));

        store.register_trust_subject(" ".to_owned());
        let blank_subject_error = store
            .trust_inputs_by_subject(" ")
            .expect_err("blank trust subject should not have been registered");
        assert!(matches!(
            blank_subject_error,
            GraphError::TrustSubjectNotFound(_)
        ));

        let policy_error = store
            .register_resolution_policy(EpistemicResolutionPolicyRegistration::new(
                " ".to_owned(),
                EpistemicResolutionPolicyKind::ConservativeDeterministic,
            ))
            .expect_err("blank policy name should be rejected");
        assert!(matches!(
        policy_error,
        GraphError::InvalidPropertyValue(message)
        if message == "resolution policy name must not be empty"
        ));

        let not_found = store
            .resolution_policy_by_name("policy://missing")
            .expect_err("unknown policy should return typed not-found error");
        assert!(matches!(
        not_found,
        GraphError::ResolutionPolicyNotFound(name) if name == "policy://missing"
        ));
    }

    #[test]
    fn duplicate_claim_id_is_rejected_for_candidate_creation() {
        let mut store = ClaimStore::new();
        let id = claim_id("claim--duplicate-candidate");
        let make_input = || {
            ClaimInput::new(
                id.clone(),
                ClaimStatement::new("duplicate candidate").expect("statement should be valid"),
                ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new("summary", None)),
            )
        };

        store
            .create_candidate_claim(make_input())
            .expect("first candidate should be created");
        let duplicate = store
            .create_candidate_claim(make_input())
            .expect_err("duplicate claim id should be rejected");

        assert!(matches!(
        duplicate,
        GraphError::InvalidVersionState(message)
        if message.contains("claim already exists")
        ));
    }
}

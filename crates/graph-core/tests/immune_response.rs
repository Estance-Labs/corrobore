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
    AntiPheromoneField, AntiPheromoneSignal, GraphError, GraphTier, GraphTierRegistry,
    ImmuneResponder, ImmuneResponseAction, PheromoneDecay, PheromoneTaskScope, RelationshipId,
    TierRecordRef, TierTransitionReason, ValidationErrorRecord, ValidationErrorSeverity,
    ValidationTarget,
};

fn relationship_id(value: &str) -> RelationshipId {
    RelationshipId::new(value).expect("response relationship ID should be valid")
}

fn finding(code: &str, target: ValidationTarget) -> ValidationErrorRecord {
    ValidationErrorRecord::new(
        code,
        ValidationErrorSeverity::Warning,
        "seeded immune finding",
        target,
    )
}

fn scope() -> PheromoneTaskScope {
    PheromoneTaskScope::task("fimi_investigation")
}

fn anti_field() -> AntiPheromoneField {
    AntiPheromoneField::new(PheromoneDecay::new(0.9).expect("decay should be valid"))
}

//
// Verify that quarantine responses route through the audited tier model: the
// record moves to quarantine with a validator-finding reason, and the response
// links the finding to the transition.
//
// Given a poisoning finding on a relationship,
// when the responder quarantines it,
// then the registry should show the quarantine with its audit entry and the
// response should carry the linked transition sequence.
#[test]
fn quarantine_routes_through_the_audited_tier_model() {
    let mut responder = ImmuneResponder::new();
    let mut registry = GraphTierRegistry::new();
    let suspect = relationship_id("relationship--poisoned");
    let poisoning_finding = finding(
        "immune-behavioral--pheromone-growth",
        ValidationTarget::relationship(suspect.as_str()),
    );

    let response = responder
        .quarantine(&mut registry, &poisoning_finding, "immune--responder")
        .expect("quarantine should be recorded")
        .clone();

    let record = TierRecordRef::Relationship(suspect);
    assert_eq!(registry.tier_of(&record), GraphTier::Quarantine);
    let trail = registry.audit_for(&record);
    assert_eq!(trail.len(), 1);
    assert_eq!(trail[0].reason, TierTransitionReason::ValidatorFinding);
    assert_eq!(trail[0].actor_ref, "immune--responder");

    assert_eq!(response.finding_code, "immune-behavioral--pheromone-growth");
    assert!(matches!(
        response.action,
        ImmuneResponseAction::Quarantine { .. }
    ));
    assert_eq!(
        response.tier_transition_sequence,
        Some(trail[0].sequence),
        "the response should link the finding to its tier transition"
    );
}

//
// Verify that quarantine rejects targets outside the tier model with a typed
// error and records nothing.
//
// Given a drift finding targeting a retrieval,
// when quarantine is attempted,
// then it should fail with the typed error and leave the audit empty.
#[test]
fn quarantine_rejects_non_tier_trackable_targets() {
    let mut responder = ImmuneResponder::new();
    let mut registry = GraphTierRegistry::new();
    let drift_finding = finding(
        "immune-behavioral--retrieval-drift",
        ValidationTarget::retrieval("request--drift"),
    );

    let error = responder
        .quarantine(&mut registry, &drift_finding, "immune--responder")
        .expect_err("non-trackable targets should fail");

    assert!(matches!(error, GraphError::InvalidTierTransition(_)));
    assert!(responder.audit().is_empty());
    assert!(registry.audit_trail().is_empty());
}

//
// Verify that priority reductions feed the anti-pheromone field through its
// typed reporting path, without any tier movement.
//
// Given a circularity finding on a support relationship,
// when the responder reduces its traversal priority with a poisoning signal,
// then the anti-pheromone vector should show the reported dimension and the
// response should carry no tier transition.
#[test]
fn priority_reductions_feed_the_anti_pheromone_field() {
    let mut responder = ImmuneResponder::new();
    let mut field = anti_field();
    let edge = relationship_id("relationship--deprioritized");
    let circularity_finding = finding(
        "immune-epistemic--source-circularity",
        ValidationTarget::relationship(edge.as_str()),
    );

    let response = responder
        .reduce_priority(
            &mut field,
            &scope(),
            &circularity_finding,
            AntiPheromoneSignal::SuspectedPoisoning,
        )
        .expect("priority reduction should be recorded")
        .clone();

    let vector = field
        .edge_anti_pheromone(&edge, &scope())
        .expect("the edge should carry the reported signal");
    assert_eq!(vector.suspected_poisoning, 1.0);
    assert!(matches!(
        response.action,
        ImmuneResponseAction::ReducePriority {
            signal: AntiPheromoneSignal::SuspectedPoisoning,
        }
    ));
    assert!(response.tier_transition_sequence.is_none());
}

//
// Verify that priority reduction only applies to relationship targets: the
// anti-pheromone field is an edge field.
//
// Given a finding targeting a node,
// when priority reduction is attempted,
// then it should fail with a typed error and record nothing.
#[test]
fn priority_reduction_rejects_non_relationship_targets() {
    let mut responder = ImmuneResponder::new();
    let mut field = anti_field();
    let node_finding = finding(
        "immune-epistemic--unsupported-claim",
        ValidationTarget::node("node--claim"),
    );

    let error = responder
        .reduce_priority(
            &mut field,
            &scope(),
            &node_finding,
            AntiPheromoneSignal::StaleEvidence,
        )
        .expect_err("non-relationship targets should fail");

    assert!(matches!(error, GraphError::InvalidTierTransition(_)));
    assert!(responder.audit().is_empty());
}

//
// Verify that verification requests are recorded as responses referencing the
// probe that will answer them, mutating neither tiers nor fields.
//
// Given an unsupported-claim finding,
// when the responder requests verification with a probe reference,
// then the response should carry the probe reference and no transition.
#[test]
fn verification_requests_reference_their_probe() {
    let mut responder = ImmuneResponder::new();
    let unsupported_finding = finding(
        "immune-epistemic--unsupported-claim",
        ValidationTarget::node("node--claim-under-review"),
    );

    let response = responder
        .request_verification(&unsupported_finding, "probe--still-supported")
        .clone();

    assert!(matches!(
        &response.action,
        ImmuneResponseAction::RequestVerification { probe_ref } if probe_ref == "probe--still-supported"
    ));
    assert!(response.tier_transition_sequence.is_none());
    assert_eq!(responder.audit().len(), 1);
}

//
// Verify that repair proposals land in the shadow tier referencing their
// finding, leave the defective canonical record untouched, and only reach
// canonical through an explicit audited promotion.
//
// Given a schema-violation finding and a proposed replacement relationship,
// when the responder proposes the repair and an analyst later promotes it,
// then the proposal should sit in shadow first, the defective record should
// stay canonical, and the promotion should be a distinct audited transition.
#[test]
fn repair_proposals_land_in_shadow_awaiting_promotion() {
    let mut responder = ImmuneResponder::new();
    let mut registry = GraphTierRegistry::new();
    let defective = relationship_id("relationship--defective");
    let proposal = TierRecordRef::Relationship(relationship_id("relationship--proposed-fix"));
    let schema_finding = finding(
        "immune-structural--schema-violation",
        ValidationTarget::relationship(defective.as_str()),
    );

    let response = responder
        .propose_repair(
            &mut registry,
            &schema_finding,
            proposal.clone(),
            "immune--repair",
        )
        .expect("repair proposal should be recorded")
        .clone();

    assert_eq!(registry.tier_of(&proposal), GraphTier::Shadow);
    assert_eq!(
        registry.tier_of(&TierRecordRef::Relationship(defective)),
        GraphTier::Canonical,
        "the defective canonical record must not be touched by the proposal"
    );
    assert!(matches!(
        &response.action,
        ImmuneResponseAction::ProposeRepair { proposal: recorded } if recorded == &proposal
    ));

    registry
        .transition(
            proposal.clone(),
            GraphTier::Canonical,
            "analyst--review",
            TierTransitionReason::AuditedPromotion,
        )
        .expect("audited promotion should succeed");
    assert_eq!(registry.tier_of(&proposal), GraphTier::Canonical);
}

//
// Verify the ordered audit: responses carry strictly increasing sequences
// linking finding, response, and transition, and identical operation
// sequences produce identical responders.
//
// Given a quarantine followed by a verification request, twice,
// when the audits are compared,
// then sequences should increase and both responders should be equal.
#[test]
fn response_audit_is_ordered_and_reproducible() {
    let build = || {
        let mut responder = ImmuneResponder::new();
        let mut registry = GraphTierRegistry::new();
        responder
            .quarantine(
                &mut registry,
                &finding(
                    "immune-structural--dangling-link",
                    ValidationTarget::relationship("relationship--audited"),
                ),
                "immune--responder",
            )
            .expect("quarantine should be recorded");
        responder.request_verification(
            &finding(
                "immune-epistemic--unsupported-claim",
                ValidationTarget::node("node--audited"),
            ),
            "probe--audited",
        );
        responder
    };

    let first = build();
    let second = build();

    assert_eq!(first.audit().len(), 2);
    assert!(first.audit()[0].sequence < first.audit()[1].sequence);
    assert_eq!(first, second);
}

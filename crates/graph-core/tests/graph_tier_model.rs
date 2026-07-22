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
    ClaimId, EvidenceId, GraphError, GraphTier, GraphTierRegistry, NodeId, RelationshipId,
    TierRecordRef, TierTransitionReason,
};

fn node_ref(value: &str) -> TierRecordRef {
    TierRecordRef::Node(NodeId::new(value).expect("tier node ID should be valid"))
}

fn relationship_ref(value: &str) -> TierRecordRef {
    TierRecordRef::Relationship(
        RelationshipId::new(value).expect("tier relationship ID should be valid"),
    )
}

fn claim_ref(value: &str) -> TierRecordRef {
    TierRecordRef::Claim(ClaimId::new(value).expect("tier claim ID should be valid"))
}

fn evidence_ref(value: &str) -> TierRecordRef {
    TierRecordRef::Evidence(EvidenceId::new(value).expect("tier evidence ID should be valid"))
}

//
// Verify that the tier vocabulary is exactly the epic's four tiers in stable
// order, so tier reports diff cleanly.
//
// Given the exported tier set,
// when its variants are enumerated,
// then it should contain canonical, shadow, quarantine, and hypothesis.
#[test]
fn tier_vocabulary_is_complete_and_stable() {
    assert_eq!(GraphTier::ALL.len(), 4);
    for tier in [
        GraphTier::Canonical,
        GraphTier::Shadow,
        GraphTier::Quarantine,
        GraphTier::Hypothesis,
    ] {
        assert!(GraphTier::ALL.contains(&tier));
    }
}

//
// Verify that existing data is canonical by default: a record with no
// transition history reads as validated canonical data.
//
// Given an empty registry,
// when the tier of an untracked record is read,
// then it should be canonical with an empty audit trail.
#[test]
fn records_default_to_the_canonical_tier() {
    let registry = GraphTierRegistry::new();
    let record = node_ref("node--untracked");

    assert_eq!(registry.tier_of(&record), GraphTier::Canonical);
    assert!(registry.audit_for(&record).is_empty());
    assert!(registry.audit_trail().is_empty());
}

//
// Verify that every tier transition is appended to the audit trail with its
// actor, typed reason, endpoints, and a strictly increasing sequence.
//
// Given quarantine and shadow transitions on two records,
// when the audit trail is read,
// then both transitions should appear in order with their full context.
#[test]
fn transitions_are_audited_append_only() {
    let mut registry = GraphTierRegistry::new();
    let suspect = relationship_ref("relationship--suspect");
    let proposal = node_ref("node--proposed-merge");

    registry
        .transition(
            suspect.clone(),
            GraphTier::Quarantine,
            "validator--epistemic",
            TierTransitionReason::ValidatorFinding,
        )
        .expect("quarantine transition should be recorded");
    registry
        .transition(
            proposal.clone(),
            GraphTier::Shadow,
            "immune--repair",
            TierTransitionReason::RepairProposal,
        )
        .expect("shadow transition should be recorded");

    let trail = registry.audit_trail();
    assert_eq!(trail.len(), 2);
    assert_eq!(trail[0].record, suspect);
    assert_eq!(trail[0].from, GraphTier::Canonical);
    assert_eq!(trail[0].to, GraphTier::Quarantine);
    assert_eq!(trail[0].actor_ref, "validator--epistemic");
    assert_eq!(trail[0].reason, TierTransitionReason::ValidatorFinding);
    assert_eq!(trail[1].record, proposal);
    assert!(trail[0].sequence < trail[1].sequence);

    assert_eq!(registry.tier_of(&suspect), GraphTier::Quarantine);
    assert_eq!(registry.tier_of(&proposal), GraphTier::Shadow);
}

//
// Verify the canonical-promotion guard: entering the canonical tier requires
// the explicit audited-promotion reason, so immune actions can never slip
// corrections into canonical silently.
//
// Given a quarantined record,
// when promotion is attempted with a non-promotion reason and then with the
// audited-promotion reason,
// then the first should fail with the typed error and the second should land
// with its audit entry.
#[test]
fn canonical_promotion_requires_the_audited_promotion_reason() {
    let mut registry = GraphTierRegistry::new();
    let record = claim_ref("claim--under-review");

    registry
        .transition(
            record.clone(),
            GraphTier::Quarantine,
            "validator--epistemic",
            TierTransitionReason::ValidatorFinding,
        )
        .expect("quarantine transition should be recorded");

    let error = registry
        .transition(
            record.clone(),
            GraphTier::Canonical,
            "validator--epistemic",
            TierTransitionReason::ValidatorFinding,
        )
        .expect_err("unaudited canonical promotion should fail");
    assert!(matches!(error, GraphError::InvalidTierTransition(_)));
    assert_eq!(registry.tier_of(&record), GraphTier::Quarantine);

    registry
        .transition(
            record.clone(),
            GraphTier::Canonical,
            "analyst--review",
            TierTransitionReason::AuditedPromotion,
        )
        .expect("audited promotion should be recorded");
    assert_eq!(registry.tier_of(&record), GraphTier::Canonical);
    assert_eq!(registry.audit_for(&record).len(), 2);
}

//
// Verify that no-op transitions are rejected: staying in the same tier is not
// a transition and must not pollute the audit trail.
//
// Given a record in its default canonical tier,
// when a transition to canonical is attempted,
// then it should fail with the typed error and leave the trail empty.
#[test]
fn same_tier_transitions_are_rejected() {
    let mut registry = GraphTierRegistry::new();
    let record = node_ref("node--already-canonical");

    let error = registry
        .transition(
            record,
            GraphTier::Canonical,
            "immune--noop",
            TierTransitionReason::AuditedPromotion,
        )
        .expect_err("same-tier transition should fail");

    assert!(matches!(error, GraphError::InvalidTierTransition(_)));
    assert!(registry.audit_trail().is_empty());
}

//
// Verify that graph and epistemic records are all trackable: nodes,
// relationships, claims, and evidence each carry a tier.
//
// Given one record of each kind moved to a non-canonical tier,
// when their tiers are read,
// then each should report its assigned tier.
#[test]
fn all_record_kinds_are_tier_trackable() {
    let mut registry = GraphTierRegistry::new();
    let records = [
        (node_ref("node--tracked"), GraphTier::Hypothesis),
        (relationship_ref("relationship--tracked"), GraphTier::Shadow),
        (claim_ref("claim--tracked"), GraphTier::Quarantine),
        (evidence_ref("evidence--tracked"), GraphTier::Quarantine),
    ];

    for (record, tier) in &records {
        registry
            .transition(
                record.clone(),
                *tier,
                "immune--seeder",
                TierTransitionReason::ValidatorFinding,
            )
            .expect("transition should be recorded");
    }

    for (record, tier) in &records {
        assert_eq!(registry.tier_of(record), *tier);
    }
}

//
// Verify deterministic tier listing: records in a tier are listed in
// first-transition order, and identically built registries are equal.
//
// Given two records quarantined in order,
// when the quarantine tier is listed on two identical registries,
// then both listings should be equal and ordered by first transition.
#[test]
fn tier_listings_are_deterministic() {
    let build = || {
        let mut registry = GraphTierRegistry::new();
        for value in ["relationship--poisoned-a", "relationship--poisoned-b"] {
            registry
                .transition(
                    relationship_ref(value),
                    GraphTier::Quarantine,
                    "validator--behavioral",
                    TierTransitionReason::ValidatorFinding,
                )
                .expect("transition should be recorded");
        }
        registry
    };

    let first = build();
    let second = build();

    let quarantined = first.records_in_tier(GraphTier::Quarantine);
    assert_eq!(
        quarantined,
        vec![
            relationship_ref("relationship--poisoned-a"),
            relationship_ref("relationship--poisoned-b"),
        ]
    );
    assert_eq!(first, second);
    assert_eq!(
        first.records_in_tier(GraphTier::Quarantine),
        second.records_in_tier(GraphTier::Quarantine)
    );
    assert!(first.records_in_tier(GraphTier::Shadow).is_empty());
}

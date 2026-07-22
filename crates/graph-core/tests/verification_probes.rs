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
    GraphError, GraphTier, GraphTierRegistry, ImmuneResponder, ProbeAnswer, ProbeKind,
    ProbeRegistry, ProbeStatus, RelationshipId, TierRecordRef, TierTransitionReason,
    ValidationErrorRecord, ValidationErrorSeverity, ValidationTarget,
};

fn finding(code: &str, target: ValidationTarget) -> ValidationErrorRecord {
    ValidationErrorRecord::new(
        code,
        ValidationErrorSeverity::Warning,
        "seeded probe finding",
        target,
    )
}

fn seeded_findings() -> Vec<ValidationErrorRecord> {
    vec![
        finding(
            "immune-epistemic--unsupported-claim",
            ValidationTarget::node("node--claim-unsupported"),
        ),
        finding(
            "immune-epistemic--source-circularity",
            ValidationTarget::node("node--claim-circular"),
        ),
        finding(
            "immune-epistemic--open-contradiction",
            ValidationTarget::relationship("relationship--contradicts"),
        ),
        finding(
            "immune-epistemic--duplicate-suspect",
            ValidationTarget::node("node--maybe-duplicate"),
        ),
    ]
}

//
// Verify that probes are generated deterministically from findings using the
// documented mapping, each carrying its typed kind, question, target, and the
// finding that generated it.
//
// Given one finding of each mapped class,
// when probes are generated,
// then four probes should exist with the mapped kinds and open status.
#[test]
fn probes_are_generated_from_findings_with_typed_kinds() {
    let mut registry = ProbeRegistry::new();

    let refs = registry.generate_from_findings(&seeded_findings());

    assert_eq!(refs.len(), 4);
    let kinds: Vec<ProbeKind> = refs
        .iter()
        .map(|probe_ref| {
            registry
                .probe(probe_ref)
                .expect("generated probe should exist")
                .kind
        })
        .collect();
    assert_eq!(
        kinds,
        vec![
            ProbeKind::StillSupported,
            ProbeKind::CircularDependency,
            ProbeKind::IndependentSource,
            ProbeKind::TrulyIdentical,
        ]
    );

    let first = registry.probe(&refs[0]).expect("probe should exist");
    assert_eq!(first.finding_code, "immune-epistemic--unsupported-claim");
    assert_eq!(
        first.target,
        ValidationTarget::node("node--claim-unsupported")
    );
    assert_eq!(first.status, ProbeStatus::Open);
    assert!(
        first.question.contains("node--claim-unsupported"),
        "the typed question should name its target"
    );
}

//
// Verify idempotent generation: an open probe of the same kind and target is
// never duplicated, so repeated validation passes do not flood the registry.
//
// Given the same findings generated twice,
// when the registry is inspected,
// then only the first generation should have produced probes.
#[test]
fn generation_is_idempotent_per_kind_and_target() {
    let mut registry = ProbeRegistry::new();

    let first = registry.generate_from_findings(&seeded_findings());
    let second = registry.generate_from_findings(&seeded_findings());

    assert_eq!(first.len(), 4);
    assert!(second.is_empty());
    assert_eq!(registry.probes().len(), 4);
}

//
// Verify that unmapped finding codes generate nothing: probe generation is a
// closed, documented mapping, not a catch-all.
//
// Given structural findings with no probe mapping,
// when probes are generated,
// then the registry should stay empty.
#[test]
fn unmapped_finding_codes_generate_no_probes() {
    let mut registry = ProbeRegistry::new();

    let refs = registry.generate_from_findings(&[finding(
        "immune-structural--dangling-link",
        ValidationTarget::relationship("relationship--dangling"),
    )]);

    assert!(refs.is_empty());
    assert!(registry.probes().is_empty());
}

//
// Verify the answered lifecycle: answering an open probe records the answer,
// the justification linkage, and an audited lifecycle transition.
//
// Given a generated probe,
// when it is answered as supported justifying a promotion,
// then the probe should be answered with its justification and the lifecycle
// log should carry the transition.
#[test]
fn answering_a_probe_records_answer_and_justification() {
    let mut registry = ProbeRegistry::new();
    let refs = registry.generate_from_findings(&seeded_findings());
    let probe_ref = &refs[0];

    registry
        .answer(
            probe_ref,
            ProbeAnswer::Supported,
            Some("transition--3".to_owned()),
        )
        .expect("open probe should be answerable");

    let probe = registry.probe(probe_ref).expect("probe should exist");
    assert_eq!(probe.status, ProbeStatus::Answered(ProbeAnswer::Supported));
    assert_eq!(probe.justifies.as_deref(), Some("transition--3"));

    let lifecycle = registry.lifecycle();
    assert_eq!(lifecycle.len(), 1);
    assert_eq!(&lifecycle[0].probe_ref, probe_ref);
    assert_eq!(
        lifecycle[0].to,
        ProbeStatus::Answered(ProbeAnswer::Supported)
    );
}

//
// Verify that terminal probes reject further transitions with a typed error:
// the lifecycle is append-only, never rewritten.
//
// Given an answered probe,
// when expiring or re-answering is attempted,
// then each attempt should fail with `GraphError::InvalidProbeTransition`.
#[test]
fn terminal_probes_reject_further_transitions() {
    let mut registry = ProbeRegistry::new();
    let refs = registry.generate_from_findings(&seeded_findings());
    let probe_ref = &refs[0];
    registry
        .answer(probe_ref, ProbeAnswer::Refuted, None)
        .expect("open probe should be answerable");

    let error = registry
        .expire(probe_ref)
        .expect_err("answered probes should not expire");
    assert!(matches!(error, GraphError::InvalidProbeTransition(_)));

    let error = registry
        .answer(probe_ref, ProbeAnswer::Supported, None)
        .expect_err("answered probes should not be re-answered");
    assert!(matches!(error, GraphError::InvalidProbeTransition(_)));

    assert_eq!(registry.lifecycle().len(), 1);
}

//
// Verify expiry: open probes can expire, and unknown probe references are a
// typed error.
//
// Given a generated probe and an unknown reference,
// when the probe expires and the unknown reference is answered,
// then the probe should be expired and the unknown reference should fail.
#[test]
fn open_probes_expire_and_unknown_refs_are_typed_errors() {
    let mut registry = ProbeRegistry::new();
    let refs = registry.generate_from_findings(&seeded_findings());
    let probe_ref = &refs[1];

    registry
        .expire(probe_ref)
        .expect("open probe should expire");
    assert_eq!(
        registry
            .probe(probe_ref)
            .expect("probe should exist")
            .status,
        ProbeStatus::Expired
    );

    let error = registry
        .answer("probe--unknown", ProbeAnswer::Supported, None)
        .expect_err("unknown probes should fail");
    assert!(matches!(error, GraphError::InvalidProbeTransition(_)));
}

//
// Verify the end-to-end verification loop: a finding requests verification
// through the responder, the probe answer justifies an audited promotion out
// of quarantine.
//
// Given a quarantined relationship whose finding generated a probe,
// when the probe is answered as supported and the record is promoted with the
// verification-outcome reason,
// then the record should be canonical again and the probe should justify the
// promotion's transition sequence.
#[test]
fn answered_probes_justify_audited_promotions() {
    let mut tier_registry = GraphTierRegistry::new();
    let mut responder = ImmuneResponder::new();
    let mut probes = ProbeRegistry::new();
    let suspect = RelationshipId::new("relationship--under-verification")
        .expect("relationship ID should be valid");
    let suspect_finding = finding(
        "immune-epistemic--stale-evidence",
        ValidationTarget::relationship(suspect.as_str()),
    );

    responder
        .quarantine(&mut tier_registry, &suspect_finding, "immune--responder")
        .expect("quarantine should be recorded");
    let refs = probes.generate_from_findings(std::slice::from_ref(&suspect_finding));
    assert_eq!(refs.len(), 1);
    responder.request_verification(&suspect_finding, refs[0].clone());

    let record = TierRecordRef::Relationship(suspect);
    let promotion_sequence = tier_registry
        .transition(
            record.clone(),
            GraphTier::Canonical,
            "analyst--review",
            TierTransitionReason::AuditedPromotion,
        )
        .expect("audited promotion should succeed");
    probes
        .answer(
            &refs[0],
            ProbeAnswer::Supported,
            Some(format!("transition--{promotion_sequence}")),
        )
        .expect("probe should be answerable");

    assert_eq!(tier_registry.tier_of(&record), GraphTier::Canonical);
    let probe = probes.probe(&refs[0]).expect("probe should exist");
    assert_eq!(
        probe.justifies.as_deref(),
        Some(format!("transition--{promotion_sequence}").as_str())
    );
}

//
// Verify reproducibility: identical generation and lifecycle sequences yield
// identical registries.
//
// Given two registries driven by the same operations,
// when they are compared,
// then they should be exactly equal.
#[test]
fn identical_operation_sequences_produce_identical_registries() {
    let build = || {
        let mut registry = ProbeRegistry::new();
        let refs = registry.generate_from_findings(&seeded_findings());
        registry
            .answer(
                &refs[0],
                ProbeAnswer::Supported,
                Some("transition--1".to_owned()),
            )
            .expect("probe should be answerable");
        registry.expire(&refs[2]).expect("probe should expire");
        registry
    };

    assert_eq!(build(), build());
}

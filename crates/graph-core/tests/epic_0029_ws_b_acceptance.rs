// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//! Acceptance suite for Epic 0029 workstream WS-B (issue #168, epic #162).
//!
//! The suite closes the core acceptance boundary. Per-verifier format and
//! consistency cases remain in the deterministic verifier suites; ABI loading
//! and provider compatibility remain in the ABI and HTTP server suites.
#![allow(clippy::unwrap_used)]

use graph_core::{
    ARITHMETIC_CONSISTENCY_VERIFIER_ID, ARITHMETIC_CONSISTENCY_VERIFIER_VERSION, BitemporalStamp,
    CONTENT_HASH_VERIFIER_ID, CONTENT_HASH_VERIFIER_VERSION, ClaimAnalyticalTarget, ClaimId,
    ClaimInput, ClaimLink, ClaimLinkKind, ClaimLinkSource, ClaimStatement, ClaimStore, ClaimTarget,
    Confidence, EvidenceRecordStore, EvidenceSourceType, GRAPH_CONSISTENCY_VERIFIER_ID,
    GRAPH_CONSISTENCY_VERIFIER_VERSION, GraphError, IDENTIFIER_SYNTAX_VERIFIER_ID,
    IDENTIFIER_SYNTAX_VERIFIER_VERSION, ObservationId, ObservationInput, ObservationModality,
    ObservationStore, ResolutionInputs, SCHEMA_CONSTRAINT_VERIFIER_ID,
    SCHEMA_CONSTRAINT_VERIFIER_VERSION, SourceId, SourceInput, SourceStore,
    TEMPORAL_ORDERING_VERIFIER_ID, TEMPORAL_ORDERING_VERIFIER_VERSION, TemporalTimestamp,
    VerdictState, VerdictStore, VerificationContext, VerificationCoverage,
    VerificationCoverageClass, VerificationInputs, VerificationOutcome, VerificationRecord,
    VerificationRecordId, VerificationRecordStore, VerificationRequest, VerificationResult,
    Verifier, VerifierCostClass, VerifierRegistry, VerifierSpec, resolve_claim_verdict,
};

fn ts(value: &str) -> TemporalTimestamp {
    TemporalTimestamp::new(value).unwrap()
}

fn stamp(system_time: &str) -> BitemporalStamp {
    BitemporalStamp::new(ts("2026-09-06T00:00:00Z"), ts(system_time)).unwrap()
}

struct FixedVerifier {
    id: &'static str,
    version: &'static str,
    deterministic: bool,
    result: VerificationResult,
}

impl Verifier for FixedVerifier {
    fn id(&self) -> &str {
        self.id
    }

    fn version(&self) -> &str {
        self.version
    }

    fn deterministic(&self) -> bool {
        self.deterministic
    }

    fn cost_class(&self) -> VerifierCostClass {
        VerifierCostClass::Low
    }

    fn verify(&self, request: &VerificationRequest<'_>) -> Result<VerificationOutcome, GraphError> {
        Ok(VerificationOutcome::new(self.result)
            .with_rationale("acceptance fixture")
            .with_limit("fixture only")
            .with_evidence_consumed(format!("claim:{}", request.claim().id().as_str())))
    }
}

struct Fixture {
    claims: ClaimStore,
    observations: ObservationStore,
    sources: SourceStore,
    evidence: EvidenceRecordStore,
    verifications: VerificationRecordStore,
    verdicts: VerdictStore,
}

impl Fixture {
    fn new() -> Self {
        let source = SourceId::new("source--ws-b").unwrap();
        let observation = ObservationId::new("observation--ws-b").unwrap();
        let mut sources = SourceStore::new();
        sources
            .register_source(SourceInput::new(
                source.clone(),
                "https://example.test/ws-b",
                EvidenceSourceType::Document,
            ))
            .unwrap();
        let mut observations = ObservationStore::new();
        observations
            .create_observation(
                ObservationInput::new(
                    observation.clone(),
                    source,
                    "A grounded acceptance observation.",
                    ObservationModality::Text,
                ),
                &sources,
            )
            .unwrap();
        let mut claims = ClaimStore::new();
        claims.register_observation(observation.clone());
        for id in [
            "claim--deterministic-fail",
            "claim--deterministic-pass",
            "claim--mechanical",
            "claim--semantic",
            "claim--unchecked",
            "claim--failing",
            "claim--pack",
        ] {
            claims
                .create_asserted_claim(
                    ClaimInput::new(
                        ClaimId::new(id).unwrap(),
                        ClaimStatement::new(format!("Acceptance claim {id}")).unwrap(),
                        ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new(id, None)),
                    )
                    .with_confidence(Confidence::new(0.99).unwrap()),
                )
                .unwrap();
            claims
                .attach_link(ClaimLink::new(
                    ClaimLinkSource::Observation(observation.clone()),
                    ClaimId::new(id).unwrap(),
                    ClaimLinkKind::Supports,
                ))
                .unwrap();
        }
        Self {
            claims,
            observations,
            sources,
            evidence: EvidenceRecordStore::new(),
            verifications: VerificationRecordStore::new(),
            verdicts: VerdictStore::new(),
        }
    }

    fn run(
        &mut self,
        registry: &VerifierRegistry,
        verifier_id: &str,
        version: &str,
        claim_id: &str,
        system_time: &str,
    ) {
        let context = VerificationContext::new(
            &self.claims,
            &self.observations,
            &self.sources,
            &self.evidence,
        );
        registry
            .run(
                verifier_id,
                version,
                &ClaimId::new(claim_id).unwrap(),
                &context,
                &mut self.verifications,
                stamp(system_time),
            )
            .unwrap();
    }

    fn resolve(&mut self, claim_id: &str, system_time: &str) -> VerdictState {
        let claim_id = ClaimId::new(claim_id).unwrap();
        let outcome = resolve_claim_verdict(
            &mut self.claims,
            &mut self.verdicts,
            &ResolutionInputs::new(
                &self.verifications,
                &self.evidence,
                &self.observations,
                &self.sources,
            ),
            &claim_id,
            stamp(system_time),
            "deterministic-first-v1",
        )
        .unwrap();
        outcome.state()
    }
}

fn fixed(
    id: &'static str,
    version: &'static str,
    deterministic: bool,
    result: VerificationResult,
) -> VerifierSpec {
    VerifierSpec::new(Box::new(FixedVerifier {
        id,
        version,
        deterministic,
        result,
    }))
}

fn append_record(
    records: &mut VerificationRecordStore,
    id: &str,
    claim_id: &str,
    verifier_id: &str,
    deterministic: bool,
    result: VerificationResult,
) {
    records
        .append(VerificationRecord::new(
            VerificationRecordId::new(id).unwrap(),
            verifier_id,
            "1.0.0",
            deterministic,
            VerificationInputs::for_claim(ClaimId::new(claim_id).unwrap()),
            result,
            stamp("2026-09-06T00:01:00Z"),
        ))
        .unwrap();
}

#[test]
fn every_core_verifier_has_a_stable_identifier_and_version() {
    let declarations = [
        (
            IDENTIFIER_SYNTAX_VERIFIER_ID,
            IDENTIFIER_SYNTAX_VERIFIER_VERSION,
        ),
        (CONTENT_HASH_VERIFIER_ID, CONTENT_HASH_VERIFIER_VERSION),
        (
            TEMPORAL_ORDERING_VERIFIER_ID,
            TEMPORAL_ORDERING_VERIFIER_VERSION,
        ),
        (
            ARITHMETIC_CONSISTENCY_VERIFIER_ID,
            ARITHMETIC_CONSISTENCY_VERIFIER_VERSION,
        ),
        (
            GRAPH_CONSISTENCY_VERIFIER_ID,
            GRAPH_CONSISTENCY_VERIFIER_VERSION,
        ),
        (
            SCHEMA_CONSTRAINT_VERIFIER_ID,
            SCHEMA_CONSTRAINT_VERIFIER_VERSION,
        ),
    ];
    let ids = declarations
        .iter()
        .map(|(id, _)| *id)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(ids.len(), declarations.len());
    assert!(declarations.iter().all(|(_, version)| *version == "1.0.0"));
}

#[test]
fn versioned_records_coexist_and_coverage_selects_current_logic() {
    let mut fixture = Fixture::new();
    let mut registry = VerifierRegistry::new();
    registry
        .register(fixed(
            "verifier.acceptance-version",
            "1.0.0",
            true,
            VerificationResult::Pass,
        ))
        .unwrap();
    registry
        .register(fixed(
            "verifier.acceptance-version",
            "2.0.0",
            true,
            VerificationResult::Fail,
        ))
        .unwrap();
    fixture.run(
        &registry,
        "verifier.acceptance-version",
        "1.0.0",
        "claim--failing",
        "2026-09-06T00:01:00Z",
    );
    fixture.run(
        &registry,
        "verifier.acceptance-version",
        "2.0.0",
        "claim--failing",
        "2026-09-06T00:02:00Z",
    );

    let claim = fixture
        .claims
        .claim_by_id(&ClaimId::new("claim--failing").unwrap())
        .unwrap();
    let coverage = VerificationCoverage::derive(claim, &fixture.verifications);
    assert_eq!(fixture.verifications.records_for_claim(claim.id()).len(), 2);
    assert_eq!(coverage.entries().len(), 1);
    assert_eq!(coverage.entries()[0].verifier_version(), Some("2.0.0"));
    assert_eq!(
        coverage.entries()[0].class(),
        VerificationCoverageClass::Failing
    );
}

#[test]
fn deterministic_authority_wins_both_disagreement_directions() {
    let mut fixture = Fixture::new();
    let mut registry = VerifierRegistry::new();
    for spec in [
        fixed(
            "verifier.mechanical-fail",
            "1.0.0",
            true,
            VerificationResult::Fail,
        ),
        fixed(
            "model.semantic-pass",
            "1.0.0",
            false,
            VerificationResult::Pass,
        ),
        fixed(
            "verifier.mechanical-pass",
            "1.0.0",
            true,
            VerificationResult::Pass,
        ),
        fixed(
            "model.semantic-fail",
            "1.0.0",
            false,
            VerificationResult::Fail,
        ),
    ] {
        registry.register(spec).unwrap();
    }

    fixture.run(
        &registry,
        "verifier.mechanical-fail",
        "1.0.0",
        "claim--deterministic-fail",
        "2026-09-06T00:01:00Z",
    );
    fixture.run(
        &registry,
        "model.semantic-pass",
        "1.0.0",
        "claim--deterministic-fail",
        "2026-09-06T00:02:00Z",
    );
    let blocked = fixture.resolve("claim--deterministic-fail", "2026-09-06T00:03:00Z");
    assert_ne!(blocked, VerdictState::Supported);

    fixture.run(
        &registry,
        "verifier.mechanical-pass",
        "1.0.0",
        "claim--deterministic-pass",
        "2026-09-06T00:04:00Z",
    );
    fixture.run(
        &registry,
        "model.semantic-fail",
        "1.0.0",
        "claim--deterministic-pass",
        "2026-09-06T00:05:00Z",
    );
    assert_eq!(
        fixture.resolve("claim--deterministic-pass", "2026-09-06T00:06:00Z"),
        VerdictState::Supported
    );
    assert_eq!(
        fixture
            .verdicts
            .verification_disagreements_for_claim(
                &ClaimId::new("claim--deterministic-pass").unwrap()
            )
            .len(),
        1
    );
}

#[test]
fn every_acceptance_claim_has_explicit_coverage_and_pack_absence_only_changes_coverage() {
    let mut fixture = Fixture::new();
    append_record(
        &mut fixture.verifications,
        "verification--mechanical",
        "claim--mechanical",
        "verifier.identifier-syntax",
        true,
        VerificationResult::Pass,
    );
    append_record(
        &mut fixture.verifications,
        "verification--semantic",
        "claim--semantic",
        "fr.estance.corrobore.domain.cti.claim.verify",
        false,
        VerificationResult::Pass,
    );
    append_record(
        &mut fixture.verifications,
        "verification--failing",
        "claim--failing",
        "verifier.schema-constraint",
        true,
        VerificationResult::Fail,
    );

    let expected = [
        (
            "claim--mechanical",
            VerificationCoverageClass::MechanicallyChecked,
        ),
        (
            "claim--semantic",
            VerificationCoverageClass::SemanticallyJudged,
        ),
        ("claim--unchecked", VerificationCoverageClass::Unchecked),
        ("claim--failing", VerificationCoverageClass::Failing),
        ("claim--pack", VerificationCoverageClass::Unchecked),
    ];
    for (claim_id, class) in expected {
        let claim = fixture
            .claims
            .claim_by_id(&ClaimId::new(claim_id).unwrap())
            .unwrap();
        assert_eq!(
            VerificationCoverage::derive(claim, &fixture.verifications).entries()[0].class(),
            class,
            "{claim_id}"
        );
    }

    let baseline = fixture.resolve("claim--pack", "2026-09-06T00:02:00Z");
    append_record(
        &mut fixture.verifications,
        "verification--pack",
        "claim--pack",
        "fr.estance.corrobore.domain.cti.claim.verify",
        false,
        VerificationResult::Fail,
    );
    let with_pack = fixture.resolve("claim--pack", "2026-09-06T00:03:00Z");
    assert_eq!(
        with_pack, baseline,
        "advisory pack result changes no verdict"
    );
    let claim = fixture
        .claims
        .claim_by_id(&ClaimId::new("claim--pack").unwrap())
        .unwrap();
    assert_eq!(
        VerificationCoverage::derive(claim, &fixture.verifications).entries()[0].class(),
        VerificationCoverageClass::Failing
    );
}

#[test]
fn graph_core_keeps_model_runtimes_outside_its_dependency_boundary() {
    let manifest = include_str!("../Cargo.toml");
    for forbidden in ["model-runtime", "llm", "openai", "anthropic"] {
        assert!(
            !manifest.contains(forbidden),
            "graph-core must not depend on {forbidden}"
        );
    }
}

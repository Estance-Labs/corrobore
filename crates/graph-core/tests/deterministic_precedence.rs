// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//! Deterministic-first verdict precedence contract (Epic 0029, WS-B item 4,
//! issue #166).
//!
//! These tests keep the authority boundary in the verdict engine: mechanical
//! failures cannot be lifted by model confidence, model failures cannot
//! downgrade mechanical success, inconclusive results carry no weight, and
//! every deterministic/advisory disagreement remains queryable.

use graph_core::{
    BitemporalStamp, ClaimAnalyticalTarget, ClaimId, ClaimInput, ClaimStatement, ClaimStatus,
    ClaimStore, ClaimTarget, Confidence, EvidenceRecordStore, EvidenceSourceType, ObservationId,
    ObservationInput, ObservationModality, ObservationStore, ResolutionInputs, SourceId,
    SourceInput, SourceStore, TemporalTimestamp, VERIFICATION_AUTHORITY_DISAGREEMENT_CODE,
    ValidationErrorSeverity, VerdictState, VerdictStore, VerificationInputs, VerificationRecord,
    VerificationRecordId, VerificationRecordStore, VerificationResult, resolve_claim_verdict,
};

fn ts(value: &str) -> TemporalTimestamp {
    TemporalTimestamp::new(value).expect("test timestamp should be valid")
}

fn stamp(transaction: &str) -> BitemporalStamp {
    BitemporalStamp::new(ts("2026-09-01T00:00:00Z"), ts(transaction))
        .expect("test stamp should be valid")
}

struct Fixture {
    claim: ClaimId,
    observation: ObservationId,
    claims: ClaimStore,
    verdicts: VerdictStore,
    verifications: VerificationRecordStore,
    evidence: EvidenceRecordStore,
    observations: ObservationStore,
    sources: SourceStore,
}

impl Fixture {
    fn new() -> Self {
        let source = SourceId::new("source--fixture").expect("source id");
        let observation = ObservationId::new("observation--fixture").expect("observation id");
        let claim = ClaimId::new("claim--authority-boundary").expect("claim id");

        let mut sources = SourceStore::new();
        sources
            .register_source(SourceInput::new(
                source.clone(),
                "https://example.test/report",
                EvidenceSourceType::Document,
            ))
            .expect("source should register");

        let mut observations = ObservationStore::new();
        observations
            .create_observation(
                ObservationInput::new(
                    observation.clone(),
                    source,
                    "The machine-checkable proposition under review.",
                    ObservationModality::Text,
                ),
                &sources,
            )
            .expect("observation should be created");

        let mut claims = ClaimStore::new();
        claims.register_observation(observation.clone());
        claims
            .create_asserted_claim(
                ClaimInput::new(
                    claim.clone(),
                    ClaimStatement::new("The proposition is true.").expect("claim statement"),
                    ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new(
                        "authority-boundary",
                        None,
                    )),
                )
                .with_confidence(Confidence::new(0.99).expect("model score should be bounded")),
            )
            .expect("claim should be created");

        Self {
            claim,
            observation,
            claims,
            verdicts: VerdictStore::new(),
            verifications: VerificationRecordStore::new(),
            evidence: EvidenceRecordStore::new(),
            observations,
            sources,
        }
    }

    fn append(
        &mut self,
        id: &str,
        verifier_id: &str,
        version: &str,
        deterministic: bool,
        result: VerificationResult,
        transaction: &str,
    ) {
        let inputs = VerificationInputs::for_claim(self.claim.clone())
            .with_observation(self.observation.clone());
        let record = VerificationRecord::new(
            VerificationRecordId::new(id).expect("verification id"),
            verifier_id,
            version,
            deterministic,
            inputs,
            result,
            stamp(transaction),
        );
        self.verifications
            .append(record)
            .expect("verification record should append");
    }

    fn resolve(&mut self, transaction: &str) -> graph_core::ResolutionOutcome {
        resolve_claim_verdict(
            &mut self.claims,
            &mut self.verdicts,
            &ResolutionInputs::new(
                &self.verifications,
                &self.evidence,
                &self.observations,
                &self.sources,
            ),
            &self.claim,
            stamp(transaction),
            "deterministic-first-v1",
        )
        .expect("verdict resolution should succeed")
    }
}

#[test]
fn deterministic_failure_outranks_a_model_success_at_point_ninety_nine() {
    let mut fixture = Fixture::new();
    fixture.append(
        "verification--model-pass",
        "verifier.model-judge",
        "1.0.0",
        false,
        VerificationResult::Pass,
        "2026-09-01T10:00:00Z",
    );
    fixture.append(
        "verification--mechanical-fail",
        "verifier.identifier-syntax",
        "1.0.0",
        true,
        VerificationResult::Fail,
        "2026-09-01T10:01:00Z",
    );

    let outcome = fixture.resolve("2026-09-01T10:02:00Z");

    assert_eq!(outcome.state(), VerdictState::Refuted);
    assert_eq!(
        fixture
            .claims
            .claim_by_id(&fixture.claim)
            .expect("claim")
            .status(),
        ClaimStatus::Contradicted
    );
    assert_eq!(
        fixture
            .verifications
            .records_for_claim(&fixture.claim)
            .len(),
        2
    );

    let disagreements = fixture
        .verdicts
        .verification_disagreements_for_claim(&fixture.claim);
    assert_eq!(disagreements.len(), 1);
    let disagreement = disagreements[0];
    assert_eq!(
        disagreement.deterministic_record_id().as_str(),
        "verification--mechanical-fail"
    );
    assert_eq!(
        disagreement.advisory_record_id().as_str(),
        "verification--model-pass"
    );
    let finding = disagreement.to_validation_record();
    assert_eq!(finding.code(), VERIFICATION_AUTHORITY_DISAGREEMENT_CODE);
    assert_eq!(finding.severity(), ValidationErrorSeverity::Warning);
    assert!(finding.message().contains("verification--mechanical-fail"));
    assert!(finding.message().contains("verification--model-pass"));

    assert_eq!(
        fixture
            .claims
            .claim_by_id(&fixture.claim)
            .expect("claim should retain the model score")
            .confidence()
            .expect("model score should remain present")
            .value(),
        0.99
    );
}

#[test]
fn deterministic_pass_is_not_downgraded_by_an_advisory_failure() {
    let mut fixture = Fixture::new();
    fixture.append(
        "verification--mechanical-pass",
        "verifier.schema-constraint",
        "1.0.0",
        true,
        VerificationResult::Pass,
        "2026-09-01T11:00:00Z",
    );
    fixture.append(
        "verification--model-fail",
        "verifier.model-judge",
        "1.0.0",
        false,
        VerificationResult::Fail,
        "2026-09-01T11:01:00Z",
    );

    let outcome = fixture.resolve("2026-09-01T11:02:00Z");

    assert_eq!(outcome.state(), VerdictState::Supported);
    assert_eq!(
        fixture
            .claims
            .claim_by_id(&fixture.claim)
            .expect("claim")
            .status(),
        ClaimStatus::Supported
    );
    let disagreements = fixture
        .verdicts
        .verification_disagreements_for_claim(&fixture.claim);
    assert_eq!(disagreements.len(), 1);
    assert_eq!(
        disagreements[0].deterministic_result(),
        VerificationResult::Pass
    );
    assert_eq!(disagreements[0].advisory_result(), VerificationResult::Fail);
}

#[test]
fn later_record_from_the_same_deterministic_verifier_lifts_an_earlier_failure() {
    let mut fixture = Fixture::new();
    fixture.append(
        "verification--syntax-fail",
        "verifier.identifier-syntax",
        "1.0.0",
        true,
        VerificationResult::Fail,
        "2026-09-01T12:00:00Z",
    );
    assert_eq!(
        fixture.resolve("2026-09-01T12:00:30Z").state(),
        VerdictState::Refuted
    );

    fixture.append(
        "verification--syntax-pass",
        "verifier.identifier-syntax",
        "1.0.0",
        true,
        VerificationResult::Pass,
        "2026-09-01T12:01:00Z",
    );
    assert_eq!(
        fixture.resolve("2026-09-01T12:02:00Z").state(),
        VerdictState::Supported
    );
    assert_eq!(fixture.verdicts.verdicts_for_claim(&fixture.claim).len(), 2);
    assert_eq!(
        fixture
            .verifications
            .records_for_claim(&fixture.claim)
            .len(),
        2
    );
}

#[test]
fn newer_deterministic_verifier_version_lifts_an_earlier_failure() {
    let mut fixture = Fixture::new();
    fixture.append(
        "verification--arithmetic-v1-fail",
        "verifier.arithmetic-consistency",
        "1.0.0",
        true,
        VerificationResult::Fail,
        "2026-09-01T13:00:00Z",
    );
    fixture.append(
        "verification--arithmetic-v2-pass",
        "verifier.arithmetic-consistency",
        "2.0.0",
        true,
        VerificationResult::Pass,
        "2026-09-01T13:01:00Z",
    );

    assert_eq!(
        fixture.resolve("2026-09-01T13:02:00Z").state(),
        VerdictState::Supported
    );
    let records = fixture.verifications.records_for_claim(&fixture.claim);
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].result(), VerificationResult::Fail);
    assert_eq!(records[1].result(), VerificationResult::Pass);
}

#[test]
fn a_different_deterministic_verifier_cannot_lift_an_active_failure() {
    let mut fixture = Fixture::new();
    fixture.append(
        "verification--temporal-fail",
        "verifier.temporal-ordering",
        "1.0.0",
        true,
        VerificationResult::Fail,
        "2026-09-01T14:00:00Z",
    );
    fixture.append(
        "verification--schema-pass",
        "verifier.schema-constraint",
        "1.0.0",
        true,
        VerificationResult::Pass,
        "2026-09-01T14:01:00Z",
    );

    assert_eq!(
        fixture.resolve("2026-09-01T14:02:00Z").state(),
        VerdictState::Mixed,
        "an unrelated deterministic pass is support, but cannot yield a trusted state while a failure remains authoritative"
    );
}

#[test]
fn inconclusive_and_advisory_only_records_carry_no_verdict_weight() {
    let mut fixture = Fixture::new();
    fixture.append(
        "verification--mechanical-inconclusive",
        "verifier.schema-constraint",
        "1.0.0",
        true,
        VerificationResult::Inconclusive,
        "2026-09-01T15:00:00Z",
    );
    fixture.append(
        "verification--model-pass",
        "verifier.model-judge",
        "1.0.0",
        false,
        VerificationResult::Pass,
        "2026-09-01T15:01:00Z",
    );

    assert_eq!(
        fixture.resolve("2026-09-01T15:02:00Z").state(),
        VerdictState::Unknown
    );
    assert!(
        fixture
            .verdicts
            .verification_disagreements_for_claim(&fixture.claim)
            .is_empty()
    );
}

#[test]
fn newer_inconclusive_run_does_not_erase_the_last_conclusive_failure() {
    let mut fixture = Fixture::new();
    fixture.append(
        "verification--graph-fail",
        "verifier.graph-consistency",
        "1.0.0",
        true,
        VerificationResult::Fail,
        "2026-09-01T16:00:00Z",
    );
    fixture.append(
        "verification--graph-inconclusive",
        "verifier.graph-consistency",
        "2.0.0",
        true,
        VerificationResult::Inconclusive,
        "2026-09-01T16:01:00Z",
    );

    assert_eq!(
        fixture.resolve("2026-09-01T16:02:00Z").state(),
        VerdictState::Refuted,
        "an inconclusive run has no authority to lift a conclusive failure"
    );
}

#[test]
fn disagreement_is_recorded_once_and_survives_serialization() {
    let mut fixture = Fixture::new();
    fixture.append(
        "verification--deterministic-pass",
        "verifier.content-hash",
        "1.0.0",
        true,
        VerificationResult::Pass,
        "2026-09-01T17:00:00Z",
    );
    fixture.append(
        "verification--advisory-fail",
        "verifier.model-judge",
        "1.0.0",
        false,
        VerificationResult::Fail,
        "2026-09-01T17:01:00Z",
    );

    assert_eq!(
        fixture.resolve("2026-09-01T17:02:00Z").state(),
        VerdictState::Supported
    );
    let repeated = fixture.resolve("2026-09-01T17:03:00Z");
    assert_eq!(repeated.state(), VerdictState::Supported);
    assert!(!repeated.changed());
    assert_eq!(fixture.verdicts.verification_disagreements().len(), 1);

    let encoded = serde_json::to_string(&fixture.verdicts).expect("serialize verdict store");
    let restored: VerdictStore = serde_json::from_str(&encoded).expect("deserialize verdict store");
    assert_eq!(restored, fixture.verdicts);
    assert_eq!(restored.verification_disagreements().len(), 1);
}

#[test]
fn deterministic_failure_removes_previously_validated_lifecycle_status() {
    let mut refuted = Fixture::new();
    assert!(
        refuted
            .claims
            .apply_verdict_projection(&refuted.claim, ClaimStatus::Validated)
            .expect("fixture claim should become validated")
    );
    refuted.append(
        "verification--blocking-fail",
        "verifier.identifier-syntax",
        "1.0.0",
        true,
        VerificationResult::Fail,
        "2026-09-01T18:00:00Z",
    );
    let outcome = refuted.resolve("2026-09-01T18:01:00Z");
    assert_eq!(outcome.state(), VerdictState::Refuted);
    assert!(outcome.lifecycle_applied());
    assert_eq!(
        refuted
            .claims
            .claim_by_id(&refuted.claim)
            .expect("claim")
            .status(),
        ClaimStatus::Contradicted
    );

    let mut mixed = Fixture::new();
    mixed
        .claims
        .apply_verdict_projection(&mixed.claim, ClaimStatus::Validated)
        .expect("fixture claim should become validated");
    mixed.append(
        "verification--blocking-fail",
        "verifier.identifier-syntax",
        "1.0.0",
        true,
        VerificationResult::Fail,
        "2026-09-01T18:00:00Z",
    );
    mixed.append(
        "verification--independent-pass",
        "verifier.content-hash",
        "1.0.0",
        true,
        VerificationResult::Pass,
        "2026-09-01T18:00:30Z",
    );
    let outcome = mixed.resolve("2026-09-01T18:01:00Z");
    assert_eq!(outcome.state(), VerdictState::Mixed);
    assert!(outcome.lifecycle_applied());
    assert_eq!(
        mixed
            .claims
            .claim_by_id(&mixed.claim)
            .expect("claim")
            .status(),
        ClaimStatus::Disputed
    );
}

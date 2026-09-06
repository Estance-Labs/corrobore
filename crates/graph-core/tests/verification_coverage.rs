// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//! Verification coverage contract for Epic 0029 WS-B item 6 (issue #168).
//!
//! Coverage is derived from governed claims and append-only verification
//! records. It is not another stored report and never changes verdict
//! precedence.
#![allow(clippy::unwrap_used)]

use graph_core::{
    BitemporalStamp, ClaimAnalyticalTarget, ClaimId, ClaimInput, ClaimProposition,
    ClaimPropositionObject, ClaimStatement, ClaimStore, ClaimTarget, PropertyValue,
    TemporalTimestamp, VerificationCoverage, VerificationCoverageClass, VerificationCoverageTarget,
    VerificationInputs, VerificationRecord, VerificationRecordId, VerificationRecordStore,
    VerificationResult,
};

fn ts(value: &str) -> TemporalTimestamp {
    TemporalTimestamp::new(value).unwrap()
}

fn stamp(system_time: &str) -> BitemporalStamp {
    BitemporalStamp::new(ts("2026-09-06T00:00:00Z"), ts(system_time)).unwrap()
}

fn claim_store() -> ClaimStore {
    let mut claims = ClaimStore::new();
    claims
        .create_asserted_claim(
            ClaimInput::new(
                ClaimId::new("claim--structured").unwrap(),
                ClaimStatement::new("The reported total is 42").unwrap(),
                ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new("total", None)),
            )
            .with_proposition(
                ClaimProposition::new(
                    "report--1",
                    "has_total",
                    ClaimPropositionObject::Literal(PropertyValue::Integer(42)),
                )
                .unwrap(),
            ),
        )
        .unwrap();
    claims
        .create_asserted_claim(ClaimInput::new(
            ClaimId::new("claim--text-only").unwrap(),
            ClaimStatement::new("An analyst supplied an unstructured assessment").unwrap(),
            ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new("assessment", None)),
        ))
        .unwrap();
    claims
}

fn record(
    id: &str,
    claim_id: &str,
    verifier_id: &str,
    version: &str,
    deterministic: bool,
    result: VerificationResult,
    system_time: &str,
) -> VerificationRecord {
    VerificationRecord::new(
        VerificationRecordId::new(id).unwrap(),
        verifier_id,
        version,
        deterministic,
        VerificationInputs::for_claim(ClaimId::new(claim_id).unwrap()),
        result,
        stamp(system_time),
    )
}

#[test]
fn coverage_distinguishes_mechanical_semantic_unchecked_and_failing_entries() {
    let claims = claim_store();
    let claim = claims
        .claim_by_id(&ClaimId::new("claim--structured").unwrap())
        .unwrap();
    let mut records = VerificationRecordStore::new();
    records
        .append(record(
            "verification--mechanical",
            "claim--structured",
            "verifier.arithmetic-consistency",
            "1.0.0",
            true,
            VerificationResult::Pass,
            "2026-09-06T00:01:00Z",
        ))
        .unwrap();
    records
        .append(record(
            "verification--semantic",
            "claim--structured",
            "fr.estance.corrobore.domain.research.claim.verify",
            "2.1.0",
            false,
            VerificationResult::Pass,
            "2026-09-06T00:02:00Z",
        ))
        .unwrap();
    records
        .append(record(
            "verification--failing",
            "claim--structured",
            "verifier.schema-constraint",
            "1.0.0",
            true,
            VerificationResult::Fail,
            "2026-09-06T00:03:00Z",
        ))
        .unwrap();

    let coverage = VerificationCoverage::derive(claim, &records);
    assert_eq!(coverage.target(), VerificationCoverageTarget::Proposition);
    assert_eq!(coverage.entries().len(), 3);
    let classes = coverage
        .entries()
        .iter()
        .map(|entry| entry.class())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(classes.contains(&VerificationCoverageClass::MechanicallyChecked));
    assert!(classes.contains(&VerificationCoverageClass::SemanticallyJudged));
    assert!(classes.contains(&VerificationCoverageClass::Failing));
    let failing = coverage
        .entries()
        .iter()
        .find(|entry| entry.class() == VerificationCoverageClass::Failing)
        .expect("failing entry");
    assert_eq!(failing.verifier_id(), Some("verifier.schema-constraint"));
    assert_eq!(failing.verifier_version(), Some("1.0.0"));
    assert!(failing.deterministic());

    let text_only = claims
        .claim_by_id(&ClaimId::new("claim--text-only").unwrap())
        .unwrap();
    let unchecked = VerificationCoverage::derive(text_only, &records);
    assert_eq!(unchecked.target(), VerificationCoverageTarget::Statement);
    assert_eq!(unchecked.entries().len(), 1);
    assert_eq!(
        unchecked.entries()[0].class(),
        VerificationCoverageClass::Unchecked
    );
    assert_eq!(unchecked.entries()[0].verifier_id(), None);
    assert_eq!(unchecked.entries()[0].verifier_version(), None);
}

#[test]
fn coverage_uses_the_current_verifier_version_without_rewriting_history() {
    let claims = claim_store();
    let claim = claims
        .claim_by_id(&ClaimId::new("claim--structured").unwrap())
        .unwrap();
    let mut records = VerificationRecordStore::new();
    records
        .append(record(
            "verification--v1",
            "claim--structured",
            "verifier.arithmetic-consistency",
            "1.0.0",
            true,
            VerificationResult::Pass,
            "2026-09-06T00:01:00Z",
        ))
        .unwrap();
    records
        .append(record(
            "verification--v2",
            "claim--structured",
            "verifier.arithmetic-consistency",
            "2.0.0",
            true,
            VerificationResult::Fail,
            "2026-09-06T00:02:00Z",
        ))
        .unwrap();

    let coverage = VerificationCoverage::derive(claim, &records);
    assert_eq!(records.len(), 2, "append-only history remains intact");
    assert_eq!(coverage.entries().len(), 1);
    assert_eq!(coverage.entries()[0].verifier_version(), Some("2.0.0"));
    assert_eq!(
        coverage.entries()[0].class(),
        VerificationCoverageClass::Failing
    );
}

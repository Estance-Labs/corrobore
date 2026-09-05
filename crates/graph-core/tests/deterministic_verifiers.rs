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
//! Contract tests for Epic 0029 WS-B item 2 (issue #164).
//!
//! These tests deliberately exercise the concrete verifiers through the
//! versioned registry. That keeps the asserted result, limits, provenance, and
//! deterministic flag identical to the audit records production callers see.
use graph_core::{
    BitemporalStamp, CONTENT_HASH_VERIFIER_ID, CONTENT_HASH_VERIFIER_VERSION,
    ClaimAnalyticalTarget, ClaimId, ClaimInput, ClaimLink, ClaimLinkKind, ClaimLinkSource,
    ClaimProposition, ClaimPropositionObject, ClaimStatement, ClaimStore, ClaimTarget,
    ContentHashVerifier, EvidenceId, EvidenceInput, EvidenceLocator, EvidenceRecordStore,
    EvidenceSourceType, IDENTIFIER_SYNTAX_VERIFIER_ID, IDENTIFIER_SYNTAX_VERIFIER_VERSION,
    IdentifierSyntaxVerifier, ObservationId, ObservationInput, ObservationModality,
    ObservationStore, PropertyValue, SourceId, SourceInput, SourceStore, TemporalTimestamp,
    VerificationContext, VerificationRecord, VerificationRecordStore, VerificationResult,
    VerifierCostClass, VerifierRegistry, VerifierSpec,
};

const SHA256_PAYLOAD: &str = "239f59ed55e737c77147cf55ad0c1b030b6d7ee748a7426952f9b852d5a935e5";
const SHA256_OTHER: &str = "d9298a10d1b0735837dc4bd85dac641b0f5db32763b51a65296ca1f51cd302a6";

fn claim_id(value: &str) -> ClaimId {
    ClaimId::new(value).expect("claim id")
}

fn observation_id(value: &str) -> ObservationId {
    ObservationId::new(value).expect("observation id")
}

fn evidence_id(value: &str) -> EvidenceId {
    EvidenceId::new(value).expect("evidence id")
}

fn source_id() -> SourceId {
    SourceId::new("source--identifier-fixture").expect("source id")
}

fn timestamp(value: &str) -> TemporalTimestamp {
    TemporalTimestamp::new(value).expect("timestamp")
}

fn stamp() -> BitemporalStamp {
    BitemporalStamp::new(
        timestamp("2026-09-01T00:00:00Z"),
        timestamp("2026-09-06T10:00:00Z"),
    )
    .expect("stamp")
}

fn sources() -> SourceStore {
    let mut sources = SourceStore::new();
    sources
        .register_source(SourceInput::new(
            source_id(),
            "https://evidence.example.org/records.json",
            EvidenceSourceType::Dataset,
        ))
        .expect("source");
    sources
}

fn claim_with_literal(predicate: &str, value: &str) -> ClaimStore {
    let mut claims = ClaimStore::new();
    let proposition = ClaimProposition::new(
        "record--fixture",
        predicate,
        ClaimPropositionObject::Literal(PropertyValue::String(value.to_owned())),
    )
    .expect("proposition");
    claims
        .create_asserted_claim(
            ClaimInput::new(
                claim_id("claim--identifier"),
                ClaimStatement::new("The record carries an identifier").expect("statement"),
                ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new("identifier", None)),
            )
            .with_proposition(proposition),
        )
        .expect("claim");
    claims
}

fn bare_claim() -> ClaimStore {
    let mut claims = ClaimStore::new();
    claims
        .create_asserted_claim(ClaimInput::new(
            claim_id("claim--identifier"),
            ClaimStatement::new("The record carries an identifier").expect("statement"),
            ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new("identifier", None)),
        ))
        .expect("claim");
    claims
}

fn registry_with(spec: VerifierSpec) -> VerifierRegistry {
    let mut registry = VerifierRegistry::new();
    registry.register(spec).expect("verifier registration");
    registry
}

fn run(
    registry: &VerifierRegistry,
    verifier_id: &str,
    version: &str,
    claims: &ClaimStore,
    observations: &ObservationStore,
    sources: &SourceStore,
    evidence: &EvidenceRecordStore,
) -> VerificationRecord {
    let mut records = VerificationRecordStore::new();
    let context = VerificationContext::new(claims, observations, sources, evidence);
    let id = registry
        .run(
            verifier_id,
            version,
            &claim_id("claim--identifier"),
            &context,
            &mut records,
            stamp(),
        )
        .expect("verifier run");
    records.record_by_id(&id).expect("record").clone()
}

fn run_identifier(predicate: &str, value: &str) -> VerificationRecord {
    let claims = claim_with_literal(predicate, value);
    run(
        &registry_with(VerifierSpec::new(Box::new(IdentifierSyntaxVerifier::new()))),
        IDENTIFIER_SYNTAX_VERIFIER_ID,
        IDENTIFIER_SYNTAX_VERIFIER_VERSION,
        &claims,
        &ObservationStore::new(),
        &SourceStore::new(),
        &EvidenceRecordStore::new(),
    )
}

// Every public format named by #164 has an accepted and rejected example.
// Invalid cases retain enough shape or an explicit predicate to identify the
// intended format, so failure cannot be mistaken for "nothing to check".
#[test]
fn identifier_syntax_accepts_and_rejects_each_public_format() {
    let valid = [
        ("md5", "d41d8cd98f00b204e9800998ecf8427e"),
        ("sha1", &"a".repeat(40)),
        ("sha224", &"b".repeat(56)),
        ("sha256", &"c".repeat(64)),
        ("sha384", &"d".repeat(96)),
        ("sha512", &"e".repeat(128)),
        ("uuid", "550e8400-e29b-41d4-a716-446655440000"),
        ("rfc3339", "2026-09-05T22:15:13+02:00"),
        ("domain", "evidence.example.org"),
        ("ipv4", "203.0.113.10"),
        ("ipv6", "2001:db8::1"),
        ("url", "https://example.org/a/path?item=1#result"),
        ("stix_id", "indicator--550e8400-e29b-41d4-a716-446655440000"),
        ("cve_id", "CVE-2026-12345"),
    ];
    let invalid = [
        ("md5", "d41d8cd98f00b204e9800998ecf8427"),
        ("sha1", &"g".repeat(40)),
        ("sha224", &"b".repeat(55)),
        ("sha256", &"c".repeat(63)),
        ("sha384", &"d".repeat(95)),
        ("sha512", &"e".repeat(127)),
        ("uuid", "550e8400-e29b-41d4-a716-44665544000z"),
        ("rfc3339", "2026-02-30T22:15:13Z"),
        ("domain", "-evidence.example.org"),
        ("ipv4", "203.0.113.999"),
        ("ipv6", "2001:db8:::1"),
        ("url", "example.org/a/path"),
        ("stix_id", "indicator--not-a-uuid"),
        ("cve_id", "CVE-26-1234"),
    ];

    for (format, value) in valid {
        let record = run_identifier(format, value);
        assert_eq!(
            record.result(),
            VerificationResult::Pass,
            "{format}: {value}"
        );
        assert!(record.rationale().is_some_and(|text| text.contains(format)));
        assert!(
            !record.limits().is_empty(),
            "passes must state their limits"
        );
    }
    for (format, value) in invalid {
        let record = run_identifier(format, value);
        assert_eq!(
            record.result(),
            VerificationResult::Fail,
            "{format}: {value}"
        );
        assert!(record.rationale().is_some_and(|text| text.contains(format)));
        assert!(
            !record.limits().is_empty(),
            "failures must state their limits"
        );
    }
}

// A syntactically UUID-shaped STIX identifier whose type is absent from the
// public STIX vocabulary has its own diagnostic. It is neither accepted nor
// flattened into the malformed-identifier message.
#[test]
fn an_unknown_stix_type_is_reported_separately_from_malformed_shape() {
    let uuid = "550e8400-e29b-41d4-a716-446655440000";
    let unknown = run_identifier("stix_id", &format!("spaceship--{uuid}"));
    let malformed = run_identifier("stix_id", "indicator--not-a-uuid");

    assert_eq!(unknown.result(), VerificationResult::Fail);
    assert!(unknown.rationale().is_some_and(|text| {
        text.contains("unknown STIX object type 'spaceship'") && text.contains("valid shape")
    }));
    assert!(malformed.rationale().is_some_and(|text| {
        text.contains("malformed STIX identifier") && !text.contains("unknown STIX object type")
    }));
}

// Record-path selectors provide a public-format hint for the exact observation
// payload. This covers identifier syntax when a structured observation, rather
// than a proposition literal, owns the value.
#[test]
fn record_path_selectors_validate_identifier_shaped_observation_payloads() {
    for (payload, expected) in [
        ("CVE-2026-12345", VerificationResult::Pass),
        ("CVE-26-1234", VerificationResult::Fail),
    ] {
        let sources = sources();
        let mut observations = ObservationStore::new();
        let observation = observation_id("observation--selected-cve");
        observations
            .create_observation(
                ObservationInput::new(
                    observation.clone(),
                    source_id(),
                    payload,
                    ObservationModality::StructuredRecord,
                )
                .with_selector(EvidenceLocator::RecordPath {
                    path: "/vulnerabilities/0/cve_id".to_owned(),
                }),
                &sources,
            )
            .expect("observation");
        let mut claims = bare_claim();
        claims.register_observation(observation.clone());
        claims
            .attach_link(ClaimLink::new(
                ClaimLinkSource::Observation(observation),
                claim_id("claim--identifier"),
                ClaimLinkKind::Supports,
            ))
            .expect("link");

        let record = run(
            &registry_with(VerifierSpec::new(Box::new(IdentifierSyntaxVerifier::new()))),
            IDENTIFIER_SYNTAX_VERIFIER_ID,
            IDENTIFIER_SYNTAX_VERIFIER_VERSION,
            &claims,
            &observations,
            &sources,
            &EvidenceRecordStore::new(),
        );
        assert_eq!(record.result(), expected);
        assert_eq!(
            record.evidence_consumed(),
            [
                "observation:observation--selected-cve:selector:record_path:/vulnerabilities/0/cve_id"
            ]
        );
    }
}

#[test]
fn identifier_verifier_records_the_public_syntax_only_limit() {
    let record = run_identifier("cve_id", "CVE-2026-12345");
    assert!(record.deterministic());
    assert!(
        record
            .limits()
            .iter()
            .any(|limit| { limit.contains("syntax only") && limit.contains("external registry") })
    );
}

#[test]
fn ordinary_semantic_predicates_are_not_mistaken_for_format_hints() {
    let record = run_identifier("relationship", "ordinary prose");
    assert_eq!(record.result(), VerificationResult::Inconclusive);
}

#[test]
fn content_hash_passes_when_all_recorded_digests_match() {
    let sources = sources();
    let mut observations = ObservationStore::new();
    let observation = observation_id("observation--hashed");
    observations
        .create_observation(
            ObservationInput::new(
                observation.clone(),
                source_id(),
                "payload",
                ObservationModality::Text,
            )
            .with_payload_sha256(SHA256_PAYLOAD),
            &sources,
        )
        .expect("observation");

    let evidence_id = evidence_id("evidence--hashed");
    let mut evidence = EvidenceRecordStore::new();
    evidence
        .create_evidence(
            EvidenceInput::new(evidence_id.clone(), "source://fixture", "payload")
                .with_content_sha256(SHA256_PAYLOAD),
        )
        .expect("evidence");

    let mut claims = bare_claim();
    claims.register_observation(observation.clone());
    claims.register_evidence(evidence_id.clone());
    for source in [
        ClaimLinkSource::Observation(observation),
        ClaimLinkSource::Evidence(evidence_id),
    ] {
        claims
            .attach_link(ClaimLink::new(
                source,
                claim_id("claim--identifier"),
                ClaimLinkKind::Supports,
            ))
            .expect("link");
    }

    let record = run(
        &registry_with(VerifierSpec::new(Box::new(ContentHashVerifier::new()))),
        CONTENT_HASH_VERIFIER_ID,
        CONTENT_HASH_VERIFIER_VERSION,
        &claims,
        &observations,
        &sources,
        &evidence,
    );
    assert_eq!(record.result(), VerificationResult::Pass);
    assert!(record.deterministic());
    assert_eq!(record.evidence_consumed().len(), 2);
    assert!(
        record
            .limits()
            .iter()
            .any(|limit| limit.contains("authenticity"))
    );
}

#[test]
fn content_hash_drift_fails_deterministically_and_names_both_digests() {
    let sources = sources();
    let mut observations = ObservationStore::new();
    let observation = observation_id("observation--drifted");
    observations
        .create_observation(
            ObservationInput::new(
                observation.clone(),
                source_id(),
                "payload",
                ObservationModality::Text,
            )
            .with_payload_sha256(SHA256_OTHER),
            &sources,
        )
        .expect("observation");
    let mut claims = bare_claim();
    claims.register_observation(observation.clone());
    claims
        .attach_link(ClaimLink::new(
            ClaimLinkSource::Observation(observation),
            claim_id("claim--identifier"),
            ClaimLinkKind::Supports,
        ))
        .expect("link");

    let record = run(
        &registry_with(VerifierSpec::new(Box::new(ContentHashVerifier::new()))),
        CONTENT_HASH_VERIFIER_ID,
        CONTENT_HASH_VERIFIER_VERSION,
        &claims,
        &observations,
        &sources,
        &EvidenceRecordStore::new(),
    );
    assert_eq!(record.result(), VerificationResult::Fail);
    assert!(record.deterministic());
    assert!(record.rationale().is_some_and(|text| {
        text.contains("observation--drifted")
            && text.contains(SHA256_OTHER)
            && text.contains(SHA256_PAYLOAD)
    }));
}

#[test]
fn evidence_content_hash_drift_is_checked_too() {
    let mut evidence = EvidenceRecordStore::new();
    let evidence_id = evidence_id("evidence--drifted");
    evidence
        .create_evidence(
            EvidenceInput::new(evidence_id.clone(), "source://fixture", "payload")
                .with_content_sha256(SHA256_OTHER),
        )
        .expect("evidence");
    let mut claims = bare_claim();
    claims.register_evidence(evidence_id.clone());
    claims
        .attach_link(ClaimLink::new(
            ClaimLinkSource::Evidence(evidence_id),
            claim_id("claim--identifier"),
            ClaimLinkKind::Supports,
        ))
        .expect("link");

    let record = run(
        &registry_with(VerifierSpec::new(Box::new(ContentHashVerifier::new()))),
        CONTENT_HASH_VERIFIER_ID,
        CONTENT_HASH_VERIFIER_VERSION,
        &claims,
        &ObservationStore::new(),
        &SourceStore::new(),
        &evidence,
    );
    assert_eq!(record.result(), VerificationResult::Fail);
    assert!(record.rationale().is_some_and(|text| {
        text.contains("evidence--drifted")
            && text.contains(SHA256_OTHER)
            && text.contains(SHA256_PAYLOAD)
    }));
}

#[test]
fn content_hash_is_inconclusive_without_a_recorded_digest() {
    let record = run(
        &registry_with(VerifierSpec::new(Box::new(ContentHashVerifier::new()))),
        CONTENT_HASH_VERIFIER_ID,
        CONTENT_HASH_VERIFIER_VERSION,
        &bare_claim(),
        &ObservationStore::new(),
        &SourceStore::new(),
        &EvidenceRecordStore::new(),
    );
    assert_eq!(record.result(), VerificationResult::Inconclusive);
    assert!(
        record
            .rationale()
            .is_some_and(|text| text.contains("no recorded digest"))
    );
    assert!(!record.limits().is_empty());
}

#[test]
fn concrete_verifier_versions_and_registration_metadata_are_stable() {
    assert_eq!(IDENTIFIER_SYNTAX_VERIFIER_ID, "verifier.identifier-syntax");
    assert_eq!(IDENTIFIER_SYNTAX_VERIFIER_VERSION, "1.0.0");
    assert_eq!(CONTENT_HASH_VERIFIER_ID, "verifier.content-hash");
    assert_eq!(CONTENT_HASH_VERIFIER_VERSION, "1.0.0");

    let identifier = IdentifierSyntaxVerifier::new();
    let content_hash = ContentHashVerifier::new();
    for spec in [
        VerifierSpec::new(Box::new(identifier)),
        VerifierSpec::new(Box::new(content_hash)),
    ] {
        assert!(spec.deterministic());
        assert_eq!(spec.cost_class(), VerifierCostClass::Low);
    }
}

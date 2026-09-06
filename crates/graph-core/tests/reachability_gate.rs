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
//! Integration contract for the observation-reachability gate and the
//! immutability guards (Epic 0029, WS-A item 6, issue #152).
//!
//! ADR-0016 hard invariant: no `Supported`, `Refuted`, or `Mixed` verdict
//! without an active evidence link whose source resolves to an `Observation`
//! bound to a `Source`. A bare evidence record without `observation_id`, a
//! claim-only link, or a verification record alone does not satisfy it. The
//! resolution then yields `InsufficientEvidence` and records a typed gap, and
//! never silently promotes. Governed records reject in-place updates with one
//! typed error.
use graph_core::{
    BitemporalStamp, CLAIM_LIFECYCLE_WITHOUT_OBSERVATION_PATH_CODE,
    CLAIM_UNREACHABLE_EVIDENCE_CODE, ClaimAnalyticalTarget, ClaimId, ClaimInput, ClaimLink,
    ClaimLinkKind, ClaimLinkSource, ClaimStatement, ClaimStatus, ClaimStore, ClaimTarget,
    EvidenceId, EvidenceInput, EvidenceRecordStore, EvidenceSourceType, GraphError,
    ImmutableRecordKind, ObservationId, ObservationInput, ObservationModality, ObservationStore,
    ResolutionInputs, SourceId, SourceInput, SourceStore, TemporalTimestamp,
    ValidationErrorSeverity, ValidationTarget, VerdictState, VerdictStore, VerificationInputs,
    VerificationRecord, VerificationRecordId, VerificationRecordStore, VerificationResult,
    resolve_claim_verdict, validate_claim_reachability,
};

fn claim_id(value: &str) -> ClaimId {
    ClaimId::new(value).expect("test claim ID should be valid")
}

fn evidence_id(value: &str) -> EvidenceId {
    EvidenceId::new(value).expect("test evidence ID should be valid")
}

fn observation_id(value: &str) -> ObservationId {
    ObservationId::new(value).expect("test observation ID should be valid")
}

fn source_id(value: &str) -> SourceId {
    SourceId::new(value).expect("test source ID should be valid")
}

fn ts(value: &str) -> TemporalTimestamp {
    TemporalTimestamp::new(value).expect("test timestamp should be valid")
}

fn stamp(transaction: &str) -> BitemporalStamp {
    BitemporalStamp::new(ts("2026-08-01T00:00:00Z"), ts(transaction))
        .expect("stamp should be valid")
}

struct Fixture {
    claims: ClaimStore,
    verdicts: VerdictStore,
    verifications: VerificationRecordStore,
    evidence: EvidenceRecordStore,
    observations: ObservationStore,
    sources: SourceStore,
}

impl Fixture {
    fn new() -> Self {
        let mut sources = SourceStore::new();
        sources
            .register_source(SourceInput::new(
                source_id("source--report"),
                "https://vendor.example/report.pdf",
                EvidenceSourceType::Document,
            ))
            .expect("source should register");
        let mut observations = ObservationStore::new();
        observations
            .create_observation(
                ObservationInput::new(
                    observation_id("observation--span"),
                    source_id("source--report"),
                    "Actor A operates Campaign B.",
                    ObservationModality::Text,
                ),
                &sources,
            )
            .expect("observation should be created");

        let mut evidence = EvidenceRecordStore::new();
        evidence
            .create_evidence(EvidenceInput::new(
                evidence_id("evidence--bare"),
                "ref--bare",
                "a payload with no observation behind it",
            ))
            .expect("bare evidence should be created");
        evidence
            .create_evidence(
                EvidenceInput::new(
                    evidence_id("evidence--anchored"),
                    "ref--anchored",
                    "payload",
                )
                .with_source_id(source_id("source--report"))
                .with_observation_id(observation_id("observation--span")),
            )
            .expect("anchored evidence should be created");

        let mut claims = ClaimStore::new();
        claims.register_evidence(evidence_id("evidence--bare"));
        claims.register_evidence(evidence_id("evidence--anchored"));
        claims.register_observation(observation_id("observation--span"));

        Self {
            claims,
            verdicts: VerdictStore::new(),
            verifications: VerificationRecordStore::new(),
            evidence,
            observations,
            sources,
        }
    }

    fn asserted_claim(&mut self, id: &str) -> ClaimId {
        let id = claim_id(id);
        self.claims
            .create_asserted_claim(ClaimInput::new(
                id.clone(),
                ClaimStatement::new(format!("statement of {}", id.as_str())).expect("statement"),
                ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new(id.as_str(), None)),
            ))
            .expect("claim should be created");
        id
    }

    fn link(&mut self, source: ClaimLinkSource, target: &ClaimId, kind: ClaimLinkKind) {
        self.claims
            .attach_link(ClaimLink::new(source, target.clone(), kind))
            .expect("link should attach");
    }

    fn resolve(&mut self, claim: &ClaimId, transaction: &str) -> graph_core::ResolutionOutcome {
        resolve_claim_verdict(
            &mut self.claims,
            &mut self.verdicts,
            &ResolutionInputs::new(
                &self.verifications,
                &self.evidence,
                &self.observations,
                &self.sources,
            ),
            claim,
            stamp(transaction),
            "ws-a-minimal-v1",
        )
        .expect("resolution should succeed")
    }
}

//
// Verify that an observation-backed support link reaches `Supported`, both
// directly from the observation and through an evidence record that names its
// observation.
#[test]
fn observation_path_allows_supported_verdict() {
    let mut fixture = Fixture::new();
    let direct = fixture.asserted_claim("claim--direct");
    let via_evidence = fixture.asserted_claim("claim--via-evidence");
    fixture.link(
        ClaimLinkSource::Observation(observation_id("observation--span")),
        &direct,
        ClaimLinkKind::Supports,
    );
    fixture.link(
        ClaimLinkSource::Evidence(evidence_id("evidence--anchored")),
        &via_evidence,
        ClaimLinkKind::Supports,
    );

    let outcome = fixture.resolve(&direct, "2026-08-30T10:00:00Z");
    assert_eq!(outcome.state(), VerdictState::Supported);
    assert!(outcome.reachability_gap().is_none());
    assert_eq!(
        fixture.claims.claim_by_id(&direct).expect("claim").status(),
        ClaimStatus::Supported
    );

    let outcome = fixture.resolve(&via_evidence, "2026-08-30T10:01:00Z");
    assert_eq!(outcome.state(), VerdictState::Supported);
    assert!(fixture.verdicts.reachability_gaps().is_empty());
}

//
// Verify the gate: the same support signal from a bare evidence record, from
// a claim-only link, or from a verification record alone yields
// `InsufficientEvidence` with a typed gap naming the claim and the sources that
// lack an observation path. The lifecycle projects to `Unresolved`, never to
// `Supported`.
#[test]
fn missing_observation_path_yields_insufficient_evidence_with_typed_gap() {
    let mut fixture = Fixture::new();
    let bare = fixture.asserted_claim("claim--bare-evidence");
    let claim_only = fixture.asserted_claim("claim--claim-only");
    let supporter = fixture.asserted_claim("claim--supporter");
    let verified = fixture.asserted_claim("claim--verified-only");

    fixture.link(
        ClaimLinkSource::Evidence(evidence_id("evidence--bare")),
        &bare,
        ClaimLinkKind::Supports,
    );
    fixture.link(
        ClaimLinkSource::Claim(supporter.clone()),
        &claim_only,
        ClaimLinkKind::Refutes,
    );
    fixture
        .verifications
        .append(VerificationRecord::new(
            VerificationRecordId::new("verification--fail").expect("id"),
            "verifier.identifier-syntax",
            "1.0.0",
            true,
            VerificationInputs::for_claim(verified.clone()),
            VerificationResult::Fail,
            stamp("2026-08-30T09:00:00Z"),
        ))
        .expect("record should append");

    for (index, claim) in [&bare, &claim_only, &verified].into_iter().enumerate() {
        let outcome = fixture.resolve(claim, &format!("2026-08-30T10:0{index}:00Z"));
        assert_eq!(
            outcome.state(),
            VerdictState::InsufficientEvidence,
            "{}",
            claim.as_str()
        );
        let gap = outcome
            .reachability_gap()
            .expect("a gap should be recorded");
        assert_eq!(gap.claim_id(), claim);
        let attempted = if claim == &bare {
            VerdictState::Supported
        } else {
            VerdictState::Refuted
        };
        assert_eq!(gap.attempted_state(), attempted);
        assert_eq!(
            fixture.claims.claim_by_id(claim).expect("claim").status(),
            ClaimStatus::Unresolved,
            "{}",
            claim.as_str()
        );

        let record = gap.to_validation_record();
        assert_eq!(record.code(), CLAIM_UNREACHABLE_EVIDENCE_CODE);
        assert_eq!(record.severity(), ValidationErrorSeverity::Warning);
        assert_eq!(record.target(), &ValidationTarget::claim(claim.as_str()));
        assert!(record.message().contains(claim.as_str()));
    }

    let bare_gap = &fixture.verdicts.reachability_gaps()[0];
    assert_eq!(bare_gap.unreachable_sources(), ["evidence:evidence--bare"]);
    let verified_gap = &fixture.verdicts.reachability_gaps()[2];
    assert_eq!(
        verified_gap.unreachable_sources(),
        ["verification:verification--fail"]
    );
    assert_eq!(fixture.verdicts.reachability_gaps().len(), 3);
    assert_eq!(
        fixture.verdicts.len(),
        3,
        "the InsufficientEvidence verdicts are recorded"
    );
}

//
// Verify that the gate covers `Mixed` too, and that once an observation path
// appears the same claim moves from `InsufficientEvidence` to the signalled
// state, with the gap kept in history.
#[test]
fn mixed_needs_a_path_and_a_later_path_lifts_the_gap() {
    let mut fixture = Fixture::new();
    let claim = fixture.asserted_claim("claim--mixed");
    let supporter = fixture.asserted_claim("claim--supporter");
    fixture.link(
        ClaimLinkSource::Evidence(evidence_id("evidence--bare")),
        &claim,
        ClaimLinkKind::Supports,
    );
    fixture.link(
        ClaimLinkSource::Claim(supporter),
        &claim,
        ClaimLinkKind::Refutes,
    );

    let first = fixture.resolve(&claim, "2026-08-30T10:00:00Z");
    assert_eq!(first.state(), VerdictState::InsufficientEvidence);
    assert_eq!(
        first.reachability_gap().expect("gap").attempted_state(),
        VerdictState::Mixed
    );

    fixture.link(
        ClaimLinkSource::Observation(observation_id("observation--span")),
        &claim,
        ClaimLinkKind::Supports,
    );
    let second = fixture.resolve(&claim, "2026-08-30T11:00:00Z");
    assert_eq!(second.state(), VerdictState::Mixed);
    assert!(second.reachability_gap().is_none());
    assert_eq!(fixture.verdicts.verdicts_for_claim(&claim).len(), 2);
    assert_eq!(
        fixture.verdicts.reachability_gaps().len(),
        1,
        "history keeps the gap"
    );
}

//
// Verify that `Superseded` and `Unknown` are exempt from the gate: neither
// asserts anything about the world that an observation could ground.
#[test]
fn superseded_and_unknown_are_exempt_from_the_gate() {
    let mut fixture = Fixture::new();
    let old = fixture.asserted_claim("claim--old");
    let newer = fixture.asserted_claim("claim--newer");
    let empty = fixture.asserted_claim("claim--empty");
    fixture.link(
        ClaimLinkSource::Claim(newer),
        &old,
        ClaimLinkKind::Supersedes,
    );

    assert_eq!(
        fixture.resolve(&old, "2026-08-30T10:00:00Z").state(),
        VerdictState::Superseded
    );
    assert_eq!(
        fixture.resolve(&empty, "2026-08-30T10:01:00Z").state(),
        VerdictState::Unknown
    );
    assert!(fixture.verdicts.reachability_gaps().is_empty());
}

//
// Verify the store-level immutability guard: every governed record rejects an
// in-place change with one typed error naming the record kind and identifier,
// and identical re-appends stay idempotent.
#[test]
fn governed_records_reject_in_place_updates_with_one_typed_error() {
    let fixture = Fixture::new();

    let mut sources = fixture.sources.clone();
    let changed_source = SourceInput::new(
        source_id("source--report"),
        "https://vendor.example/report.pdf",
        EvidenceSourceType::Document,
    )
    .with_publisher("Someone Else");
    assert!(matches!(
        sources.register_source(changed_source),
        Err(GraphError::ImmutableRecordConflict { kind: ImmutableRecordKind::Source, id })
            if id == "source--report"
    ));

    let mut observations = fixture.observations.clone();
    let changed_observation = ObservationInput::new(
        observation_id("observation--span"),
        source_id("source--report"),
        "a different payload",
        ObservationModality::Text,
    );
    assert!(matches!(
        observations.create_observation(changed_observation, &fixture.sources),
        Err(GraphError::ImmutableRecordConflict { kind: ImmutableRecordKind::Observation, id })
            if id == "observation--span"
    ));
    let orphan = ObservationInput::new(
        observation_id("observation--orphan"),
        source_id("source--missing"),
        "payload",
        ObservationModality::Text,
    );
    assert!(matches!(
        observations.create_observation(orphan, &fixture.sources),
        Err(GraphError::SourceNotFound(id)) if id == source_id("source--missing")
    ));

    let mut verifications = VerificationRecordStore::new();
    let record = VerificationRecord::new(
        VerificationRecordId::new("verification--1").expect("id"),
        "verifier.x",
        "1.0.0",
        true,
        VerificationInputs::for_claim(claim_id("claim--x")),
        VerificationResult::Pass,
        stamp("2026-08-30T09:00:00Z"),
    );
    verifications.append(record.clone()).expect("append");
    verifications
        .append(record.clone())
        .expect("identical re-append is idempotent");
    let changed_record = VerificationRecord::new(
        VerificationRecordId::new("verification--1").expect("id"),
        "verifier.x",
        "1.0.0",
        true,
        VerificationInputs::for_claim(claim_id("claim--x")),
        VerificationResult::Fail,
        stamp("2026-08-30T09:00:00Z"),
    );
    assert!(matches!(
        verifications.append(changed_record),
        Err(GraphError::ImmutableRecordConflict { kind: ImmutableRecordKind::VerificationRecord, id })
            if id == "verification--1"
    ));

    let mut verdicts = VerdictStore::new();
    let verdict_id = graph_core::VerdictId::new("verdict--1").expect("id");
    verdicts
        .append_verdict(
            verdict_id.clone(),
            claim_id("claim--x"),
            VerdictState::Unknown,
            graph_core::ConfidenceDimensions::default(),
            "policy",
            stamp("2026-08-30T09:00:00Z"),
        )
        .expect("append");
    assert!(matches!(
        verdicts.append_verdict(
            verdict_id,
            claim_id("claim--x"),
            VerdictState::Supported,
            graph_core::ConfidenceDimensions::default(),
            "policy",
            stamp("2026-08-30T09:30:00Z"),
        ),
        Err(GraphError::ImmutableRecordConflict { kind: ImmutableRecordKind::Verdict, id })
            if id == "verdict--1"
    ));
    assert_eq!(verdicts.len(), 1);
    assert_eq!(
        ImmutableRecordKind::StateTransition.as_str(),
        "state_transition"
    );
}

//
// Verify the legacy-data rule: a claim whose lifecycle says `Supported` or
// `Validated` while its current verdict has no observation path is reported,
// and the rule mutates nothing.
#[test]
fn legacy_rule_flags_supported_lifecycle_without_observation_path_and_does_not_mutate() {
    let mut fixture = Fixture::new();
    let legacy = fixture.asserted_claim("claim--legacy");
    let grounded = fixture.asserted_claim("claim--grounded");
    let unresolved = fixture.asserted_claim("claim--unresolved");

    // Legacy shape: lifecycle already Supported through a bare evidence link
    // and the Epic 0005 typed helper, which knows nothing about observations.
    fixture
        .claims
        .attach_supporting_evidence_to_claim(evidence_id("evidence--bare"), legacy.clone())
        .expect("legacy support");
    assert!(
        fixture
            .claims
            .apply_verdict_projection(&legacy, ClaimStatus::Supported)
            .expect("projection")
    );

    fixture.link(
        ClaimLinkSource::Observation(observation_id("observation--span")),
        &grounded,
        ClaimLinkKind::Supports,
    );
    fixture.resolve(&grounded, "2026-08-30T10:00:00Z");

    let before_legacy = fixture.claims.claim_by_id(&legacy).expect("legacy").clone();
    let before_grounded = fixture
        .claims
        .claim_by_id(&grounded)
        .expect("grounded")
        .clone();
    let before_links = fixture.claims.claim_links().to_vec();
    let before_verdicts = fixture.verdicts.clone();
    let findings = validate_claim_reachability(
        &fixture.claims,
        &fixture.verdicts,
        &fixture.evidence,
        &fixture.observations,
        &fixture.sources,
    );

    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(
        findings[0].code(),
        CLAIM_LIFECYCLE_WITHOUT_OBSERVATION_PATH_CODE
    );
    assert_eq!(findings[0].severity(), ValidationErrorSeverity::Warning);
    assert_eq!(
        findings[0].target(),
        &ValidationTarget::claim(legacy.as_str())
    );
    assert!(findings[0].message().contains("legacy"));
    let _ = unresolved;

    assert_eq!(
        fixture.claims.claim_by_id(&legacy).expect("legacy"),
        &before_legacy,
        "the rule must not mutate claims"
    );
    assert_eq!(
        fixture.claims.claim_by_id(&grounded).expect("grounded"),
        &before_grounded
    );
    assert_eq!(fixture.claims.claim_links(), before_links.as_slice());
    assert_eq!(
        fixture.verdicts, before_verdicts,
        "the rule must not mutate verdicts"
    );
}

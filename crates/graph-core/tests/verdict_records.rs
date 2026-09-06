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
//! Integration contract for verification records, verdicts, and state
//! transitions (Epic 0029, WS-A item 5, issue #151).
//!
//! The verdict is a computed view over active evidence links and verification
//! records, appended with a bitemporal stamp and never rewritten. A state
//! transition is appended on every verdict change. The lifecycle `ClaimStatus`
//! follows the verdict through the ADR-0016 projection table when the
//! transition matrix allows it. Every historical verdict stays queryable at
//! any system time.
use graph_core::{
    BitemporalStamp, ClaimAnalyticalTarget, ClaimId, ClaimInput, ClaimLink, ClaimLinkKind,
    ClaimLinkSource, ClaimStatement, ClaimStatus, ClaimStore, ClaimTarget, Confidence,
    EvidenceRecordStore, EvidenceSourceType, GraphError, ObservationId, ObservationInput,
    ObservationModality, ObservationStore, ResolutionInputs, SourceId, SourceInput, SourceStore,
    TemporalTimestamp, TransitionTrigger, VerdictAsOf, VerdictId, VerdictState, VerdictStore,
    VerificationInputs, VerificationRecord, VerificationRecordId, VerificationRecordStore,
    VerificationResult, project_verdict_state, resolve_claim_verdict,
};

fn claim_id(value: &str) -> ClaimId {
    ClaimId::new(value).expect("test claim ID should be valid")
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

fn stamp(valid_from: &str, transaction: &str) -> BitemporalStamp {
    BitemporalStamp::new(ts(valid_from), ts(transaction)).expect("stamp should be valid")
}

fn create_claim(store: &mut ClaimStore, id: &str, asserted: bool) -> ClaimId {
    let id = claim_id(id);
    let input = ClaimInput::new(
        id.clone(),
        ClaimStatement::new(format!("statement of {}", id.as_str()))
            .expect("statement should be valid"),
        ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new(id.as_str(), None)),
    );
    if asserted {
        store.create_asserted_claim(input)
    } else {
        store.create_candidate_claim(input)
    }
    .expect("claim should be created");
    id
}

/// Observation-backed evidence: one source, observations `observation--<name>`.
fn grounding(names: &[&str]) -> (SourceStore, ObservationStore) {
    let mut sources = SourceStore::new();
    sources
        .register_source(SourceInput::new(
            source_id("source--grounding"),
            "https://grounding.example/doc",
            EvidenceSourceType::Document,
        ))
        .expect("grounding source");
    let mut observations = ObservationStore::new();
    for name in names {
        observations
            .create_observation(
                ObservationInput::new(
                    observation_id(&format!("observation--{name}")),
                    source_id("source--grounding"),
                    format!("span {name}"),
                    ObservationModality::Text,
                ),
                &sources,
            )
            .expect("grounding observation");
    }
    (sources, observations)
}

fn observation_link(store: &mut ClaimStore, name: &str, target: &ClaimId, kind: ClaimLinkKind) {
    let id = observation_id(&format!("observation--{name}"));
    store.register_observation(id.clone());
    store
        .attach_link(ClaimLink::new(
            ClaimLinkSource::Observation(id),
            target.clone(),
            kind,
        ))
        .expect("observation link should attach");
}

fn resolve(
    claims: &mut ClaimStore,
    verdicts: &mut VerdictStore,
    verifications: &VerificationRecordStore,
    observations: &ObservationStore,
    sources: &SourceStore,
    claim: &ClaimId,
    stamp: BitemporalStamp,
) -> Result<graph_core::ResolutionOutcome, GraphError> {
    let evidence = EvidenceRecordStore::new();
    resolve_claim_verdict(
        claims,
        verdicts,
        &ResolutionInputs::new(verifications, &evidence, observations, sources),
        claim,
        stamp,
        "ws-a-minimal-v1",
    )
}

fn verification(
    id: &str,
    claim: &ClaimId,
    result: VerificationResult,
    transaction: &str,
) -> VerificationRecord {
    VerificationRecord::new(
        VerificationRecordId::new(id).expect("record ID should be valid"),
        "verifier.identifier-syntax",
        "1.2.0",
        true,
        VerificationInputs::for_claim(claim.clone()),
        result,
        stamp("2026-08-01T00:00:00Z", transaction),
    )
    .with_rationale("CVE identifier matches the canonical pattern")
    .with_limit("syntax only; existence in the NVD is not checked")
    .with_evidence_consumed("observation:observation--span-1")
}

//
// Verify the ADR-0016 projection table from computed verdict state to the
// lifecycle status, including the actionability variant for `Supported`.
#[test]
fn projection_table_matches_adr_0016() {
    assert_eq!(
        project_verdict_state(VerdictState::Supported, false),
        ClaimStatus::Supported
    );
    assert_eq!(
        project_verdict_state(VerdictState::Supported, true),
        ClaimStatus::Validated
    );
    assert_eq!(
        project_verdict_state(VerdictState::Refuted, false),
        ClaimStatus::Contradicted
    );
    assert_eq!(
        project_verdict_state(VerdictState::Mixed, false),
        ClaimStatus::Disputed
    );
    assert_eq!(
        project_verdict_state(VerdictState::Contested, false),
        ClaimStatus::Disputed
    );
    assert_eq!(
        project_verdict_state(VerdictState::Unknown, false),
        ClaimStatus::Unresolved
    );
    assert_eq!(
        project_verdict_state(VerdictState::InsufficientEvidence, false),
        ClaimStatus::Unresolved
    );
    assert_eq!(
        project_verdict_state(VerdictState::Superseded, true),
        ClaimStatus::Superseded
    );
    assert_eq!(VerdictState::ALL.len(), 7);
    assert_eq!(
        VerdictState::InsufficientEvidence.as_str(),
        "insufficient_evidence"
    );
}

//
// Verify that verification records are append-only: an identical re-append
// is a no-op, a differing record under the same identifier is a conflict, and
// records are queryable per claim in transaction order.
#[test]
fn verification_records_are_append_only() {
    let mut claims = ClaimStore::new();
    let claim = create_claim(&mut claims, "claim--cve", true);
    let mut store = VerificationRecordStore::new();

    let record = verification(
        "verification--1",
        &claim,
        VerificationResult::Pass,
        "2026-08-30T10:00:00Z",
    );
    store
        .append(record.clone())
        .expect("first append should succeed");
    store
        .append(record.clone())
        .expect("identical re-append is idempotent");
    assert_eq!(store.len(), 1);

    let conflicting = verification(
        "verification--1",
        &claim,
        VerificationResult::Fail,
        "2026-08-30T10:00:00Z",
    );
    let error = store
        .append(conflicting)
        .expect_err("a differing record under the same id is a conflict");
    assert!(matches!(
        error,
        GraphError::ImmutableRecordConflict { kind: graph_core::ImmutableRecordKind::VerificationRecord, id }
            if id == "verification--1"
    ));

    store
        .append(verification(
            "verification--2",
            &claim,
            VerificationResult::Inconclusive,
            "2026-08-31T10:00:00Z",
        ))
        .expect("second record should append");
    let for_claim = store.records_for_claim(&claim);
    assert_eq!(for_claim.len(), 2);
    assert_eq!(for_claim[0].id(), record.id());
    assert!(for_claim[0].deterministic());
    assert_eq!(for_claim[0].verifier_id(), "verifier.identifier-syntax");
    assert_eq!(for_claim[0].verifier_version(), "1.2.0");
    assert_eq!(for_claim[0].result(), VerificationResult::Pass);
    assert_eq!(
        for_claim[0].limits(),
        ["syntax only; existence in the NVD is not checked"]
    );
    assert_eq!(
        for_claim[0].evidence_consumed(),
        ["observation:observation--span-1"]
    );
    assert_eq!(for_claim[1].result(), VerificationResult::Inconclusive);
}

//
// Verify the minimal resolution policy: supports only gives `Supported`,
// refutes only gives `Refuted`, both give `Mixed`, none gives `Unknown`, a
// superseding link gives `Superseded`; each resolution appends one verdict and
// one transition only when the state changes.
#[test]
fn minimal_resolution_covers_every_link_configuration() {
    let mut claims = ClaimStore::new();
    let mut verdicts = VerdictStore::new();
    let verifications = VerificationRecordStore::new();
    let (sources, observations) = grounding(&["s1", "s2", "r1", "r2"]);

    let bare = create_claim(&mut claims, "claim--bare", true);
    let supported = create_claim(&mut claims, "claim--supported", true);
    let refuted = create_claim(&mut claims, "claim--refuted", true);
    let mixed = create_claim(&mut claims, "claim--mixed", true);
    let old = create_claim(&mut claims, "claim--old", true);
    let newer = create_claim(&mut claims, "claim--newer", true);

    observation_link(&mut claims, "s1", &supported, ClaimLinkKind::Supports);
    observation_link(&mut claims, "r1", &refuted, ClaimLinkKind::Refutes);
    observation_link(&mut claims, "s2", &mixed, ClaimLinkKind::Supports);
    observation_link(&mut claims, "r2", &mixed, ClaimLinkKind::Refutes);
    claims
        .attach_superseding_claim_to_claim(newer.clone(), old.clone(), None)
        .expect("supersede");

    let expectations = [
        (&bare, VerdictState::Unknown, ClaimStatus::Unresolved),
        (&supported, VerdictState::Supported, ClaimStatus::Supported),
        (&refuted, VerdictState::Refuted, ClaimStatus::Contradicted),
        (&mixed, VerdictState::Mixed, ClaimStatus::Disputed),
        (&old, VerdictState::Superseded, ClaimStatus::Superseded),
    ];

    for (index, (claim, state, status)) in expectations.iter().enumerate() {
        let outcome = resolve(
            &mut claims,
            &mut verdicts,
            &verifications,
            &observations,
            &sources,
            claim,
            stamp(
                "2026-08-01T00:00:00Z",
                &format!("2026-08-30T10:0{index}:00Z"),
            ),
        )
        .expect("resolution should succeed");
        assert_eq!(outcome.state(), *state, "{}", claim.as_str());
        assert!(outcome.changed());
        assert!(outcome.lifecycle_applied(), "{}", claim.as_str());

        let current = verdicts
            .current_verdict(claim)
            .expect("verdict should exist");
        assert_eq!(current.state(), *state);
        assert_eq!(current.policy_version(), "ws-a-minimal-v1");
        assert!(current.confidence_dimensions().is_empty());
        assert_eq!(
            claims.claim_by_id(claim).expect("claim").status(),
            *status,
            "{}",
            claim.as_str()
        );

        let transitions = verdicts.transitions_for_claim(claim);
        assert_eq!(transitions.len(), 1);
        assert_eq!(transitions[0].from_state(), None);
        assert_eq!(transitions[0].to_state(), *state);
        let expected_trigger = if *state == VerdictState::Superseded {
            TransitionTrigger::Supersession
        } else {
            TransitionTrigger::ResolutionRun
        };
        assert_eq!(transitions[0].trigger(), &expected_trigger);
    }

    let unchanged = resolve(
        &mut claims,
        &mut verdicts,
        &verifications,
        &observations,
        &sources,
        &supported,
        stamp("2026-08-01T00:00:00Z", "2026-08-30T11:00:00Z"),
    )
    .expect("re-resolution should succeed");
    assert!(!unchanged.changed());
    assert_eq!(verdicts.verdicts_for_claim(&supported).len(), 1);
    assert_eq!(verdicts.transitions_for_claim(&supported).len(), 1);
}

//
// Verify that a verification record participates in the minimal policy: a
// deterministic failure refutes, a deterministic pass supports, and the
// verdict names the record in its transition trigger when it is the newest
// input.
#[test]
fn verification_records_feed_the_minimal_policy() {
    let mut claims = ClaimStore::new();
    let mut verdicts = VerdictStore::new();
    let mut verifications = VerificationRecordStore::new();
    let (sources, observations) = grounding(&["r1", "s1"]);
    let claim = create_claim(&mut claims, "claim--cve", true);

    // An observation-backed refutation grounds the claim; the deterministic
    // failure recorded later is the newest signal and names the transition.
    observation_link(&mut claims, "r1", &claim, ClaimLinkKind::Refutes);
    verifications
        .append(verification(
            "verification--fail",
            &claim,
            VerificationResult::Fail,
            "2026-08-30T10:00:00Z",
        ))
        .expect("record should append");

    let outcome = resolve(
        &mut claims,
        &mut verdicts,
        &verifications,
        &observations,
        &sources,
        &claim,
        stamp("2026-08-01T00:00:00Z", "2026-08-30T10:05:00Z"),
    )
    .expect("resolution should succeed");
    assert_eq!(outcome.state(), VerdictState::Refuted);
    let transitions = verdicts.transitions_for_claim(&claim);
    assert_eq!(
        transitions[0].trigger(),
        &TransitionTrigger::VerificationRecord(
            VerificationRecordId::new("verification--fail").expect("id")
        )
    );

    observation_link(&mut claims, "s1", &claim, ClaimLinkKind::Supports);
    let outcome = resolve(
        &mut claims,
        &mut verdicts,
        &verifications,
        &observations,
        &sources,
        &claim,
        stamp("2026-08-01T00:00:00Z", "2026-08-30T10:10:00Z"),
    )
    .expect("resolution should succeed");
    assert_eq!(outcome.state(), VerdictState::Mixed);
    assert_eq!(verdicts.transitions_for_claim(&claim).len(), 2);
}

//
// Verify the lifecycle projection is applied only when the transition matrix
// allows it: a candidate claim keeps its lifecycle status while its verdict is
// still recorded.
#[test]
fn lifecycle_projection_respects_the_transition_matrix() {
    let mut claims = ClaimStore::new();
    let mut verdicts = VerdictStore::new();
    let verifications = VerificationRecordStore::new();
    let (sources, observations) = grounding(&["s1"]);
    let candidate = create_claim(&mut claims, "claim--candidate", false);
    observation_link(&mut claims, "s1", &candidate, ClaimLinkKind::Supports);

    let outcome = resolve(
        &mut claims,
        &mut verdicts,
        &verifications,
        &observations,
        &sources,
        &candidate,
        stamp("2026-08-01T00:00:00Z", "2026-08-30T10:00:00Z"),
    )
    .expect("resolution should succeed");
    assert_eq!(outcome.state(), VerdictState::Supported);
    assert!(!outcome.lifecycle_applied());
    assert_eq!(
        claims.claim_by_id(&candidate).expect("claim").status(),
        ClaimStatus::Candidate
    );
    assert!(verdicts.current_verdict(&candidate).is_some());
}

//
// Spike E: a claim moves supported, refuted, superseded across three system
// times. Every historical verdict, the links active at that time, and the
// source path behind them stay queryable at any system time; nothing is
// deleted.
#[test]
fn spike_e_replays_supported_refuted_superseded_at_any_system_time() {
    let mut sources = SourceStore::new();
    sources
        .register_source(SourceInput::new(
            source_id("source--vendor"),
            "https://vendor.example/report.pdf",
            EvidenceSourceType::Document,
        ))
        .expect("source");
    sources
        .register_source(SourceInput::new(
            source_id("source--court"),
            "https://court.example/ruling.pdf",
            EvidenceSourceType::Document,
        ))
        .expect("source");
    let mut observations = ObservationStore::new();
    for (id, source, payload) in [
        (
            "observation--vendor-span",
            "source--vendor",
            "Actor A operates Campaign B.",
        ),
        (
            "observation--court-span",
            "source--court",
            "Campaign B was operated by Actor C.",
        ),
    ] {
        observations
            .create_observation(
                ObservationInput::new(
                    observation_id(id),
                    source_id(source),
                    payload,
                    ObservationModality::Text,
                ),
                &sources,
            )
            .expect("observation");
    }

    let mut claims = ClaimStore::new();
    let mut verdicts = VerdictStore::new();
    let verifications = VerificationRecordStore::new();
    let claim = create_claim(&mut claims, "claim--attribution", true);
    let replacement = create_claim(&mut claims, "claim--attribution-v2", true);
    claims.register_observation(observation_id("observation--vendor-span"));
    claims.register_observation(observation_id("observation--court-span"));

    // T1: the vendor span supports the claim.
    claims
        .attach_link(
            ClaimLink::new(
                ClaimLinkSource::Observation(observation_id("observation--vendor-span")),
                claim.clone(),
                ClaimLinkKind::Supports,
            )
            .with_bitemporal(stamp("2026-01-01T00:00:00Z", "2026-03-01T00:00:00Z")),
        )
        .expect("support link");
    let t1 = resolve(
        &mut claims,
        &mut verdicts,
        &verifications,
        &observations,
        &sources,
        &claim,
        stamp("2026-01-01T00:00:00Z", "2026-03-01T00:00:01Z"),
    )
    .expect("t1");
    assert_eq!(t1.state(), VerdictState::Supported);

    // T2: the court span refutes it and the vendor support is closed in valid time.
    claims
        .attach_link(
            ClaimLink::new(
                ClaimLinkSource::Observation(observation_id("observation--court-span")),
                claim.clone(),
                ClaimLinkKind::Refutes,
            )
            .with_bitemporal(stamp("2026-01-01T00:00:00Z", "2026-06-01T00:00:00Z")),
        )
        .expect("refute link");
    claims
        .close_link_validity(
            &ClaimLinkSource::Observation(observation_id("observation--vendor-span")),
            &claim,
            ClaimLinkKind::Supports,
            ts("2026-05-31T00:00:00Z"),
        )
        .expect("support validity should close");
    let t2 = resolve(
        &mut claims,
        &mut verdicts,
        &verifications,
        &observations,
        &sources,
        &claim,
        stamp("2026-06-01T00:00:00Z", "2026-06-01T00:00:01Z"),
    )
    .expect("t2");
    assert_eq!(t2.state(), VerdictState::Refuted);

    // T3: a newer claim supersedes it.
    claims
        .attach_link(
            ClaimLink::new(
                ClaimLinkSource::Claim(replacement.clone()),
                claim.clone(),
                ClaimLinkKind::Supersedes,
            )
            .with_bitemporal(stamp("2026-06-01T00:00:00Z", "2026-09-01T00:00:00Z")),
        )
        .expect("supersede link");
    let t3 = resolve(
        &mut claims,
        &mut verdicts,
        &verifications,
        &observations,
        &sources,
        &claim,
        stamp("2026-06-01T00:00:00Z", "2026-09-01T00:00:01Z"),
    )
    .expect("t3");
    assert_eq!(t3.state(), VerdictState::Superseded);
    assert_eq!(
        claims.claim_by_id(&claim).expect("claim").status(),
        ClaimStatus::Superseded
    );

    // Nothing was deleted.
    let history = verdicts.verdicts_for_claim(&claim);
    assert_eq!(history.len(), 3);
    assert_eq!(
        history
            .iter()
            .map(|verdict| verdict.state())
            .collect::<Vec<_>>(),
        [
            VerdictState::Supported,
            VerdictState::Refuted,
            VerdictState::Superseded
        ]
    );
    let transitions = verdicts.transitions_for_claim(&claim);
    assert_eq!(transitions.len(), 3);
    assert_eq!(transitions[1].from_state(), Some(VerdictState::Supported));
    assert_eq!(transitions[1].to_state(), VerdictState::Refuted);
    assert_eq!(transitions[2].trigger(), &TransitionTrigger::Supersession);
    assert_eq!(
        transitions[2].superseding_verdict_id(),
        Some(history[2].id())
    );
    assert_eq!(claims.claim_links().len(), 3);

    // Replay at each system time.
    let as_of = |valid: &str, system: &str| VerdictAsOf::new(ts(valid), ts(system));
    let at_t1 = verdicts
        .verdict_as_of(
            &claim,
            &as_of("2026-02-01T00:00:00Z", "2026-04-01T00:00:00Z"),
        )
        .expect("verdict at T1");
    assert_eq!(at_t1.state(), VerdictState::Supported);
    let at_t2 = verdicts
        .verdict_as_of(
            &claim,
            &as_of("2026-06-15T00:00:00Z", "2026-07-01T00:00:00Z"),
        )
        .expect("verdict at T2");
    assert_eq!(at_t2.state(), VerdictState::Refuted);
    let at_t3 = verdicts
        .verdict_as_of(
            &claim,
            &as_of("2026-06-15T00:00:00Z", "2026-10-01T00:00:00Z"),
        )
        .expect("verdict at T3");
    assert_eq!(at_t3.state(), VerdictState::Superseded);
    assert!(
        verdicts
            .verdict_as_of(
                &claim,
                &as_of("2026-01-15T00:00:00Z", "2026-02-01T00:00:00Z")
            )
            .is_none(),
        "before the first resolution there is no verdict"
    );

    // Links active at T1 resolve to the vendor source; at T2 to the court source.
    let links_t1 = claims.links_active_at(
        &claim,
        &as_of("2026-02-01T00:00:00Z", "2026-04-01T00:00:00Z"),
    );
    assert_eq!(links_t1.len(), 1);
    let vendor = observations
        .observation_by_id(links_t1[0].source().observation_id().expect("observation"))
        .expect("observation");
    assert_eq!(vendor.source_id(), &source_id("source--vendor"));

    let links_t2 = claims.links_active_at(
        &claim,
        &as_of("2026-06-15T00:00:00Z", "2026-07-01T00:00:00Z"),
    );
    assert_eq!(links_t2.len(), 1);
    assert_eq!(links_t2[0].kind(), ClaimLinkKind::Refutes);
    let court = observations
        .observation_by_id(links_t2[0].source().observation_id().expect("observation"))
        .expect("observation");
    assert_eq!(court.source_id(), &source_id("source--court"));

    // "Was true then" versus "is currently supported".
    assert_eq!(at_t1.state(), VerdictState::Supported);
    assert_eq!(
        verdicts.current_verdict(&claim).expect("current").state(),
        VerdictState::Superseded
    );
}

//
// Verify deterministic as-of selection: among verdicts valid at the as-of
// valid time and recorded no later than the as-of system time, the latest
// transaction time wins and an exact tie is broken by the greater verdict id.
#[test]
fn as_of_selection_is_deterministic_with_id_tie_break() {
    let mut store = VerdictStore::new();
    let claim = claim_id("claim--tie");
    let dimension = |value: f64| graph_core::ConfidenceDimensions {
        evidence_sufficiency: Some(Confidence::new(value).expect("confidence")),
        ..Default::default()
    };

    store
        .append_verdict(
            VerdictId::new("verdict--a").expect("id"),
            claim.clone(),
            VerdictState::Supported,
            dimension(0.4),
            "policy-v1",
            stamp("2026-01-01T00:00:00Z", "2026-03-01T00:00:00Z"),
        )
        .expect("append a");
    store
        .append_verdict(
            VerdictId::new("verdict--b").expect("id"),
            claim.clone(),
            VerdictState::Refuted,
            dimension(0.7),
            "policy-v1",
            stamp("2026-01-01T00:00:00Z", "2026-03-01T00:00:00Z"),
        )
        .expect("append b");

    let picked = store
        .verdict_as_of(
            &claim,
            &VerdictAsOf::new(ts("2026-02-01T00:00:00Z"), ts("2026-04-01T00:00:00Z")),
        )
        .expect("verdict");
    assert_eq!(
        picked.id().as_str(),
        "verdict--b",
        "greater id wins an exact tie"
    );
    assert_eq!(
        picked.confidence_dimensions().evidence_sufficiency,
        Some(Confidence::new(0.7).expect("confidence"))
    );

    let duplicate = store.append_verdict(
        VerdictId::new("verdict--b").expect("id"),
        claim.clone(),
        VerdictState::Supported,
        dimension(0.1),
        "policy-v1",
        stamp("2026-01-01T00:00:00Z", "2026-03-02T00:00:00Z"),
    );
    assert!(matches!(
        duplicate,
        Err(GraphError::ImmutableRecordConflict { .. })
    ));
    assert_eq!(store.verdicts_for_claim(&claim).len(), 2);

    let json = serde_json::to_string(&store).expect("store should serialize");
    let restored: VerdictStore = serde_json::from_str(&json).expect("store should deserialize");
    assert_eq!(restored, store);
}

// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//! Acceptance suite for Epic 0029 workstream WS-A (issue #153, epic #147).
//!
//! Gates:
//! - Spike E: a claim moves supported, refuted, superseded; every historical
//!   verdict, its links, and its source path stay queryable at any system
//!   time, across a persistence-snapshot round trip; nothing is deleted.
//! - A report of 20 propositions renders claim-by-claim verdicts, not one
//!   document-level score, through the epistemic projection.
//! - Every trusted claim renders the verbatim span of the observation behind
//!   it.
//! - No client path sets a verdict: the projection is read-only and the
//!   `Verdict` type has no public constructor.
use graph_core::{
    BitemporalStamp, ClaimAnalyticalTarget, ClaimId, ClaimInput, ClaimLink, ClaimLinkKind,
    ClaimLinkSource, ClaimProposition, ClaimPropositionObject, ClaimStatement, ClaimStatus,
    ClaimTarget, EpistemicStores, EvidenceRecordStore, EvidenceSourceType, Graph, NodeId,
    ObservationId, ObservationInput, ObservationModality, PropertyValue, ResolutionInputs,
    SourceId, SourceInput, TemporalTimestamp, VerdictAsOf, VerdictState, resolve_claim_verdict,
};

fn claim_id(value: &str) -> ClaimId {
    ClaimId::new(value).expect("claim id")
}

fn ts(value: &str) -> TemporalTimestamp {
    TemporalTimestamp::new(value).expect("timestamp")
}

fn stamp(valid_from: &str, transaction: &str) -> BitemporalStamp {
    BitemporalStamp::new(ts(valid_from), ts(transaction)).expect("stamp")
}

fn register_source(stores: &mut EpistemicStores, id: &str, uri: &str) {
    stores
        .sources
        .register_source(SourceInput::new(
            SourceId::new(id).expect("source id"),
            uri,
            EvidenceSourceType::Document,
        ))
        .expect("source");
}

fn observe(stores: &mut EpistemicStores, id: &str, source: &str, payload: &str) -> ObservationId {
    let id = ObservationId::new(id).expect("observation id");
    stores
        .observations
        .create_observation(
            ObservationInput::new(
                id.clone(),
                SourceId::new(source).expect("source id"),
                payload,
                ObservationModality::Text,
            ),
            &stores.sources,
        )
        .expect("observation");
    stores.claims.register_observation(id.clone());
    id
}

fn assert_claim(
    stores: &mut EpistemicStores,
    id: &str,
    subject: &str,
    predicate: &str,
    object: &str,
) -> ClaimId {
    let id = claim_id(id);
    stores
        .claims
        .create_asserted_claim(
            ClaimInput::new(
                id.clone(),
                ClaimStatement::new(format!("{subject} {predicate} {object}")).expect("statement"),
                ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new(id.as_str(), None)),
            )
            .with_proposition(
                ClaimProposition::new(
                    subject,
                    predicate,
                    ClaimPropositionObject::Literal(PropertyValue::String(object.to_owned())),
                )
                .expect("proposition"),
            ),
        )
        .expect("claim");
    id
}

fn link(
    stores: &mut EpistemicStores,
    source: ClaimLinkSource,
    target: &ClaimId,
    kind: ClaimLinkKind,
    stamp: Option<BitemporalStamp>,
) {
    let mut link = ClaimLink::new(source, target.clone(), kind);
    if let Some(stamp) = stamp {
        link = link.with_bitemporal(stamp);
    }
    stores.claims.attach_link(link).expect("link");
}

fn resolve(stores: &mut EpistemicStores, claim: &ClaimId, stamp: BitemporalStamp) -> VerdictState {
    let evidence = EvidenceRecordStore::new();
    let mut claims = std::mem::take(&mut stores.claims);
    let mut verdicts = std::mem::take(&mut stores.verdicts);
    let outcome = {
        let inputs = ResolutionInputs::new(
            &stores.verifications,
            &evidence,
            &stores.observations,
            &stores.sources,
        );
        resolve_claim_verdict(
            &mut claims,
            &mut verdicts,
            &inputs,
            claim,
            stamp,
            "ws-a-minimal-v1",
        )
        .expect("resolution")
    };
    stores.claims = claims;
    stores.verdicts = verdicts;
    outcome.state()
}

//
// Gate: Spike E across a persistence-snapshot round trip.
#[test]
fn spike_e_survives_a_persistence_round_trip() {
    let mut stores = EpistemicStores::default();
    register_source(
        &mut stores,
        "source--vendor",
        "https://vendor.example/report.pdf",
    );
    register_source(
        &mut stores,
        "source--court",
        "https://court.example/ruling.pdf",
    );
    let vendor = observe(
        &mut stores,
        "observation--vendor",
        "source--vendor",
        "Actor A operates Campaign B.",
    );
    let court = observe(
        &mut stores,
        "observation--court",
        "source--court",
        "Campaign B was operated by Actor C.",
    );
    let claim = assert_claim(
        &mut stores,
        "claim--attribution",
        "actor--a",
        "operates",
        "campaign--b",
    );
    let replacement = assert_claim(
        &mut stores,
        "claim--attribution-v2",
        "actor--c",
        "operates",
        "campaign--b",
    );

    link(
        &mut stores,
        ClaimLinkSource::Observation(vendor.clone()),
        &claim,
        ClaimLinkKind::Supports,
        Some(stamp("2026-01-01T00:00:00Z", "2026-03-01T00:00:00Z")),
    );
    assert_eq!(
        resolve(
            &mut stores,
            &claim,
            stamp("2026-01-01T00:00:00Z", "2026-03-01T00:00:01Z")
        ),
        VerdictState::Supported
    );

    link(
        &mut stores,
        ClaimLinkSource::Observation(court.clone()),
        &claim,
        ClaimLinkKind::Refutes,
        Some(stamp("2026-01-01T00:00:00Z", "2026-06-01T00:00:00Z")),
    );
    stores
        .claims
        .close_link_validity(
            &ClaimLinkSource::Observation(vendor.clone()),
            &claim,
            ClaimLinkKind::Supports,
            ts("2026-05-31T00:00:00Z"),
        )
        .expect("close");
    assert_eq!(
        resolve(
            &mut stores,
            &claim,
            stamp("2026-06-01T00:00:00Z", "2026-06-01T00:00:01Z")
        ),
        VerdictState::Refuted
    );

    link(
        &mut stores,
        ClaimLinkSource::Claim(replacement),
        &claim,
        ClaimLinkKind::Supersedes,
        Some(stamp("2026-06-01T00:00:00Z", "2026-09-01T00:00:00Z")),
    );
    assert_eq!(
        resolve(
            &mut stores,
            &claim,
            stamp("2026-06-01T00:00:00Z", "2026-09-01T00:00:01Z")
        ),
        VerdictState::Superseded
    );

    // Persist and restore through the graph snapshot.
    let mut graph = Graph::new();
    graph.replace_epistemic_stores(stores);
    let restored = Graph::from_persistence_snapshot(graph.persistence_snapshot()).expect("restore");
    let stores = restored.epistemic_stores();

    let as_of = |valid: &str, system: &str| VerdictAsOf::new(ts(valid), ts(system));
    let expectations = [
        (
            "2026-02-01T00:00:00Z",
            "2026-04-01T00:00:00Z",
            VerdictState::Supported,
            "source--vendor",
        ),
        (
            "2026-06-15T00:00:00Z",
            "2026-07-01T00:00:00Z",
            VerdictState::Refuted,
            "source--court",
        ),
    ];
    for (valid, system, state, source) in expectations {
        let verdict = stores
            .verdicts
            .verdict_as_of(&claim, &as_of(valid, system))
            .expect("verdict");
        assert_eq!(verdict.state(), state);
        let links = stores.claims.links_active_at(&claim, &as_of(valid, system));
        let observation = stores
            .observations
            .observation_by_id(
                links[0]
                    .source()
                    .observation_id()
                    .expect("observation source"),
            )
            .expect("observation");
        assert_eq!(observation.source_id().as_str(), source);
    }
    assert_eq!(
        stores
            .verdicts
            .verdict_as_of(
                &claim,
                &as_of("2026-06-15T00:00:00Z", "2026-10-01T00:00:00Z")
            )
            .expect("t3")
            .state(),
        VerdictState::Superseded
    );
    assert_eq!(stores.verdicts.verdicts_for_claim(&claim).len(), 3);
    assert_eq!(stores.verdicts.transitions_for_claim(&claim).len(), 3);
    assert_eq!(stores.claims.claim_links().len(), 3);
    assert_eq!(
        stores.claims.claim_by_id(&claim).expect("claim").status(),
        ClaimStatus::Superseded
    );
}

//
// Gate: a 20-proposition report renders claim-by-claim verdicts through the
// projection, with mixed states and no document-level score; every trusted
// claim renders the verbatim span behind it.
#[test]
fn twenty_proposition_report_renders_claim_by_claim_verdicts_and_spans() {
    let mut stores = EpistemicStores::default();
    register_source(
        &mut stores,
        "source--report",
        "https://vendor.example/quarterly-report.pdf",
    );
    register_source(
        &mut stores,
        "source--rebuttal",
        "https://other.example/rebuttal.html",
    );

    let mut expected = Vec::new();
    for index in 0..20 {
        let claim = assert_claim(
            &mut stores,
            &format!("claim--report-{index:02}"),
            &format!("actor--{index:02}"),
            "targets",
            &format!("sector--{}", index % 4),
        );
        let state = match index % 4 {
            0 => {
                let span = observe(
                    &mut stores,
                    &format!("observation--report-{index:02}"),
                    "source--report",
                    &format!("Paragraph {index}: actor {index:02} targets sector 0."),
                );
                link(
                    &mut stores,
                    ClaimLinkSource::Observation(span),
                    &claim,
                    ClaimLinkKind::Supports,
                    None,
                );
                VerdictState::Supported
            }
            1 => {
                let span = observe(
                    &mut stores,
                    &format!("observation--rebuttal-{index:02}"),
                    "source--rebuttal",
                    &format!("Rebuttal {index}: actor {index:02} does not target sector 1."),
                );
                link(
                    &mut stores,
                    ClaimLinkSource::Observation(span),
                    &claim,
                    ClaimLinkKind::Refutes,
                    None,
                );
                VerdictState::Refuted
            }
            2 => {
                let support = observe(
                    &mut stores,
                    &format!("observation--report-{index:02}"),
                    "source--report",
                    &format!("Paragraph {index}: actor {index:02} targets sector 2."),
                );
                let refute = observe(
                    &mut stores,
                    &format!("observation--rebuttal-{index:02}"),
                    "source--rebuttal",
                    &format!("Rebuttal {index}: sector 2 attribution is contested."),
                );
                link(
                    &mut stores,
                    ClaimLinkSource::Observation(support),
                    &claim,
                    ClaimLinkKind::Supports,
                    None,
                );
                link(
                    &mut stores,
                    ClaimLinkSource::Observation(refute),
                    &claim,
                    ClaimLinkKind::Refutes,
                    None,
                );
                VerdictState::Mixed
            }
            _ => VerdictState::Unknown,
        };
        let resolved = resolve(
            &mut stores,
            &claim,
            stamp(
                "2026-08-01T00:00:00Z",
                &format!("2026-08-30T10:{index:02}:00Z"),
            ),
        );
        assert_eq!(resolved, state, "{}", claim.as_str());
        expected.push((claim, state));
    }

    let mut graph = Graph::new();
    graph.replace_epistemic_stores(stores);
    let projection = graph.epistemic_projection().expect("projection");
    let nodes = projection.list_nodes().expect("nodes");

    let claim_nodes: Vec<_> = nodes
        .iter()
        .filter(|node| node.has_label("Claim"))
        .collect();
    assert_eq!(claim_nodes.len(), 20);
    for (claim, state) in &expected {
        let node = claim_nodes
            .iter()
            .find(|node| {
                node.property("claim_id") == Some(&PropertyValue::String(claim.as_str().to_owned()))
            })
            .expect("claim node");
        assert_eq!(
            node.property("verdict_state"),
            Some(&PropertyValue::String(state.as_str().to_owned())),
            "{}",
            claim.as_str()
        );
    }
    let distinct_states: std::collections::BTreeSet<String> = claim_nodes
        .iter()
        .filter_map(|node| match node.property("verdict_state") {
            Some(PropertyValue::String(state)) => Some(state.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        distinct_states.len(),
        4,
        "the report shows mixed states claim by claim"
    );
    assert!(
        nodes
            .iter()
            .all(|node| node.property("report_score").is_none()),
        "no document-level score exists"
    );

    // Every trusted (Supported) claim renders the verbatim span behind it.
    let stores = graph.epistemic_stores();
    let trusted: Vec<_> = expected
        .iter()
        .filter(|(_, state)| *state == VerdictState::Supported)
        .collect();
    assert_eq!(trusted.len(), 5);
    for (claim, _) in trusted {
        let links = stores.claims.links_active_at(
            claim,
            &VerdictAsOf::new(ts("2026-08-01T00:00:00Z"), ts("2026-12-31T00:00:00Z")),
        );
        let observation = stores
            .observations
            .observation_by_id(links[0].source().observation_id().expect("observation"))
            .expect("observation");
        assert!(
            observation.payload().starts_with("Paragraph "),
            "verbatim span renders: {}",
            observation.payload()
        );
        assert!(
            stores
                .sources
                .current_source(observation.source_id())
                .is_some()
        );
    }
}

//
// Gate: the projection is read-only and verdict nodes carry no write path; the
// projected graph has generated node ids that never collide with source ids.
#[test]
fn projection_is_read_only_and_ids_are_synthetic() {
    let mut graph = Graph::new();
    let mut stores = EpistemicStores::default();
    register_source(
        &mut stores,
        "source--report",
        "https://vendor.example/report.pdf",
    );
    let span = observe(
        &mut stores,
        "observation--span",
        "source--report",
        "payload",
    );
    let claim = assert_claim(&mut stores, "claim--x", "a", "b", "c");
    link(
        &mut stores,
        ClaimLinkSource::Observation(span),
        &claim,
        ClaimLinkKind::Supports,
        None,
    );
    resolve(
        &mut stores,
        &claim,
        stamp("2026-08-01T00:00:00Z", "2026-08-30T10:00:00Z"),
    );
    graph.replace_epistemic_stores(stores);

    let before = graph.persistence_snapshot();
    let projection = graph.epistemic_projection().expect("projection");
    let after = graph.persistence_snapshot();
    assert_eq!(
        serde_json::to_string(&before).expect("json"),
        serde_json::to_string(&after).expect("json"),
        "building the projection leaves the source graph byte-identical"
    );
    assert!(
        projection
            .get_node(&NodeId::new("claim--x").expect("id"))
            .expect("lookup")
            .is_none(),
        "record ids are properties, not node ids"
    );
    assert!(
        projection.epistemic_stores().sources.is_empty(),
        "the projection does not carry the stores again"
    );
}

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
//! Integration contract for the epistemic store bundle carried by `Graph`
//! (Epic 0029, WS-A item 7, issue #153).
//!
//! `EpistemicStores` bundles sources, observations, claims, verification
//! records, and verdicts so the graph, its persistence snapshot, and the
//! durable store move them together. A read-only projection renders every
//! governed record as nodes and relationships of the epistemic vocabulary so
//! Cypher reads can traverse them without a bespoke API.
use graph_core::{
    BitemporalStamp, ClaimAnalyticalTarget, ClaimId, ClaimInput, ClaimLink, ClaimLinkKind,
    ClaimLinkSource, ClaimProposition, ClaimPropositionObject, ClaimStatement, ClaimStatus,
    ClaimStore, ClaimTarget, EpistemicNodeKind, EpistemicRelationKind, EpistemicStores,
    EvidenceInput, EvidenceRecordStore, EvidenceSourceType, Graph, GraphPersistenceSnapshot,
    ObservationId, ObservationInput, ObservationModality, PropertyValue, ResolutionInputs,
    SourceId, SourceInput, TemporalTimestamp, VerdictState, VerificationInputs, VerificationRecord,
    VerificationRecordId, VerificationResult, resolve_claim_verdict,
};

fn claim_id(value: &str) -> ClaimId {
    ClaimId::new(value).expect("test claim ID should be valid")
}

fn ts(value: &str) -> TemporalTimestamp {
    TemporalTimestamp::new(value).expect("test timestamp should be valid")
}

fn stamp(transaction: &str) -> BitemporalStamp {
    BitemporalStamp::new(ts("2026-08-01T00:00:00Z"), ts(transaction)).expect("stamp")
}

/// One grounded claim: a source, an observation, a claim with a proposition,
/// a support link from the observation, a deterministic verification record,
/// and a resolved verdict.
fn grounded_stores() -> EpistemicStores {
    let mut stores = EpistemicStores::default();
    stores
        .sources
        .register_source(SourceInput::new(
            SourceId::new("source--report").expect("id"),
            "https://vendor.example/report.pdf",
            EvidenceSourceType::Document,
        ))
        .expect("source");
    stores
        .observations
        .create_observation(
            ObservationInput::new(
                ObservationId::new("observation--span").expect("id"),
                SourceId::new("source--report").expect("id"),
                "Actor A operates Campaign B.",
                ObservationModality::Text,
            ),
            &stores.sources,
        )
        .expect("observation");
    stores
        .claims
        .create_asserted_claim(
            ClaimInput::new(
                claim_id("claim--attribution"),
                ClaimStatement::new("Actor A operates Campaign B").expect("statement"),
                ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new("attribution", None)),
            )
            .with_proposition(
                ClaimProposition::new(
                    "actor--a",
                    "operates",
                    ClaimPropositionObject::Literal(PropertyValue::String("campaign--b".into())),
                )
                .expect("proposition"),
            ),
        )
        .expect("claim");
    stores
        .claims
        .register_observation(ObservationId::new("observation--span").expect("id"));
    stores
        .claims
        .attach_link(ClaimLink::new(
            ClaimLinkSource::Observation(ObservationId::new("observation--span").expect("id")),
            claim_id("claim--attribution"),
            ClaimLinkKind::Supports,
        ))
        .expect("link");
    stores
        .verifications
        .append(VerificationRecord::new(
            VerificationRecordId::new("verification--syntax").expect("id"),
            "verifier.identifier-syntax",
            "1.0.0",
            true,
            VerificationInputs::for_claim(claim_id("claim--attribution")),
            VerificationResult::Pass,
            stamp("2026-08-30T09:00:00Z"),
        ))
        .expect("verification");

    let evidence = EvidenceRecordStore::new();
    let inputs = ResolutionInputs::new(
        &stores.verifications,
        &evidence,
        &stores.observations,
        &stores.sources,
    );
    let mut claims = std::mem::take(&mut stores.claims);
    let mut verdicts = std::mem::take(&mut stores.verdicts);
    resolve_claim_verdict(
        &mut claims,
        &mut verdicts,
        &inputs,
        &claim_id("claim--attribution"),
        stamp("2026-08-30T10:00:00Z"),
        "ws-a-minimal-v1",
    )
    .expect("resolution");
    stores.claims = claims;
    stores.verdicts = verdicts;
    stores
}

//
// Verify that the bundle round-trips through serde with every store intact,
// including the claim store, whose maps are snapshotted deterministically.
#[test]
fn epistemic_stores_round_trip_through_serde() {
    let stores = grounded_stores();

    let json = serde_json::to_string(&stores).expect("stores should serialize");
    let restored: EpistemicStores = serde_json::from_str(&json).expect("stores should deserialize");
    assert_eq!(restored, stores);

    let again = serde_json::to_string(&restored).expect("stores should serialize again");
    assert_eq!(again, json, "serialization is deterministic");

    let claim = restored
        .claims
        .claim_by_id(&claim_id("claim--attribution"))
        .expect("claim survives");
    assert_eq!(claim.status(), ClaimStatus::Supported);
    assert!(claim.proposition().is_some());
    assert_eq!(restored.claims.claim_links().len(), 1);
    assert_eq!(
        restored
            .claims
            .explain_claim(&claim_id("claim--attribution"))
            .expect("explanations survive")
            .len(),
        2
    );
    assert_eq!(
        restored
            .verdicts
            .current_verdict(&claim_id("claim--attribution"))
            .expect("verdict survives")
            .state(),
        VerdictState::Supported
    );
}

//
// Verify that the claim store snapshot preserves every sub-collection, not
// only claims and links: decisions, stances, hypothesis workspaces, trust
// inputs, resolution policies, and explanations.
#[test]
fn claim_store_snapshot_preserves_every_collection() {
    let mut store = ClaimStore::new();
    let kept = claim_id("claim--kept");
    let retracted = claim_id("claim--retracted");
    for id in [&kept, &retracted] {
        store
            .create_asserted_claim(ClaimInput::new(
                id.clone(),
                ClaimStatement::new(id.as_str()).expect("statement"),
                ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new(id.as_str(), None)),
            ))
            .expect("claim");
    }
    store
        .retract_claim(
            retracted.clone(),
            "superseded by analyst".to_owned(),
            None,
            None,
        )
        .expect("retraction");
    store.register_trust_subject("source--x".to_owned());
    store
        .register_resolution_policy(graph_core::EpistemicResolutionPolicyRegistration::new(
            "conservative".to_owned(),
            graph_core::EpistemicResolutionPolicyKind::ConservativeDeterministic,
        ))
        .expect("policy");

    let json = serde_json::to_string(&store).expect("claim store should serialize");
    let restored: ClaimStore = serde_json::from_str(&json).expect("claim store should deserialize");
    assert_eq!(restored, store);
    assert_eq!(
        restored
            .claim_decisions_for_claim(&retracted)
            .expect("decisions")
            .len(),
        1
    );
    assert!(restored.resolution_policy_by_name("conservative").is_ok());
}

//
// Verify that the graph carries the bundle, that its persistence snapshot
// includes it, and that a snapshot written before WS-A (no `epistemic` key)
// restores with empty stores.
#[test]
fn graph_persistence_snapshot_carries_epistemic_stores() {
    let mut graph = Graph::new();
    assert!(graph.epistemic_stores().sources.is_empty());
    graph.replace_epistemic_stores(grounded_stores());

    let snapshot = graph.persistence_snapshot();
    let restored = Graph::from_persistence_snapshot(snapshot.clone()).expect("restore");
    assert_eq!(restored.epistemic_stores(), graph.epistemic_stores());

    let mut json = serde_json::to_value(&snapshot).expect("snapshot should serialize");
    assert!(json.get("epistemic").is_some());
    json.as_object_mut().expect("object").remove("epistemic");
    let legacy: GraphPersistenceSnapshot =
        serde_json::from_value(json).expect("pre-WS-A snapshot should deserialize");
    let legacy_graph = Graph::from_persistence_snapshot(legacy).expect("restore legacy");
    assert_eq!(legacy_graph.epistemic_stores(), &EpistemicStores::default());
}

//
// Verify the read-only projection: every governed record becomes a node in
// the epistemic vocabulary with namespaced properties, and the relations
// between them become vocabulary relationships. The source graph is not
// mutated.
#[test]
fn epistemic_projection_renders_records_in_the_vocabulary() {
    let mut graph = Graph::new();
    graph
        .create_evidence(EvidenceInput::new(
            graph_core::EvidenceId::new("evidence--legacy").expect("id"),
            "ref--legacy",
            "legacy payload",
        ))
        .expect("evidence");
    graph.replace_epistemic_stores(grounded_stores());
    let node_count_before = graph.list_nodes().expect("nodes").len();

    let projection = graph
        .epistemic_projection()
        .expect("projection should build");
    assert_eq!(
        graph.list_nodes().expect("nodes").len(),
        node_count_before,
        "projection must not mutate the source graph"
    );

    let nodes = projection.list_nodes().expect("projection nodes");
    let count = |kind: &str| nodes.iter().filter(|node| node.has_label(kind)).count();
    assert_eq!(count(EpistemicNodeKind::Source.canonical_label()), 1);
    assert_eq!(count(EpistemicNodeKind::Observation.canonical_label()), 1);
    assert_eq!(count(EpistemicNodeKind::Claim.canonical_label()), 1);
    assert_eq!(count(EpistemicNodeKind::Evidence.canonical_label()), 1);
    assert_eq!(count("Verdict"), 1);
    assert_eq!(count("VerificationRecord"), 1);
    assert_eq!(count("StateTransition"), 1);
    assert_eq!(
        count(EpistemicNodeKind::Assessment.canonical_label()),
        2,
        "verdicts and verification records are assessments"
    );
    assert_eq!(count(EpistemicNodeKind::Decision.canonical_label()), 1);

    let claim = nodes
        .iter()
        .find(|node| node.has_label("Claim"))
        .expect("claim node");
    assert_eq!(
        claim.property("claim_id"),
        Some(&PropertyValue::String("claim--attribution".to_owned()))
    );
    assert_eq!(
        claim.property("claim_status"),
        Some(&PropertyValue::String("supported".to_owned()))
    );
    assert_eq!(
        claim.property("verdict_state"),
        Some(&PropertyValue::String("supported".to_owned()))
    );
    assert_eq!(
        claim.property("verdict_lifecycle_projection"),
        Some(&PropertyValue::String("supported".to_owned()))
    );
    assert_eq!(
        claim.property("proposition_predicate"),
        Some(&PropertyValue::String("operates".to_owned()))
    );

    let verdict = nodes
        .iter()
        .find(|node| node.has_label("Verdict"))
        .expect("verdict node");
    assert_eq!(
        verdict.property("verdict_state"),
        Some(&PropertyValue::String("supported".to_owned()))
    );
    assert_eq!(
        verdict.property("verdict_policy_version"),
        Some(&PropertyValue::String("ws-a-minimal-v1".to_owned()))
    );

    let relationships = projection.list_relationships().expect("relationships");
    let rel_count = |kind: EpistemicRelationKind| {
        relationships
            .iter()
            .filter(|relationship| relationship.rel_type() == &kind.canonical_relationship_type())
            .count()
    };
    assert_eq!(
        rel_count(EpistemicRelationKind::Reports),
        1,
        "source REPORTS observation"
    );
    assert_eq!(
        rel_count(EpistemicRelationKind::Supports),
        1,
        "observation SUPPORTS claim"
    );
    assert_eq!(
        rel_count(EpistemicRelationKind::Assesses),
        2,
        "verdict and verification record ASSESS the claim"
    );
    assert_eq!(
        rel_count(EpistemicRelationKind::Decides),
        1,
        "transition DECIDES the claim"
    );
}

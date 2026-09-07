// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//! WS-G acceptance gate for the core side of epic #195 (issue #221).
//!
//! Reuse the canonical fixtures so the workstream gate cannot silently diverge
//! from the individual feature contracts, and pull in the WS-D gate the
//! workstream depends on.
//!
//! Epic criteria proven here:
//!
//! - campaign clustering by narrative, infrastructure and probable generation
//!   pipeline works independently on one fixture corpus
//!   (`coordination::campaign_clustering_works_independently_by_narrative_infrastructure_and_pipeline`);
//! - a generation fingerprint alone never produces an attribution claim
//!   (`coordination::a_generation_fingerprint_alone_never_supports_attribution`);
//! - an installation without the pack keeps every base capability
//!   (`an_installation_without_the_pack_keeps_every_base_capability`).
//!
//! The remaining criteria belong to the exporter and the pack: FIMI export
//! separation and byte-identity are gated by
//! `export-fimi/tests/epic_0029_ws_g_acceptance.rs`, the misleadingness
//! mechanisms and their traceability by `corrobore-domain-fimi`, and the
//! measured mechanism and grounding metrics by the misleadingness suite in
//! `corrobore-benchmarks`. Vocabulary neutrality is gated by
//! `scripts/ws-g-neutrality.test.mjs`.
#![allow(clippy::unwrap_used)]

#[path = "campaign_signals.rs"]
mod coordination;
#[path = "narrative_campaign_records.rs"]
mod primitives;
#[path = "epic_0029_ws_d_acceptance.rs"]
mod ws_d_clusters;

use graph_core::*;

// An installation with no domain pack keeps the whole base path: register a
// source, observe it, claim it, resolve a verdict, and plan an export. The
// collection and coordination stores stay empty and cost nothing, which is what
// "the core carries structure, the pack carries meaning" has to mean in
// practice.
#[test]
fn an_installation_without_the_pack_keeps_every_base_capability() {
    let time = TemporalTimestamp::new("2026-09-07T00:00:00Z").unwrap();
    let stamp = BitemporalStamp::new(time.clone(), time).unwrap();
    let mut graph = Graph::new();
    let node = graph
        .create_node(NodeInput::new(["Entity"]).with_status(RecordStatus::Exportable))
        .unwrap();
    let claim = ClaimId::new("claim--base-capability").unwrap();
    let source = SourceId::new("source--base").unwrap();
    let observation = ObservationId::new("observation--base").unwrap();
    let stores = graph.epistemic_stores_mut();
    stores
        .sources
        .register_source(SourceInput::new(
            source.clone(),
            "https://base.test/report",
            EvidenceSourceType::Document,
        ))
        .unwrap();
    stores
        .observations
        .create_observation(
            ObservationInput::new(
                observation.clone(),
                source,
                "the recorded measurement",
                ObservationModality::Text,
            ),
            &stores.sources,
        )
        .unwrap();
    stores
        .claims
        .create_asserted_claim(ClaimInput::new(
            claim.clone(),
            ClaimStatement::new("the measurement was recorded").unwrap(),
            ClaimTarget::Node(node),
        ))
        .unwrap();
    stores.claims.register_observation(observation.clone());
    stores
        .claims
        .attach_link(
            ClaimLink::new(
                ClaimLinkSource::Observation(observation),
                claim.clone(),
                ClaimLinkKind::Supports,
            )
            .with_bitemporal(stamp.clone()),
        )
        .unwrap();
    let evidence = EvidenceRecordStore::new();
    let inputs = ResolutionInputs::new(
        &stores.verifications,
        &evidence,
        &stores.observations,
        &stores.sources,
    );
    resolve_claim_verdict(
        &mut stores.claims,
        &mut stores.verdicts,
        &inputs,
        &claim,
        stamp,
        "ws-a-minimal-v1",
    )
    .unwrap();

    assert_eq!(
        graph
            .epistemic_stores()
            .verdicts
            .current_verdict(&claim)
            .unwrap()
            .state(),
        VerdictState::Supported
    );
    assert!(graph.epistemic_stores().narrative_campaigns.is_empty());
    assert!(graph.evidence_store().campaign_signals().is_empty());
    let exported = graph.export_memory_json().unwrap();
    assert!(!exported.contains("narrative"));
    assert!(!exported.contains("campaign"));
    assert_eq!(
        Graph::from_memory_json(&exported)
            .unwrap()
            .export_memory_json()
            .unwrap(),
        exported
    );
}

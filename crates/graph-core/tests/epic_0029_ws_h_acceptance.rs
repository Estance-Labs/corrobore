// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//! WS-H acceptance gate for the graph side of epic #196 (issue #227).
//!
//! Reuse the canonical fixtures so the workstream gate cannot silently diverge
//! from the individual feature contracts.
//!
//! Epic criteria proven here:
//!
//! - a claim audit answers what could make Corrobore change its mind
//!   (`corrective::the_claim_audit_answers_what_would_change_its_mind`);
//! - an investigation artifact regenerates from live records while preserving
//!   analyst annotations
//!   (`artifacts::regeneration_refreshes_the_binding_and_preserves_every_annotation`);
//! - a published brief has version lineage and names stable identifiers
//!   (`artifacts::an_artifact_binds_records_by_identity_and_carries_no_factual_text`,
//!   `artifacts::publication_is_independent_of_evidence_and_truth_state`);
//! - no artifact embeds copied factual text as its source of truth
//!   (`artifacts::an_artifact_that_binds_nothing_is_refused` and the serialized
//!   assertion in the binding contract);
//! - the open-source runtime runs with no control plane
//!   (`the_open_source_runtime_runs_without_a_control_plane`).
//!
//! The remaining criteria belong to the runtime and the adapters: agent write
//! policy, budget termination, the injection fixture and the attributable
//! mutation chain are gated by `shared-runtime/tests/agent_write_policy.rs`;
//! protocol isolation and adapter coverage by
//! `shared-runtime/tests/capability_catalogue.rs` and
//! `scripts/ws-h-capability-adapters.test.mjs`; memory fusion and the authority
//! cap by `corrobore-engine/tests/memory_fusion_contract.rs`; why-provenance by
//! the planner, executor and exporter contracts. Runtime-object isolation is
//! gated by `scripts/ws-h-runtime-isolation.test.mjs`.
#![allow(clippy::unwrap_used)]

#[path = "investigation_artifacts.rs"]
mod artifacts;
#[path = "corrective_routes.rs"]
mod corrective;

use graph_core::*;

// The open-source runtime owns no control plane: a full investigation runs
// end to end with no runtime object anywhere near the graph. Claims resolve,
// an artifact is built and regenerated, and the stores that would hold agent
// runtime state do not exist.
#[test]
fn the_open_source_runtime_runs_without_a_control_plane() {
    let time = TemporalTimestamp::new("2026-09-07T00:00:00Z").unwrap();
    let stamp = BitemporalStamp::new(time.clone(), time).unwrap();
    let mut graph = Graph::new();
    let claim = ClaimId::new("claim--standalone").unwrap();
    let source = SourceId::new("source--standalone").unwrap();
    let observation = ObservationId::new("observation--standalone").unwrap();
    let stores = graph.epistemic_stores_mut();
    stores
        .sources
        .register_source(SourceInput::new(
            source.clone(),
            "https://standalone.test/report",
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
            ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new("measurement", None)),
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
        stamp.clone(),
        "ws-a-minimal-v1",
    )
    .unwrap();

    graph
        .create_artifact(ArtifactInput::new(
            "artifact--standalone",
            ArtifactKind::EvidenceMap,
            "Standalone evidence map",
            ArtifactBinding {
                claims: vec![claim.clone()],
                evidence: vec![],
                narratives: vec![],
                campaigns: vec![],
            },
            stamp.clone(),
        ))
        .unwrap();
    let regenerated = graph
        .regenerate_artifact(
            "artifact--standalone",
            ArtifactBinding {
                claims: vec![claim.clone()],
                evidence: vec![],
                narratives: vec![],
                campaigns: vec![],
            },
            stamp,
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
    assert_eq!(regenerated, 2);
    let exported = graph.export_memory_json().unwrap();
    for runtime_object in ["run_ref", "tool_call", "agent_definition", "session_token"] {
        assert!(
            !exported.contains(runtime_object),
            "{runtime_object} is a control-plane object and never a graph record"
        );
    }
    assert_eq!(
        Graph::from_memory_json(&exported)
            .unwrap()
            .export_memory_json()
            .unwrap(),
        exported
    );
}

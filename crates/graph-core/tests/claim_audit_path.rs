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
//! WS-F audit reads must use retained records, with no resolver or verifier calls.
use graph_core::*;
use serde_json::json;
#[path = "support/ingestion_quality.rs"]
mod fixtures;

fn seeded() -> (Graph, ClaimId) {
    let mut graph = fixtures::seeded();
    let id = ClaimId::new("claim--audit").expect("id");
    let obs = ObservationId::new("observation--left-0").expect("observation");
    let stores = graph.epistemic_stores_mut();
    stores.claims.register_observation(obs.clone());
    stores
        .claims
        .create_asserted_claim(ClaimInput::new(
            id.clone(),
            ClaimStatement::new("The source identifies the organization").expect("statement"),
            ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new("audit", None)),
        ))
        .expect("claim");
    stores
        .claims
        .attach_link(
            ClaimLink::new(
                ClaimLinkSource::Observation(obs),
                id.clone(),
                ClaimLinkKind::Supports,
            )
            .with_independence_cluster("registry"),
        )
        .expect("link");
    let obs = ObservationId::new("observation--right-0").expect("observation");
    stores.claims.register_observation(obs.clone());
    stores
        .claims
        .attach_link(
            ClaimLink::new(
                ClaimLinkSource::Observation(obs),
                id.clone(),
                ClaimLinkKind::Refutes,
            )
            .with_independence_cluster("registry"),
        )
        .expect("refutation");
    graph
        .link_claim_audit_record(
            &id,
            ClaimAuditReference::Candidate(CandidateId::new("repair-0").expect("candidate")),
        )
        .expect("lineage");
    graph
        .link_claim_audit_record(
            &id,
            ClaimAuditReference::Reconciliation(
                ReconciliationRecordId::new("decision-0").expect("decision"),
            ),
        )
        .expect("reconciliation lineage");
    (graph, id)
}
#[test]
fn audit_joins_exact_provenance_and_preserves_unchecked_state() {
    let (graph, id) = seeded();
    let before = serde_json::to_value(graph.persistence_snapshot()).expect("snapshot");
    let audit = graph.claim_audit_path(&id).expect("audit");
    assert_eq!(
        audit["observations"]
            .as_array()
            .expect("observations")
            .len(),
        2
    );
    assert_eq!(audit["observations"][0]["payload"], "A");
    assert_eq!(audit["evidence_links"].as_array().expect("links").len(), 2);
    assert_eq!(
        audit["reconciliations"]
            .as_array()
            .expect("reconciliations")
            .len(),
        1
    );
    assert_eq!(
        audit["candidates"].as_array().expect("lineage").len(),
        2,
        "only the named repair and its predecessor, not all candidates sharing an extraction run"
    );
    assert!(audit["current_verdict"].is_null());
    assert!(
        !audit["unverified_steps"]
            .as_array()
            .expect("gaps")
            .is_empty()
    );
    assert_eq!(audit, graph.claim_audit_path(&id).expect("second read"));
    assert_eq!(
        before,
        serde_json::to_value(graph.persistence_snapshot()).expect("snapshot")
    );
    let restored = Graph::from_persistence_snapshot(graph.persistence_snapshot()).expect("restore");
    assert_eq!(
        audit,
        restored.claim_audit_path(&id).expect("restored audit")
    );
}
#[test]
fn stored_verdict_dimensions_clusters_and_history_are_returned_without_recomputation() {
    let (mut graph, id) = seeded();
    let stores = graph.epistemic_stores_mut();
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
        &id,
        BitemporalStamp::new(
            TemporalTimestamp::new("2026-09-06T12:00:00Z").expect("time"),
            TemporalTimestamp::new("2026-09-06T12:00:01Z").expect("time"),
        )
        .expect("stamp"),
        "ws-a-minimal-v1",
    )
    .expect("seed stored verdict");
    let stored = serde_json::to_value(stores.verdicts.current_verdict(&id)).expect("stored");
    let before = serde_json::to_value(graph.persistence_snapshot()).expect("snapshot");
    for _ in 0..20 {
        let audit = graph.claim_audit_path(&id).expect("audit");
        assert_eq!(audit["current_verdict"], stored);
        assert!(audit["explanation"]["dimensions"].is_object());
        assert!(
            !audit["explanation"]["clusters"]
                .as_array()
                .expect("clusters")
                .is_empty()
        );
        assert!(
            audit["link_membership"]
                .as_array()
                .expect("membership")
                .iter()
                .all(|m| !m["stored_cluster_ids"]
                    .as_array()
                    .expect("clusters")
                    .is_empty())
        );
        assert_eq!(
            audit["verdict_history"].as_array().expect("history").len(),
            1
        );
        assert!(
            !audit["state_transitions"]
                .as_array()
                .expect("transitions")
                .is_empty()
        );
    }
    assert_eq!(
        before,
        serde_json::to_value(graph.persistence_snapshot()).expect("snapshot")
    );
}
#[test]
fn audit_reports_unknown_claim_and_rejects_dangling_provenance_bindings() {
    let (mut graph, id) = seeded();
    assert!(matches!(
        graph.claim_audit_path(&ClaimId::new("unknown").expect("id")),
        Err(GraphError::ClaimNotFound(_))
    ));
    assert!(
        graph
            .link_claim_audit_record(
                &id,
                ClaimAuditReference::Candidate(CandidateId::new("missing").expect("candidate"))
            )
            .is_err()
    );
    let before = graph.claim_audit_path(&id).expect("audit");
    graph
        .link_claim_audit_record(
            &id,
            ClaimAuditReference::Candidate(CandidateId::new("repair-0").expect("candidate")),
        )
        .expect("retry");
    assert_eq!(before, graph.claim_audit_path(&id).expect("audit"));
    assert_ne!(before, json!({}));
}

#[test]
fn large_unrelated_fixture_and_cyclic_claims_preserve_stored_verification_coverage() {
    let (mut graph, id) = seeded();
    let peer = ClaimId::new("peer").expect("id");
    let stores = graph.epistemic_stores_mut();
    for n in 0..1000 {
        stores
            .claims
            .create_asserted_claim(ClaimInput::new(
                ClaimId::new(format!("unrelated-{n}")).expect("id"),
                ClaimStatement::new("Unrelated assertion").expect("statement"),
                ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new("unrelated", None)),
            ))
            .expect("claim");
    }
    stores
        .claims
        .create_asserted_claim(ClaimInput::new(
            peer.clone(),
            ClaimStatement::new("Related claim").expect("statement"),
            ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new("related", None)),
        ))
        .expect("peer");
    for (source, target) in [(peer.clone(), id.clone()), (id.clone(), peer.clone())] {
        stores
            .claims
            .attach_link(ClaimLink::new(
                ClaimLinkSource::Claim(source),
                target,
                ClaimLinkKind::DependsOn,
            ))
            .expect("cycle");
    }
    for (name, mechanical, result) in [
        ("mechanical", true, VerificationResult::Pass),
        ("semantic", false, VerificationResult::Fail),
    ] {
        stores
            .verifications
            .append(
                VerificationRecord::new(
                    VerificationRecordId::new(name).expect("id"),
                    name,
                    "v1",
                    mechanical,
                    VerificationInputs::for_claim(id.clone()),
                    result,
                    BitemporalStamp::new(
                        TemporalTimestamp::new("2026-09-06T12:00:00Z").expect("time"),
                        TemporalTimestamp::new("2026-09-06T12:00:01Z").expect("time"),
                    )
                    .expect("stamp"),
                )
                .with_rationale("Retained result")
                .with_limit("Only cited inputs"),
            )
            .expect("verification");
    }
    let before = serde_json::to_value(graph.persistence_snapshot()).expect("snapshot");
    let audit = graph.claim_audit_path(&id).expect("audit");
    assert_eq!(audit["related_claims"].as_array().expect("claims").len(), 1);
    assert_eq!(audit["verifications"].as_array().expect("records").len(), 2);
    assert!(audit["coverage"].to_string().contains("failing"));
    assert!(audit["coverage"].to_string().contains("mechanical"));
    assert!(audit["coverage"].to_string().contains("unchecked"));
    assert!(
        audit["current_verdict"].is_null(),
        "read must not resolve even with stored checks"
    );
    assert!(
        audit["unverified_steps"]
            .as_array()
            .expect("gaps")
            .iter()
            .all(|gap| gap["claim_id"] != id.as_str()
                || !matches!(
                    gap["kind"].as_str(),
                    Some("mechanical_verification" | "semantic_verification")
                ))
    );
    assert_eq!(audit, graph.claim_audit_path(&id).expect("repeat"));
    assert_eq!(
        before,
        serde_json::to_value(graph.persistence_snapshot()).expect("snapshot")
    );
}

#[test]
fn snapshot_rejects_tampered_provenance_and_cluster_members_refer_to_store_positions() {
    let (graph, id) = seeded();
    let audit = graph.claim_audit_path(&id).expect("audit");
    for member in audit["link_membership"].as_array().expect("membership") {
        let link = &graph.epistemic_stores().claims.claim_links()
            [member["store_index"].as_u64().expect("index") as usize];
        assert_eq!(member["reference"], link.reference_key());
    }
    let mut snapshot = serde_json::to_value(graph.persistence_snapshot()).expect("snapshot");
    snapshot["epistemic"]["audit_bindings"]["links"][0][1] = serde_json::to_value(
        ClaimAuditReference::Candidate(CandidateId::new("missing").expect("id")),
    )
    .expect("reference");
    assert!(
        Graph::from_persistence_snapshot(serde_json::from_value(snapshot).expect("decode"))
            .is_err()
    );
}

#[test]
fn evidence_spans_promotions_and_merge_reversals_are_included_from_stored_records() {
    let (mut graph, id) = seeded();
    let evidence = graph
        .create_evidence(
            EvidenceInput::new(
                EvidenceId::new("audit-evidence").expect("id"),
                "source://original",
                "A",
            )
            .with_offsets(0, 1)
            .with_source_id(SourceId::new("source--left-0").expect("source"))
            .with_observation_id(ObservationId::new("observation--left-0").expect("observation")),
        )
        .expect("evidence");
    graph
        .epistemic_stores_mut()
        .claims
        .register_evidence(evidence.clone());
    graph
        .epistemic_stores_mut()
        .claims
        .attach_link(ClaimLink::new(
            ClaimLinkSource::Evidence(evidence),
            id.clone(),
            ClaimLinkKind::Supports,
        ))
        .expect("evidence link");
    graph
        .promote_candidate(
            &CandidateId::new("repair-0").expect("id"),
            ActorId::new("reviewer").expect("actor"),
            "Reviewed candidate",
            CandidatePromotionInput::Node(NodeInput::new(["Entity"])),
        )
        .expect("promotion");
    let decision = ReconciliationRecordId::new("decision-0").expect("id");
    graph.apply_reconciliation_merge(&decision).expect("merge");
    graph
        .undo_reconciliation_merge(
            MergeUndo::new(
                "audit-undo",
                decision,
                ActorId::new("reviewer").expect("actor"),
                TemporalTimestamp::new("2026-09-06T13:00:00Z").expect("time"),
                "Reversed on review",
            )
            .expect("undo record"),
        )
        .expect("undo");
    let before = serde_json::to_value(graph.persistence_snapshot()).expect("snapshot");
    let audit = graph.claim_audit_path(&id).expect("audit");
    assert_eq!(audit["evidence"].as_array().expect("evidence").len(), 1);
    assert_eq!(audit["promotions"].as_array().expect("promotions").len(), 1);
    assert_eq!(audit["merge_undos"].as_array().expect("undos").len(), 1);
    assert_eq!(
        audit["source_versions"].as_array().expect("sources").len(),
        2
    );
    assert_eq!(
        audit["contradictions"]
            .as_array()
            .expect("contradictions")
            .len(),
        1
    );
    assert_eq!(
        before,
        serde_json::to_value(graph.persistence_snapshot()).expect("snapshot")
    );
}

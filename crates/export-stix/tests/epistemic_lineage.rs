// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//! Epic 0029 WS-A item 7 (issue #153): STIX exports carry epistemic lineage
//! additively, and stay byte-identical for graphs with no governed record.
use export_stix::export_stix_subset_bundle;
use graph_core::{
    BitemporalStamp, ClaimId, ClaimInput, ClaimStatement, ClaimTarget, Confidence, EvidenceId,
    EvidenceInput, EvidenceSourceType, ExportMetadata, ExportMode, ExportProfile, Graph, NodeInput,
    ObservationId, ObservationInput, ObservationModality, PropertyValue, RecordStatus, SourceId,
    SourceInput, TemporalTimestamp, TransactionId, VerificationInputs, VerificationRecord,
    VerificationRecordId, VerificationResult, build_deterministic_export_plan,
};
use serde_json::{Value, json};

fn metadata() -> ExportMetadata {
    ExportMetadata::new(
        "snapshot--epistemic-lineage",
        TransactionId::new("transaction--epistemic-lineage").expect("transaction id"),
        "stix-mvp-v2",
        ExportProfile::StixMvp,
        ExportMode::Strict,
        None,
    )
    .expect("metadata")
}

fn graph_with_object(evidence: EvidenceInput) -> Graph {
    let mut graph = Graph::new();
    let evidence_id = EvidenceId::new("evidence--lineage").expect("id");
    graph.create_evidence(evidence).expect("evidence");
    graph
        .create_node(
            NodeInput::new([
                "OpenCtiObject",
                "OpenCtiStixDomainObject",
                "OpenCtiType__Imported",
            ])
            .with_property(
                "opencti.family",
                PropertyValue::String("stix_domain_object".to_owned()),
            )
            .with_property(
                "opencti.raw",
                PropertyValue::Json(json!({
                    "type": "indicator",
                    "id": "indicator--4f5c2a1e-9a3b-4c6d-8e7f-0a1b2c3d4e5f",
                    "spec_version": "2.1",
                    "created": "2026-08-01T00:00:00.000Z",
                    "modified": "2026-08-01T00:00:00.000Z",
                    "name": "lineage indicator",
                    "pattern": "[domain-name:value = 'lineage.example']",
                    "pattern_type": "stix",
                    "valid_from": "2026-08-01T00:00:00.000Z"
                })),
            )
            .with_status(RecordStatus::Exportable)
            .with_confidence(Confidence::new(0.9).expect("confidence"))
            .with_evidence_ref(evidence_id),
        )
        .expect("node");
    graph
}

fn bundle_bytes(graph: &Graph) -> Vec<u8> {
    let plan = build_deterministic_export_plan(graph, metadata(), &[]).expect("plan");
    serde_json::to_vec(&export_stix_subset_bundle(graph, &plan)).expect("bundle")
}

#[test]
fn exports_without_governed_records_stay_byte_identical() {
    let graph = graph_with_object(EvidenceInput::new(
        EvidenceId::new("evidence--lineage").expect("id"),
        "synthetic-report",
        "payload",
    ));
    let before = bundle_bytes(&graph);
    let restored = Graph::from_persistence_snapshot(graph.persistence_snapshot()).expect("restore");
    assert_eq!(bundle_bytes(&restored), before);

    let value: Value = serde_json::from_slice(&before).expect("json");
    assert!(value.get("x_corrobore_audit_archive").is_none());
    for object in value["objects"].as_array().expect("objects") {
        assert!(
            object.get("x_corrobore_lineage").is_none(),
            "no lineage key without governed records"
        );
    }
}

#[test]
fn exports_with_governed_records_carry_additive_lineage() {
    let mut graph = graph_with_object(
        EvidenceInput::new(
            EvidenceId::new("evidence--lineage").expect("id"),
            "synthetic-report",
            "payload",
        )
        .with_source_id(SourceId::new("source--report").expect("id"))
        .with_observation_id(ObservationId::new("observation--span").expect("id")),
    );
    let stores = graph.epistemic_stores_mut();
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
                "lineage.example was observed",
                ObservationModality::Text,
            ),
            &stores.sources,
        )
        .expect("observation");

    let value: Value = serde_json::from_slice(&bundle_bytes(&graph)).expect("json");
    let object = &value["objects"].as_array().expect("objects")[0];
    let lineage = object["x_corrobore_lineage"]
        .as_array()
        .expect("lineage array");
    assert_eq!(lineage.len(), 1);
    assert_eq!(lineage[0]["evidence_id"], "evidence--lineage");
    assert_eq!(lineage[0]["source_id"], "source--report");
    assert_eq!(lineage[0]["observation_id"], "observation--span");
    assert_eq!(
        lineage[0]["source_uri"],
        "https://vendor.example/report.pdf"
    );
    assert!(
        lineage[0].get("verdicts").is_none(),
        "no verdict lineage without claims on this node"
    );
}

#[test]
fn claim_lineage_exports_current_verification_coverage() {
    let mut graph = graph_with_object(EvidenceInput::new(
        EvidenceId::new("evidence--lineage").expect("id"),
        "synthetic-report",
        "payload",
    ));
    let node_id = graph
        .list_nodes()
        .expect("nodes")
        .into_iter()
        .next()
        .expect("exported node")
        .id()
        .clone();
    let claim_id = ClaimId::new("claim--lineage").expect("claim id");
    let stores = graph.epistemic_stores_mut();
    stores
        .claims
        .create_asserted_claim(ClaimInput::new(
            claim_id.clone(),
            ClaimStatement::new("The indicator is valid").expect("statement"),
            ClaimTarget::Node(node_id),
        ))
        .expect("claim");
    stores
        .verifications
        .append(VerificationRecord::new(
            VerificationRecordId::new("verification--lineage").expect("verification id"),
            "verifier.identifier-syntax",
            "1.0.0",
            true,
            VerificationInputs::for_claim(claim_id.clone()),
            VerificationResult::Pass,
            BitemporalStamp::new(
                TemporalTimestamp::new("2026-09-06T00:00:00Z").expect("valid time"),
                TemporalTimestamp::new("2026-09-06T00:01:00Z").expect("system time"),
            )
            .expect("stamp"),
        ))
        .expect("verification");

    make_claim_actionable(&mut graph, &claim_id);
    let value: Value = serde_json::from_slice(&bundle_bytes(&graph)).expect("json");
    let lineage = value["objects"][0]["x_corrobore_lineage"]
        .as_array()
        .expect("lineage array");
    let claim = lineage
        .iter()
        .find(|entry| entry["claim_id"] == "claim--lineage")
        .expect("claim lineage");
    assert_eq!(claim["confidence_band"], "Exportable");
    assert_eq!(
        claim["verdict_explanation"]["dimensions"]["actionability"],
        1.0
    );
    assert_eq!(
        claim["verdict_explanation"]["clusters"]
            .as_array()
            .expect("clusters")
            .len(),
        2
    );
    assert!(claim["verdict_explanation"]["uncertainty_kind"].is_null());
    assert_eq!(
        claim["verification_coverage"]["entries"][0]["class"],
        "mechanically_checked"
    );
    assert_eq!(
        claim["verification_coverage"]["entries"][0]["verifier_id"],
        "verifier.identifier-syntax"
    );
    assert_eq!(
        claim["verification_coverage"]["entries"][0]["verifier_version"],
        "1.0.0"
    );
}

fn make_claim_actionable(graph: &mut graph_core::Graph, claim: &graph_core::ClaimId) {
    use graph_core::*;
    let t = TemporalTimestamp::new("2026-09-06T00:01:00Z").expect("time");
    let stamp = BitemporalStamp::new(t.clone(), t).expect("stamp");
    let stores = graph.epistemic_stores_mut();
    let mut bindings = Vec::new();
    for name in ["first", "second"] {
        let source = SourceId::new(format!("source--gate-{name}")).expect("id");
        stores
            .sources
            .register_source(SourceInput::new(
                source.clone(),
                format!("https://{name}.test"),
                EvidenceSourceType::Document,
            ))
            .expect("source");
        let obs = ObservationId::new(format!("observation--gate-{name}")).expect("id");
        stores
            .observations
            .create_observation(
                ObservationInput::new(
                    obs.clone(),
                    source.clone(),
                    "grounded support",
                    ObservationModality::Text,
                ),
                &stores.sources,
            )
            .expect("observation");
        stores.claims.register_observation(obs.clone());
        stores
            .claims
            .attach_link(
                ClaimLink::new(
                    ClaimLinkSource::Observation(obs),
                    claim.clone(),
                    ClaimLinkKind::Supports,
                )
                .with_strength(Confidence::new(1.0).expect("score"))
                .with_bitemporal(stamp.clone()),
            )
            .expect("link");
        bindings.push(
            SourceAuthority::new(
                source,
                "test",
                "fact",
                Confidence::new(1.0).expect("score"),
                "lineage-authority-v1",
            )
            .expect("authority"),
        );
    }
    stores
        .verifications
        .append(VerificationRecord::new(
            VerificationRecordId::new("verification--grounded").expect("id"),
            "zz.grounded",
            "1",
            true,
            VerificationInputs::for_claim(claim.clone())
                .with_observation(ObservationId::new("observation--gate-first").expect("id")),
            VerificationResult::Pass,
            stamp.clone(),
        ))
        .expect("verification");
    stores
        .verdicts
        .register_source_authority_policy(
            SourceAuthorityPolicy::new("lineage-authority-v1", bindings).expect("policy"),
        )
        .expect("register");
    let evidence = EvidenceRecordStore::new();
    let inputs = ResolutionInputs::new(
        &stores.verifications,
        &evidence,
        &stores.observations,
        &stores.sources,
    )
    .with_source_authority("lineage-authority-v1", "test", "fact");
    resolve_current_claim_verdict(
        &mut stores.claims,
        &mut stores.verdicts,
        &inputs,
        claim,
        stamp,
    )
    .expect("resolve");
}

#[test]
fn relationship_claims_export_the_same_explanation_payload() {
    use graph_core::*;
    let mut graph = graph_with_object(EvidenceInput::new(
        EvidenceId::new("evidence--lineage").expect("id"),
        "synthetic-report",
        "payload",
    ));
    let source = graph.list_nodes().expect("nodes")[0].id().clone();
    let target = graph
        .create_node(
            NodeInput::new(["Malware"])
                .with_status(RecordStatus::Exportable)
                .with_evidence_ref(EvidenceId::new("evidence--lineage").expect("id")),
        )
        .expect("node");
    let relationship = graph
        .create_relationship(
            RelationshipInput::new(source, "USES", target)
                .expect("input")
                .with_status(RecordStatus::Exportable)
                .with_evidence_ref(EvidenceId::new("evidence--lineage").expect("id")),
        )
        .expect("relationship");
    let claim = ClaimId::new("claim--relationship").expect("id");
    graph
        .epistemic_stores_mut()
        .claims
        .create_asserted_claim(ClaimInput::new(
            claim.clone(),
            ClaimStatement::new("relationship is supported").expect("statement"),
            ClaimTarget::Relationship(relationship),
        ))
        .expect("claim");
    make_claim_actionable(&mut graph, &claim);
    let value: Value = serde_json::from_slice(&bundle_bytes(&graph)).expect("json");
    let object = value["objects"]
        .as_array()
        .expect("objects")
        .iter()
        .find(|o| o["type"] == "relationship")
        .expect("relationship");
    assert_eq!(
        object["x_corrobore_lineage"][0]["verdict_explanation"]["clusters"]
            .as_array()
            .expect("clusters")
            .len(),
        2
    );
    let restored = Graph::from_exported_audit_bundle(&value).expect("restore relationship audit");
    assert_eq!(
        restored.claim_audit_path(&claim).expect("restored audit"),
        graph.claim_audit_path(&claim).expect("original audit")
    );
}

#[test]
fn scoped_stix_archive_round_trips_the_complete_audit_and_human_judgment()
-> Result<(), Box<dyn std::error::Error>> {
    use graph_core::*;
    let mut graph = graph_with_object(EvidenceInput::new(
        EvidenceId::new("evidence--lineage")?,
        "source",
        "original",
    ));
    let node = graph.list_nodes()?[0].id().clone();
    let claim = ClaimId::new("exported-claim")?;
    graph
        .epistemic_stores_mut()
        .claims
        .create_asserted_claim(ClaimInput::new(
            claim.clone(),
            ClaimStatement::new("Audited claim")?,
            ClaimTarget::Node(node),
        ))?;
    make_claim_actionable(&mut graph, &claim);
    graph.record_analyst_decision(AnalystDecision::new(
        "human",
        claim.clone(),
        ActorId::new("reviewer")?,
        TemporalTimestamp::new("2026-09-06T12:00:00Z")?,
        AnalystDecisionAction::Override {
            judgment: "Human conclusion".into(),
            rationale: "Reviewed".into(),
        },
    )?)?;
    let excluded = graph.create_node(
        NodeInput::new(["Malware"])
            .with_status(RecordStatus::Exportable)
            .with_evidence_ref(EvidenceId::new("evidence--lineage")?),
    )?;
    graph
        .epistemic_stores_mut()
        .claims
        .create_asserted_claim(ClaimInput::new(
            ClaimId::new("excluded-claim")?,
            ClaimStatement::new("secret unrelated assertion")?,
            ClaimTarget::Node(excluded),
        ))?;
    let metadata = ExportMetadata::new(
        "archive",
        TransactionId::new("archive")?,
        "stix-mvp-v2",
        ExportProfile::StixMvp,
        ExportMode::Permissive,
        None,
    )?;
    let plan = build_deterministic_export_plan(&graph, metadata, &[])?;
    let exported = serde_json::to_value(export_stix_subset_bundle(&graph, &plan))?;
    let archive = &exported["x_corrobore_audit_archive"];
    assert!(archive.is_object());
    assert!(!archive.to_string().contains("secret unrelated assertion"));
    let restored = Graph::from_claim_audit_archive(archive)?;
    assert_eq!(
        restored.claim_audit_path(&claim)?,
        graph.claim_audit_path(&claim)?
    );
    assert_eq!(restored.list_nodes()?.len(), 1);
    Ok(())
}

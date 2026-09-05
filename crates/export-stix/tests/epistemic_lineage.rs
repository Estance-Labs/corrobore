// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//! Epic 0029 WS-A item 7 (issue #153): STIX exports carry epistemic lineage
//! additively, and stay byte-identical for graphs with no governed record.
use export_stix::export_stix_subset_bundle;
use graph_core::{
    Confidence, EvidenceId, EvidenceInput, EvidenceSourceType, ExportMetadata, ExportMode,
    ExportProfile, Graph, NodeInput, ObservationId, ObservationInput, ObservationModality,
    PropertyValue, RecordStatus, SourceId, SourceInput, TransactionId,
    build_deterministic_export_plan,
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

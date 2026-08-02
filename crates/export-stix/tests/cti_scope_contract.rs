// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

use export_stix::export_stix_subset_bundle;
use graph_core::{
    Confidence, EvidenceId, EvidenceInput, ExportMetadata, ExportMode, ExportProfile, Graph,
    NodeId, NodeInput, PropertyValue, RecordStatus, RelationshipInput, TransactionId,
    build_deterministic_export_plan,
};
use serde_json::{Value, json};

fn metadata(mode: ExportMode) -> ExportMetadata {
    ExportMetadata::new(
        "snapshot--cti-scope-contract",
        TransactionId::new("transaction--cti-scope-contract")
            .expect("transaction id should be valid"),
        "stix-mvp-v2",
        ExportProfile::StixMvp,
        mode,
        None,
    )
    .expect("metadata should be valid")
}

fn evidence_id(value: &str) -> EvidenceId {
    EvidenceId::new(value).expect("evidence id should be valid")
}

fn confidence(value: f64) -> Confidence {
    Confidence::new(value).expect("confidence should be valid")
}

fn imported_object(graph: &mut Graph, raw: Value, evidence: &str) -> NodeId {
    let evidence = evidence_id(evidence);
    graph
        .create_evidence(EvidenceInput::new(
            evidence.clone(),
            "synthetic-report",
            "synthetic evidence payload",
        ))
        .expect("evidence should be retained");
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
            .with_property("opencti.raw", PropertyValue::Json(raw))
            .with_status(RecordStatus::Exportable)
            .with_confidence(confidence(0.91))
            .with_evidence_ref(evidence),
        )
        .expect("imported object should be created")
}

fn exported_bundle(graph: &Graph, mode: ExportMode) -> Value {
    let plan = build_deterministic_export_plan(graph, metadata(mode), &[])
        .expect("CTI-scoped plan should succeed");
    serde_json::to_value(export_stix_subset_bundle(graph, &plan)).expect("bundle should serialize")
}

fn exported_objects(graph: &Graph, mode: ExportMode) -> Vec<Value> {
    exported_bundle(graph, mode)["objects"]
        .as_array()
        .expect("objects should be an array")
        .clone()
}

#[test]
fn strict_export_excludes_domain_neutral_memory_and_preserves_original_object() {
    let mut graph = Graph::new();
    imported_object(
        &mut graph,
        json!({
            "type": "malware",
            "spec_version": "2.1",
            "id": "malware--11111111-1111-4111-8111-111111111111",
            "created": "2026-08-01T10:00:00.000Z",
            "modified": "2026-08-01T10:00:00.000Z",
            "name": "Synthetic Loader",
            "description": "Preserve this description exactly",
            "is_family": true,
            "aliases": ["Loader-A", "Loader-B"],
            "pattern": "[file:hashes.'SHA-256' = 'abc123']",
            "hashes": {"SHA-256": "abc123"},
            "object_refs": ["indicator--aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"],
            "object_marking_refs": ["marking-definition--bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"],
            "external_references": [{
                "source_name": "synthetic-catalog",
                "external_id": "SYN-001"
            }],
            "x_corrobore_supported_extension": {"rank": 3}
        }),
        "evidence--malware",
    );
    graph
        .create_node(
            NodeInput::new(["CorroboreMemory"])
                .with_property("kind", PropertyValue::String("working_state".to_owned())),
        )
        .expect("generic memory should be created");
    graph
        .create_node(NodeInput::new(["CorroboreMemoryReceipt"]))
        .expect("generic receipt should be created");

    let bundle = exported_bundle(&graph, ExportMode::Strict);
    let objects = bundle["objects"]
        .as_array()
        .expect("objects should be an array");

    assert_eq!(objects.len(), 1, "generic records must not leak into STIX");
    assert_eq!(
        objects[0]["id"],
        "malware--11111111-1111-4111-8111-111111111111"
    );
    assert_eq!(objects[0]["type"], "malware");
    assert_eq!(
        objects[0]["description"],
        "Preserve this description exactly"
    );
    assert_eq!(objects[0]["aliases"], json!(["Loader-A", "Loader-B"]));
    assert_eq!(objects[0]["pattern"], "[file:hashes.'SHA-256' = 'abc123']");
    assert_eq!(objects[0]["hashes"]["SHA-256"], "abc123");
    assert_eq!(
        objects[0]["object_refs"],
        json!(["indicator--aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"])
    );
    assert_eq!(
        objects[0]["object_marking_refs"],
        json!(["marking-definition--bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"])
    );
    assert_eq!(
        objects[0]["external_references"][0]["external_id"],
        "SYN-001"
    );
    assert_eq!(objects[0]["x_corrobore_supported_extension"]["rank"], 3);
    assert_eq!(objects[0]["confidence"], 91);
    assert_eq!(
        objects[0]["x_corrobore_evidence_refs"],
        json!(["evidence--malware"])
    );
    assert_eq!(bundle["x_corrobore_evidence"][0]["id"], "evidence--malware");
}

#[test]
fn relationship_uses_original_identity_and_actual_exported_endpoint_ids() {
    let mut graph = Graph::new();
    let source = imported_object(
        &mut graph,
        json!({
            "type": "intrusion-set",
            "spec_version": "2.1",
            "id": "intrusion-set--22222222-2222-4222-8222-222222222222",
            "created": "2026-08-01T10:00:00.000Z",
            "modified": "2026-08-01T10:00:00.000Z",
            "name": "Synthetic Group"
        }),
        "evidence--source",
    );
    let target = imported_object(
        &mut graph,
        json!({
            "type": "malware",
            "spec_version": "2.1",
            "id": "malware--33333333-3333-4333-8333-333333333333",
            "created": "2026-08-01T10:00:00.000Z",
            "modified": "2026-08-01T10:00:00.000Z",
            "name": "Synthetic Implant",
            "is_family": false
        }),
        "evidence--target",
    );
    let relationship_evidence = evidence_id("evidence--relationship");
    graph
        .create_evidence(EvidenceInput::new(
            relationship_evidence.clone(),
            "synthetic-report",
            "synthetic relationship evidence",
        ))
        .expect("relationship evidence should be retained");
    graph
        .create_relationship(
            RelationshipInput::new(source, "uses", target)
                .expect("relationship input should be valid")
                .with_property(
                    "opencti.family",
                    PropertyValue::String("stix_core_relationship".to_owned()),
                )
                .with_property(
                    "opencti.raw",
                    PropertyValue::Json(json!({
                        "type": "relationship",
                        "spec_version": "2.1",
                        "id": "relationship--44444444-4444-4444-8444-444444444444",
                        "created": "2026-08-01T10:00:00.000Z",
                        "modified": "2026-08-01T10:00:00.000Z",
                        "relationship_type": "uses",
                        "source_ref": "intrusion-set--22222222-2222-4222-8222-222222222222",
                        "target_ref": "malware--33333333-3333-4333-8333-333333333333",
                        "description": "Relationship semantics survive"
                    })),
                )
                .with_status(RecordStatus::Exportable)
                .with_confidence(confidence(0.88))
                .with_evidence_ref(relationship_evidence),
        )
        .expect("relationship should be created");

    let objects = exported_objects(&graph, ExportMode::Strict);
    let relationship = objects
        .iter()
        .find(|object| object["type"] == "relationship")
        .expect("relationship should be exported");

    assert_eq!(
        relationship["id"],
        "relationship--44444444-4444-4444-8444-444444444444"
    );
    assert_eq!(
        relationship["source_ref"],
        "intrusion-set--22222222-2222-4222-8222-222222222222"
    );
    assert_eq!(
        relationship["target_ref"],
        "malware--33333333-3333-4333-8333-333333333333"
    );
    assert_eq!(
        relationship["description"],
        "Relationship semantics survive"
    );
}

#[test]
fn strict_export_rejects_only_eligible_cti_candidates_with_named_readiness_issues() {
    let mut graph = Graph::new();
    graph
        .create_node(
            NodeInput::new(["OpenCtiObject", "OpenCtiStixDomainObject"])
                .with_property(
                    "opencti.family",
                    PropertyValue::String("stix_domain_object".to_owned()),
                )
                .with_property(
                    "opencti.raw",
                    PropertyValue::Json(json!({
                        "type": "indicator",
                        "id": "indicator--55555555-5555-4555-8555-555555555555"
                    })),
                )
                .with_status(RecordStatus::Exportable),
        )
        .expect("incomplete CTI candidate should be created");
    let low_confidence_evidence = evidence_id("evidence--low-confidence");
    graph
        .create_evidence(EvidenceInput::new(
            low_confidence_evidence.clone(),
            "synthetic-report",
            "low-confidence evidence",
        ))
        .expect("low-confidence evidence should be retained");
    graph
        .create_node(
            NodeInput::new(["OpenCtiObject", "OpenCtiStixDomainObject"])
                .with_property(
                    "opencti.family",
                    PropertyValue::String("stix_domain_object".to_owned()),
                )
                .with_property(
                    "opencti.raw",
                    PropertyValue::Json(json!({
                        "type": "indicator",
                        "id": "indicator--56565656-5656-4565-8565-565656565656"
                    })),
                )
                .with_confidence(confidence(0.79))
                .with_evidence_ref(low_confidence_evidence),
        )
        .expect("low-confidence CTI candidate should be created");
    graph
        .create_node(NodeInput::new(["CorroboreMemory"]))
        .expect("generic memory should be created");

    let error = build_deterministic_export_plan(&graph, metadata(ExportMode::Strict), &[])
        .expect_err("strict CTI export should reject missing native readiness metadata");
    let message = error.to_string();

    assert!(message.contains("CTI_CONFIDENCE_REQUIRED"), "{message}");
    assert!(message.contains("CTI_CONFIDENCE_TOO_LOW"), "{message}");
    assert!(message.contains("CTI_EVIDENCE_REQUIRED"), "{message}");
    assert!(message.contains("EXPORT_STATUS_NOT_READY"), "{message}");
    assert!(
        !message.contains("CorroboreMemory"),
        "generic memory must not become a CTI readiness failure: {message}"
    );
}

#[test]
fn permissive_export_reports_bounded_exclusions_without_fabricating_objects() {
    let mut graph = Graph::new();
    graph
        .create_node(
            NodeInput::new(["OpenCtiObject", "OpenCtiStixDomainObject"])
                .with_property(
                    "opencti.family",
                    PropertyValue::String("stix_domain_object".to_owned()),
                )
                .with_property(
                    "opencti.raw",
                    PropertyValue::Json(json!({
                        "type": "report",
                        "id": "report--66666666-6666-4666-8666-666666666666",
                        "name": "Incomplete synthetic report"
                    })),
                ),
        )
        .expect("incomplete report should be created");

    let plan = build_deterministic_export_plan(&graph, metadata(ExportMode::Permissive), &[])
        .expect("permissive plan should succeed");
    let bundle = serde_json::to_value(export_stix_subset_bundle(&graph, &plan))
        .expect("bundle should serialize");

    assert_eq!(bundle["objects"], json!([]));
    let diagnostics = bundle["export_diagnostics"]["exclusions"]
        .as_array()
        .expect("permissive exclusions should be machine readable");
    assert!(
        diagnostics
            .iter()
            .any(|gap| gap["code"] == "EXPORT_STATUS_NOT_READY")
    );
}

#[test]
fn strict_export_rejects_malformed_imported_identity_with_named_issue() {
    let mut graph = Graph::new();
    imported_object(
        &mut graph,
        json!({
            "type": "malware",
            "id": "identity--wrong-type-prefix",
            "name": "Malformed imported identity"
        }),
        "evidence--malformed-identity",
    );

    let error = build_deterministic_export_plan(&graph, metadata(ExportMode::Strict), &[])
        .expect_err("strict export should reject a malformed imported STIX identity");

    assert!(
        error.to_string().contains("STIX_IDENTITY_INVALID"),
        "{error}"
    );
}

#[test]
fn relationship_is_excluded_when_an_endpoint_fails_readiness() {
    let mut graph = Graph::new();
    let source = imported_object(
        &mut graph,
        json!({
            "type": "intrusion-set",
            "id": "intrusion-set--77777777-7777-4777-8777-777777777777",
            "name": "Ready source"
        }),
        "evidence--ready-source",
    );
    let target = graph
        .create_node(
            NodeInput::new(["OpenCtiObject", "OpenCtiStixDomainObject"])
                .with_property(
                    "opencti.family",
                    PropertyValue::String("stix_domain_object".to_owned()),
                )
                .with_property(
                    "opencti.raw",
                    PropertyValue::Json(json!({
                        "type": "malware",
                        "id": "malware--88888888-8888-4888-8888-888888888888",
                        "name": "Unready target"
                    })),
                )
                .with_status(RecordStatus::Exportable),
        )
        .expect("unready target should be created");
    let relationship_evidence = evidence_id("evidence--endpoint-relationship");
    graph
        .create_evidence(EvidenceInput::new(
            relationship_evidence.clone(),
            "synthetic-report",
            "endpoint relationship evidence",
        ))
        .expect("relationship evidence should be retained");
    graph
        .create_relationship(
            RelationshipInput::new(source, "uses", target)
                .expect("relationship input should be valid")
                .with_status(RecordStatus::Exportable)
                .with_confidence(confidence(0.9))
                .with_evidence_ref(relationship_evidence),
        )
        .expect("relationship should be created");

    let plan = build_deterministic_export_plan(&graph, metadata(ExportMode::Permissive), &[])
        .expect("permissive export should report endpoint exclusion");
    let bundle = serde_json::to_value(export_stix_subset_bundle(&graph, &plan))
        .expect("bundle should serialize");

    assert_eq!(bundle["objects"].as_array().map(Vec::len), Some(1));
    assert!(
        bundle["export_diagnostics"]["exclusions"]
            .as_array()
            .expect("diagnostics should be present")
            .iter()
            .any(|gap| gap["code"] == "CTI_ENDPOINT_EXCLUDED")
    );
}

#[test]
fn permissive_export_reports_unknown_opencti_family_without_fabricating_identity() {
    let mut graph = Graph::new();
    graph
        .create_node(
            NodeInput::new(["OpenCtiObject", "OpenCtiUnknownObject"])
                .with_property(
                    "opencti.family",
                    PropertyValue::String("unknown_object".to_owned()),
                )
                .with_property(
                    "opencti.raw",
                    PropertyValue::Json(json!({
                        "type": "x-opencti-future-object",
                        "id": "x-opencti-future-object--99999999-9999-4999-8999-999999999999"
                    })),
                ),
        )
        .expect("unknown OpenCTI object should be retained");

    let plan = build_deterministic_export_plan(&graph, metadata(ExportMode::Permissive), &[])
        .expect("permissive export should report unsupported CTI records");
    let bundle = serde_json::to_value(export_stix_subset_bundle(&graph, &plan))
        .expect("bundle should serialize");

    assert_eq!(bundle["objects"], json!([]));
    assert!(
        bundle["export_diagnostics"]["exclusions"]
            .as_array()
            .expect("diagnostics should be present")
            .iter()
            .any(|gap| gap["code"] == "CTI_PROFILE_UNSUPPORTED_RECORD")
    );
}

#[test]
fn export_is_byte_identical_after_persistence_snapshot_restore() {
    let mut graph = Graph::new();
    imported_object(
        &mut graph,
        json!({
            "type": "report",
            "id": "report--aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
            "name": "Deterministic report",
            "object_refs": ["malware--bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"]
        }),
        "evidence--deterministic-report",
    );

    let before_plan = build_deterministic_export_plan(&graph, metadata(ExportMode::Strict), &[])
        .expect("initial export plan should succeed");
    let before = serde_json::to_vec(&export_stix_subset_bundle(&graph, &before_plan))
        .expect("initial bundle should serialize");

    let restored = Graph::from_persistence_snapshot(graph.persistence_snapshot())
        .expect("persisted graph should restore");
    let after_plan = build_deterministic_export_plan(&restored, metadata(ExportMode::Strict), &[])
        .expect("restored export plan should succeed");
    let after = serde_json::to_vec(&export_stix_subset_bundle(&restored, &after_plan))
        .expect("restored bundle should serialize");

    assert_eq!(before, after);
}

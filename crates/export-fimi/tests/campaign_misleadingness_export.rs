// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//! Epic 0029 WS-G item 4 (issue #220): FIMI exports carry campaign lineage and
//! misleadingness additively, as fields distinct from every factual verdict,
//! and stay byte-identical for graphs with no narrative or campaign record.
use export_fimi::{export_fimi_json, export_fimi_json_document};
use graph_core::{
    BitemporalStamp, CampaignId, CampaignInput, CampaignSignal, CampaignSignalFeatures, ClaimId,
    ClaimInput, ClaimLink, ClaimLinkKind, ClaimLinkSource, ClaimStatement, ClaimTarget,
    ContextMembership, EvidenceId, EvidenceInput, EvidenceSourceType, ExportMetadata, ExportMode,
    ExportProfile, Graph, NarrativeId, NarrativeInput, NodeId, NodeInput, ObservationId,
    ObservationInput, ObservationModality, RecordStatus, ResolutionInputs, SignalScope, SourceId,
    SourceInput, TemporalTimestamp, TransactionId, build_deterministic_export_plan,
    resolve_claim_verdict,
};
use serde_json::{Value, json};

const NARRATIVE: &str = "narrative--relief-convoy";
const CAMPAIGN: &str = "campaign--relief-convoy";
const CLAIM: &str = "claim--convoy-delay";
const ANNOTATION: &str = "evidence--assessment";
const SECOND: &str = "evidence--second-outlet";
const OUTLET: &str = "source--outlet-a";
const OTHER_OUTLET: &str = "source--outlet-b";
const OBSERVATION: &str = "observation--convoy-delay";
const FINGERPRINT: &str = "style:relief-cadence-v1";

fn metadata() -> ExportMetadata {
    ExportMetadata::new(
        "snapshot--fimi-ws-g",
        TransactionId::new("transaction--fimi-ws-g").expect("transaction id"),
        "fimi-json-v1",
        ExportProfile::FimiJsonMvp,
        ExportMode::Permissive,
        None,
    )
    .expect("metadata")
}

fn stamp() -> BitemporalStamp {
    let time = TemporalTimestamp::new("2026-09-07T00:00:00Z").expect("time");
    BitemporalStamp::new(time.clone(), time).expect("stamp")
}

// The pack records its assessment as evidence: the annotation the exporter
// carries is exactly what `corrobore-domain-fimi` wrote, including the band and
// the explanation its report derived. The exporter never recomputes them.
fn recorded_assessment() -> Value {
    json!({
        "fimi_misleadingness": {
            "subject": {"kind": "narrative", "id": NARRATIVE},
            "gap": {
                "reader_interpretation": "the aid convoy was blocked to starve the region",
                "evidence_warranted_interpretation":
                    "one convoy of twelve was delayed at a checkpoint for four hours",
                "kind": "overreach"
            },
            "findings": [
                {
                    "mechanism": "omission",
                    "fragment": "the aid convoy was stopped at the checkpoint",
                    "omitted_context": "the convoy was released after four hours",
                    "anchors": [{"kind": "omission_pattern", "id": "omission-pattern--release"}]
                },
                {
                    "mechanism": "unsupported_inference",
                    "fragment": "the four hour delay proves a deliberate starvation policy",
                    "inference_marker": "proves",
                    "anchors": [{"kind": "observation", "id": OBSERVATION}]
                }
            ],
            "band": "high",
            "mechanisms": ["unsupported_inference", "omission"],
            "explanation":
                "reader interpretation overreaches the evidence-warranted interpretation; \
                 misleadingness band high; this assessment is not a factual determination"
        }
    })
}

fn assessment_without_a_recorded_report() -> Value {
    let mut envelope = recorded_assessment();
    let assessment = envelope["fimi_misleadingness"]
        .as_object_mut()
        .expect("assessment object");
    assessment.remove("band");
    assessment.remove("mechanisms");
    assessment.remove("explanation");
    envelope
}

// One exported content node: a supported claim about it, an annotation carried
// as evidence, and a second outlet so a coordination signal has two sources.
fn fixture(annotation: &Value) -> (Graph, NodeId) {
    let mut graph = Graph::new();
    for (source, uri) in [
        (OUTLET, "https://outlet-a.example/post"),
        (OTHER_OUTLET, "https://outlet-b.example/post"),
    ] {
        graph
            .epistemic_stores_mut()
            .sources
            .register_source(SourceInput::new(
                SourceId::new(source).expect("id"),
                uri,
                EvidenceSourceType::Url,
            ))
            .expect("source");
    }
    graph
        .create_evidence(
            EvidenceInput::new(
                EvidenceId::new(ANNOTATION).expect("id"),
                "https://outlet-a.example/post",
                annotation.to_string(),
            )
            .with_source_id(SourceId::new(OUTLET).expect("id")),
        )
        .expect("annotation evidence");
    graph
        .create_evidence(
            EvidenceInput::new(
                EvidenceId::new(SECOND).expect("id"),
                "https://outlet-b.example/post",
                "the aid convoy was stopped at the checkpoint",
            )
            .with_source_id(SourceId::new(OTHER_OUTLET).expect("id")),
        )
        .expect("second evidence");
    let node = graph
        .create_node(
            NodeInput::new(["Outlet"])
                .with_status(RecordStatus::Exportable)
                .with_evidence_ref(EvidenceId::new(ANNOTATION).expect("id")),
        )
        .expect("node");

    let claim = ClaimId::new(CLAIM).expect("claim id");
    let stores = graph.epistemic_stores_mut();
    stores
        .claims
        .create_asserted_claim(ClaimInput::new(
            claim.clone(),
            ClaimStatement::new("one aid convoy was delayed at a checkpoint for four hours")
                .expect("statement"),
            ClaimTarget::Node(node.clone()),
        ))
        .expect("claim");
    stores
        .observations
        .create_observation(
            ObservationInput::new(
                ObservationId::new(OBSERVATION).expect("id"),
                SourceId::new(OUTLET).expect("id"),
                "convoy 7 of 12 held four hours at checkpoint, released same day",
                ObservationModality::Text,
            ),
            &stores.sources,
        )
        .expect("observation");
    stores
        .claims
        .register_observation(ObservationId::new(OBSERVATION).expect("id"));
    stores
        .claims
        .attach_link(
            ClaimLink::new(
                ClaimLinkSource::Observation(ObservationId::new(OBSERVATION).expect("id")),
                claim.clone(),
                ClaimLinkKind::Supports,
            )
            .with_bitemporal(stamp()),
        )
        .expect("observation link");
    for evidence in [ANNOTATION, SECOND] {
        let evidence = EvidenceId::new(evidence).expect("id");
        stores.claims.register_evidence(evidence.clone());
        stores
            .claims
            .attach_link(
                ClaimLink::new(
                    ClaimLinkSource::Evidence(evidence),
                    claim.clone(),
                    ClaimLinkKind::Supports,
                )
                .with_bitemporal(stamp()),
            )
            .expect("evidence link");
    }
    (graph, node)
}

fn support_claim(graph: &mut Graph) {
    let claim = ClaimId::new(CLAIM).expect("claim id");
    let evidence = graph.evidence_store().clone();
    let stores = graph.epistemic_stores_mut();
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
        stamp(),
        "ws-a-minimal-v1",
    )
    .expect("resolve");
}

fn collect(graph: &mut Graph, node: &NodeId) {
    graph
        .create_narrative(NarrativeInput::new(
            NarrativeId::new(NARRATIVE).expect("id"),
            ContextMembership {
                claims: vec![ClaimId::new(CLAIM).expect("id")],
                themes: vec!["relief".into()],
                content: vec![SourceId::new(OUTLET).expect("id")],
                infrastructure: vec![],
                actors: vec![],
            },
            stamp(),
        ))
        .expect("narrative");
    graph
        .create_campaign(CampaignInput::new(
            CampaignId::new(CAMPAIGN).expect("id"),
            vec![NarrativeId::new(NARRATIVE).expect("id")],
            ContextMembership {
                claims: vec![],
                themes: vec!["influence".into()],
                content: vec![],
                infrastructure: vec![],
                actors: vec![node.clone()],
            },
            stamp(),
        ))
        .expect("campaign");
}

fn record_signal(graph: &mut Graph) -> String {
    let features: Vec<_> = [ANNOTATION, SECOND]
        .into_iter()
        .map(|evidence| {
            let mut feature = CampaignSignalFeatures::new(
                EvidenceId::new(evidence).expect("id"),
                "coordination-review-v1",
            );
            feature.generation_style_fingerprint = Some(FINGERPRINT.into());
            feature
        })
        .collect();
    let findings = graph
        .record_campaign_signals(
            &SignalScope::Narrative(NarrativeId::new(NARRATIVE).expect("id")),
            &features,
            stamp(),
        )
        .expect("signals");
    assert_eq!(findings.len(), 1);
    assert_eq!(
        findings[0].signal(),
        CampaignSignal::GenerationStyleFingerprint
    );
    findings[0].group_id().to_owned()
}

fn document(graph: &Graph) -> Value {
    let plan = build_deterministic_export_plan(graph, metadata(), &[]).expect("plan");
    serde_json::to_value(export_fimi_json_document(graph, &plan)).expect("document")
}

fn record(document: &Value) -> Value {
    document["records"]
        .as_array()
        .expect("records")
        .iter()
        .find(|record| record["kind"] == "outlet")
        .expect("exported outlet record")
        .clone()
}

//
// The combination that defines an influence operation must be expressible: the
// claims hold up and the piece still misleads. Both are exported, in separate
// fields, and neither is folded into the other.
#[test]
fn a_supported_claim_and_a_high_misleadingness_assessment_export_distinctly() {
    let (mut graph, node) = fixture(&recorded_assessment());
    collect(&mut graph, &node);
    support_claim(&mut graph);
    let exported = record(&document(&graph));

    let claim_lineage = exported["lineage"]
        .as_array()
        .expect("lineage")
        .iter()
        .find(|entry| entry["claim_id"] == CLAIM)
        .expect("claim lineage")
        .clone();
    assert_eq!(claim_lineage["verdict_state"], "supported");

    let assessment = &exported["misleadingness"][0];
    assert_eq!(assessment["band"], "high");
    assert_eq!(assessment["gap"], "overreach");
    assert_eq!(assessment["evidence_id"], ANNOTATION);
    assert_eq!(assessment["subject_id"], NARRATIVE);
    assert_eq!(
        assessment["mechanisms"],
        json!(["unsupported_inference", "omission"])
    );
    assert_eq!(
        assessment["traced_records"],
        json!(["omission-pattern--release", OBSERVATION])
    );
    assert_eq!(assessment["not_a_factual_determination"], true);

    // Separation is structural: no verdict field appears inside an assessment,
    // and no assessment field appears inside a verdict's lineage.
    for key in ["verdict_state", "verdict_id", "confidence_band"] {
        assert!(assessment[key].is_null(), "{key} must stay out of the assessment");
    }
    for key in ["band", "mechanisms", "gap", "not_a_factual_determination"] {
        assert!(
            claim_lineage[key].is_null(),
            "{key} must stay out of the claim lineage"
        );
    }
}

//
// Campaign lineage names the collections that contextualize a record, why each
// one matched, and the coordination evidence retained for it.
#[test]
fn campaign_lineage_carries_the_collections_and_their_coordination_signals() {
    let (mut graph, node) = fixture(&recorded_assessment());
    collect(&mut graph, &node);
    let group_id = record_signal(&mut graph);
    let exported = record(&document(&graph));

    let lineage = exported["campaign_lineage"]
        .as_array()
        .expect("campaign lineage");
    assert_eq!(lineage.len(), 2);

    let campaign = &lineage[0];
    assert_eq!(campaign["collection"], "campaign");
    assert_eq!(campaign["collection_id"], CAMPAIGN);
    assert_eq!(campaign["membership_roles"], json!(["actor"]));
    assert_eq!(campaign["narratives"], json!([NARRATIVE]));
    assert_eq!(campaign["themes"], json!(["influence"]));

    let narrative = &lineage[1];
    assert_eq!(narrative["collection"], "narrative");
    assert_eq!(narrative["collection_id"], NARRATIVE);
    assert_eq!(narrative["membership_roles"], json!(["claim", "content"]));
    assert_eq!(narrative["valid_from"], "2026-09-07T00:00:00Z");

    let signal = &narrative["coordination_signals"][0];
    assert_eq!(signal["signal"], "generation_style_fingerprint");
    assert_eq!(signal["group_id"], group_id);
    assert_eq!(signal["evidence_refs"], json!([ANNOTATION, SECOND]));
    assert_eq!(signal["source_refs"], json!([OUTLET, OTHER_OUTLET]));
    assert!(
        signal["reason"]
            .as_str()
            .expect("reason")
            .contains("coordination-review-v1")
    );
    // A shared production pattern is not authorship, and the export says so
    // rather than leaving a consumer to assume it.
    assert_eq!(signal["attribution"], "not_asserted");
    assert_eq!(signal["affects_independence"], true);
}

//
// A graph with governed lineage but no collection carries none of the new keys.
#[test]
fn governed_records_without_collections_carry_no_campaign_or_assessment_keys() {
    let (mut graph, _) = fixture(&json!({"unrelated": true}));
    support_claim(&mut graph);
    let plan = build_deterministic_export_plan(&graph, metadata(), &[]).expect("plan");
    let json = export_fimi_json(&graph, &plan).expect("json");

    assert!(json.contains("\"lineage\""), "governed lineage still exports");
    assert!(!json.contains("campaign_lineage"));
    assert!(!json.contains("misleadingness"));
    assert!(!json.contains("coordination_signals"));
}

//
// The WS-A discipline: a collection that references nothing exported leaves the
// bytes exactly as they were.
#[test]
fn a_collection_that_references_no_exported_record_keeps_the_export_byte_identical() {
    let (mut graph, _) = fixture(&json!({"unrelated": true}));
    support_claim(&mut graph);
    let plan = build_deterministic_export_plan(&graph, metadata(), &[]).expect("plan");
    let before = export_fimi_json(&graph, &plan).expect("json");

    graph
        .create_narrative(NarrativeInput::new(
            NarrativeId::new("narrative--unrelated").expect("id"),
            ContextMembership {
                claims: vec![],
                themes: vec!["unrelated".into()],
                content: vec![SourceId::new(OTHER_OUTLET).expect("id")],
                infrastructure: vec![],
                actors: vec![],
            },
            stamp(),
        ))
        .expect("narrative");

    assert_eq!(export_fimi_json(&graph, &plan).expect("json"), before);
}

//
// An annotation without a recorded report is carried as it stands: the exporter
// never derives a band, because the assessment policy belongs to the pack.
#[test]
fn an_annotation_without_a_recorded_report_exports_without_a_band() {
    let (mut graph, node) = fixture(&assessment_without_a_recorded_report());
    collect(&mut graph, &node);
    let exported = record(&document(&graph));
    let assessment = &exported["misleadingness"][0];

    assert!(assessment["band"].is_null());
    assert!(assessment["mechanisms"].is_null());
    assert!(assessment["explanation"].is_null());
    assert_eq!(
        assessment["declared_mechanisms"],
        json!(["omission", "unsupported_inference"])
    );
    assert_eq!(
        assessment["traced_records"],
        json!(["omission-pattern--release", OBSERVATION])
    );
    assert_eq!(
        assessment["reader_interpretation"],
        "the aid convoy was blocked to starve the region"
    );
}

//
// Pack data the exporter cannot read is skipped, never fatal: an export is a
// projection, not a validator.
#[test]
fn a_malformed_annotation_is_skipped_without_failing_the_export() {
    let (mut graph, node) = fixture(&json!({"fimi_misleadingness": {"gap": null}}));
    collect(&mut graph, &node);
    let exported = record(&document(&graph));

    assert!(exported["misleadingness"].is_null());
    assert_eq!(
        exported["campaign_lineage"][0]["collection_id"],
        CAMPAIGN,
        "campaign lineage is unaffected by an unreadable assessment"
    );
}

//
// Determinism holds with collections, signals and assessments present, and
// survives a native round trip.
#[test]
fn the_export_stays_deterministic_with_collections_and_signals() {
    let (mut graph, node) = fixture(&recorded_assessment());
    collect(&mut graph, &node);
    record_signal(&mut graph);
    support_claim(&mut graph);

    let first = build_deterministic_export_plan(&graph, metadata(), &[]).expect("plan");
    let second = build_deterministic_export_plan(&graph, metadata(), &[]).expect("plan");
    let json = export_fimi_json(&graph, &first).expect("json");

    assert_eq!(export_fimi_json(&graph, &second).expect("json"), json);
    let restored = Graph::from_persistence_snapshot(graph.persistence_snapshot()).expect("restore");
    assert_eq!(export_fimi_json(&restored, &first).expect("json"), json);
}

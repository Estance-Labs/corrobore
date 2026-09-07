// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//! Epic 0029 WS-G item 4 (issue #220): FIMI exports carry campaign lineage and
//! misleadingness additively, as fields distinct from every factual verdict,
//! and stay byte-identical for graphs with no narrative or campaign record.
use export_fimi::{export_fimi_json, export_fimi_json_document};
use graph_core::{
    BitemporalStamp, CampaignId, CampaignInput, CampaignSignal, CampaignSignalFeatures,
    ClaimAnalyticalTarget, ClaimId, ClaimInput, ClaimLink, ClaimLinkKind, ClaimLinkSource,
    ClaimStatement, ClaimTarget, Confidence, ContextMembership, EvidenceId, EvidenceInput,
    EvidenceRecordStore, EvidenceSourceType, ExportMetadata, ExportMode, ExportProfile, Graph,
    NarrativeId, NarrativeInput, NodeId, NodeInput, ObservationId, ObservationInput,
    ObservationModality, RecordStatus, ResolutionInputs, SignalScope, SourceAuthority,
    SourceAuthorityPolicy, SourceId, SourceInput, TemporalTimestamp, TransactionId,
    VerificationInputs, VerificationRecord, VerificationRecordId, VerificationResult,
    build_deterministic_export_plan, resolve_current_claim_verdict,
};
use serde_json::{Value, json};

const NARRATIVE: &str = "narrative--relief-convoy";
const CAMPAIGN: &str = "campaign--relief-convoy";
const CLAIM: &str = "claim--convoy-delay";
const CONTEXT_CLAIM: &str = "claim--convoy-context";
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
    let context = ClaimId::new(CONTEXT_CLAIM).expect("claim id");
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
    // A second claim of the same collection carries the records a coordination
    // signal needs, which is also what makes the campaign scope cross-claim.
    stores
        .claims
        .create_asserted_claim(ClaimInput::new(
            context.clone(),
            ClaimStatement::new("the checkpoint delay was reported by two outlets")
                .expect("statement"),
            ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new("convoy", None)),
        ))
        .expect("context claim");
    for evidence in [ANNOTATION, SECOND] {
        let evidence = EvidenceId::new(evidence).expect("id");
        stores.claims.register_evidence(evidence.clone());
        stores
            .claims
            .attach_link(
                ClaimLink::new(
                    ClaimLinkSource::Evidence(evidence),
                    context.clone(),
                    ClaimLinkKind::Supports,
                )
                .with_bitemporal(stamp()),
            )
            .expect("evidence link");
    }
    (graph, node)
}

// The export plan refuses a record whose claim is not actionable, so the
// exported claim gets grounded, authoritative support and a deterministic pass.
fn make_claim_actionable(graph: &mut Graph) {
    let claim = ClaimId::new(CLAIM).expect("claim id");
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
        let observation = ObservationId::new(format!("observation--gate-{name}")).expect("id");
        stores
            .observations
            .create_observation(
                ObservationInput::new(
                    observation.clone(),
                    source.clone(),
                    "convoy 7 of 12 held four hours at checkpoint, released same day",
                    ObservationModality::Text,
                ),
                &stores.sources,
            )
            .expect("observation");
        stores.claims.register_observation(observation.clone());
        stores
            .claims
            .attach_link(
                ClaimLink::new(
                    ClaimLinkSource::Observation(observation),
                    claim.clone(),
                    ClaimLinkKind::Supports,
                )
                .with_strength(Confidence::new(1.0).expect("score"))
                .with_bitemporal(stamp()),
            )
            .expect("link");
        bindings.push(
            SourceAuthority::new(
                source,
                "test",
                "fact",
                Confidence::new(1.0).expect("score"),
                "fimi-ws-g-authority-v1",
            )
            .expect("authority"),
        );
    }
    stores
        .verifications
        .append(VerificationRecord::new(
            VerificationRecordId::new("verification--convoy").expect("id"),
            "zz.grounded",
            "1",
            true,
            VerificationInputs::for_claim(claim.clone())
                .with_observation(ObservationId::new("observation--gate-first").expect("id")),
            VerificationResult::Pass,
            stamp(),
        ))
        .expect("verification");
    stores
        .verdicts
        .register_source_authority_policy(
            SourceAuthorityPolicy::new("fimi-ws-g-authority-v1", bindings).expect("policy"),
        )
        .expect("register");
    let evidence = EvidenceRecordStore::new();
    let inputs = ResolutionInputs::new(
        &stores.verifications,
        &evidence,
        &stores.observations,
        &stores.sources,
    )
    .with_source_authority("fimi-ws-g-authority-v1", "test", "fact");
    resolve_current_claim_verdict(
        &mut stores.claims,
        &mut stores.verdicts,
        &inputs,
        &claim,
        stamp(),
    )
    .expect("resolve");
}

fn collect(graph: &mut Graph, node: &NodeId) {
    graph
        .create_narrative(NarrativeInput::new(
            NarrativeId::new(NARRATIVE).expect("id"),
            ContextMembership {
                claims: vec![
                    ClaimId::new(CLAIM).expect("id"),
                    ClaimId::new(CONTEXT_CLAIM).expect("id"),
                ],
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
    make_claim_actionable(&mut graph);
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
        assert!(
            assessment[key].is_null(),
            "{key} must stay out of the assessment"
        );
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
    make_claim_actionable(&mut graph);
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
    make_claim_actionable(&mut graph);
    let plan = build_deterministic_export_plan(&graph, metadata(), &[]).expect("plan");
    let json = export_fimi_json(&graph, &plan).expect("json");

    assert!(
        json.contains("\"lineage\""),
        "governed lineage still exports"
    );
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
    make_claim_actionable(&mut graph);
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
    make_claim_actionable(&mut graph);
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
    make_claim_actionable(&mut graph);
    let exported = record(&document(&graph));

    assert!(exported["misleadingness"].is_null());
    assert_eq!(
        exported["campaign_lineage"][0]["collection_id"], CAMPAIGN,
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
    make_claim_actionable(&mut graph);

    let first = build_deterministic_export_plan(&graph, metadata(), &[]).expect("plan");
    let second = build_deterministic_export_plan(&graph, metadata(), &[]).expect("plan");
    let json = export_fimi_json(&graph, &first).expect("json");

    assert_eq!(export_fimi_json(&graph, &second).expect("json"), json);
    let restored = Graph::from_persistence_snapshot(graph.persistence_snapshot()).expect("restore");
    assert_eq!(export_fimi_json(&restored, &first).expect("json"), json);
}

//
// Epic acceptance: traceability has to be visible in the export. The core
// cannot enforce the pack's anchoring rule, so an annotation that cites nothing
// must export as visibly untraceable rather than as a grounded assessment.
#[test]
fn an_annotation_that_cites_nothing_exports_as_visibly_untraceable() {
    let mut unanchored = recorded_assessment();
    for finding in unanchored["fimi_misleadingness"]["findings"]
        .as_array_mut()
        .expect("findings")
    {
        finding["anchors"] = json!([]);
    }
    let (mut graph, node) = fixture(&unanchored);
    collect(&mut graph, &node);
    make_claim_actionable(&mut graph);
    let assessment = record(&document(&graph))["misleadingness"][0].clone();

    assert_eq!(assessment["band"], "high");
    assert!(
        assessment["traced_records"].is_null(),
        "an assessment citing nothing must not appear to cite something"
    );
    assert_eq!(assessment["not_a_factual_determination"], true);
}

//
// Epic acceptance: every assessment the exporter carries surfaces the records
// it traces to, and each one resolves to a record the graph holds.
#[test]
fn every_exported_assessment_surfaces_records_the_graph_holds() {
    let (mut graph, node) = fixture(&recorded_assessment());
    collect(&mut graph, &node);
    make_claim_actionable(&mut graph);
    let exported = record(&document(&graph));

    let assessments = exported["misleadingness"]
        .as_array()
        .expect("assessments")
        .clone();
    assert!(!assessments.is_empty());
    for assessment in assessments {
        let traced = assessment["traced_records"]
            .as_array()
            .expect("traced records")
            .clone();
        assert!(!traced.is_empty());
        for record in traced {
            let record = record.as_str().expect("record identity");
            assert!(
                record.starts_with("observation--") || record.starts_with("omission-pattern--"),
                "{record} must name an observation or an omission pattern record"
            );
        }
        assert_eq!(assessment["evidence_id"], ANNOTATION);
    }
}

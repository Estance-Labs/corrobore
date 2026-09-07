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
//! PROV-O mapping beside the Corrobore epistemic relations.
//!
//! The mapping is a projection of retained records, never a second provenance
//! model: the Corrobore relations stay authoritative and unchanged, and a graph
//! with no governed record gains nothing.
use export_stix::export_stix_subset_bundle;
use graph_core::{
    BitemporalStamp, ClaimId, ClaimInput, ClaimLink, ClaimLinkKind, ClaimLinkSource,
    ClaimStatement, ClaimTarget, EvidenceId, EvidenceInput, EvidenceSourceType, ExportMetadata,
    ExportMode, ExportProfile, Graph, NodeInput, ObservationId, ObservationInput,
    ObservationModality, PropertyValue, RecordStatus, SourceId, SourceInput, TemporalTimestamp,
    TransactionId, build_deterministic_export_plan,
};
use serde_json::Value;

fn metadata() -> ExportMetadata {
    ExportMetadata::new(
        "snapshot--prov",
        TransactionId::new("transaction--prov").expect("transaction id"),
        "stix-mvp-v2",
        ExportProfile::StixMvp,
        ExportMode::Permissive,
        None,
    )
    .expect("metadata")
}

fn stamp() -> BitemporalStamp {
    let time = TemporalTimestamp::new("2026-09-07T00:00:00Z").expect("time");
    BitemporalStamp::new(time.clone(), time).expect("stamp")
}

fn governed() -> Graph {
    let mut graph = Graph::new();
    graph
        .create_evidence(
            EvidenceInput::new(
                EvidenceId::new("evidence--prov").expect("id"),
                "https://vendor.example/report.pdf",
                "the recorded span",
            )
            .with_source_id(SourceId::new("source--report").expect("id"))
            .with_observation_id(ObservationId::new("observation--span").expect("id")),
        )
        .expect("evidence");
    let node = graph
        .create_node(
            NodeInput::new(["ThreatActor"])
                .with_status(RecordStatus::Exportable)
                .with_property("name", PropertyValue::String("provenance actor".to_owned()))
                .with_evidence_ref(EvidenceId::new("evidence--prov").expect("id")),
        )
        .expect("node");
    let claim = ClaimId::new("claim--prov").expect("claim id");
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
                "the recorded span",
                ObservationModality::Text,
            ),
            &stores.sources,
        )
        .expect("observation");
    stores
        .claims
        .create_asserted_claim(ClaimInput::new(
            claim.clone(),
            ClaimStatement::new("the actor was reported").expect("statement"),
            ClaimTarget::Node(node),
        ))
        .expect("claim");
    stores
        .claims
        .register_observation(ObservationId::new("observation--span").expect("id"));
    stores
        .claims
        .attach_link(
            ClaimLink::new(
                ClaimLinkSource::Observation(ObservationId::new("observation--span").expect("id")),
                claim.clone(),
                ClaimLinkKind::Supports,
            )
            .with_bitemporal(stamp()),
        )
        .expect("link");
    make_claim_actionable(&mut graph, &claim);
    graph
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

fn exported(graph: &Graph) -> Value {
    let plan = build_deterministic_export_plan(graph, metadata(), &[]).expect("plan");
    serde_json::to_value(export_stix_subset_bundle(graph, &plan)).expect("json")
}

fn object(bundle: &Value) -> Value {
    bundle["objects"]
        .as_array()
        .expect("objects")
        .iter()
        .find(|object| object["x_corrobore_lineage"].is_array())
        .expect("an object with retained lineage")
        .clone()
}

//
// Every lineage entry gains its PROV-O reading, and the Corrobore relation it
// maps stays exactly where it was.
#[test]
fn lineage_entries_carry_a_prov_o_mapping_beside_the_corrobore_relations() {
    let bundle = exported(&governed());
    let lineage = object(&bundle)["x_corrobore_lineage"]
        .as_array()
        .expect("lineage")
        .clone();

    let evidence = lineage
        .iter()
        .find(|entry| entry["evidence_id"] == "evidence--prov")
        .expect("evidence lineage entry");
    assert_eq!(evidence["source_id"], "source--report");
    assert_eq!(evidence["observation_id"], "observation--span");
    let prov = &evidence["prov"];
    assert_eq!(prov["@type"], "prov:Entity");
    assert_eq!(prov["@id"], "observation--span");
    assert_eq!(
        prov["prov:wasDerivedFrom"],
        serde_json::json!(["source--report"])
    );

    let claim = lineage
        .iter()
        .find(|entry| entry["claim_id"] == "claim--prov")
        .expect("claim lineage entry");
    assert_eq!(claim["verdict_state"], "supported");
    let prov = &claim["prov"];
    assert_eq!(prov["@type"], "prov:Entity");
    assert_eq!(prov["@id"], "claim--prov");
    assert_eq!(
        prov["prov:wasGeneratedBy"]["@type"], "prov:Activity",
        "a verdict is the activity that generated the claim's state"
    );
    assert_eq!(
        prov["prov:wasGeneratedBy"]["@id"], claim["verdict_id"],
        "the activity is the retained verdict, not a synthesized identity"
    );
    let used = prov["prov:used"].as_array().expect("used entities").clone();
    assert!(
        used.contains(&Value::String("observation--span".to_owned())),
        "the activity used the observations the claim links to"
    );
    let mut sorted = used.clone();
    sorted.sort_by_key(|entry| entry.as_str().unwrap_or_default().to_owned());
    sorted.dedup();
    assert_eq!(
        used, sorted,
        "used entities are deterministic and deduplicated"
    );
    assert!(
        used.iter().all(|entry| entry
            .as_str()
            .is_some_and(|id| id.starts_with("observation--"))),
        "only observations the claim links to are named"
    );
}

//
// A graph with no governed record gains no lineage and therefore no mapping:
// exports stay byte-identical.
#[test]
fn a_graph_without_governed_records_gains_no_prov_mapping() {
    let mut graph = Graph::new();
    graph
        .create_node(
            NodeInput::new(["ThreatActor"])
                .with_status(RecordStatus::Exportable)
                .with_property("name", PropertyValue::String("bare".to_owned())),
        )
        .expect("node");
    let plan = build_deterministic_export_plan(&graph, metadata(), &[]).expect("plan");
    let json = serde_json::to_string(&export_stix_subset_bundle(&graph, &plan)).expect("json");

    assert!(!json.contains("x_corrobore_lineage"));
    assert!(!json.contains("prov:"));
}

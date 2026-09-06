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
#![warn(missing_docs)]

//! Deterministic FIMI JSON exporter for the intelligence graph engine.
//!
//! Transforms a graph and its export plan into a stable FIMI-compatible JSON
//! document with deterministic record ordering.

use graph_core::{
    DeterministicExportPlan, ExportMode, ExportProfile, ExportRecordKind, Graph, Node, NodeId,
    RelationshipId, VerificationCoverage,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
/// Fimi export document.
pub struct FimiExportDocument {
    schema: &'static str,
    records: Vec<FimiRecord>,
    export_metadata: ExportMetadataView,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct FimiRecord {
    id: String,
    kind: String,
    source_record_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_node_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_node_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    relationship_type: Option<String>,
    evidence_refs: Vec<String>,
    /// Epic 0029 WS-A item 7: source and observation behind each evidence
    /// reference, present only when governed records exist.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    lineage: Vec<FimiLineage>,
}

/// Epistemic lineage of one evidence reference.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct FimiLineage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    confidence_band: Option<domain_common::ConfidenceBand>,
    #[serde(skip_serializing_if = "Option::is_none")]
    evidence_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    observation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    claim_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    verdict_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    verdict_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    transitions: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    verification_coverage: Option<VerificationCoverage>,
}

fn epistemic_lineage(
    graph: &Graph,
    evidence_refs: &[String],
    node_id: Option<&NodeId>,
    relationship_id: Option<&RelationshipId>,
) -> Vec<FimiLineage> {
    let stores = graph.epistemic_stores();
    let mut lineage: Vec<FimiLineage> = evidence_refs
        .iter()
        .filter_map(|evidence_ref| {
            let evidence_id = graph_core::EvidenceId::new(evidence_ref).ok()?;
            let record = graph.evidence_by_id(&evidence_id)?;
            if record.source_id().is_none() && record.observation_id().is_none() {
                return None;
            }
            Some(FimiLineage {
                confidence_band: None,
                evidence_id: Some(evidence_ref.clone()),
                source_id: record.source_id().map(|id| id.as_str().to_owned()),
                source_uri: record
                    .source_id()
                    .and_then(|id| stores.sources.current_source(id))
                    .map(|source| source.uri().to_owned()),
                observation_id: record.observation_id().map(|id| id.as_str().to_owned()),
                claim_id: None,
                verdict_id: None,
                verdict_state: None,
                transitions: None,
                verification_coverage: None,
            })
        })
        .collect();

    let mut claims: Vec<_> = stores
        .claims
        .claims()
        .into_iter()
        .filter(|claim| match claim.target() {
            graph_core::ClaimTarget::Node(target) => node_id == Some(target),
            graph_core::ClaimTarget::Relationship(target) => relationship_id == Some(target),
            _ => false,
        })
        .collect();
    claims.sort_by(|left, right| left.id().as_str().cmp(right.id().as_str()));
    for claim in claims {
        let verdict = stores.verdicts.current_verdict(claim.id());
        lineage.push(FimiLineage {
            confidence_band: verdict.map(|v| {
                domain_common::classify_confidence_band_with_actionability(
                    v.confidence_dimensions(),
                )
            }),
            evidence_id: None,
            source_id: None,
            source_uri: None,
            observation_id: None,
            claim_id: Some(claim.id().as_str().to_owned()),
            verdict_id: verdict.map(|value| value.id().as_str().to_owned()),
            verdict_state: verdict.map(|value| value.state().as_str().to_owned()),
            transitions: verdict.map(|_| stores.verdicts.transitions_for_claim(claim.id()).len()),
            verification_coverage: Some(VerificationCoverage::derive(claim, &stores.verifications)),
        });
    }
    lineage
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct ExportMetadataView {
    snapshot_id: String,
    transaction_id: String,
    exporter_version: String,
    profile: &'static str,
    mode: &'static str,
    determinism_key: String,
}

/// Export fimi json document.
pub fn export_fimi_json_document(
    graph: &Graph,
    plan: &DeterministicExportPlan,
) -> FimiExportDocument {
    let mut records = plan
        .records()
        .iter()
        .filter_map(|record| {
            let mut evidence_refs = record
                .evidence_refs()
                .iter()
                .map(|id| id.as_str().to_owned())
                .collect::<Vec<String>>();
            evidence_refs.sort();

            match record.kind() {
                ExportRecordKind::Node => {
                    let node_id = NodeId::new(record.record_id()).ok()?;
                    let node = graph.get_node(&node_id).ok().flatten()?;

                    Some(FimiRecord {
                        id: record.export_record_id().to_owned(),
                        kind: fimi_node_kind(&node).to_owned(),
                        source_record_id: record.record_id().to_owned(),
                        source_node_id: Some(node.id().as_str().to_owned()),
                        target_node_id: None,
                        relationship_type: None,
                        lineage: epistemic_lineage(graph, &evidence_refs, Some(node.id()), None),
                        evidence_refs,
                    })
                }
                ExportRecordKind::Relationship => {
                    let relationship_id = RelationshipId::new(record.record_id()).ok()?;
                    let relationship = graph.get_relationship(&relationship_id).ok().flatten()?;

                    Some(FimiRecord {
                        id: record.export_record_id().to_owned(),
                        kind: "coordination_link".to_owned(),
                        source_record_id: record.record_id().to_owned(),
                        source_node_id: Some(relationship.source().as_str().to_owned()),
                        target_node_id: Some(relationship.target().as_str().to_owned()),
                        relationship_type: Some(relationship.rel_type().as_str().to_lowercase()),
                        lineage: epistemic_lineage(
                            graph,
                            &evidence_refs,
                            None,
                            Some(relationship.id()),
                        ),
                        evidence_refs,
                    })
                }
            }
        })
        .collect::<Vec<FimiRecord>>();

    records.sort_by(|left, right| left.id.cmp(&right.id));

    let metadata = plan.metadata();

    FimiExportDocument {
        // Schema.
        schema: "fimi-json-mvp",
        records,
        // Export metadata.
        export_metadata: ExportMetadataView {
            // Snapshot id.
            snapshot_id: metadata.snapshot_id().to_owned(),
            // Transaction id.
            transaction_id: metadata.transaction_id().as_str().to_owned(),
            // Exporter version.
            exporter_version: metadata.exporter_version().to_owned(),
            // Profile.
            profile: profile_label(metadata.profile()),
            mode: mode_label(metadata.mode()),
            // Determinism key.
            determinism_key: metadata.determinism_key(),
        },
    }
}

/// Export fimi json.
pub fn export_fimi_json(
    graph: &Graph,
    plan: &DeterministicExportPlan,
) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(&export_fimi_json_document(graph, plan))
}

fn fimi_node_kind(node: &Node) -> &'static str {
    if node.has_label("Claim") {
        return "claim";
    }
    if node.has_label("Narrative") {
        return "narrative";
    }
    if node.has_label("Actor") || node.has_label("ThreatActor") {
        return "actor";
    }
    if node.has_label("Account") {
        return "account";
    }
    if node.has_label("Outlet") {
        return "outlet";
    }
    if node.has_label("Campaign") {
        return "campaign";
    }
    if node.has_label("CoordinationCluster") {
        return "coordination_cluster";
    }

    "entity"
}

fn profile_label(profile: &ExportProfile) -> &'static str {
    match profile {
        ExportProfile::StixMvp => "stix-mvp",
        ExportProfile::FimiJsonMvp => "fimi-json-mvp",
    }
}

fn mode_label(mode: ExportMode) -> &'static str {
    match mode {
        ExportMode::Strict => "strict",
        ExportMode::Permissive => "permissive",
    }
}

#[cfg(test)]
mod tests {
    use graph_core::{
        ExportMetadata, ExportMode, ExportProfile, Graph, NodeInput, RecordStatus,
        RelationshipInput, TransactionId, build_deterministic_export_plan,
    };

    use super::{
        export_fimi_json, export_fimi_json_document, fimi_node_kind, mode_label, profile_label,
    };

    fn permissive_metadata() -> ExportMetadata {
        ExportMetadata::new(
            "snapshot--fimi",
            TransactionId::new("transaction--fimi").expect("transaction ID should be valid"),
            "fimi-json-v1",
            ExportProfile::FimiJsonMvp,
            ExportMode::Permissive,
            None,
        )
        .expect("metadata should be valid")
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
    fn fimi_export_json_is_deterministic_for_same_inputs() {
        let mut graph = Graph::new();
        let source = graph
            .create_node(NodeInput::new(["Actor"]).with_status(RecordStatus::Exportable))
            .expect("source node creation should succeed");
        let target = graph
            .create_node(NodeInput::new(["Narrative"]).with_status(RecordStatus::Exportable))
            .expect("target node creation should succeed");

        graph
            .create_relationship(
                RelationshipInput::new(source, "linked_to", target)
                    .expect("relationship input should be valid")
                    .with_status(RecordStatus::Exportable),
            )
            .expect("relationship creation should succeed");

        let plan_a = build_deterministic_export_plan(&graph, permissive_metadata(), &[])
            .expect("plan A should succeed");
        let plan_b = build_deterministic_export_plan(&graph, permissive_metadata(), &[])
            .expect("plan B should succeed");

        let json_a = export_fimi_json(&graph, &plan_a).expect("fimi json A should serialize");
        let json_b = export_fimi_json(&graph, &plan_b).expect("fimi json B should serialize");

        assert_eq!(json_a, json_b);
        assert!(json_a.contains("\"kind\": \"actor\""));
        assert!(json_a.contains("\"kind\": \"narrative\""));
        assert!(json_a.contains("\"kind\": \"coordination_link\""));
    }

    //
    // Epic 0029 WS-A item 7 (issue #153): records carry epistemic lineage
    // additively and stay byte-identical without governed records.
    #[test]
    fn fimi_records_carry_additive_epistemic_lineage() {
        use graph_core::{
            EvidenceId, EvidenceInput, EvidenceSourceType, ObservationId, ObservationInput,
            ObservationModality, SourceId, SourceInput,
        };

        let mut graph = Graph::new();
        graph
            .create_evidence(EvidenceInput::new(
                EvidenceId::new("evidence--fimi").expect("id"),
                "ref--fimi",
                "payload",
            ))
            .expect("evidence");
        graph
            .create_node(
                NodeInput::new(["Narrative"])
                    .with_status(RecordStatus::Exportable)
                    .with_evidence_ref(EvidenceId::new("evidence--fimi").expect("id")),
            )
            .expect("node");
        let plan =
            build_deterministic_export_plan(&graph, permissive_metadata(), &[]).expect("plan");
        let bare = export_fimi_json(&graph, &plan).expect("json");
        assert!(
            !bare.contains("\"lineage\""),
            "no lineage key without governed records"
        );

        let mut governed = Graph::new();
        governed
            .create_evidence(
                EvidenceInput::new(
                    EvidenceId::new("evidence--fimi").expect("id"),
                    "ref--fimi",
                    "payload",
                )
                .with_source_id(SourceId::new("source--outlet").expect("id"))
                .with_observation_id(ObservationId::new("observation--post").expect("id")),
            )
            .expect("evidence");
        governed
            .create_node(
                NodeInput::new(["Narrative"])
                    .with_status(RecordStatus::Exportable)
                    .with_evidence_ref(EvidenceId::new("evidence--fimi").expect("id")),
            )
            .expect("node");
        let stores = governed.epistemic_stores_mut();
        stores
            .sources
            .register_source(SourceInput::new(
                SourceId::new("source--outlet").expect("id"),
                "https://outlet.example/post",
                EvidenceSourceType::Url,
            ))
            .expect("source");
        stores
            .observations
            .create_observation(
                ObservationInput::new(
                    ObservationId::new("observation--post").expect("id"),
                    SourceId::new("source--outlet").expect("id"),
                    "the post text",
                    ObservationModality::Text,
                ),
                &stores.sources,
            )
            .expect("observation");
        let plan =
            build_deterministic_export_plan(&governed, permissive_metadata(), &[]).expect("plan");
        let document = export_fimi_json_document(&governed, &plan);
        let json = serde_json::to_value(&document).expect("json");
        let lineage = &json["records"][0]["lineage"];
        assert_eq!(lineage[0]["evidence_id"], "evidence--fimi");
        assert_eq!(lineage[0]["source_id"], "source--outlet");
        assert_eq!(lineage[0]["observation_id"], "observation--post");
        assert_eq!(lineage[0]["source_uri"], "https://outlet.example/post");
    }

    #[test]
    fn fimi_claim_lineage_carries_verification_coverage() {
        use graph_core::{
            BitemporalStamp, ClaimId, ClaimInput, ClaimStatement, ClaimTarget, TemporalTimestamp,
            VerificationInputs, VerificationRecord, VerificationRecordId, VerificationResult,
        };

        let mut graph = Graph::new();
        let node_id = graph
            .create_node(NodeInput::new(["Narrative"]).with_status(RecordStatus::Exportable))
            .expect("node");
        let claim_id = ClaimId::new("claim--fimi-coverage").expect("claim id");
        let stores = graph.epistemic_stores_mut();
        stores
            .claims
            .create_asserted_claim(ClaimInput::new(
                claim_id.clone(),
                ClaimStatement::new("The narrative claim was checked").expect("statement"),
                ClaimTarget::Node(node_id),
            ))
            .expect("claim");
        stores
            .verifications
            .append(VerificationRecord::new(
                VerificationRecordId::new("verification--fimi-coverage").expect("verification id"),
                "fr.estance.corrobore.domain.fimi.claim.verify",
                "1.4.0",
                false,
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
        let plan =
            build_deterministic_export_plan(&graph, permissive_metadata(), &[]).expect("plan");
        let json =
            serde_json::to_value(export_fimi_json_document(&graph, &plan)).expect("document");
        let lineage = json["records"][0]["lineage"]
            .as_array()
            .expect("lineage array");
        let claim = lineage
            .iter()
            .find(|entry| entry["claim_id"] == "claim--fimi-coverage")
            .expect("claim lineage");
        assert_eq!(claim["confidence_band"], "Exportable");
        assert_eq!(
            claim["verification_coverage"]["entries"][0]["class"],
            "semantically_judged"
        );
        assert_eq!(
            claim["verification_coverage"]["entries"][0]["verifier_id"],
            "fr.estance.corrobore.domain.fimi.claim.verify"
        );
        assert_eq!(
            claim["verification_coverage"]["entries"][0]["verifier_version"],
            "1.4.0"
        );
    }

    #[test]
    fn fimi_node_kind_maps_supported_labels_and_falls_back_to_entity() {
        let mut graph = Graph::new();

        let claim = graph
            .create_node(NodeInput::new(["Claim"]))
            .expect("node creation should succeed");
        let threat_actor = graph
            .create_node(NodeInput::new(["ThreatActor"]))
            .expect("node creation should succeed");
        let outlet = graph
            .create_node(NodeInput::new(["Outlet"]))
            .expect("node creation should succeed");
        let fallback = graph
            .create_node(NodeInput::new(["UnknownLabel"]))
            .expect("node creation should succeed");

        let claim = graph
            .get_node(&claim)
            .expect("graph lookup should succeed")
            .expect("node should exist");
        let threat_actor = graph
            .get_node(&threat_actor)
            .expect("graph lookup should succeed")
            .expect("node should exist");
        let outlet = graph
            .get_node(&outlet)
            .expect("graph lookup should succeed")
            .expect("node should exist");
        let fallback = graph
            .get_node(&fallback)
            .expect("graph lookup should succeed")
            .expect("node should exist");

        assert_eq!(fimi_node_kind(&claim), "claim");
        assert_eq!(fimi_node_kind(&threat_actor), "actor");
        assert_eq!(fimi_node_kind(&outlet), "outlet");
        assert_eq!(fimi_node_kind(&fallback), "entity");
    }

    #[test]
    fn helper_profile_and_mode_labels_cover_all_variants() {
        assert_eq!(profile_label(&ExportProfile::StixMvp), "stix-mvp");
        assert_eq!(profile_label(&ExportProfile::FimiJsonMvp), "fimi-json-mvp");
        assert_eq!(mode_label(ExportMode::Strict), "strict");
        assert_eq!(mode_label(ExportMode::Permissive), "permissive");
    }

    #[test]
    fn fimi_document_records_are_sorted_by_record_id_and_include_metadata() {
        let mut graph = Graph::new();
        let first = graph
            .create_node(NodeInput::new(["Actor"]).with_status(RecordStatus::Exportable))
            .expect("first node creation should succeed");
        let second = graph
            .create_node(NodeInput::new(["Narrative"]).with_status(RecordStatus::Exportable))
            .expect("second node creation should succeed");
        graph
            .create_relationship(
                RelationshipInput::new(second, "linked_to", first)
                    .expect("relationship input should be valid")
                    .with_status(RecordStatus::Exportable),
            )
            .expect("relationship creation should succeed");

        let plan = build_deterministic_export_plan(&graph, permissive_metadata(), &[])
            .expect("plan should succeed");
        let document = export_fimi_json_document(&graph, &plan);

        let ids = document
            .records
            .iter()
            .map(|record| record.id.clone())
            .collect::<Vec<String>>();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted);

        assert_eq!(document.schema, "fimi-json-mvp");
        assert_eq!(document.export_metadata.profile, "fimi-json-mvp");
        assert_eq!(document.export_metadata.mode, "permissive");
    }
}

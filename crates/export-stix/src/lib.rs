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

//! Deterministic STIX 2.1 subset exporter for the intelligence graph engine.
//!
//! Transforms a graph and its export plan into a stable STIX-compatible JSON
//! bundle with deterministic record ordering.

use graph_core::{
    DeterministicExportPlan, ExportMode, ExportProfile, ExportRecordKind, Graph, Node, NodeId,
    RelationshipId,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
/// Stix export bundle.
pub struct StixExportBundle {
    #[serde(rename = "type")]
    bundle_type: &'static str,
    id: String,
    spec_version: &'static str,
    objects: Vec<StixObject>,
    export_metadata: ExportMetadataView,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
struct StixObject {
    #[serde(rename = "type")]
    object_type: String,
    id: String,
    spec_version: &'static str,
    created: &'static str,
    modified: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    relationship_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_ref: Option<String>,
    source_record_id: String,
    evidence_refs: Vec<String>,
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

/// Export stix subset bundle.
pub fn export_stix_subset_bundle(
    graph: &Graph,
    plan: &DeterministicExportPlan,
) -> StixExportBundle {
    let mut objects = plan
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
                    let object_type = stix_node_type(&node).to_owned();

                    Some(StixObject {
                        id: stix_id(object_type.as_str(), record.record_id()),
                        object_type,
                        spec_version: "2.1",
                        created: "1970-01-01T00:00:00.000Z",
                        modified: "1970-01-01T00:00:00.000Z",
                        name: Some(record.record_id().to_owned()),
                        relationship_type: None,
                        source_ref: None,
                        target_ref: None,
                        source_record_id: record.record_id().to_owned(),
                        evidence_refs,
                    })
                }
                ExportRecordKind::Relationship => {
                    let relationship_id = RelationshipId::new(record.record_id()).ok()?;
                    let relationship = graph.get_relationship(&relationship_id).ok().flatten()?;
                    let source_ref = stix_id_for_node_ref(relationship.source().as_str());
                    let target_ref = stix_id_for_node_ref(relationship.target().as_str());

                    Some(StixObject {
                        id: stix_id("relationship", record.record_id()),
                        object_type: "relationship".to_owned(),
                        spec_version: "2.1",
                        created: "1970-01-01T00:00:00.000Z",
                        modified: "1970-01-01T00:00:00.000Z",
                        name: None,
                        relationship_type: Some(relationship.rel_type().as_str().to_lowercase()),
                        source_ref: Some(source_ref),
                        target_ref: Some(target_ref),
                        source_record_id: record.record_id().to_owned(),
                        evidence_refs,
                    })
                }
            }
        })
        .collect::<Vec<StixObject>>();

    objects.sort_by(|left, right| left.id.cmp(&right.id));

    let metadata = plan.metadata();

    StixExportBundle {
        // Bundle type.
        bundle_type: "bundle",
        // Id.
        id: format!("bundle--{}", plan.determinism_fingerprint()),
        // Spec version.
        spec_version: "2.1",
        objects,
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

/// Export stix subset json.
pub fn export_stix_subset_json(
    graph: &Graph,
    plan: &DeterministicExportPlan,
) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(&export_stix_subset_bundle(graph, plan))
}

fn stix_node_type(node: &Node) -> &'static str {
    if node.has_label("ThreatActor") {
        return "threat-actor";
    }
    if node.has_label("Indicator") {
        return "indicator";
    }
    if node.has_label("Malware") {
        return "malware";
    }
    if node.has_label("Tool") {
        return "tool";
    }
    if node.has_label("Campaign") {
        return "campaign";
    }
    if node.has_label("Infrastructure") {
        return "infrastructure";
    }
    if node.has_label("Vulnerability") {
        return "vulnerability";
    }
    if node.has_label("Identity") {
        return "identity";
    }
    if node.has_label("Location") {
        return "location";
    }
    if node.has_label("Report") {
        return "report";
    }

    "identity"
}

fn stix_id_for_node_ref(node_id: &str) -> String {
    // Relationship references point to node SDO identifiers.
    stix_id("identity", node_id)
}

fn stix_id(object_type: &str, source_record_id: &str) -> String {
    format!("{}--{}", object_type, stable_hex_hash(source_record_id))
}

fn stable_hex_hash(input: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{:016x}", hash)
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
        export_stix_subset_bundle, export_stix_subset_json, mode_label, profile_label,
        stable_hex_hash, stix_id, stix_id_for_node_ref, stix_node_type,
    };

    fn strict_metadata() -> ExportMetadata {
        ExportMetadata::new(
            "snapshot--stix",
            TransactionId::new("transaction--stix").expect("transaction ID should be valid"),
            "stix-mvp-v1",
            ExportProfile::StixMvp,
            ExportMode::Strict,
            None,
        )
        .expect("metadata should be valid")
    }

    #[test]
    fn stix_export_json_is_deterministic_for_same_inputs() {
        let mut graph = Graph::new();
        let source = graph
            .create_node(NodeInput::new(["ThreatActor"]).with_status(RecordStatus::Exportable))
            .expect("source node creation should succeed");
        let target = graph
            .create_node(NodeInput::new(["Indicator"]).with_status(RecordStatus::Exportable))
            .expect("target node creation should succeed");

        graph
            .create_relationship(
                RelationshipInput::new(source, "indicates", target)
                    .expect("relationship input should be valid")
                    .with_status(RecordStatus::Exportable),
            )
            .expect("relationship creation should succeed");

        let plan_a = build_deterministic_export_plan(&graph, strict_metadata(), &[])
            .expect("plan A should succeed");
        let plan_b = build_deterministic_export_plan(&graph, strict_metadata(), &[])
            .expect("plan B should succeed");

        let json_a =
            export_stix_subset_json(&graph, &plan_a).expect("stix json A should serialize");
        let json_b =
            export_stix_subset_json(&graph, &plan_b).expect("stix json B should serialize");

        assert_eq!(json_a, json_b);
        assert!(json_a.contains("\"type\": \"threat-actor\""));
        assert!(json_a.contains("\"type\": \"indicator\""));
        assert!(json_a.contains("\"type\": \"relationship\""));
    }

    #[test]
    fn stix_node_type_maps_supported_labels_and_falls_back_to_identity() {
        let mut graph = Graph::new();

        let threat_actor = graph
            .create_node(NodeInput::new(["ThreatActor"]))
            .expect("node creation should succeed");
        let malware = graph
            .create_node(NodeInput::new(["Malware"]))
            .expect("node creation should succeed");
        let location = graph
            .create_node(NodeInput::new(["Location"]))
            .expect("node creation should succeed");
        let fallback = graph
            .create_node(NodeInput::new(["CustomLabel"]))
            .expect("node creation should succeed");

        let threat_actor = graph
            .get_node(&threat_actor)
            .expect("graph lookup should succeed")
            .expect("node should exist");
        let malware = graph
            .get_node(&malware)
            .expect("graph lookup should succeed")
            .expect("node should exist");
        let location = graph
            .get_node(&location)
            .expect("graph lookup should succeed")
            .expect("node should exist");
        let fallback = graph
            .get_node(&fallback)
            .expect("graph lookup should succeed")
            .expect("node should exist");

        assert_eq!(stix_node_type(&threat_actor), "threat-actor");
        assert_eq!(stix_node_type(&malware), "malware");
        assert_eq!(stix_node_type(&location), "location");
        assert_eq!(stix_node_type(&fallback), "identity");
    }

    #[test]
    fn helper_labels_and_ids_are_deterministic() {
        assert_eq!(profile_label(&ExportProfile::StixMvp), "stix-mvp");
        assert_eq!(profile_label(&ExportProfile::FimiJsonMvp), "fimi-json-mvp");
        assert_eq!(mode_label(ExportMode::Strict), "strict");
        assert_eq!(mode_label(ExportMode::Permissive), "permissive");

        let a = stable_hex_hash("record--1");
        let b = stable_hex_hash("record--1");
        let c = stable_hex_hash("record--2");
        assert_eq!(a, b);
        assert_ne!(a, c);

        let indicator_id = stix_id("indicator", "record--indicator-1");
        let node_ref_id = stix_id_for_node_ref("node--1");
        assert!(indicator_id.starts_with("indicator--"));
        assert!(node_ref_id.starts_with("identity--"));
    }

    #[test]
    fn stix_export_bundle_sorts_objects_by_id() {
        let mut graph = Graph::new();
        let first = graph
            .create_node(NodeInput::new(["Indicator"]).with_status(RecordStatus::Exportable))
            .expect("first node creation should succeed");
        let second = graph
            .create_node(NodeInput::new(["Indicator"]).with_status(RecordStatus::Exportable))
            .expect("second node creation should succeed");
        graph
            .create_relationship(
                RelationshipInput::new(second, "related-to", first)
                    .expect("relationship input should be valid")
                    .with_status(RecordStatus::Exportable),
            )
            .expect("relationship creation should succeed");

        let plan = build_deterministic_export_plan(&graph, strict_metadata(), &[])
            .expect("plan should succeed");
        let bundle = export_stix_subset_bundle(&graph, &plan);

        let ids = bundle
            .objects
            .iter()
            .map(|object| object.id.clone())
            .collect::<Vec<String>>();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted);

        // Ensure metadata shape remains deterministic and complete.
        assert_eq!(bundle.bundle_type, "bundle");
        assert_eq!(bundle.spec_version, "2.1");
        assert_eq!(bundle.export_metadata.profile, "stix-mvp");
        assert_eq!(bundle.export_metadata.mode, "strict");
    }
}

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

use std::collections::{BTreeMap, BTreeSet};

use graph_core::{
    DeterministicExportPlan, ExportMode, ExportProfile, ExportRecordKind, Graph, Node, NodeId,
    PropertyValue, Relationship, RelationshipId, ValidationTarget,
};
use serde::Serialize;
use serde_json::{Map, Number, Value};

#[derive(Clone, Debug, PartialEq, Serialize)]
/// Stix export bundle.
pub struct StixExportBundle {
    #[serde(rename = "type")]
    bundle_type: &'static str,
    id: String,
    spec_version: &'static str,
    objects: Vec<Value>,
    export_metadata: ExportMetadataView,
    export_diagnostics: ExportDiagnostics,
    #[serde(rename = "x_corrobore_evidence", skip_serializing_if = "Vec::is_empty")]
    evidence: Vec<Value>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
struct ExportDiagnostics {
    exclusions: Vec<ExportDiagnostic>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct ExportDiagnostic {
    code: String,
    message: String,
    target_kind: &'static str,
    record_id: String,
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
    let mut objects = Vec::new();
    let mut exported_node_ids = BTreeMap::<String, String>::new();

    for record in plan
        .records()
        .iter()
        .filter(|record| record.kind() == ExportRecordKind::Node)
    {
        let Some(node) = NodeId::new(record.record_id())
            .ok()
            .and_then(|node_id| graph.get_node(&node_id).ok().flatten())
        else {
            continue;
        };
        let Some(object) = project_node(&node, record.record_id(), record.evidence_refs()) else {
            continue;
        };
        let Some(id) = object.get("id").and_then(Value::as_str) else {
            continue;
        };
        exported_node_ids.insert(record.record_id().to_owned(), id.to_owned());
        objects.push(object);
    }

    for record in plan
        .records()
        .iter()
        .filter(|record| record.kind() == ExportRecordKind::Relationship)
    {
        let Some(relationship) = RelationshipId::new(record.record_id())
            .ok()
            .and_then(|relationship_id| graph.get_relationship(&relationship_id).ok().flatten())
        else {
            continue;
        };
        let Some(source_ref) = exported_node_ids.get(relationship.source().as_str()) else {
            continue;
        };
        let Some(target_ref) = exported_node_ids.get(relationship.target().as_str()) else {
            continue;
        };
        if let Some(object) = project_relationship(
            &relationship,
            record.record_id(),
            source_ref,
            target_ref,
            record.evidence_refs(),
        ) {
            objects.push(object);
        }
    }

    objects.sort_by(|left, right| object_id(left).cmp(object_id(right)));

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
        export_diagnostics: ExportDiagnostics {
            exclusions: plan
                .warnings()
                .iter()
                .take(256)
                .filter_map(export_diagnostic)
                .collect(),
        },
        evidence: retained_evidence(graph, plan),
    }
}

fn project_node(
    node: &Node,
    record_id: &str,
    evidence_refs: &[graph_core::EvidenceId],
) -> Option<Value> {
    let mut object = if let Some(raw) = canonical_stix_object(node) {
        raw.clone()
    } else {
        let object_type = stix_node_type(node)?;
        let id =
            native_stix_id(node, object_type).unwrap_or_else(|| stix_id(object_type, record_id));
        let mut object = Map::new();
        object.insert("type".to_owned(), Value::String(object_type.to_owned()));
        object.insert("id".to_owned(), Value::String(id));
        object.insert("spec_version".to_owned(), Value::String("2.1".to_owned()));
        object.insert(
            "created".to_owned(),
            property_or_default(node.property("created"), "1970-01-01T00:00:00.000Z"),
        );
        object.insert(
            "modified".to_owned(),
            property_or_default(node.property("modified"), "1970-01-01T00:00:00.000Z"),
        );
        for (key, value) in node.properties() {
            if key.starts_with("opencti.")
                || matches!(
                    key.as_str(),
                    "stix_id" | "external_id" | "confidence" | "status" | "evidence_refs"
                )
            {
                continue;
            }
            object.insert(key.clone(), property_value_to_json(value));
        }
        if !object.contains_key("name") && object_type_supports_name(object_type) {
            object.insert("name".to_owned(), Value::String(record_id.to_owned()));
        }
        object
    };
    apply_native_metadata(
        &mut object,
        node.confidence().map(|value| value.value()),
        evidence_refs,
        record_id,
    );
    Some(Value::Object(object))
}

fn project_relationship(
    relationship: &Relationship,
    record_id: &str,
    source_ref: &str,
    target_ref: &str,
    evidence_refs: &[graph_core::EvidenceId],
) -> Option<Value> {
    let mut object = canonical_stix_relationship(relationship)
        .cloned()
        .unwrap_or_else(|| {
            let mut object = Map::new();
            object.insert("type".to_owned(), Value::String("relationship".to_owned()));
            object.insert(
                "id".to_owned(),
                Value::String(
                    native_relationship_stix_id(relationship)
                        .unwrap_or_else(|| stix_id("relationship", record_id)),
                ),
            );
            object.insert("spec_version".to_owned(), Value::String("2.1".to_owned()));
            object.insert(
                "created".to_owned(),
                property_or_default(relationship.property("created"), "1970-01-01T00:00:00.000Z"),
            );
            object.insert(
                "modified".to_owned(),
                property_or_default(
                    relationship.property("modified"),
                    "1970-01-01T00:00:00.000Z",
                ),
            );
            object.insert(
                "relationship_type".to_owned(),
                Value::String(relationship.rel_type().as_str().to_ascii_lowercase()),
            );
            object
        });
    object.get("type").and_then(Value::as_str)?;
    object.insert(
        "source_ref".to_owned(),
        Value::String(source_ref.to_owned()),
    );
    object.insert(
        "target_ref".to_owned(),
        Value::String(target_ref.to_owned()),
    );
    apply_native_metadata(
        &mut object,
        relationship.confidence().map(|value| value.value()),
        evidence_refs,
        record_id,
    );
    Some(Value::Object(object))
}

fn apply_native_metadata(
    object: &mut Map<String, Value>,
    confidence: Option<f64>,
    evidence_refs: &[graph_core::EvidenceId],
    record_id: &str,
) {
    if let Some(confidence) = confidence {
        let stix_confidence = (confidence * 100.0).round().clamp(0.0, 100.0) as u64;
        object.insert(
            "confidence".to_owned(),
            Value::Number(Number::from(stix_confidence)),
        );
    }
    let mut evidence_refs = evidence_refs
        .iter()
        .map(|id| Value::String(id.as_str().to_owned()))
        .collect::<Vec<_>>();
    evidence_refs.sort_by(|left, right| left.as_str().cmp(&right.as_str()));
    object.insert(
        "x_corrobore_evidence_refs".to_owned(),
        Value::Array(evidence_refs),
    );
    object.insert(
        "x_corrobore_source_record_id".to_owned(),
        Value::String(record_id.to_owned()),
    );
}

fn canonical_stix_relationship(relationship: &Relationship) -> Option<&Map<String, Value>> {
    match relationship.property("opencti.raw") {
        Some(PropertyValue::Json(Value::Object(raw))) => Some(raw),
        _ => None,
    }
}

fn native_stix_id(node: &Node, object_type: &str) -> Option<String> {
    ["stix_id", "external_id"]
        .into_iter()
        .filter_map(|key| match node.property(key) {
            Some(PropertyValue::String(value)) => Some(value),
            _ => None,
        })
        .find(|value| value.starts_with(&format!("{object_type}--")))
        .cloned()
}

fn native_relationship_stix_id(relationship: &Relationship) -> Option<String> {
    ["stix_id", "external_id"]
        .into_iter()
        .filter_map(|key| match relationship.property(key) {
            Some(PropertyValue::String(value)) => Some(value),
            _ => None,
        })
        .find(|value| value.starts_with("relationship--") || value.starts_with("sighting--"))
        .cloned()
}

fn property_or_default(value: Option<&PropertyValue>, default: &str) -> Value {
    match value {
        Some(value) => property_value_to_json(value),
        None => Value::String(default.to_owned()),
    }
}

fn property_value_to_json(value: &PropertyValue) -> Value {
    match value {
        PropertyValue::Null => Value::Null,
        PropertyValue::Bool(value) => Value::Bool(*value),
        PropertyValue::Integer(value) => Value::Number(Number::from(*value)),
        PropertyValue::Float(value) => Number::from_f64(*value)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        PropertyValue::String(value) => Value::String(value.clone()),
        PropertyValue::StringList(values) => {
            Value::Array(values.iter().cloned().map(Value::String).collect())
        }
        PropertyValue::IntegerList(values) => Value::Array(
            values
                .iter()
                .copied()
                .map(Number::from)
                .map(Value::Number)
                .collect(),
        ),
        PropertyValue::FloatList(values) => Value::Array(
            values
                .iter()
                .filter_map(|value| Number::from_f64(*value))
                .map(Value::Number)
                .collect(),
        ),
        PropertyValue::BoolList(values) => {
            Value::Array(values.iter().copied().map(Value::Bool).collect())
        }
        PropertyValue::Json(value) => value.clone(),
    }
}

fn retained_evidence(graph: &Graph, plan: &DeterministicExportPlan) -> Vec<Value> {
    plan.records()
        .iter()
        .flat_map(|record| record.evidence_refs())
        .map(|id| id.as_str().to_owned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter_map(|id| graph_core::EvidenceId::new(id).ok())
        .filter_map(|id| graph.evidence_by_id(&id))
        .filter_map(|record| {
            let mut value = serde_json::to_value(record).ok()?;
            value.as_object_mut()?.insert(
                "id".to_owned(),
                Value::String(record.id().as_str().to_owned()),
            );
            Some(value)
        })
        .collect()
}

fn export_diagnostic(finding: &graph_core::ValidationErrorRecord) -> Option<ExportDiagnostic> {
    let (target_kind, record_id) = match finding.target() {
        ValidationTarget::Node(id) => ("node", id.clone()),
        ValidationTarget::Relationship(id) => ("relationship", id.clone()),
        ValidationTarget::Claim(_)
        | ValidationTarget::ExportRecord(_)
        | ValidationTarget::Retrieval(_)
        | ValidationTarget::Source(_) => return None,
    };
    Some(ExportDiagnostic {
        code: finding.code().to_owned(),
        message: finding.message().to_owned(),
        target_kind,
        record_id,
    })
}

fn object_id(object: &Value) -> &str {
    object.get("id").and_then(Value::as_str).unwrap_or_default()
}

/// Export stix subset json.
pub fn export_stix_subset_json(
    graph: &Graph,
    plan: &DeterministicExportPlan,
) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(&export_stix_subset_bundle(graph, plan))
}

fn stix_node_type(node: &Node) -> Option<&'static str> {
    const TYPES: &[(&str, &str)] = &[
        ("AttackPattern", "attack-pattern"),
        ("Campaign", "campaign"),
        ("CourseOfAction", "course-of-action"),
        ("Grouping", "grouping"),
        ("Identity", "identity"),
        ("Incident", "incident"),
        ("Indicator", "indicator"),
        ("Infrastructure", "infrastructure"),
        ("IntrusionSet", "intrusion-set"),
        ("Location", "location"),
        ("Malware", "malware"),
        ("MalwareAnalysis", "malware-analysis"),
        ("Note", "note"),
        ("ObservedData", "observed-data"),
        ("Opinion", "opinion"),
        ("Report", "report"),
        ("ThreatActor", "threat-actor"),
        ("Tool", "tool"),
        ("Vulnerability", "vulnerability"),
        ("Artifact", "artifact"),
        ("AutonomousSystem", "autonomous-system"),
        ("Directory", "directory"),
        ("DomainName", "domain-name"),
        ("EmailAddress", "email-addr"),
        ("EmailMessage", "email-message"),
        ("File", "file"),
        ("Ipv4Addr", "ipv4-addr"),
        ("Ipv6Addr", "ipv6-addr"),
        ("MacAddress", "mac-addr"),
        ("Mutex", "mutex"),
        ("NetworkTraffic", "network-traffic"),
        ("Process", "process"),
        ("Software", "software"),
        ("Url", "url"),
        ("UserAccount", "user-account"),
        ("X509Certificate", "x509-certificate"),
    ];
    TYPES
        .iter()
        .find_map(|(label, object_type)| node.has_label(label).then_some(*object_type))
}

fn canonical_stix_object(node: &Node) -> Option<&serde_json::Map<String, serde_json::Value>> {
    match node.property("opencti.raw") {
        Some(PropertyValue::Json(Value::Object(raw))) => Some(raw),
        _ => None,
    }
}

fn object_type_supports_name(object_type: &str) -> bool {
    !matches!(
        object_type,
        "artifact"
            | "autonomous-system"
            | "directory"
            | "domain-name"
            | "email-addr"
            | "email-message"
            | "file"
            | "ipv4-addr"
            | "ipv6-addr"
            | "mac-addr"
            | "mutex"
            | "network-traffic"
            | "process"
            | "user-account"
            | "x509-certificate"
    )
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
        Confidence, EvidenceId, EvidenceInput, ExportMetadata, ExportMode, ExportProfile, Graph,
        NodeInput, RecordStatus, RelationshipInput, TransactionId, build_deterministic_export_plan,
    };
    use serde_json::Value;

    use super::{
        export_stix_subset_bundle, export_stix_subset_json, mode_label, profile_label,
        stable_hex_hash, stix_id, stix_node_type,
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

    fn retained_evidence(graph: &mut Graph, value: &str) -> EvidenceId {
        let id = EvidenceId::new(value).expect("evidence id should be valid");
        graph
            .create_evidence(EvidenceInput::new(
                id.clone(),
                "synthetic-source",
                "synthetic evidence",
            ))
            .expect("evidence should be retained");
        id
    }

    fn ready_confidence() -> Confidence {
        Confidence::new(0.9).expect("confidence should be valid")
    }

    #[test]
    fn stix_export_json_is_deterministic_for_same_inputs() {
        let mut graph = Graph::new();
        let source_evidence = retained_evidence(&mut graph, "evidence--source");
        let target_evidence = retained_evidence(&mut graph, "evidence--target");
        let relationship_evidence = retained_evidence(&mut graph, "evidence--relationship");
        let source = graph
            .create_node(
                NodeInput::new(["ThreatActor"])
                    .with_status(RecordStatus::Exportable)
                    .with_confidence(ready_confidence())
                    .with_evidence_ref(source_evidence),
            )
            .expect("source node creation should succeed");
        let target = graph
            .create_node(
                NodeInput::new(["Indicator"])
                    .with_status(RecordStatus::Exportable)
                    .with_confidence(ready_confidence())
                    .with_evidence_ref(target_evidence),
            )
            .expect("target node creation should succeed");

        graph
            .create_relationship(
                RelationshipInput::new(source, "indicates", target)
                    .expect("relationship input should be valid")
                    .with_status(RecordStatus::Exportable)
                    .with_confidence(ready_confidence())
                    .with_evidence_ref(relationship_evidence),
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
    fn stix_node_type_maps_supported_labels_and_rejects_unknown_labels() {
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

        assert_eq!(stix_node_type(&threat_actor), Some("threat-actor"));
        assert_eq!(stix_node_type(&malware), Some("malware"));
        assert_eq!(stix_node_type(&location), Some("location"));
        assert_eq!(stix_node_type(&fallback), None);
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
        assert!(indicator_id.starts_with("indicator--"));
    }

    #[test]
    fn stix_export_bundle_sorts_objects_by_id() {
        let mut graph = Graph::new();
        let first_evidence = retained_evidence(&mut graph, "evidence--first");
        let second_evidence = retained_evidence(&mut graph, "evidence--second");
        let relationship_evidence = retained_evidence(&mut graph, "evidence--sort-relationship");
        let first = graph
            .create_node(
                NodeInput::new(["Indicator"])
                    .with_status(RecordStatus::Exportable)
                    .with_confidence(ready_confidence())
                    .with_evidence_ref(first_evidence),
            )
            .expect("first node creation should succeed");
        let second = graph
            .create_node(
                NodeInput::new(["Indicator"])
                    .with_status(RecordStatus::Exportable)
                    .with_confidence(ready_confidence())
                    .with_evidence_ref(second_evidence),
            )
            .expect("second node creation should succeed");
        graph
            .create_relationship(
                RelationshipInput::new(second, "related-to", first)
                    .expect("relationship input should be valid")
                    .with_status(RecordStatus::Exportable)
                    .with_confidence(ready_confidence())
                    .with_evidence_ref(relationship_evidence),
            )
            .expect("relationship creation should succeed");

        let plan = build_deterministic_export_plan(&graph, strict_metadata(), &[])
            .expect("plan should succeed");
        let bundle = export_stix_subset_bundle(&graph, &plan);

        let ids = bundle
            .objects
            .iter()
            .filter_map(|object| object.get("id").and_then(Value::as_str).map(str::to_owned))
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

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
use graph_core::{
    EvidenceId, ExportMetadata, ExportMode, ExportProfile, Graph, GraphError, NodeInput,
    RecordStatus, RelationshipInput, TransactionId, ValidationErrorRecord, ValidationErrorSeverity,
    ValidationTarget, build_deterministic_export_plan,
};

fn metadata(mode: ExportMode) -> ExportMetadata {
    ExportMetadata::new(
        "snapshot--epic-0010",
        TransactionId::new("transaction--epic-0010").expect("transaction ID should be valid"),
        "stix-mvp-v1",
        // These tests exercise profile-neutral planning mechanics. CTI/STIX
        // readiness and selection have their own issue-119 contract suite.
        ExportProfile::FimiJsonMvp,
        mode,
        None,
    )
    .expect("export metadata should be valid")
}

//
// Verify strict mode enforces export readiness and fails when a visible record
// is not exportable.
#[test]
fn strict_mode_rejects_non_exportable_records() {
    let mut graph = Graph::new();
    graph
        .create_node(NodeInput::new(["Indicator"]).with_status(RecordStatus::Candidate))
        .expect("node creation should succeed");

    let error = build_deterministic_export_plan(&graph, metadata(ExportMode::Strict), &[])
        .expect_err("strict mode should reject non-exportable records");

    assert!(matches!(error, GraphError::ExportStrictModeRejected(_)));
}

//
// Verify permissive mode skips non-exportable records and returns warnings
// instead of failing the export plan.
#[test]
fn permissive_mode_skips_non_exportable_records_and_returns_warnings() {
    let mut graph = Graph::new();
    graph
        .create_node(NodeInput::new(["Indicator"]).with_status(RecordStatus::Candidate))
        .expect("candidate node creation should succeed");
    let exportable_node = graph
        .create_node(NodeInput::new(["Indicator"]).with_status(RecordStatus::Exportable))
        .expect("exportable node creation should succeed");

    let plan = build_deterministic_export_plan(&graph, metadata(ExportMode::Permissive), &[])
        .expect("permissive mode should produce a plan");

    let exported_ids: Vec<&str> = plan
        .records()
        .iter()
        .map(|record| record.record_id())
        .collect();

    assert_eq!(plan.records().len(), 1);
    assert_eq!(exported_ids, vec![exportable_node.as_str()]);
    assert_eq!(plan.warnings().len(), 1);
    assert_eq!(plan.warnings()[0].code(), "EXPORT_STATUS_NOT_READY");
}

//
// Verify permissive mode skips records with blocking validation findings and
// keeps those findings in warning output for downstream review.
#[test]
fn permissive_mode_skips_records_with_error_severity_validation_findings() {
    let mut graph = Graph::new();
    let node_id = graph
        .create_node(NodeInput::new(["Indicator"]).with_status(RecordStatus::Exportable))
        .expect("node creation should succeed");
    let finding = ValidationErrorRecord::new(
        "EXPORT_MISSING_EVIDENCE",
        ValidationErrorSeverity::Error,
        "exportable record must include at least one evidence reference",
        ValidationTarget::node(node_id.as_str()),
    );

    let plan = build_deterministic_export_plan(
        &graph,
        metadata(ExportMode::Permissive),
        std::slice::from_ref(&finding),
    )
    .expect("permissive mode should return a partial plan");

    assert!(plan.records().is_empty());
    assert_eq!(plan.warnings().len(), 1);
    assert_eq!(&plan.warnings()[0], &finding);
}

//
// Verify deterministic reproducibility: same graph snapshot, same metadata,
// and same findings must produce the same plan fingerprint and record ordering.
#[test]
fn same_inputs_produce_same_export_plan_fingerprint() {
    let mut graph = Graph::new();
    graph
        .create_node(NodeInput::new(["Indicator"]).with_status(RecordStatus::Exportable))
        .expect("first node creation should succeed");
    graph
        .create_node(NodeInput::new(["ThreatActor"]).with_status(RecordStatus::Exportable))
        .expect("second node creation should succeed");

    let first = build_deterministic_export_plan(&graph, metadata(ExportMode::Strict), &[])
        .expect("first plan should succeed");
    let second = build_deterministic_export_plan(&graph, metadata(ExportMode::Strict), &[])
        .expect("second plan should succeed");

    assert_eq!(first.records(), second.records());
    assert_eq!(
        first.determinism_fingerprint(),
        second.determinism_fingerprint()
    );
}

//
// Verify deterministic export record IDs are generated from kind and graph
// record ID to keep cross-run IDs stable.
#[test]
fn export_record_ids_are_deterministic_for_nodes_and_relationships() {
    let mut graph = Graph::new();
    let source = graph
        .create_node(NodeInput::new(["ThreatActor"]).with_status(RecordStatus::Exportable))
        .expect("source node creation should succeed");
    let target = graph
        .create_node(NodeInput::new(["Indicator"]).with_status(RecordStatus::Exportable))
        .expect("target node creation should succeed");
    let relationship = graph
        .create_relationship(
            RelationshipInput::new(source.clone(), "indicates", target)
                .expect("relationship input should be valid")
                .with_status(RecordStatus::Exportable),
        )
        .expect("relationship creation should succeed");

    let plan = build_deterministic_export_plan(&graph, metadata(ExportMode::Strict), &[])
        .expect("strict mode should produce a plan");

    let node_record = plan
        .records()
        .iter()
        .find(|record| record.record_id() == source.as_str())
        .expect("node export record should exist");
    let relationship_record = plan
        .records()
        .iter()
        .find(|record| record.record_id() == relationship.as_str())
        .expect("relationship export record should exist");

    assert_eq!(
        node_record.export_record_id(),
        format!("export-record--node--{}", source.as_str())
    );
    assert_eq!(
        relationship_record.export_record_id(),
        format!("export-record--relationship--{}", relationship.as_str())
    );
}

//
// Verify relationship export records carry evidence references so deterministic
// exporters can preserve provenance links in payload mappings.
#[test]
fn relationship_export_record_preserves_evidence_references() {
    let mut graph = Graph::new();
    let source = graph
        .create_node(NodeInput::new(["ThreatActor"]).with_status(RecordStatus::Exportable))
        .expect("source node creation should succeed");
    let target = graph
        .create_node(NodeInput::new(["Indicator"]).with_status(RecordStatus::Exportable))
        .expect("target node creation should succeed");
    let evidence_id = EvidenceId::new("evidence--epic-0010").expect("evidence ID should be valid");
    let relationship = graph
        .create_relationship(
            RelationshipInput::new(source, "indicates", target)
                .expect("relationship input should be valid")
                .with_status(RecordStatus::Exportable)
                .with_evidence_ref(evidence_id.clone()),
        )
        .expect("relationship creation should succeed");

    let plan = build_deterministic_export_plan(&graph, metadata(ExportMode::Strict), &[])
        .expect("strict mode should produce a plan");
    let relationship_record = plan
        .records()
        .iter()
        .find(|record| record.record_id() == relationship.as_str())
        .expect("relationship export record should exist");

    assert_eq!(relationship_record.evidence_refs(), &[evidence_id]);
}

//
// Verify records already marked as Exported remain export-ready for deterministic
// planning in strict mode.
#[test]
fn strict_mode_accepts_exported_status_records() {
    let mut graph = Graph::new();
    graph
        .create_node(NodeInput::new(["Indicator"]).with_status(RecordStatus::Exported))
        .expect("exported node creation should succeed");

    let plan = build_deterministic_export_plan(&graph, metadata(ExportMode::Strict), &[])
        .expect("strict mode should accept exported status records");

    assert_eq!(plan.records().len(), 1);
    assert!(plan.warnings().is_empty());
}

//
// Verify strict mode rejects export when a relationship has an error-severity
// validation finding, even if its status is exportable.
#[test]
fn strict_mode_rejects_relationship_with_blocking_validation_finding() {
    let mut graph = Graph::new();
    let source = graph
        .create_node(NodeInput::new(["ThreatActor"]).with_status(RecordStatus::Exportable))
        .expect("source node creation should succeed");
    let target = graph
        .create_node(NodeInput::new(["Indicator"]).with_status(RecordStatus::Exportable))
        .expect("target node creation should succeed");
    let relationship = graph
        .create_relationship(
            RelationshipInput::new(source, "indicates", target)
                .expect("relationship input should be valid")
                .with_status(RecordStatus::Exportable),
        )
        .expect("relationship creation should succeed");

    let finding = ValidationErrorRecord::new(
        "EXPORT_RELATIONSHIP_BLOCKED",
        ValidationErrorSeverity::Error,
        "relationship is blocked for strict export",
        ValidationTarget::relationship(relationship.as_str()),
    );

    let error = build_deterministic_export_plan(
        &graph,
        metadata(ExportMode::Strict),
        std::slice::from_ref(&finding),
    )
    .expect_err("strict mode should reject relationship blocking findings");

    assert!(
        matches!(error, GraphError::ExportStrictModeRejected(message) if message.contains("relationship is blocked for strict export"))
    );
}

//
// Verify permissive mode emits warning for non-export-ready relationships
// without rejecting the plan.
#[test]
fn permissive_mode_warns_for_non_exportable_relationships() {
    let mut graph = Graph::new();
    let source = graph
        .create_node(NodeInput::new(["ThreatActor"]).with_status(RecordStatus::Exportable))
        .expect("source node creation should succeed");
    let target = graph
        .create_node(NodeInput::new(["Indicator"]).with_status(RecordStatus::Exportable))
        .expect("target node creation should succeed");
    graph
        .create_relationship(
            RelationshipInput::new(source.clone(), "indicates", target)
                .expect("relationship input should be valid")
                .with_status(RecordStatus::Candidate),
        )
        .expect("relationship creation should succeed");

    let plan = build_deterministic_export_plan(&graph, metadata(ExportMode::Permissive), &[])
        .expect("permissive mode should return a plan");

    assert!(
        plan.records()
            .iter()
            .any(|record| record.record_id() == source.as_str())
    );
    assert_eq!(plan.warnings().len(), 1);
    assert_eq!(plan.warnings()[0].code(), "EXPORT_STATUS_NOT_READY");
}

//
// Verify findings targeting claim/export-record scopes are ignored by graph
// record blocking collection and do not remove exportable node records.
#[test]
fn non_graph_target_findings_do_not_block_export_records() {
    let mut graph = Graph::new();
    let node_id = graph
        .create_node(NodeInput::new(["Indicator"]).with_status(RecordStatus::Exportable))
        .expect("node creation should succeed");

    let claim_finding = ValidationErrorRecord::new(
        "CLAIM_SCOPE_ERROR",
        ValidationErrorSeverity::Error,
        "claim-scope issue should not block node export",
        ValidationTarget::claim("claim--001"),
    );
    let export_record_finding = ValidationErrorRecord::new(
        "EXPORT_RECORD_SCOPE_ERROR",
        ValidationErrorSeverity::Error,
        "export-record-scope issue should not block node export",
        ValidationTarget::export_record("export-record--001"),
    );

    let plan = build_deterministic_export_plan(
        &graph,
        metadata(ExportMode::Strict),
        &[claim_finding, export_record_finding],
    )
    .expect("strict mode should ignore non-graph-target findings");

    assert_eq!(plan.records().len(), 1);
    assert_eq!(plan.records()[0].record_id(), node_id.as_str());
}

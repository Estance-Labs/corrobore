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
use std::collections::HashMap;

use crate::{
    EvidenceId, ExportMetadata, ExportMode, Graph, GraphError, RecordStatus, ValidationErrorRecord,
    ValidationErrorSeverity, ValidationTarget,
};

/// Deterministic export record category.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportRecordKind {
    /// Node.
    Node,
    /// Relationship.
    Relationship,
}

/// Deterministic export record reference emitted by export planning.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportRecord {
    record_id: String,
    export_record_id: String,
    kind: ExportRecordKind,
    evidence_refs: Vec<EvidenceId>,
}

impl ExportRecord {
    /// Record id.
    pub fn record_id(&self) -> &str {
        self.record_id.as_str()
    }

    /// Export record id.
    pub fn export_record_id(&self) -> &str {
        self.export_record_id.as_str()
    }

    /// Kind.
    pub fn kind(&self) -> ExportRecordKind {
        self.kind
    }

    /// Evidence refs.
    pub fn evidence_refs(&self) -> &[EvidenceId] {
        self.evidence_refs.as_slice()
    }
}

/// Deterministic export plan assembled from graph snapshot inputs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeterministicExportPlan {
    metadata: ExportMetadata,
    records: Vec<ExportRecord>,
    warnings: Vec<ValidationErrorRecord>,
}

impl DeterministicExportPlan {
    /// Metadata.
    pub fn metadata(&self) -> &ExportMetadata {
        &self.metadata
    }

    /// Records.
    pub fn records(&self) -> &[ExportRecord] {
        self.records.as_slice()
    }

    /// Warnings.
    pub fn warnings(&self) -> &[ValidationErrorRecord] {
        self.warnings.as_slice()
    }

    ///
    /// derive a deterministic fingerprint for reproducibility checks.
    pub fn determinism_fingerprint(&self) -> String {
        let record_fingerprint = self
            .records
            .iter()
            .map(|record| format!("{}:{}", record.kind.as_str(), record.record_id))
            .collect::<Vec<String>>()
            .join("|");
        format!(
            "{}||{}",
            self.metadata.determinism_key(),
            record_fingerprint
        )
    }
}

///
/// Build a deterministic export plan from graph records, export metadata, and
/// validation findings.
pub fn build_deterministic_export_plan(
    graph: &Graph,
    metadata: ExportMetadata,
    findings: &[ValidationErrorRecord],
) -> Result<DeterministicExportPlan, GraphError> {
    let mode = metadata.mode();
    let blocking_by_record_id = collect_blocking_findings(findings);

    let mut records = Vec::new();
    let mut warnings = Vec::new();
    let mut strict_reasons = Vec::new();

    for node in graph.list_nodes()? {
        let node_id = node.id().as_str();
        let readiness = export_readiness(node.status());
        let has_blocking_finding = blocking_by_record_id.contains_key(node_id);

        if readiness && !has_blocking_finding {
            records.push(ExportRecord {
                record_id: node_id.to_owned(),
                export_record_id: deterministic_export_record_id(ExportRecordKind::Node, node_id),
                kind: ExportRecordKind::Node,
                evidence_refs: node.evidence_refs().to_vec(),
            });
            continue;
        }

        if !readiness {
            let warning = ValidationErrorRecord::new(
                "EXPORT_STATUS_NOT_READY",
                ValidationErrorSeverity::Warning,
                format!(
                    "record {} is not export-ready for mode {}",
                    node_id,
                    mode_label(mode)
                ),
                ValidationTarget::node(node_id),
            );
            match mode {
                ExportMode::Strict => strict_reasons.push(warning.message().to_owned()),
                ExportMode::Permissive => warnings.push(warning),
            }
        }

        if let Some(blocking_finding) = blocking_by_record_id.get(node_id) {
            match mode {
                ExportMode::Strict => strict_reasons.push(blocking_finding.message().to_owned()),
                ExportMode::Permissive => warnings.push((*blocking_finding).clone()),
            }
        }
    }

    for relationship in graph.list_relationships()? {
        let relationship_id = relationship.id().as_str();
        let readiness = export_readiness(relationship.status());
        let has_blocking_finding = blocking_by_record_id.contains_key(relationship_id);

        if readiness && !has_blocking_finding {
            records.push(ExportRecord {
                record_id: relationship_id.to_owned(),
                export_record_id: deterministic_export_record_id(
                    ExportRecordKind::Relationship,
                    relationship_id,
                ),
                kind: ExportRecordKind::Relationship,
                evidence_refs: relationship.evidence_refs().to_vec(),
            });
            continue;
        }

        if !readiness {
            let warning = ValidationErrorRecord::new(
                "EXPORT_STATUS_NOT_READY",
                ValidationErrorSeverity::Warning,
                format!(
                    "record {} is not export-ready for mode {}",
                    relationship_id,
                    mode_label(mode)
                ),
                ValidationTarget::relationship(relationship_id),
            );
            match mode {
                ExportMode::Strict => strict_reasons.push(warning.message().to_owned()),
                ExportMode::Permissive => warnings.push(warning),
            }
        }

        if let Some(blocking_finding) = blocking_by_record_id.get(relationship_id) {
            match mode {
                ExportMode::Strict => strict_reasons.push(blocking_finding.message().to_owned()),
                ExportMode::Permissive => warnings.push((*blocking_finding).clone()),
            }
        }
    }

    if mode == ExportMode::Strict && !strict_reasons.is_empty() {
        return Err(GraphError::ExportStrictModeRejected(
            strict_reasons.join("; "),
        ));
    }

    Ok(DeterministicExportPlan {
        metadata,
        records,
        warnings,
    })
}

impl ExportRecordKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Node => "node",
            Self::Relationship => "relationship",
        }
    }
}

fn export_readiness(status: RecordStatus) -> bool {
    matches!(status, RecordStatus::Exportable | RecordStatus::Exported)
}

fn mode_label(mode: ExportMode) -> &'static str {
    match mode {
        ExportMode::Strict => "strict",
        ExportMode::Permissive => "permissive",
    }
}

fn collect_blocking_findings(
    findings: &[ValidationErrorRecord],
) -> HashMap<&str, &ValidationErrorRecord> {
    let mut by_record_id = HashMap::new();

    for finding in findings {
        if finding.severity() != ValidationErrorSeverity::Error {
            continue;
        }

        match finding.target() {
            ValidationTarget::Node(record_id) | ValidationTarget::Relationship(record_id) => {
                by_record_id.insert(record_id.as_str(), finding);
            }
            ValidationTarget::Claim(_)
            | ValidationTarget::ExportRecord(_)
            | ValidationTarget::Retrieval(_) => {}
        }
    }

    by_record_id
}

fn deterministic_export_record_id(kind: ExportRecordKind, record_id: &str) -> String {
    format!("export-record--{}--{}", kind.as_str(), record_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ExportProfile, NodeInput, RelationshipInput, TransactionId};

    fn metadata(mode: ExportMode) -> ExportMetadata {
        ExportMetadata::new(
            "snapshot--unit-export-plan",
            TransactionId::new("transaction--unit-export-plan")
                .expect("transaction ID should be valid"),
            "stix-mvp-v1",
            ExportProfile::StixMvp,
            mode,
            None,
        )
        .expect("metadata should be valid")
    }

    #[test]
    fn helper_functions_cover_readiness_mode_and_blocking_filtering() {
        assert!(export_readiness(RecordStatus::Exportable));
        assert!(export_readiness(RecordStatus::Exported));
        assert!(!export_readiness(RecordStatus::Candidate));

        assert_eq!(mode_label(ExportMode::Strict), "strict");
        assert_eq!(mode_label(ExportMode::Permissive), "permissive");

        let warning_node = ValidationErrorRecord::new(
            "WARN_NODE",
            ValidationErrorSeverity::Warning,
            "warning should be ignored by blocking collector",
            ValidationTarget::node("node--warn"),
        );
        let error_node = ValidationErrorRecord::new(
            "ERR_NODE",
            ValidationErrorSeverity::Error,
            "node should block",
            ValidationTarget::node("node--blocked"),
        );
        let error_relationship = ValidationErrorRecord::new(
            "ERR_REL",
            ValidationErrorSeverity::Error,
            "relationship should block",
            ValidationTarget::relationship("relationship--blocked"),
        );
        let error_claim = ValidationErrorRecord::new(
            "ERR_CLAIM",
            ValidationErrorSeverity::Error,
            "claim scope should be ignored",
            ValidationTarget::claim("claim--ignored"),
        );

        let findings = vec![warning_node, error_node, error_relationship, error_claim];
        let blocking = collect_blocking_findings(&findings);

        assert_eq!(blocking.len(), 2);
        assert_eq!(
            blocking
                .get("node--blocked")
                .expect("node blocking finding should be present")
                .code(),
            "ERR_NODE"
        );
        assert_eq!(
            blocking
                .get("relationship--blocked")
                .expect("relationship blocking finding should be present")
                .code(),
            "ERR_REL"
        );
    }

    #[test]
    fn strict_mode_accumulates_not_ready_and_blocking_reason_for_same_record() {
        let mut graph = Graph::new();
        let node_id = graph
            .create_node(NodeInput::new(["Indicator"]).with_status(RecordStatus::Candidate))
            .expect("node creation should succeed");

        let blocking = ValidationErrorRecord::new(
            "BLOCKED_NODE",
            ValidationErrorSeverity::Error,
            "node has explicit blocking finding",
            ValidationTarget::node(node_id.as_str()),
        );

        let error = build_deterministic_export_plan(
            &graph,
            metadata(ExportMode::Strict),
            std::slice::from_ref(&blocking),
        )
        .expect_err("strict mode should reject combined readiness and finding failures");

        assert!(
            matches!(error, GraphError::ExportStrictModeRejected(message)
 if message.contains("not export-ready for mode strict")
 && message.contains("node has explicit blocking finding"))
        );
    }

    #[test]
    fn export_plan_accessors_and_fingerprint_cover_node_and_relationship_records() {
        let mut graph = Graph::new();
        let source = graph
            .create_node(NodeInput::new(["ThreatActor"]).with_status(RecordStatus::Exportable))
            .expect("source node should be created");
        let target = graph
            .create_node(NodeInput::new(["Indicator"]).with_status(RecordStatus::Exportable))
            .expect("target node should be created");
        let relationship = graph
            .create_relationship(
                RelationshipInput::new(source.clone(), "INDICATES", target)
                    .expect("relationship input should be valid")
                    .with_status(RecordStatus::Exported),
            )
            .expect("relationship should be created");

        let plan = build_deterministic_export_plan(&graph, metadata(ExportMode::Strict), &[])
            .expect("plan should be built");

        assert_eq!(plan.metadata().mode(), ExportMode::Strict);
        assert!(plan.warnings().is_empty());

        let node_record = plan
            .records()
            .iter()
            .find(|record| record.record_id() == source.as_str())
            .expect("node export record should exist");
        assert_eq!(node_record.kind(), ExportRecordKind::Node);
        assert!(node_record.evidence_refs().is_empty());

        let relationship_record = plan
            .records()
            .iter()
            .find(|record| record.record_id() == relationship.as_str())
            .expect("relationship export record should exist");
        assert_eq!(relationship_record.kind(), ExportRecordKind::Relationship);

        let fingerprint = plan.determinism_fingerprint();
        assert!(fingerprint.contains("node:"));
        assert!(fingerprint.contains("relationship:"));
    }
}

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
use std::collections::{HashMap, HashSet};

use crate::{
    EvidenceId, ExportMetadata, ExportMode, ExportProfile, Graph, GraphError, Node, PropertyValue,
    RecordStatus, Relationship, ValidationErrorRecord, ValidationErrorSeverity, ValidationTarget,
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

/// Operator-selected controls for deterministic export planning.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExportPlanOptions {
    force_validation: bool,
}

impl ExportPlanOptions {
    /// Chooses whether overridable semantic validation findings are retained
    /// as diagnostics instead of excluding an otherwise eligible record.
    pub fn with_force_validation(mut self, force_validation: bool) -> Self {
        self.force_validation = force_validation;
        self
    }

    /// Returns whether semantic validation findings are forced into diagnostic
    /// output rather than enforced as record exclusions.
    pub fn force_validation(self) -> bool {
        self.force_validation
    }
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
    build_deterministic_export_plan_with_options(
        graph,
        metadata,
        findings,
        ExportPlanOptions::default(),
    )
}

/// Builds a deterministic export plan with explicit operator controls.
///
/// Forced validation must preserve every bypassed finding in deterministic
/// diagnostics while continuing to enforce lifecycle and structural safety.
/// The implementation separates those categories before record selection so a
/// force request cannot accidentally become a blanket export bypass.
pub fn build_deterministic_export_plan_with_options(
    graph: &Graph,
    metadata: ExportMetadata,
    findings: &[ValidationErrorRecord],
    options: ExportPlanOptions,
) -> Result<DeterministicExportPlan, GraphError> {
    let mode = metadata.mode();
    let blocking_by_record_id = collect_blocking_findings(findings);

    let mut records = Vec::new();
    let mut warnings = Vec::new();
    let mut strict_reasons = Vec::new();

    // Profile selection is resolved before readiness. This ordering is the
    // contract that prevents unrelated domain records from becoming strict
    // export failures for a profile they do not belong to.
    let nodes = graph.list_nodes()?;
    let selected_node_ids = nodes
        .iter()
        .filter(|node| node_eligible_for_export_profile(metadata.profile(), node))
        .map(|node| node.id().as_str().to_owned())
        .collect::<HashSet<_>>();
    if metadata.profile() == &ExportProfile::StixMvp && mode == ExportMode::Permissive {
        warnings.extend(nodes.iter().filter_map(|node| {
            let family = canonical_family(node.property("opencti.family"))?;
            (!selected_node_ids.contains(node.id().as_str())).then(|| {
                ValidationErrorRecord::new(
                    "CTI_PROFILE_UNSUPPORTED_RECORD",
                    ValidationErrorSeverity::Warning,
                    format!(
                        "OpenCTI node {} with family {family} is outside the supported STIX export profile",
                        node.id().as_str()
                    ),
                    ValidationTarget::node(node.id().as_str()),
                )
            })
        }));
    }

    // Keep compatibility diagnostics from older providers visible, scoped to
    // selected records. They cannot replace the non-overridable claim gate.
    warnings.extend(findings.iter().filter(|finding| {
        finding.code() == "EXPORT_LEGACY_CONFIDENCE_DIAGNOSTIC"
            && finding.severity() == ValidationErrorSeverity::Warning
            && matches!(finding.target(), ValidationTarget::Node(id) if selected_node_ids.contains(id))
    }).cloned());

    let mut exported_node_ids = HashSet::new();

    for node in nodes {
        if !selected_node_ids.contains(node.id().as_str()) {
            continue;
        }
        let node_id = node.id().as_str();
        let readiness_findings = profile_readiness_findings(
            graph,
            metadata.profile(),
            mode,
            node_id,
            node.status(),
            node.evidence_refs(),
            ValidationTarget::node(node_id),
        );
        let mut record_findings = readiness_findings.enforced;
        collect_validation_findings(
            options,
            readiness_findings.overridable,
            &mut record_findings,
            &mut warnings,
        );
        record_findings.extend(canonical_node_identity_findings(metadata.profile(), &node));
        if let Some(blocking_findings) = blocking_by_record_id.get(node_id) {
            collect_validation_findings(
                options,
                blocking_findings.iter().map(|finding| (*finding).clone()),
                &mut record_findings,
                &mut warnings,
            );
        }

        if record_findings.is_empty() {
            exported_node_ids.insert(node_id.to_owned());
            records.push(ExportRecord {
                record_id: node_id.to_owned(),
                export_record_id: deterministic_export_record_id(ExportRecordKind::Node, node_id),
                kind: ExportRecordKind::Node,
                evidence_refs: node.evidence_refs().to_vec(),
            });
            continue;
        }
        collect_mode_findings(mode, record_findings, &mut warnings, &mut strict_reasons);
    }

    let relationships = graph.list_relationships()?;
    if metadata.profile() == &ExportProfile::StixMvp && mode == ExportMode::Permissive {
        warnings.extend(relationships.iter().filter_map(|relationship| {
            let family = canonical_family(relationship.property("opencti.family"))?;
            (!relationship_selected_for_profile(
                metadata.profile(),
                relationship,
                &selected_node_ids,
            ))
            .then(|| {
                ValidationErrorRecord::new(
                    "CTI_PROFILE_UNSUPPORTED_RECORD",
                    ValidationErrorSeverity::Warning,
                    format!(
                        "OpenCTI relationship {} with family {family} is outside the supported STIX export profile",
                        relationship.id().as_str()
                    ),
                    ValidationTarget::relationship(relationship.id().as_str()),
                )
            })
        }));
    }

    for relationship in relationships {
        if !relationship_selected_for_profile(metadata.profile(), &relationship, &selected_node_ids)
        {
            continue;
        }
        let relationship_id = relationship.id().as_str();
        let readiness_findings = profile_readiness_findings(
            graph,
            metadata.profile(),
            mode,
            relationship_id,
            relationship.status(),
            relationship.evidence_refs(),
            ValidationTarget::relationship(relationship_id),
        );
        let mut record_findings = readiness_findings.enforced;
        collect_validation_findings(
            options,
            readiness_findings.overridable,
            &mut record_findings,
            &mut warnings,
        );
        record_findings.extend(canonical_relationship_identity_findings(
            metadata.profile(),
            &relationship,
        ));
        if metadata.profile() == &ExportProfile::StixMvp {
            for (endpoint_role, endpoint_id) in [
                ("source", relationship.source().as_str()),
                ("target", relationship.target().as_str()),
            ] {
                if !exported_node_ids.contains(endpoint_id) {
                    record_findings.push(ValidationErrorRecord::new(
                        "CTI_ENDPOINT_EXCLUDED",
                        ValidationErrorSeverity::Error,
                        format!(
                            "CTI relationship {relationship_id} {endpoint_role} endpoint {endpoint_id} is not exportable"
                        ),
                        ValidationTarget::relationship(relationship_id),
                    ));
                }
            }
        }
        if let Some(blocking_findings) = blocking_by_record_id.get(relationship_id) {
            collect_validation_findings(
                options,
                blocking_findings.iter().map(|finding| (*finding).clone()),
                &mut record_findings,
                &mut warnings,
            );
        }

        if record_findings.is_empty() {
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
        collect_mode_findings(mode, record_findings, &mut warnings, &mut strict_reasons);
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

/// Returns whether a graph node belongs to the requested deterministic export
/// profile before lifecycle, confidence, evidence, or provider readiness is
/// evaluated.
pub fn node_eligible_for_export_profile(profile: &ExportProfile, node: &Node) -> bool {
    match profile {
        ExportProfile::FimiJsonMvp => true,
        ExportProfile::StixMvp => {
            canonical_family(node.property("opencti.family")).is_some_and(is_stix_object_family)
                || graph_native_stix_type(node).is_some()
        }
    }
}

fn relationship_selected_for_profile(
    profile: &ExportProfile,
    relationship: &Relationship,
    selected_node_ids: &HashSet<String>,
) -> bool {
    match profile {
        ExportProfile::FimiJsonMvp => true,
        ExportProfile::StixMvp => {
            canonical_family(relationship.property("opencti.family"))
                .is_some_and(is_stix_relationship_family)
                || (selected_node_ids.contains(relationship.source().as_str())
                    && selected_node_ids.contains(relationship.target().as_str()))
        }
    }
}

fn canonical_family(value: Option<&PropertyValue>) -> Option<&str> {
    match value {
        Some(PropertyValue::String(value)) => Some(value.as_str()),
        _ => None,
    }
}

fn is_stix_object_family(family: &str) -> bool {
    matches!(
        family,
        "stix_domain_object" | "stix_cyber_observable" | "stix_meta_object"
    )
}

fn is_stix_relationship_family(family: &str) -> bool {
    matches!(
        family,
        "stix_core_relationship" | "stix_ref_relationship" | "stix_sighting_relationship"
    )
}

fn graph_native_stix_type(node: &Node) -> Option<&'static str> {
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

fn canonical_node_identity_findings(
    profile: &ExportProfile,
    node: &Node,
) -> Vec<ValidationErrorRecord> {
    if profile != &ExportProfile::StixMvp
        || !canonical_family(node.property("opencti.family")).is_some_and(is_stix_object_family)
    {
        return Vec::new();
    }
    canonical_identity_findings(
        node.property("opencti.raw"),
        ValidationTarget::node(node.id().as_str()),
        node.id().as_str(),
    )
}

fn canonical_relationship_identity_findings(
    profile: &ExportProfile,
    relationship: &Relationship,
) -> Vec<ValidationErrorRecord> {
    if profile != &ExportProfile::StixMvp
        || !canonical_family(relationship.property("opencti.family"))
            .is_some_and(is_stix_relationship_family)
    {
        return Vec::new();
    }
    canonical_identity_findings(
        relationship.property("opencti.raw"),
        ValidationTarget::relationship(relationship.id().as_str()),
        relationship.id().as_str(),
    )
}

fn canonical_identity_findings(
    raw: Option<&PropertyValue>,
    target: ValidationTarget,
    record_id: &str,
) -> Vec<ValidationErrorRecord> {
    let Some(PropertyValue::Json(serde_json::Value::Object(raw))) = raw else {
        return vec![ValidationErrorRecord::new(
            "STIX_RAW_REQUIRED",
            ValidationErrorSeverity::Error,
            format!("imported CTI record {record_id} requires canonical opencti.raw content"),
            target,
        )];
    };
    let object_type = raw.get("type").and_then(serde_json::Value::as_str);
    let stix_id = raw.get("id").and_then(serde_json::Value::as_str);
    let mut findings = Vec::new();
    if object_type.is_none_or(str::is_empty) {
        findings.push(ValidationErrorRecord::new(
            "STIX_TYPE_REQUIRED",
            ValidationErrorSeverity::Error,
            format!("imported CTI record {record_id} requires its original STIX type"),
            target.clone(),
        ));
    }
    if stix_id.is_none_or(str::is_empty) {
        findings.push(ValidationErrorRecord::new(
            "STIX_ID_REQUIRED",
            ValidationErrorSeverity::Error,
            format!("imported CTI record {record_id} requires its original STIX id"),
            target.clone(),
        ));
    }
    if let (Some(object_type), Some(stix_id)) = (object_type, stix_id)
        && !stix_id.starts_with(&format!("{object_type}--"))
    {
        findings.push(ValidationErrorRecord::new(
            "STIX_IDENTITY_INVALID",
            ValidationErrorSeverity::Error,
            format!(
                "imported CTI record {record_id} id {stix_id} does not match type {object_type}"
            ),
            target,
        ));
    }
    findings
}

#[allow(clippy::too_many_arguments)]
fn profile_readiness_findings(
    graph: &Graph,
    profile: &ExportProfile,
    mode: ExportMode,
    record_id: &str,
    status: RecordStatus,
    evidence_refs: &[EvidenceId],
    target: ValidationTarget,
) -> ProfileReadinessFindings {
    let mut findings = ProfileReadinessFindings::default();
    if !export_readiness(status) {
        findings.enforced.push(ValidationErrorRecord::new(
            "EXPORT_STATUS_NOT_READY",
            ValidationErrorSeverity::Warning,
            format!(
                "record {record_id} is not export-ready for mode {}",
                mode_label(mode)
            ),
            target.clone(),
        ));
    }
    // Permission is mandatory for every governed claim attached to the record.
    // Historical or unresolved claims abstain; a lifecycle label cannot grant it.
    let stores = graph.epistemic_stores();
    for claim in stores.claims.claims() {
        let matches_target = match (claim.target(), &target) {
            (crate::ClaimTarget::Node(id), ValidationTarget::Node(target_id)) => {
                id.as_str() == target_id
            }
            (crate::ClaimTarget::Relationship(id), ValidationTarget::Relationship(target_id)) => {
                id.as_str() == target_id
            }
            _ => false,
        };
        if !matches_target {
            continue;
        }
        let assessment = stores
            .verdicts
            .current_verdict(claim.id())
            .and_then(crate::Verdict::actionability);
        if assessment.is_some_and(|a| a.is_actionable()) {
            continue;
        }
        let reasons = assessment.map_or_else(
            || "actionability_assessment_missing".to_owned(),
            |a| serde_json::to_string(a.blockers()).expect("serializable blockers"),
        );
        findings.enforced.push(ValidationErrorRecord::new(
            "EXPORT_ACTIONABILITY_BLOCKED",
            ValidationErrorSeverity::Error,
            format!(
                "record {record_id} claim {} actionability blocked: {reasons}",
                claim.id().as_str()
            ),
            target.clone(),
        ));
    }
    if profile != &ExportProfile::StixMvp {
        return findings;
    }
    if evidence_refs.is_empty() {
        findings.overridable.push(ValidationErrorRecord::new(
            "CTI_EVIDENCE_REQUIRED",
            ValidationErrorSeverity::Error,
            format!("CTI record {record_id} requires native evidence"),
            target,
        ));
    } else {
        for evidence_id in evidence_refs {
            if graph.evidence_by_id(evidence_id).is_none() {
                findings.enforced.push(ValidationErrorRecord::new(
                    "CTI_EVIDENCE_NOT_FOUND",
                    ValidationErrorSeverity::Error,
                    format!(
                        "CTI record {record_id} references missing evidence {}",
                        evidence_id.as_str()
                    ),
                    target.clone(),
                ));
            }
        }
    }
    findings
}

#[derive(Default)]
struct ProfileReadinessFindings {
    overridable: Vec<ValidationErrorRecord>,
    enforced: Vec<ValidationErrorRecord>,
}

fn collect_validation_findings(
    options: ExportPlanOptions,
    findings: impl IntoIterator<Item = ValidationErrorRecord>,
    record_findings: &mut Vec<ValidationErrorRecord>,
    warnings: &mut Vec<ValidationErrorRecord>,
) {
    if options.force_validation() {
        warnings.extend(findings);
    } else {
        record_findings.extend(findings);
    }
}

fn mode_label(mode: ExportMode) -> &'static str {
    match mode {
        ExportMode::Strict => "strict",
        ExportMode::Permissive => "permissive",
    }
}

fn collect_mode_findings(
    mode: ExportMode,
    findings: Vec<ValidationErrorRecord>,
    warnings: &mut Vec<ValidationErrorRecord>,
    strict_reasons: &mut Vec<String>,
) {
    match mode {
        ExportMode::Strict => strict_reasons.extend(
            findings
                .iter()
                .map(|finding| format!("{}: {}", finding.code(), finding.message())),
        ),
        ExportMode::Permissive => warnings.extend(findings),
    }
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

fn collect_blocking_findings(
    findings: &[ValidationErrorRecord],
) -> HashMap<&str, Vec<&ValidationErrorRecord>> {
    let mut by_record_id = HashMap::new();

    for finding in findings {
        if finding.severity() != ValidationErrorSeverity::Error {
            continue;
        }

        match finding.target() {
            ValidationTarget::Node(record_id) | ValidationTarget::Relationship(record_id) => {
                by_record_id
                    .entry(record_id.as_str())
                    .or_insert_with(Vec::new)
                    .push(finding);
            }
            ValidationTarget::Evidence(_)
            | ValidationTarget::Claim(_)
            | ValidationTarget::ExportRecord(_)
            | ValidationTarget::Retrieval(_)
            | ValidationTarget::Source(_) => {}
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
            ExportProfile::FimiJsonMvp,
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
                .first()
                .expect("node blocking finding list should not be empty")
                .code(),
            "ERR_NODE"
        );
        assert_eq!(
            blocking
                .get("relationship--blocked")
                .expect("relationship blocking finding should be present")
                .first()
                .expect("relationship blocking finding list should not be empty")
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

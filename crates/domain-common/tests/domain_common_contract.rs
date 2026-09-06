// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
use domain_common::{
    ConfidenceBand, ConfidencePolicy, DomainSchemaRegistry, DomainSchemaRule,
    DomainValidationIssue, DomainValidationResult, DomainValidationSeverity, EvidenceRequirement,
    classify_confidence_band, validate_evidence_requirement,
};
use graph_core::{
    BitemporalStamp, ClaimId, ClaimInput, ClaimStatement, ClaimStore, ClaimTarget, Confidence,
    EvidenceRecordStore, Graph, NodeInput, ObservationStore, PropertyValue,
    SCHEMA_CONSTRAINT_VERIFIER_ID, SCHEMA_CONSTRAINT_VERIFIER_VERSION, SchemaConstraintVerifier,
    SourceStore, TemporalTimestamp, VerificationContext, VerificationRecordStore,
    VerificationResult, VerifierRegistry, VerifierSpec,
};

//
// Verify domain validation result reports warnings separately from hard errors.
#[test]
fn validation_result_exposes_warning_state_without_invalidating_result() {
    let warning = DomainValidationIssue::new(
        "WARN_ONLY",
        "non-blocking warning",
        Some("field".to_owned()),
        DomainValidationSeverity::Warning,
    )
    .expect("warning issue should be valid");

    let result = DomainValidationResult::fail(vec![warning]);

    assert!(result.is_valid());
    assert!(result.has_warnings());
}

//
// Verify optional evidence requirement does not produce issues when references
// are missing.
#[test]
fn evidence_requirement_optional_allows_empty_references() {
    let result = validate_evidence_requirement(EvidenceRequirement::Optional, &[]);

    assert!(result.is_valid());
    assert!(result.issues().is_empty());
}

//
// Verify confidence classification returns unknown when confidence is absent.
#[test]
fn confidence_band_unknown_when_confidence_missing() {
    let policy = ConfidencePolicy::new(0.5, 0.8).expect("policy should be valid");

    let band = classify_confidence_band(None, policy);

    assert_eq!(band, ConfidenceBand::Unknown);
}

//
// Verify exportable confidence classification for high confidence values.
#[test]
fn confidence_band_exportable_for_high_confidence() {
    let policy = ConfidencePolicy::new(0.5, 0.8).expect("policy should be valid");
    let confidence = Confidence::new(0.95).expect("confidence should be valid");

    let band = classify_confidence_band(Some(confidence), policy);

    assert_eq!(band, ConfidenceBand::Exportable);
}

//
// Epic 0029 WS-A item 6: graph-core validation findings (for example the
// verdict reachability gap) surface through the domain validation result
// additively, keeping code, message, severity, and the target as the field.
#[test]
fn validation_records_surface_as_domain_validation_issues() {
    use domain_common::{DomainValidationIssue, DomainValidationResult, DomainValidationSeverity};
    use graph_core::{ValidationErrorRecord, ValidationErrorSeverity, ValidationTarget};

    let warning = ValidationErrorRecord::new(
        "claim.verdict.unreachable_evidence",
        ValidationErrorSeverity::Warning,
        "claim claim--x has no observation path",
        ValidationTarget::claim("claim--x"),
    );
    let error = ValidationErrorRecord::new(
        "source.content_drift",
        ValidationErrorSeverity::Error,
        "artifact changed",
        ValidationTarget::source("source--y"),
    );
    let info = ValidationErrorRecord::new(
        "note",
        ValidationErrorSeverity::Info,
        "informational",
        ValidationTarget::node("node--z"),
    );

    let issue = DomainValidationIssue::from_validation_record(&warning);
    assert_eq!(issue.code, "claim.verdict.unreachable_evidence");
    assert_eq!(issue.message, "claim claim--x has no observation path");
    assert_eq!(issue.field.as_deref(), Some("claim:claim--x"));
    assert_eq!(issue.severity, DomainValidationSeverity::Warning);

    let result = DomainValidationResult::from_validation_records(&[warning, error, info]);
    assert_eq!(
        result.issues().len(),
        2,
        "info findings do not become issues"
    );
    assert_eq!(result.issues()[1].severity, DomainValidationSeverity::Error);
    assert_eq!(
        result.issues()[1].field.as_deref(),
        Some("source:source--y")
    );
    assert!(!result.is_valid());
    assert!(result.has_warnings());

    let clean = DomainValidationResult::from_validation_records(&[]);
    assert!(clean.is_valid());
}

fn timestamp(value: &str) -> TemporalTimestamp {
    TemporalTimestamp::new(value).expect("timestamp")
}

fn run_schema_check(
    graph: &Graph,
    claims: &ClaimStore,
    schemas: &DomainSchemaRegistry,
) -> graph_core::VerificationRecord {
    let mut registry = VerifierRegistry::new();
    registry
        .register(VerifierSpec::new(Box::new(SchemaConstraintVerifier::new())))
        .expect("verifier registration");
    let observations = ObservationStore::new();
    let sources = SourceStore::new();
    let evidence = EvidenceRecordStore::new();
    let context = VerificationContext::new(claims, &observations, &sources, &evidence)
        .with_graph(graph)
        .with_schema_constraints(schemas);
    let mut records = VerificationRecordStore::new();
    let record_id = registry
        .run(
            SCHEMA_CONSTRAINT_VERIFIER_ID,
            SCHEMA_CONSTRAINT_VERIFIER_VERSION,
            &ClaimId::new("claim--domain-schema").expect("claim id"),
            &context,
            &mut records,
            BitemporalStamp::new(
                timestamp("2026-09-01T00:00:00Z"),
                timestamp("2026-09-06T12:00:00Z"),
            )
            .expect("stamp"),
        )
        .expect("schema verifier run");
    records.record_by_id(&record_id).expect("record").clone()
}

#[test]
fn installed_domain_schema_supplies_required_properties_and_type_assertions() {
    let mut graph = Graph::new();
    let node = graph
        .create_node(
            NodeInput::new(["Entity", "Study"])
                .with_property("name", PropertyValue::String("Trial A".to_owned())),
        )
        .expect("node");
    let mut claims = ClaimStore::new();
    claims
        .create_asserted_claim(ClaimInput::new(
            ClaimId::new("claim--domain-schema").expect("claim id"),
            ClaimStatement::new("The node follows the installed schema").expect("statement"),
            ClaimTarget::Node(node),
        ))
        .expect("claim");
    let mut schemas = DomainSchemaRegistry::new();
    schemas
        .register(
            DomainSchemaRule::for_node_label("research", "Entity")
                .with_required_property("name")
                .with_required_type_assertion("Study"),
        )
        .expect("schema rule");

    let record = run_schema_check(&graph, &claims, &schemas);

    assert_eq!(record.result(), VerificationResult::Pass);
    assert!(
        record
            .rationale()
            .is_some_and(|text| text.contains("research") && text.contains("Entity"))
    );
}

#[test]
fn installed_domain_schema_names_missing_property_and_type_assertion() {
    let mut graph = Graph::new();
    let node = graph.create_node(NodeInput::new(["Entity"])).expect("node");
    let mut claims = ClaimStore::new();
    claims
        .create_asserted_claim(ClaimInput::new(
            ClaimId::new("claim--domain-schema").expect("claim id"),
            ClaimStatement::new("The node violates the installed schema").expect("statement"),
            ClaimTarget::Node(node),
        ))
        .expect("claim");
    let mut schemas = DomainSchemaRegistry::new();
    schemas
        .register(
            DomainSchemaRule::for_node_label("research", "Entity")
                .with_required_property("name")
                .with_required_type_assertion("Study"),
        )
        .expect("schema rule");

    let record = run_schema_check(&graph, &claims, &schemas);

    assert_eq!(record.result(), VerificationResult::Fail);
    assert!(record.rationale().is_some_and(|text| {
        text.contains("required property 'name'")
            && text.contains("required type assertion 'Study'")
    }));
}

#[test]
fn actionability_path_never_promotes_a_blocked_or_absent_permission() {
    let mut dimensions = graph_core::ConfidenceDimensions::default();
    assert_eq!(
        domain_common::classify_confidence_band_with_actionability(&dimensions),
        ConfidenceBand::Unknown
    );
    dimensions.actionability = Some(Confidence::new(0.0).expect("score"));
    assert_eq!(
        domain_common::classify_confidence_band_with_actionability(&dimensions),
        ConfidenceBand::BelowValidated
    );
    dimensions.actionability = Some(Confidence::new(1.0).expect("score"));
    assert_eq!(
        domain_common::classify_confidence_band_with_actionability(&dimensions),
        ConfidenceBand::Exportable
    );
}

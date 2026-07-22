// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
use domain_common::{
    ConfidenceBand, ConfidencePolicy, DomainValidationIssue, DomainValidationResult,
    DomainValidationSeverity, EvidenceRequirement, classify_confidence_band,
    validate_evidence_requirement,
};
use graph_core::Confidence;

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

// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
#![warn(missing_docs)]

//! Shared MVP domain validation contracts and helper utilities.
//!
//! This crate provides reusable abstractions for domain-level validation flows
//! that can be shared by CTI, FIMI, and crisis modules.

use graph_core::Confidence;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Domain validation severity.
pub enum DomainValidationSeverity {
    /// Warning.
    Warning,
    /// Error.
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Domain validation issue.
pub struct DomainValidationIssue {
    /// Code.
    pub code: String,
    /// Message.
    pub message: String,
    /// Field.
    pub field: Option<String>,
    /// Severity.
    pub severity: DomainValidationSeverity,
}

impl DomainValidationIssue {
    /// Creates a new instance.
    pub fn new(
        code: impl Into<String>,
        message: impl Into<String>,
        field: Option<String>,
        severity: DomainValidationSeverity,
    ) -> Result<Self, DomainValidationError> {
        let code = code.into();
        if code.trim().is_empty() {
            return Err(DomainValidationError::InvalidValidationIssueField("code"));
        }

        let message = message.into();
        if message.trim().is_empty() {
            return Err(DomainValidationError::InvalidValidationIssueField(
                "message",
            ));
        }

        Ok(Self {
            code,
            message,
            field,
            severity,
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
/// Domain validation result.
pub struct DomainValidationResult {
    issues: Vec<DomainValidationIssue>,
}

impl DomainValidationResult {
    /// Pass.
    pub fn pass() -> Self {
        Self::default()
    }

    /// Fail.
    pub fn fail(issues: Vec<DomainValidationIssue>) -> Self {
        Self { issues }
    }

    /// Issues.
    pub fn issues(&self) -> &[DomainValidationIssue] {
        self.issues.as_slice()
    }

    /// Returns `true` if valid.
    pub fn is_valid(&self) -> bool {
        !self
            .issues
            .iter()
            .any(|issue| issue.severity == DomainValidationSeverity::Error)
    }

    /// Returns `true` if has warnings.
    pub fn has_warnings(&self) -> bool {
        self.issues
            .iter()
            .any(|issue| issue.severity == DomainValidationSeverity::Warning)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Evidence requirement.
pub enum EvidenceRequirement {
    /// Required.
    Required,
    /// Optional.
    Optional,
    /// Forbidden.
    Forbidden,
}

/// Validates the evidence requirement.
pub fn validate_evidence_requirement(
    requirement: EvidenceRequirement,
    evidence_refs: &[String],
) -> DomainValidationResult {
    match requirement {
        EvidenceRequirement::Required if evidence_refs.is_empty() => {
            DomainValidationResult::fail(vec![
                DomainValidationIssue::new(
                    "EVIDENCE_REQUIRED",
                    "at least one evidence reference is required",
                    Some("evidence_refs".to_owned()),
                    DomainValidationSeverity::Error,
                )
                .expect("static issue payload should be valid"),
            ])
        }
        EvidenceRequirement::Forbidden if !evidence_refs.is_empty() => {
            DomainValidationResult::fail(vec![
                DomainValidationIssue::new(
                    "EVIDENCE_FORBIDDEN",
                    "evidence references are not allowed for this rule",
                    Some("evidence_refs".to_owned()),
                    DomainValidationSeverity::Error,
                )
                .expect("static issue payload should be valid"),
            ])
        }
        _ => DomainValidationResult::pass(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
/// Confidence policy.
pub struct ConfidencePolicy {
    /// Min for validated.
    pub min_for_validated: f64,
    /// Min for exportable.
    pub min_for_exportable: f64,
}

impl ConfidencePolicy {
    /// Creates a new instance.
    pub fn new(
        min_for_validated: f64,
        min_for_exportable: f64,
    ) -> Result<Self, DomainValidationError> {
        if !(0.0..=1.0).contains(&min_for_validated) {
            return Err(DomainValidationError::InvalidConfidencePolicyField(
                "min_for_validated",
            ));
        }

        if !(0.0..=1.0).contains(&min_for_exportable) {
            return Err(DomainValidationError::InvalidConfidencePolicyField(
                "min_for_exportable",
            ));
        }

        if min_for_validated > min_for_exportable {
            return Err(DomainValidationError::InvalidConfidencePolicyField(
                "threshold_ordering",
            ));
        }

        Ok(Self {
            min_for_validated,
            min_for_exportable,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Confidence band.
pub enum ConfidenceBand {
    /// Unknown.
    Unknown,
    /// Below validated.
    BelowValidated,
    /// Validated.
    Validated,
    /// Exportable.
    Exportable,
}

/// Classify confidence band.
pub fn classify_confidence_band(
    confidence: Option<Confidence>,
    policy: ConfidencePolicy,
) -> ConfidenceBand {
    let Some(confidence) = confidence else {
        return ConfidenceBand::Unknown;
    };

    let value = confidence.value();
    if value < policy.min_for_validated {
        return ConfidenceBand::BelowValidated;
    }
    if value < policy.min_for_exportable {
        return ConfidenceBand::Validated;
    }

    ConfidenceBand::Exportable
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
/// Domain validation error.
pub enum DomainValidationError {
    #[error("invalid validation issue field: {0}")]
    /// Invalid validation issue field.
    InvalidValidationIssueField(&'static str),

    #[error("invalid confidence policy field: {0}")]
    /// Invalid confidence policy field.
    InvalidConfidencePolicyField(&'static str),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evidence_requirement_required_rejects_empty_references() {
        let result = validate_evidence_requirement(EvidenceRequirement::Required, &[]);

        assert!(!result.is_valid());
        assert_eq!(result.issues().len(), 1);
        assert_eq!(result.issues()[0].code, "EVIDENCE_REQUIRED");
    }

    #[test]
    fn confidence_band_classification_uses_policy_thresholds() {
        let policy = ConfidencePolicy::new(0.6, 0.8).expect("policy should be valid");

        let low = classify_confidence_band(
            Some(Confidence::new(0.45).expect("confidence should be valid")),
            policy,
        );
        let validated = classify_confidence_band(
            Some(Confidence::new(0.7).expect("confidence should be valid")),
            policy,
        );
        let exportable = classify_confidence_band(
            Some(Confidence::new(0.92).expect("confidence should be valid")),
            policy,
        );

        assert_eq!(low, ConfidenceBand::BelowValidated);
        assert_eq!(validated, ConfidenceBand::Validated);
        assert_eq!(exportable, ConfidenceBand::Exportable);
    }

    #[test]
    fn confidence_policy_rejects_invalid_threshold_ordering() {
        let error = ConfidencePolicy::new(0.9, 0.5)
            .expect_err("validated threshold above exportable should be rejected");

        assert!(matches!(
        error,
        DomainValidationError::InvalidConfidencePolicyField(field)
        if field == "threshold_ordering"
        ));
    }
}

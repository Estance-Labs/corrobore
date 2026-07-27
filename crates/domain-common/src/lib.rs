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

/// A domain-attributed type assertion carried by a memory node in addition to
/// its application-defined base kind.
///
/// Composition places every domain's typing on one node rather than minting a
/// parallel node per pack. Each assertion records the provider that owns it so a
/// rejection can name its origin.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DomainTypeAssertion {
    /// Originating provider or domain name, for example `medical`.
    pub provider: String,
    /// Domain-defined node type, for example `Study`.
    pub node_type: String,
}

impl DomainTypeAssertion {
    /// Creates a new instance.
    pub fn new(
        provider: impl Into<String>,
        node_type: impl Into<String>,
    ) -> Result<Self, DomainValidationError> {
        let provider = provider.into();
        if provider.trim().is_empty() {
            return Err(DomainValidationError::InvalidMultiTypingField("provider"));
        }

        let node_type = node_type.into();
        if node_type.trim().is_empty() {
            return Err(DomainValidationError::InvalidMultiTypingField("node_type"));
        }

        Ok(Self {
            provider,
            node_type,
        })
    }
}

/// Policy bounding how many domain type assertions one node may carry.
///
/// The graph-label guidance behind the bound is that type sets past a small
/// count degrade rather than help. The default ceiling of four leaves room for
/// one base kind plus a handful of installed packs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MultiTypingPolicy {
    max_assertions: usize,
}

impl MultiTypingPolicy {
    /// Default ceiling on domain type assertions per node.
    pub const DEFAULT_MAX_ASSERTIONS: usize = 4;

    /// Creates a new instance. The ceiling must be at least one.
    pub fn new(max_assertions: usize) -> Result<Self, DomainValidationError> {
        if max_assertions == 0 {
            return Err(DomainValidationError::InvalidMultiTypingField(
                "max_assertions",
            ));
        }

        Ok(Self { max_assertions })
    }

    /// Policy using the default ceiling.
    pub fn default_bounded() -> Self {
        Self {
            max_assertions: Self::DEFAULT_MAX_ASSERTIONS,
        }
    }

    /// Max assertions.
    pub fn max_assertions(&self) -> usize {
        self.max_assertions
    }
}

/// Validates bounded multi-typing for one memory node.
///
/// A node carries one non-empty base kind plus at most one type assertion per
/// installed pack, and no more than the policy ceiling in total. A second
/// assertion from the same provider is rejected with an issue that names the
/// provider, so composition cannot silently overwrite a peer pack's typing.
pub fn validate_multi_typing(
    base_kind: &str,
    assertions: &[DomainTypeAssertion],
    policy: MultiTypingPolicy,
) -> DomainValidationResult {
    let mut issues = Vec::new();

    if base_kind.trim().is_empty() {
        issues.push(
            DomainValidationIssue::new(
                "MULTI_TYPING_BASE_KIND_REQUIRED",
                "a memory node requires a non-empty base kind",
                Some("base_kind".to_owned()),
                DomainValidationSeverity::Error,
            )
            .expect("static issue payload should be valid"),
        );
    }

    let mut seen_providers: Vec<&str> = Vec::new();
    for assertion in assertions {
        let provider = assertion.provider.trim();
        if provider.is_empty() || assertion.node_type.trim().is_empty() {
            issues.push(
                DomainValidationIssue::new(
                    "MULTI_TYPING_ASSERTION_INCOMPLETE",
                    "a domain type assertion requires a provider and a node type",
                    Some("assertions".to_owned()),
                    DomainValidationSeverity::Error,
                )
                .expect("static issue payload should be valid"),
            );
            continue;
        }

        if seen_providers.contains(&provider) {
            issues.push(
                DomainValidationIssue::new(
                    "MULTI_TYPING_DUPLICATE_PROVIDER",
                    format!("provider {provider} asserted more than one node type"),
                    Some("provider".to_owned()),
                    DomainValidationSeverity::Error,
                )
                .expect("dynamic issue payload should be valid"),
            );
        } else {
            seen_providers.push(provider);
        }
    }

    if assertions.len() > policy.max_assertions() {
        issues.push(
            DomainValidationIssue::new(
                "MULTI_TYPING_LIMIT_EXCEEDED",
                format!(
                    "a node carries {} type assertions but the policy allows {}",
                    assertions.len(),
                    policy.max_assertions()
                ),
                Some("assertions".to_owned()),
                DomainValidationSeverity::Error,
            )
            .expect("dynamic issue payload should be valid"),
        );
    }

    if issues.is_empty() {
        DomainValidationResult::pass()
    } else {
        DomainValidationResult::fail(issues)
    }
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

    #[error("invalid multi-typing field: {0}")]
    /// Invalid multi-typing field.
    InvalidMultiTypingField(&'static str),
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

    fn assertion(provider: &str, node_type: &str) -> DomainTypeAssertion {
        DomainTypeAssertion::new(provider, node_type).expect("assertion should be valid")
    }

    #[test]
    fn multi_typing_accepts_one_assertion_per_provider() {
        // A clinical trial typed by both packs is one node with two assertions.
        let result = validate_multi_typing(
            "study",
            &[assertion("medical", "Study"), assertion("research", "Study")],
            MultiTypingPolicy::default_bounded(),
        );

        assert!(result.is_valid());
        assert!(result.issues().is_empty());
    }

    #[test]
    fn multi_typing_rejects_a_second_assertion_from_the_same_provider() {
        let result = validate_multi_typing(
            "study",
            &[
                assertion("medical", "Study"),
                assertion("medical", "ClinicalTrial"),
            ],
            MultiTypingPolicy::default_bounded(),
        );

        assert!(!result.is_valid());
        let issue = &result.issues()[0];
        assert_eq!(issue.code, "MULTI_TYPING_DUPLICATE_PROVIDER");
        // The rejection names the originating provider.
        assert!(issue.message.contains("medical"));
    }

    #[test]
    fn multi_typing_requires_a_base_kind() {
        let result = validate_multi_typing(
            "   ",
            &[assertion("medical", "Study")],
            MultiTypingPolicy::default_bounded(),
        );

        assert!(!result.is_valid());
        assert_eq!(result.issues()[0].code, "MULTI_TYPING_BASE_KIND_REQUIRED");
    }

    #[test]
    fn multi_typing_rejects_unbounded_type_sets() {
        let policy = MultiTypingPolicy::new(2).expect("policy should be valid");
        let result = validate_multi_typing(
            "study",
            &[
                assertion("medical", "Study"),
                assertion("research", "Study"),
                assertion("cti", "Report"),
            ],
            policy,
        );

        assert!(!result.is_valid());
        assert!(
            result
                .issues()
                .iter()
                .any(|issue| issue.code == "MULTI_TYPING_LIMIT_EXCEEDED")
        );
    }

    #[test]
    fn multi_typing_policy_rejects_a_zero_ceiling() {
        let error =
            MultiTypingPolicy::new(0).expect_err("a zero ceiling should be rejected");
        assert!(matches!(
            error,
            DomainValidationError::InvalidMultiTypingField(field) if field == "max_assertions"
        ));
    }

    #[test]
    fn domain_type_assertion_rejects_empty_fields() {
        assert!(DomainTypeAssertion::new("", "Study").is_err());
        assert!(DomainTypeAssertion::new("medical", "  ").is_err());
    }
}

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
//! phase-1 acceptance tests for validation error and rule registry contracts.
//!
//! These tests intentionally define the public contract before implementation.

use graph_core::{
    GraphError, RuleId, ValidationErrorRecord, ValidationErrorSeverity, ValidationErrorStatus,
    ValidationRuleContext, ValidationRuleRegistry, ValidationTarget,
};

//
// Verify that validation errors are explicit, graph-addressable records with
// actionable remediation guidance suitable for agent self-correction loops.
#[test]
fn validation_error_record_exposes_actionable_fields() {
    let error = ValidationErrorRecord::new(
        "DOMAIN_CONFIDENCE_OUT_OF_RANGE",
        ValidationErrorSeverity::Error,
        "confidence must be between 0.0 and 1.0",
        ValidationTarget::node("node--42"),
    )
    .with_suggested_remediation("set confidence to a value in [0.0, 1.0]");

    assert_eq!(error.code(), "DOMAIN_CONFIDENCE_OUT_OF_RANGE");
    assert_eq!(error.severity(), ValidationErrorSeverity::Error);
    assert_eq!(error.message(), "confidence must be between 0.0 and 1.0");
    assert_eq!(error.target(), &ValidationTarget::node("node--42"));
    assert_eq!(
        error.suggested_remediation(),
        Some("set confidence to a value in [0.0, 1.0]")
    );
}

//
// Verify that the validation rule registry boundary supports deterministic
// registration and execution for node and relationship checks.
#[test]
fn validation_rule_registry_runs_registered_rules_deterministically() {
    let mut registry = ValidationRuleRegistry::new();

    registry
        .register_node_rule(
            RuleId::new("confidence-range"),
            Box::new(|context: &ValidationRuleContext| {
                let mut errors = Vec::new();

                if context.record_ref() == "node--low" {
                    errors.push(
                        ValidationErrorRecord::new(
                            "DOMAIN_CONFIDENCE_OUT_OF_RANGE",
                            ValidationErrorSeverity::Error,
                            "confidence must be between 0.0 and 1.0",
                            ValidationTarget::node(context.record_ref()),
                        )
                        .with_suggested_remediation("set confidence to a value in [0.0, 1.0]"),
                    );
                }

                errors
            }),
        )
        .expect("rule should register");

    registry
        .register_node_rule(
            RuleId::new("evidence-required"),
            Box::new(|context: &ValidationRuleContext| {
                if context.record_ref() == "node--low" {
                    return vec![
                        ValidationErrorRecord::new(
                            "EVIDENCE_REQUIRED",
                            ValidationErrorSeverity::Warning,
                            "validated record requires supporting evidence",
                            ValidationTarget::node(context.record_ref()),
                        )
                        .with_suggested_remediation(
                            "attach at least one evidence reference before validation",
                        ),
                    ];
                }

                Vec::new()
            }),
        )
        .expect("rule should register");

    let context = ValidationRuleContext::new("node--low");
    let errors = registry
        .evaluate_node(&context)
        .expect("evaluation should work");

    assert_eq!(errors.len(), 2);
    assert_eq!(errors[0].code(), "DOMAIN_CONFIDENCE_OUT_OF_RANGE");
    assert_eq!(errors[1].code(), "EVIDENCE_REQUIRED");
}

//
// Verify that duplicate rule IDs are rejected with a typed graph-core error
// instead of string-only or panic behavior.
#[test]
fn validation_rule_registry_rejects_duplicate_rule_ids() {
    let mut registry = ValidationRuleRegistry::new();

    registry
        .register_node_rule(RuleId::new("confidence-range"), Box::new(|_| Vec::new()))
        .expect("first registration should succeed");

    let duplicate = registry
        .register_node_rule(RuleId::new("confidence-range"), Box::new(|_| Vec::new()))
        .expect_err("duplicate ID should be rejected");

    assert!(matches!(
    duplicate,
    GraphError::InvalidPropertyValue(message)
    if message == "validation rule already registered: confidence-range"
    ));
}

//
// Verify blank rule code falls back to a deterministic unknown ID suffix while
// preserving the original code payload unchanged.
#[test]
fn validation_error_record_uses_unknown_id_for_blank_code() {
    let error = ValidationErrorRecord::new(
        " ",
        ValidationErrorSeverity::Info,
        "missing code should still produce typed ID",
        ValidationTarget::claim("claim--1"),
    );

    assert_eq!(error.error_id().as_str(), "validation-error--unknown");
    assert_eq!(error.code(), " ");
    assert_eq!(error.status(), ValidationErrorStatus::Open);
}

//
// Verify tracking metadata mutator replaces lifecycle fields and can be chained
// with remediation guidance.
#[test]
fn validation_error_record_updates_tracking_metadata_and_remediation() {
    let created_at = graph_core::TemporalTimestamp::new("2026-07-07T10:00:00Z")
        .expect("timestamp should be valid");
    let transaction_id =
        graph_core::TransactionId::new("transaction--777").expect("transaction ID should be valid");
    let tracked_id = graph_core::ValidationErrorId::new("validation-error--tracked")
        .expect("validation error ID should be valid");

    let error = ValidationErrorRecord::new(
        "RULE_A",
        ValidationErrorSeverity::Warning,
        "rule warning",
        ValidationTarget::export_record("export-record--1"),
    )
    .with_tracking_metadata(
        tracked_id.clone(),
        created_at.clone(),
        transaction_id.clone(),
    )
    .with_status(ValidationErrorStatus::Resolved)
    .with_suggested_remediation("attach supporting evidence");

    assert_eq!(error.error_id(), &tracked_id);
    assert_eq!(error.created_at(), Some(&created_at));
    assert_eq!(error.transaction_id(), Some(&transaction_id));
    assert_eq!(error.status(), ValidationErrorStatus::Resolved);
    assert_eq!(
        error.suggested_remediation(),
        Some("attach supporting evidence")
    );
}

//
// Verify relationship rule evaluation executes only registered relationship
// rules and does not depend on node rule registration.
#[test]
fn validation_rule_registry_evaluates_relationship_rules() {
    let mut registry = ValidationRuleRegistry::new();

    registry
        .register_relationship_rule(
            RuleId::new("relationship-type-required"),
            Box::new(|context: &ValidationRuleContext| {
                if context.record_ref() == "relationship--missing-type" {
                    return vec![ValidationErrorRecord::new(
                        "RELATIONSHIP_TYPE_REQUIRED",
                        ValidationErrorSeverity::Error,
                        "relationship type must be present",
                        ValidationTarget::relationship(context.record_ref()),
                    )];
                }

                Vec::new()
            }),
        )
        .expect("relationship rule should register");

    let errors = registry
        .evaluate_relationship(&ValidationRuleContext::new("relationship--missing-type"))
        .expect("relationship evaluation should succeed");

    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code(), "RELATIONSHIP_TYPE_REQUIRED");
    assert_eq!(
        errors[0].target(),
        &ValidationTarget::relationship("relationship--missing-type")
    );
}

//
// Verify duplicate rule IDs are also rejected when the relationship scope is
// registered first, covering both uniqueness lookup branches.
#[test]
fn validation_rule_registry_rejects_duplicate_rule_ids_from_relationship_scope() {
    let mut registry = ValidationRuleRegistry::new();

    registry
        .register_relationship_rule(
            RuleId::new("shared-id"),
            Box::new(|_: &ValidationRuleContext| Vec::new()),
        )
        .expect("first relationship rule should register");

    let duplicate = registry
        .register_node_rule(
            RuleId::new("shared-id"),
            Box::new(|_: &ValidationRuleContext| Vec::new()),
        )
        .expect_err("duplicate should be rejected");

    assert!(matches!(
    duplicate,
    GraphError::InvalidPropertyValue(message)
    if message == "validation rule already registered: shared-id"
    ));
}

//
// Verify validation context attributes can be attached and read while missing
// keys remain absent.
#[test]
fn validation_rule_context_stores_and_reads_attributes() {
    let context = ValidationRuleContext::new("node--ctx")
        .with_attribute("confidence", "0.62")
        .with_attribute("status", "validated");

    assert_eq!(context.record_ref(), "node--ctx");
    assert_eq!(context.attribute("confidence"), Some("0.62"));
    assert_eq!(context.attribute("status"), Some("validated"));
    assert_eq!(context.attribute("missing"), None);
}

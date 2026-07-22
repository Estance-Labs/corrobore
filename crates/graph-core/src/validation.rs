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

use crate::{GraphError, TemporalTimestamp, TransactionId, ValidationErrorId};

/// Severity level for deterministic pre-commit validation feedback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValidationErrorSeverity {
    /// Info.
    Info,
    /// Warning.
    Warning,
    /// Error.
    Error,
}

/// Lifecycle status for graph-addressable validation errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValidationErrorStatus {
    /// Open.
    Open,
    /// Resolved.
    Resolved,
}

/// Graph-addressable target for a validation error.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ValidationTarget {
    /// Node.
    Node(String),
    /// Relationship.
    Relationship(String),
    /// Claim.
    Claim(String),
    /// Export record.
    ExportRecord(String),
    /// Recorded retrieval.
    Retrieval(String),
}

impl ValidationTarget {
    /// Node.
    pub fn node(value: impl Into<String>) -> Self {
        Self::Node(value.into())
    }

    /// Relationship.
    pub fn relationship(value: impl Into<String>) -> Self {
        Self::Relationship(value.into())
    }

    /// Claim.
    pub fn claim(value: impl Into<String>) -> Self {
        Self::Claim(value.into())
    }

    /// Export record.
    pub fn export_record(value: impl Into<String>) -> Self {
        Self::ExportRecord(value.into())
    }

    /// Recorded retrieval.
    pub fn retrieval(value: impl Into<String>) -> Self {
        Self::Retrieval(value.into())
    }
}

/// Actionable deterministic validation error payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidationErrorRecord {
    error_id: ValidationErrorId,
    status: ValidationErrorStatus,
    created_at: Option<TemporalTimestamp>,
    transaction_id: Option<TransactionId>,
    code: String,
    severity: ValidationErrorSeverity,
    message: String,
    target: ValidationTarget,
    suggested_remediation: Option<String>,
}

/// Tracking metadata attached to validation errors that come from persisted transactions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidationTrackingMetadata {
    error_id: ValidationErrorId,
    status: ValidationErrorStatus,
    created_at: TemporalTimestamp,
    transaction_id: TransactionId,
}

impl ValidationTrackingMetadata {
    /// Creates new tracking metadata.
    pub fn new(
        error_id: ValidationErrorId,
        status: ValidationErrorStatus,
        created_at: TemporalTimestamp,
        transaction_id: TransactionId,
    ) -> Self {
        Self {
            error_id,
            status,
            created_at,
            transaction_id,
        }
    }
}

impl ValidationErrorRecord {
    /// Creates a new instance.
    pub fn new(
        code: impl Into<String>,
        severity: ValidationErrorSeverity,
        message: impl Into<String>,
        target: ValidationTarget,
    ) -> Self {
        let code = code.into();

        Self {
            // Error id.
            error_id: default_error_id(code.as_str()),
            // Status.
            status: ValidationErrorStatus::Open,
            // Created at.
            created_at: None,
            // Transaction id.
            transaction_id: None,
            code,
            severity,
            // Message.
            message: message.into(),
            target,
            // Suggested remediation.
            suggested_remediation: None,
        }
    }

    /// Creates a new instance.
    pub fn new_tracked(
        tracking: ValidationTrackingMetadata,
        code: impl Into<String>,
        severity: ValidationErrorSeverity,
        message: impl Into<String>,
        target: ValidationTarget,
    ) -> Self {
        Self {
            error_id: tracking.error_id,
            status: tracking.status,
            // Created at.
            created_at: Some(tracking.created_at),
            // Transaction id.
            transaction_id: Some(tracking.transaction_id),
            // Code.
            code: code.into(),
            severity,
            // Message.
            message: message.into(),
            target,
            // Suggested remediation.
            suggested_remediation: None,
        }
    }

    /// Error id.
    pub fn error_id(&self) -> &ValidationErrorId {
        &self.error_id
    }

    /// Status.
    pub fn status(&self) -> ValidationErrorStatus {
        self.status
    }

    /// Created at.
    pub fn created_at(&self) -> Option<&TemporalTimestamp> {
        self.created_at.as_ref()
    }

    /// Transaction id.
    pub fn transaction_id(&self) -> Option<&TransactionId> {
        self.transaction_id.as_ref()
    }

    /// Code.
    pub fn code(&self) -> &str {
        self.code.as_str()
    }

    /// Severity.
    pub fn severity(&self) -> ValidationErrorSeverity {
        self.severity
    }

    /// Message.
    pub fn message(&self) -> &str {
        self.message.as_str()
    }

    /// Target.
    pub fn target(&self) -> &ValidationTarget {
        &self.target
    }

    /// Suggested remediation.
    pub fn suggested_remediation(&self) -> Option<&str> {
        self.suggested_remediation.as_deref()
    }

    /// Sets the status.
    pub fn with_status(mut self, status: ValidationErrorStatus) -> Self {
        self.status = status;
        self
    }

    /// Sets the tracking metadata.
    pub fn with_tracking_metadata(
        mut self,
        error_id: ValidationErrorId,
        created_at: TemporalTimestamp,
        transaction_id: TransactionId,
    ) -> Self {
        self.error_id = error_id;
        self.created_at = Some(created_at);
        self.transaction_id = Some(transaction_id);
        self
    }

    /// Sets the suggested remediation.
    pub fn with_suggested_remediation(mut self, remediation: impl Into<String>) -> Self {
        self.suggested_remediation = Some(remediation.into());
        self
    }
}

fn default_error_id(code: &str) -> ValidationErrorId {
    let normalized = code.trim();
    let suffix = if normalized.is_empty() {
        "unknown"
    } else {
        normalized
    };

    ValidationErrorId::new(format!("validation-error--{}", suffix))
        .expect("generated validation error ID should always be non-empty")
}

/// Stable rule identifier for registry-level deduplication.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RuleId(String);

impl RuleId {
    /// Creates a new instance.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the value as str.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// Context passed to validation rule functions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ValidationRuleContext {
    record_ref: String,
    attributes: HashMap<String, String>,
}

impl ValidationRuleContext {
    /// Creates a new instance.
    pub fn new(record_ref: impl Into<String>) -> Self {
        Self {
            // Record ref.
            record_ref: record_ref.into(),
            // Attributes.
            attributes: HashMap::new(),
        }
    }

    /// Record ref.
    pub fn record_ref(&self) -> &str {
        self.record_ref.as_str()
    }

    /// Sets the attribute.
    pub fn with_attribute(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(key.into(), value.into());
        self
    }

    /// Attribute.
    pub fn attribute(&self, key: &str) -> Option<&str> {
        self.attributes.get(key).map(String::as_str)
    }
}

type ValidationRuleFn = dyn Fn(&ValidationRuleContext) -> Vec<ValidationErrorRecord>;

/// Registry boundary for deterministic domain validation rule execution.
#[derive(Default)]
pub struct ValidationRuleRegistry {
    node_rules: Vec<(RuleId, Box<ValidationRuleFn>)>,
    relationship_rules: Vec<(RuleId, Box<ValidationRuleFn>)>,
}

impl ValidationRuleRegistry {
    /// Creates a new instance.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register node rule.
    pub fn register_node_rule(
        &mut self,
        rule_id: RuleId,
        rule: Box<ValidationRuleFn>,
    ) -> Result<(), GraphError> {
        self.ensure_rule_id_is_unique(rule_id.as_str())?;
        self.node_rules.push((rule_id, rule));
        Ok(())
    }

    /// Register relationship rule.
    pub fn register_relationship_rule(
        &mut self,
        rule_id: RuleId,
        rule: Box<ValidationRuleFn>,
    ) -> Result<(), GraphError> {
        self.ensure_rule_id_is_unique(rule_id.as_str())?;
        self.relationship_rules.push((rule_id, rule));
        Ok(())
    }

    /// Evaluate node.
    pub fn evaluate_node(
        &self,
        context: &ValidationRuleContext,
    ) -> Result<Vec<ValidationErrorRecord>, GraphError> {
        Ok(Self::evaluate_rules(self.node_rules.as_slice(), context))
    }

    /// Evaluate relationship.
    pub fn evaluate_relationship(
        &self,
        context: &ValidationRuleContext,
    ) -> Result<Vec<ValidationErrorRecord>, GraphError> {
        Ok(Self::evaluate_rules(
            self.relationship_rules.as_slice(),
            context,
        ))
    }

    fn evaluate_rules(
        rules: &[(RuleId, Box<ValidationRuleFn>)],
        context: &ValidationRuleContext,
    ) -> Vec<ValidationErrorRecord> {
        let mut errors = Vec::new();

        for (_, rule) in rules {
            errors.extend(rule(context));
        }

        errors
    }

    fn ensure_rule_id_is_unique(&self, candidate: &str) -> Result<(), GraphError> {
        let exists_in_nodes = self
            .node_rules
            .iter()
            .any(|(rule_id, _)| rule_id.as_str() == candidate);
        let exists_in_relationships = self
            .relationship_rules
            .iter()
            .any(|(rule_id, _)| rule_id.as_str() == candidate);

        if exists_in_nodes || exists_in_relationships {
            return Err(GraphError::InvalidPropertyValue(format!(
                "validation rule already registered: {}",
                candidate
            )));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{TemporalTimestamp, TransactionId, ValidationErrorId};

    //
    // Verify deterministic execution order follows registration order.
    #[test]
    fn registry_executes_node_rules_in_registration_order() {
        let mut registry = ValidationRuleRegistry::new();

        registry
            .register_node_rule(
                RuleId::new("rule-a"),
                Box::new(|context| {
                    vec![ValidationErrorRecord::new(
                        "A",
                        ValidationErrorSeverity::Warning,
                        "first",
                        ValidationTarget::node(context.record_ref()),
                    )]
                }),
            )
            .expect("rule-a should register");

        registry
            .register_node_rule(
                RuleId::new("rule-b"),
                Box::new(|context| {
                    vec![ValidationErrorRecord::new(
                        "B",
                        ValidationErrorSeverity::Error,
                        "second",
                        ValidationTarget::node(context.record_ref()),
                    )]
                }),
            )
            .expect("rule-b should register");

        let errors = registry
            .evaluate_node(&ValidationRuleContext::new("node--1"))
            .expect("evaluation should succeed");

        assert_eq!(errors.len(), 2);
        assert_eq!(errors[0].code(), "A");
        assert_eq!(errors[1].code(), "B");
    }

    //
    // Verify duplicate IDs are rejected across registry scopes.
    #[test]
    fn registry_rejects_duplicate_rule_ids_across_scopes() {
        let mut registry = ValidationRuleRegistry::new();

        registry
            .register_node_rule(RuleId::new("same-id"), Box::new(|_| Vec::new()))
            .expect("first rule should register");

        let duplicate = registry
            .register_relationship_rule(RuleId::new("same-id"), Box::new(|_| Vec::new()))
            .expect_err("duplicate should be rejected");

        assert!(matches!(
        duplicate,
        GraphError::InvalidPropertyValue(message)
        if message == "validation rule already registered: same-id"
        ));
    }

    //
    // Verify tracked constructor stores lifecycle metadata required by PRD 10.1.
    #[test]
    fn validation_error_record_can_store_tracking_metadata() {
        let error_id = ValidationErrorId::new("validation-error--1")
            .expect("validation error ID should be valid");
        let created_at =
            TemporalTimestamp::new("2026-07-06T14:33:45Z").expect("timestamp should be valid");
        let transaction_id =
            TransactionId::new("transaction--193").expect("transaction ID should be valid");
        let tracking = ValidationTrackingMetadata::new(
            error_id.clone(),
            ValidationErrorStatus::Open,
            created_at.clone(),
            transaction_id.clone(),
        );

        let record = ValidationErrorRecord::new_tracked(
            tracking,
            "VALIDATION_RULE_FAILED",
            ValidationErrorSeverity::Error,
            "required field is missing",
            ValidationTarget::node("node--1"),
        );

        assert_eq!(record.error_id(), &error_id);
        assert_eq!(record.status(), ValidationErrorStatus::Open);
        assert_eq!(record.created_at(), Some(&created_at));
        assert_eq!(record.transaction_id(), Some(&transaction_id));
    }

    //
    // Verify status lifecycle can be transitioned without mutating identity fields.
    #[test]
    fn validation_error_record_status_transition_keeps_identity() {
        let error_id = ValidationErrorId::new("validation-error--2")
            .expect("validation error ID should be valid");
        let created_at =
            TemporalTimestamp::new("2026-07-06T14:33:45Z").expect("timestamp should be valid");
        let transaction_id =
            TransactionId::new("transaction--194").expect("transaction ID should be valid");
        let tracking = ValidationTrackingMetadata::new(
            error_id.clone(),
            ValidationErrorStatus::Open,
            created_at.clone(),
            transaction_id.clone(),
        );

        let record = ValidationErrorRecord::new_tracked(
            tracking,
            "VALIDATION_RULE_FAILED",
            ValidationErrorSeverity::Error,
            "required field is missing",
            ValidationTarget::relationship("relationship--1"),
        )
        .with_status(ValidationErrorStatus::Resolved);

        assert_eq!(record.error_id(), &error_id);
        assert_eq!(record.status(), ValidationErrorStatus::Resolved);
        assert_eq!(record.created_at(), Some(&created_at));
        assert_eq!(record.transaction_id(), Some(&transaction_id));
    }
}

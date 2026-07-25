// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

#![warn(missing_docs)]

//! Shared OpenCTI authorization contract.
//!
//! The adapter extracts [`AccessMetadata`], request boundaries compile an
//! [`AccessContext`] into [`OpenCtiAccessPolicy`], and storage plus execution
//! evaluate the same policy before materializing a payload or exposing a graph
//! edge. Keeping the contract in a dependency-leaf crate prevents semantic
//! drift between persistent index selection and in-memory verification.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Authorization facts propagated without transport credentials.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessContext {
    /// Stable caller identity.
    pub subject_id: String,
    /// Organizations visible to the caller.
    pub organization_ids: Vec<String>,
    /// Markings visible to the caller.
    pub marking_ids: Vec<String>,
    /// Optional tenant boundary.
    pub tenant_id: Option<String>,
    /// Provider-neutral role names.
    pub roles: Vec<String>,
    /// Negotiated extension attributes, including policy version and grants.
    pub attributes: BTreeMap<String, String>,
}

/// Access-policy inputs carried by an OpenCTI node or relationship.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AccessMetadata {
    /// Object markings.
    pub marking_ids: Vec<String>,
    /// Organizations granted access.
    pub organization_ids: Vec<String>,
    /// Authorized member documents, including group restrictions and rights.
    pub authorized_members: Vec<Value>,
    /// Tenant boundaries.
    pub tenant_ids: Vec<String>,
    /// Creator identifiers.
    pub creator_ids: Vec<String>,
    /// Owner identifiers.
    pub owner_ids: Vec<String>,
    /// Provider-neutral sharing-policy document.
    pub sharing_policy: Option<Value>,
    /// OpenCTI authority identifiers.
    pub authorized_authorities: Vec<String>,
}

impl AccessMetadata {
    /// Decode canonical `opencti.access` JSON without consulting payload fields.
    pub fn from_value(value: &Value) -> Result<Self, AccessPolicyError> {
        serde_json::from_value(value.clone()).map_err(|error| AccessPolicyError::InvalidMetadata {
            reason: error.to_string(),
        })
    }
}

/// Stable reason for an authorization decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessDecisionReason {
    /// System role bypass.
    System,
    /// The record carries no access restriction.
    Unrestricted,
    /// Every marking and tenant constraint matched and organization access won.
    Organization,
    /// An authorized-member exception granted access.
    AuthorizedMember,
    /// The caller created the record.
    Creator,
    /// The caller owns the record.
    Owner,
    /// A sharing-policy grant matched.
    SharingPolicy,
    /// An explicit sharing-policy deny matched the caller.
    SharingDenied,
    /// An OpenCTI authority grant matched.
    Authority,
    /// One or more required markings are missing.
    MissingMarking,
    /// The record tenant does not match the caller tenant.
    TenantMismatch,
    /// No identity-scoped grant matched.
    IdentityMismatch,
    /// The access metadata could not be evaluated safely.
    InvalidMetadata,
    /// The request completed under policy pushdown without a record-specific grant.
    PolicyApplied,
}

/// Typed policy outcome used by selection, execution, and audit boundaries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessDecision {
    allowed: bool,
    reason: AccessDecisionReason,
}

impl AccessDecision {
    /// Whether the record or relationship may be observed.
    pub const fn allowed(self) -> bool {
        self.allowed
    }

    /// Stable reason safe to include in an audit event.
    pub const fn reason(self) -> AccessDecisionReason {
        self.reason
    }
}

/// Invalid access context or metadata.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum AccessPolicyError {
    /// The request context is incomplete.
    #[error("invalid access context: {reason}")]
    InvalidContext {
        /// Safe reason.
        reason: String,
    },
    /// Canonical access metadata is malformed.
    #[error("invalid access metadata: {reason}")]
    InvalidMetadata {
        /// Safe reason.
        reason: String,
    },
}

/// Request-scoped compiled OpenCTI policy.
#[derive(Clone, Debug)]
pub struct OpenCtiAccessPolicy {
    context: AccessContext,
    organizations: BTreeSet<String>,
    markings: BTreeSet<String>,
    identity_grants: BTreeSet<String>,
    authority_ids: BTreeSet<String>,
    sharing_grants: BTreeSet<String>,
    fingerprint: String,
}

impl OpenCtiAccessPolicy {
    /// Validate and normalize one request context before candidate selection.
    pub fn compile(context: &AccessContext) -> Result<Self, AccessPolicyError> {
        let mut normalized = context.clone();
        normalized.subject_id = normalized.subject_id.trim().to_owned();
        normalized.tenant_id = normalized
            .tenant_id
            .map(|tenant| tenant.trim().to_owned())
            .filter(|tenant| !tenant.is_empty());
        normalize(&mut normalized.organization_ids);
        normalize(&mut normalized.marking_ids);
        normalize(&mut normalized.roles);
        normalize_attribute(&mut normalized, "policy_version");
        for key in ["member_ids", "authority_ids", "sharing_grants"] {
            normalize_attribute_list(&mut normalized, key);
        }
        if normalized.subject_id.is_empty() && !normalized.roles.iter().any(|role| role == "system")
        {
            return Err(AccessPolicyError::InvalidContext {
                reason: "subject_id is required outside the system role".to_owned(),
            });
        }
        let organizations = normalized.organization_ids.iter().cloned().collect();
        let markings = normalized.marking_ids.iter().cloned().collect();
        let mut identity_grants = BTreeSet::from([normalized.subject_id.clone()]);
        identity_grants.extend(normalized.organization_ids.iter().cloned());
        identity_grants.extend(normalized.roles.iter().cloned());
        identity_grants.extend(attribute_values(&normalized, "member_ids"));
        let authority_ids = attribute_values(&normalized, "authority_ids");
        let sharing_grants = attribute_values(&normalized, "sharing_grants");
        let fingerprint = policy_fingerprint(&normalized);
        Ok(Self {
            context: normalized,
            organizations,
            markings,
            identity_grants,
            authority_ids,
            sharing_grants,
            fingerprint,
        })
    }

    /// Evaluate one compact access document.
    pub fn evaluate(&self, metadata: &AccessMetadata) -> AccessDecision {
        if self.context.roles.iter().any(|role| role == "system") {
            return decision(true, AccessDecisionReason::System);
        }
        if metadata
            .marking_ids
            .iter()
            .any(|marking| !self.markings.contains(marking))
        {
            return decision(false, AccessDecisionReason::MissingMarking);
        }
        if !metadata.tenant_ids.is_empty()
            && self
                .context
                .tenant_id
                .as_ref()
                .is_none_or(|tenant| !metadata.tenant_ids.contains(tenant))
        {
            return decision(false, AccessDecisionReason::TenantMismatch);
        }
        if metadata.sharing_policy.as_ref().is_some_and(|sharing| {
            ["deny", "denied", "denied_members"]
                .iter()
                .filter_map(|key| sharing.get(*key))
                .any(|denied| value_matches_any(denied, &self.identity_grants))
        }) {
            return decision(false, AccessDecisionReason::SharingDenied);
        }
        if metadata
            .owner_ids
            .iter()
            .any(|owner| self.identity_grants.contains(owner))
        {
            return decision(true, AccessDecisionReason::Owner);
        }
        if metadata
            .creator_ids
            .iter()
            .any(|creator| self.identity_grants.contains(creator))
        {
            return decision(true, AccessDecisionReason::Creator);
        }
        if metadata
            .authorized_members
            .iter()
            .any(|member| member_grants(member, &self.identity_grants))
        {
            return decision(true, AccessDecisionReason::AuthorizedMember);
        }
        if metadata
            .authorized_authorities
            .iter()
            .any(|authority| self.authority_ids.contains(authority))
        {
            return decision(true, AccessDecisionReason::Authority);
        }
        if metadata.sharing_policy.as_ref().is_some_and(|sharing| {
            sharing.get("public").and_then(Value::as_bool) == Some(true)
                || sharing
                    .get("grants")
                    .is_some_and(|grants| value_matches_any(grants, &self.sharing_grants))
        }) {
            return decision(true, AccessDecisionReason::SharingPolicy);
        }
        if metadata
            .organization_ids
            .iter()
            .any(|organization| self.organizations.contains(organization))
        {
            return decision(true, AccessDecisionReason::Organization);
        }
        let identity_restricted = !metadata.organization_ids.is_empty()
            || !metadata.authorized_members.is_empty()
            || !metadata.creator_ids.is_empty()
            || !metadata.owner_ids.is_empty()
            || metadata.sharing_policy.is_some()
            || !metadata.authorized_authorities.is_empty();
        if identity_restricted {
            decision(false, AccessDecisionReason::IdentityMismatch)
        } else {
            decision(true, AccessDecisionReason::Unrestricted)
        }
    }

    /// Evaluate optional canonical access JSON, denying malformed metadata.
    pub fn evaluate_value(&self, value: Option<&Value>) -> AccessDecision {
        match value {
            None => self.evaluate(&AccessMetadata::default()),
            Some(value) => AccessMetadata::from_value(value)
                .map(|metadata| self.evaluate(&metadata))
                .unwrap_or_else(|_| decision(false, AccessDecisionReason::InvalidMetadata)),
        }
    }

    /// Stable policy version used by cursors, caches, and audit events.
    pub fn policy_version(&self) -> &str {
        self.context
            .attributes
            .get("policy_version")
            .map(String::as_str)
            .unwrap_or("unversioned")
    }

    /// Stable request-policy fingerprint used to scope cached state.
    pub fn fingerprint(&self) -> String {
        self.fingerprint.clone()
    }

    /// Whether this compiled context carries the explicit system-role bypass.
    pub fn is_system(&self) -> bool {
        self.context.roles.iter().any(|role| role == "system")
    }
}

fn decision(allowed: bool, reason: AccessDecisionReason) -> AccessDecision {
    AccessDecision { allowed, reason }
}

fn normalize(values: &mut Vec<String>) {
    for value in values.iter_mut() {
        *value = value.trim().to_owned();
    }
    values.retain(|value| !value.is_empty());
    values.sort();
    values.dedup();
}

fn attribute_values(context: &AccessContext, key: &str) -> BTreeSet<String> {
    context
        .attributes
        .get(key)
        .into_iter()
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}

fn normalize_attribute(context: &mut AccessContext, key: &str) {
    let normalized = context
        .attributes
        .get(key)
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    match normalized {
        Some(value) => {
            context.attributes.insert(key.to_owned(), value);
        }
        None => {
            context.attributes.remove(key);
        }
    }
}

fn normalize_attribute_list(context: &mut AccessContext, key: &str) {
    let normalized = attribute_values(context, key)
        .into_iter()
        .collect::<Vec<_>>()
        .join(",");
    if normalized.is_empty() {
        context.attributes.remove(key);
    } else {
        context.attributes.insert(key.to_owned(), normalized);
    }
}

fn value_matches_any(value: &Value, expected: &BTreeSet<String>) -> bool {
    match value {
        Value::String(value) => expected.contains(value),
        Value::Array(values) => values
            .iter()
            .any(|value| value_matches_any(value, expected)),
        Value::Object(values) => values
            .values()
            .any(|value| value_matches_any(value, expected)),
        _ => false,
    }
}

fn member_grants(value: &Value, expected: &BTreeSet<String>) -> bool {
    match value {
        Value::String(value) => expected.contains(value),
        Value::Array(values) => values.iter().any(|value| member_grants(value, expected)),
        Value::Object(values) => {
            let explicitly_denied = ["allowed", "can_access", "access_right"]
                .iter()
                .filter_map(|key| values.get(*key))
                .any(|value| value == &Value::Bool(false))
                || values
                    .get("right")
                    .and_then(Value::as_str)
                    .is_some_and(|right| {
                        matches!(
                            right.to_ascii_lowercase().as_str(),
                            "deny" | "denied" | "none"
                        )
                    });
            !explicitly_denied
                && [
                    "id",
                    "member_id",
                    "user_id",
                    "organization_id",
                    "group_id",
                    "role",
                    "value",
                ]
                .iter()
                .filter_map(|key| values.get(*key))
                .any(|value| value_matches_any(value, expected))
        }
        _ => false,
    }
}

fn policy_fingerprint(context: &AccessContext) -> String {
    let bytes = serde_json::to_vec(context).unwrap_or_default();
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

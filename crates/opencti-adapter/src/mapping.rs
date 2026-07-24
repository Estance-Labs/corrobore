// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! Versioned OpenCTI record mapping contract.

use std::collections::BTreeSet;

use chrono::DateTime;
use graph_core::{Node, NodeId, NodeInput, PropertyValue, Relationship, RelationshipInput};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::ProjectionRecord;

/// Version of the OpenCTI-to-Corrobore canonical mapping.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MappingVersion {
    /// Breaking mapping generation.
    pub major: u16,
    /// Backward-compatible mapping generation.
    pub minor: u16,
}

impl MappingVersion {
    /// Mapping implemented for issue #40.
    pub const CURRENT: Self = Self { major: 1, minor: 0 };
}

/// Canonical object or relationship family from the pinned OpenCTI schema.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordFamily {
    /// STIX domain object.
    StixDomainObject,
    /// STIX cyber observable.
    StixCyberObservable,
    /// STIX meta object.
    StixMetaObject,
    /// OpenCTI internal object.
    InternalObject,
    /// STIX core relationship.
    StixCoreRelationship,
    /// STIX reference relationship.
    StixRefRelationship,
    /// STIX sighting relationship.
    StixSightingRelationship,
    /// OpenCTI internal relationship.
    InternalRelationship,
    /// Forward-compatible object whose type is not in the pinned schema.
    UnknownObject,
    /// Forward-compatible relationship whose type is not in the pinned schema.
    UnknownRelationship,
}

impl RecordFamily {
    /// Stable family name stored in canonical graph metadata.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::StixDomainObject => "stix_domain_object",
            Self::StixCyberObservable => "stix_cyber_observable",
            Self::StixMetaObject => "stix_meta_object",
            Self::InternalObject => "internal_object",
            Self::StixCoreRelationship => "stix_core_relationship",
            Self::StixRefRelationship => "stix_ref_relationship",
            Self::StixSightingRelationship => "stix_sighting_relationship",
            Self::InternalRelationship => "internal_relationship",
            Self::UnknownObject => "unknown_object",
            Self::UnknownRelationship => "unknown_relationship",
        }
    }
}

/// Graph record category used by identifier projections.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordKind {
    /// Node-like OpenCTI record.
    Object,
    /// Edge-like OpenCTI record.
    Relationship,
}

/// Stable canonical record reference independent from graph-core allocation.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RecordRef {
    pub(crate) kind: RecordKind,
    pub(crate) canonical_id: String,
}

impl RecordRef {
    /// Record category.
    pub const fn kind(&self) -> RecordKind {
        self.kind
    }

    /// Canonical OpenCTI record identity.
    pub fn canonical_id(&self) -> &str {
        self.canonical_id.as_str()
    }
}

/// Identifier namespaces supported by the OpenCTI compatibility projection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentifierKind {
    /// OpenCTI internal identifier.
    Internal,
    /// Canonical standard identifier.
    Standard,
    /// Current or historical STIX identifier.
    Stix,
    /// Indexed external-reference identifier.
    External,
    /// Searchable alias.
    Alias,
    /// Merge or deduplication identity.
    Deduplication,
}

/// Typed identifier key.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Identifier {
    pub(crate) kind: IdentifierKind,
    pub(crate) value: String,
}

impl Identifier {
    /// Build a non-empty typed identifier.
    pub fn new(kind: IdentifierKind, value: impl Into<String>) -> Result<Self, MappingError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(MappingError::InvalidField {
                field: format!("{kind:?}_identifier"),
                reason: "identifier cannot be empty".to_owned(),
            });
        }
        Ok(Self { kind, value })
    }

    /// Identifier namespace.
    pub const fn kind(&self) -> IdentifierKind {
        self.kind
    }

    /// Exact identifier value.
    pub fn value(&self) -> &str {
        self.value.as_str()
    }
}

/// One reference-bearing field extracted without enforcing authorization.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reference {
    /// Source field name.
    pub field: String,
    /// Referenced identifier.
    pub target: String,
}

/// Access-policy inputs carried by an OpenCTI record.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
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

/// Semantically typed timestamp fields preserved from OpenCTI.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenCtiTimestamps {
    /// STIX creation timestamp.
    pub created: Option<String>,
    /// STIX modification timestamp.
    pub modified: Option<String>,
    /// OpenCTI creation timestamp.
    pub created_at: Option<String>,
    /// OpenCTI update timestamp.
    pub updated_at: Option<String>,
    /// OpenCTI refresh timestamp.
    pub refreshed_at: Option<String>,
    /// Validity start.
    pub valid_from: Option<String>,
    /// Validity end.
    pub valid_until: Option<String>,
    /// First observation.
    pub first_seen: Option<String>,
    /// Last observation.
    pub last_seen: Option<String>,
}

/// Provenance values preserved separately from authorization inputs.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Provenance {
    /// Embedded external-reference documents.
    pub external_references: Vec<Value>,
    /// Source reference identifiers.
    pub source_references: Vec<String>,
    /// Migration metadata copied from the canonical record.
    pub migration: Option<Value>,
}

/// Mapped node-like OpenCTI record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MappedObject {
    pub(crate) record_ref: RecordRef,
    pub(crate) family: RecordFamily,
    pub(crate) entity_type: String,
    pub(crate) raw: Value,
    pub(crate) identifiers: BTreeSet<Identifier>,
    pub(crate) references: Vec<Reference>,
    pub(crate) access: AccessMetadata,
    pub(crate) timestamps: OpenCtiTimestamps,
    pub(crate) provenance: Provenance,
    pub(crate) mapping_version: MappingVersion,
}

impl MappedObject {
    /// Convert the adapter record into domain-neutral graph input.
    pub fn to_node_input(&self) -> NodeInput {
        let mut labels = vec![
            "OpenCtiObject".to_owned(),
            family_label(self.family).to_owned(),
            entity_type_label(&self.entity_type),
        ];
        labels.dedup();

        let mut input = NodeInput::new(labels);
        for (key, value) in canonical_properties(
            &self.record_ref,
            self.family,
            &self.raw,
            &self.identifiers,
            &self.references,
            &self.access,
            &self.timestamps,
            &self.provenance,
            self.mapping_version,
        ) {
            input = input.with_property(key, value);
        }
        input.with_property(
            "opencti.entity_type",
            PropertyValue::String(self.entity_type.clone()),
        )
    }
}

/// Mapped edge-like OpenCTI record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MappedRelationship {
    pub(crate) record_ref: RecordRef,
    pub(crate) family: RecordFamily,
    pub(crate) relationship_type: String,
    pub(crate) source_ref: String,
    pub(crate) target_ref: String,
    pub(crate) raw: Value,
    pub(crate) identifiers: BTreeSet<Identifier>,
    pub(crate) references: Vec<Reference>,
    pub(crate) access: AccessMetadata,
    pub(crate) timestamps: OpenCtiTimestamps,
    pub(crate) provenance: Provenance,
    pub(crate) mapping_version: MappingVersion,
}

impl MappedRelationship {
    /// Relationship semantic type.
    pub fn relationship_type(&self) -> &str {
        self.relationship_type.as_str()
    }

    /// Original source identifier.
    pub fn source_ref(&self) -> &str {
        self.source_ref.as_str()
    }

    /// Original target identifier.
    pub fn target_ref(&self) -> &str {
        self.target_ref.as_str()
    }

    /// Original lossless relationship body.
    pub const fn raw(&self) -> &Value {
        &self.raw
    }

    /// Access-policy inputs attached to the relationship.
    pub const fn access(&self) -> &AccessMetadata {
        &self.access
    }

    /// Convert the adapter record into domain-neutral graph input after endpoint
    /// resolution by the caller.
    pub fn to_relationship_input(
        &self,
        source: NodeId,
        target: NodeId,
    ) -> Result<RelationshipInput, MappingError> {
        let mut input = RelationshipInput::new(source, self.relationship_type.clone(), target)
            .map_err(|error| MappingError::Graph(error.to_string()))?;
        for (key, value) in canonical_properties(
            &self.record_ref,
            self.family,
            &self.raw,
            &self.identifiers,
            &self.references,
            &self.access,
            &self.timestamps,
            &self.provenance,
            self.mapping_version,
        ) {
            input = input.with_property(key, value);
        }
        Ok(input
            .with_property(
                "opencti.relationship_type",
                PropertyValue::String(self.relationship_type.clone()),
            )
            .with_property(
                "opencti.source_ref",
                PropertyValue::String(self.source_ref.clone()),
            )
            .with_property(
                "opencti.target_ref",
                PropertyValue::String(self.target_ref.clone()),
            ))
    }
}

/// Lossless canonical result for either record category.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "record_kind", rename_all = "snake_case")]
pub enum MappedRecord {
    /// Node-like record.
    Object(MappedObject),
    /// Edge-like record.
    Relationship(MappedRelationship),
}

impl MappedRecord {
    /// Original lossless record body.
    pub fn raw(&self) -> &Value {
        match self {
            Self::Object(object) => &object.raw,
            Self::Relationship(relationship) => &relationship.raw,
        }
    }

    /// Pinned or generic record family.
    pub fn family(&self) -> RecordFamily {
        match self {
            Self::Object(object) => object.family,
            Self::Relationship(relationship) => relationship.family,
        }
    }

    /// Mapping version stored with canonical data.
    pub fn mapping_version(&self) -> MappingVersion {
        match self {
            Self::Object(object) => object.mapping_version,
            Self::Relationship(relationship) => relationship.mapping_version,
        }
    }

    /// Extracted identifier keys.
    pub fn identifiers(&self) -> &BTreeSet<Identifier> {
        match self {
            Self::Object(object) => &object.identifiers,
            Self::Relationship(relationship) => &relationship.identifiers,
        }
    }

    /// Extracted access-policy inputs.
    pub fn access(&self) -> &AccessMetadata {
        match self {
            Self::Object(object) => &object.access,
            Self::Relationship(relationship) => &relationship.access,
        }
    }

    /// Extracted reference fields.
    pub fn references(&self) -> &[Reference] {
        match self {
            Self::Object(object) => object.references.as_slice(),
            Self::Relationship(relationship) => relationship.references.as_slice(),
        }
    }

    /// Extracted provenance.
    pub fn provenance(&self) -> &Provenance {
        match self {
            Self::Object(object) => &object.provenance,
            Self::Relationship(relationship) => &relationship.provenance,
        }
    }

    /// Extracted timestamp fields.
    pub fn timestamps(&self) -> &OpenCtiTimestamps {
        match self {
            Self::Object(object) => &object.timestamps,
            Self::Relationship(relationship) => &relationship.timestamps,
        }
    }

    /// Stable canonical record reference.
    pub fn record_ref(&self) -> RecordRef {
        match self {
            Self::Object(object) => object.record_ref.clone(),
            Self::Relationship(relationship) => relationship.record_ref.clone(),
        }
    }

    /// Build one versioned identifier-projection input.
    pub fn projection_record(&self, revision: u64) -> ProjectionRecord {
        ProjectionRecord {
            record_ref: self.record_ref(),
            revision,
            identifiers: self.identifiers().clone(),
            deleted: false,
        }
    }

    /// Borrow the object variant.
    pub fn as_object(&self) -> Option<&MappedObject> {
        match self {
            Self::Object(object) => Some(object),
            Self::Relationship(_) => None,
        }
    }

    /// Borrow the relationship variant.
    pub fn as_relationship(&self) -> Option<&MappedRelationship> {
        match self {
            Self::Object(_) => None,
            Self::Relationship(relationship) => Some(relationship),
        }
    }
}

/// Stateless OpenCTI compatibility adapter for one pinned source release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenCtiAdapter {
    opencti_version: String,
    mapping_version: MappingVersion,
}

impl OpenCtiAdapter {
    /// Adapter for the source lock committed by issue #38.
    pub fn pinned() -> Self {
        Self {
            opencti_version: "7.260722.0".to_owned(),
            mapping_version: MappingVersion::CURRENT,
        }
    }

    /// OpenCTI source version supported by this adapter instance.
    pub fn opencti_version(&self) -> &str {
        self.opencti_version.as_str()
    }

    /// Map one OpenCTI object or relationship without losing unknown fields.
    pub fn map(&self, raw: Value) -> Result<MappedRecord, MappingError> {
        let object = raw.as_object().ok_or_else(|| MappingError::InvalidField {
            field: "record".to_owned(),
            reason: "record must be a JSON object".to_owned(),
        })?;
        let entity_type = non_empty_string(object, "entity_type")
            .or_else(|| non_empty_string(object, "type"))
            .ok_or(MappingError::MissingRequiredField {
                field: "entity_type or type",
            })?;
        let canonical_id = non_empty_string(object, "internal_id")
            .or_else(|| non_empty_string(object, "id"))
            .ok_or(MappingError::MissingRequiredField {
                field: "internal_id or id",
            })?;
        let relationship = is_relationship_record(object);
        let family = classify_family(object, &entity_type, relationship);
        let record_ref = RecordRef {
            kind: if relationship {
                RecordKind::Relationship
            } else {
                RecordKind::Object
            },
            canonical_id,
        };
        let identifiers = extract_identifiers(object, family)?;
        let references = extract_references(object);
        let access = extract_access(object);
        let timestamps = extract_timestamps(object)?;
        let provenance = extract_provenance(object);

        if relationship {
            let relationship_type = non_empty_string(object, "relationship_type")
                .or_else(|| {
                    let stix_type = non_empty_string(object, "type")?;
                    (stix_type == "sighting").then_some(stix_type)
                })
                .or_else(|| {
                    (!is_abstract_relationship_type(&entity_type)).then_some(entity_type.clone())
                })
                .ok_or(MappingError::MissingRequiredField {
                    field: "relationship_type",
                })?;
            let source_ref = extract_endpoint(
                object,
                &["source_ref", "from", "from_id", "sighting_of_ref"],
            )
            .ok_or(MappingError::MissingRequiredField {
                field: "source_ref",
            })?;
            let target_ref =
                extract_endpoint(object, &["target_ref", "to", "to_id", "where_sighted_refs"])
                    .ok_or(MappingError::MissingRequiredField {
                        field: "target_ref",
                    })?;
            Ok(MappedRecord::Relationship(MappedRelationship {
                record_ref,
                family,
                relationship_type,
                source_ref,
                target_ref,
                raw,
                identifiers,
                references,
                access,
                timestamps,
                provenance,
                mapping_version: self.mapping_version,
            }))
        } else {
            Ok(MappedRecord::Object(MappedObject {
                record_ref,
                family,
                entity_type,
                raw,
                identifiers,
                references,
                access,
                timestamps,
                provenance,
                mapping_version: self.mapping_version,
            }))
        }
    }

    /// Restore a mapped OpenCTI object from a generic graph node.
    pub fn restore_node(&self, node: &Node) -> Result<MappedRecord, MappingError> {
        let raw = graph_raw(node.property("opencti.raw"))?;
        let mapped = self.map(raw)?;
        if !matches!(mapped, MappedRecord::Object(_)) {
            return Err(MappingError::InvalidGraphPayload {
                reason: "node payload maps to a relationship".to_owned(),
            });
        }
        validate_graph_metadata(
            node.property("opencti.mapping_version"),
            node.property("opencti.family"),
            node.property("opencti.canonical_id"),
            &mapped,
        )?;
        Ok(mapped)
    }

    /// Restore a mapped OpenCTI relationship from a generic graph edge.
    pub fn restore_relationship(
        &self,
        relationship: &Relationship,
    ) -> Result<MappedRecord, MappingError> {
        let raw = graph_raw(relationship.property("opencti.raw"))?;
        let mapped = self.map(raw)?;
        if !matches!(mapped, MappedRecord::Relationship(_)) {
            return Err(MappingError::InvalidGraphPayload {
                reason: "relationship payload maps to an object".to_owned(),
            });
        }
        validate_graph_metadata(
            relationship.property("opencti.mapping_version"),
            relationship.property("opencti.family"),
            relationship.property("opencti.canonical_id"),
            &mapped,
        )?;
        Ok(mapped)
    }
}

const STIX_DOMAIN_TYPES: &[&str] = &[
    "attack-pattern",
    "campaign",
    "case-incident",
    "case-rfi",
    "case-rft",
    "course-of-action",
    "data-component",
    "data-source",
    "feedback",
    "grouping",
    "identity",
    "incident",
    "individual",
    "indicator",
    "infrastructure",
    "intrusion-set",
    "location",
    "malware",
    "malware-analysis",
    "narrative",
    "note",
    "observed-data",
    "opinion",
    "organization",
    "report",
    "sector",
    "security-coverage",
    "security-platform",
    "system",
    "task",
    "threat-actor",
    "threat-actor-group",
    "threat-actor-individual",
    "tool",
    "vulnerability",
];

const STIX_OBSERVABLE_TYPES: &[&str] = &[
    "ai-prompt",
    "artifact",
    "autonomous-system",
    "bank-account",
    "credential",
    "cryptocurrency-wallet",
    "cryptographic-key",
    "directory",
    "domain-name",
    "email-addr",
    "email-message",
    "email-mime-part-type",
    "hostname",
    "iccid",
    "imei",
    "imsi",
    "ipv4-addr",
    "ipv6-addr",
    "mac-addr",
    "media-content",
    "mutex",
    "network-traffic",
    "payment-card",
    "persona",
    "phone-number",
    "process",
    "software",
    "ssh-key",
    "stixfile",
    "text",
    "tracking-number",
    "url",
    "user-account",
    "user-agent",
    "windows-registry-key",
    "windows-registry-value-type",
    "x509-certificate",
];

const STIX_META_TYPES: &[&str] = &[
    "external-reference",
    "kill-chain-phase",
    "label",
    "marking-definition",
];

const INTERNAL_OBJECT_TYPES: &[&str] = &[
    "activity",
    "backgroundtask",
    "capability",
    "connector",
    "connectormanager",
    "customview",
    "deleteoperation",
    "draftworkspace",
    "emailtemplate",
    "exclusionlist",
    "feed",
    "finteldesign",
    "finteltemplate",
    "form",
    "group",
    "history",
    "internalfile",
    "migration-marker",
    "migrationreference",
    "migrationstatus",
    "pir",
    "pirhistory",
    "playbook",
    "publicdashboard",
    "retentionrule",
    "role",
    "rule",
    "rulemanager",
    "savedfilter",
    "settings",
    "smtpconfiguration",
    "status",
    "statustemplate",
    "streamcollection",
    "sync",
    "taxiicollection",
    "theme",
    "user",
    "work",
    "workflowdefinition",
    "workflowinstance",
    "workspace",
    "file",
];

const STIX_REF_RELATIONSHIP_TYPES: &[&str] = &[
    "created-by",
    "external-reference",
    "kill-chain-phase",
    "object",
    "object-label",
    "object-marking",
];

const INTERNAL_RELATIONSHIP_TYPES: &[&str] = &["member-of"];

fn normalized_type(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace([' ', '_'], "-")
}

fn classify_family(
    object: &serde_json::Map<String, Value>,
    entity_type: &str,
    relationship: bool,
) -> RecordFamily {
    let parent_types = string_values(object.get("parent_types"))
        .into_iter()
        .map(|value| normalized_type(&value))
        .collect::<BTreeSet<_>>();
    if relationship {
        let relationship_type =
            non_empty_string(object, "relationship_type").map(|value| normalized_type(&value));
        if parent_types.contains("stix-sighting-relationship")
            || normalized_type(entity_type) == "stix-sighting-relationship"
            || non_empty_string(object, "type").is_some_and(|value| value == "sighting")
        {
            RecordFamily::StixSightingRelationship
        } else if parent_types.contains("stix-ref-relationship")
            || parent_types.contains("stix-meta-relationship")
            || relationship_type
                .as_deref()
                .is_some_and(|value| STIX_REF_RELATIONSHIP_TYPES.contains(&value))
        {
            RecordFamily::StixRefRelationship
        } else if parent_types.contains("internal-relationship")
            || relationship_type
                .as_deref()
                .is_some_and(|value| INTERNAL_RELATIONSHIP_TYPES.contains(&value))
        {
            RecordFamily::InternalRelationship
        } else if parent_types.contains("stix-core-relationship")
            || non_empty_string(object, "type").is_some_and(|value| value == "relationship")
        {
            RecordFamily::StixCoreRelationship
        } else {
            RecordFamily::UnknownRelationship
        }
    } else if parent_types.contains("stix-domain-object") {
        RecordFamily::StixDomainObject
    } else if parent_types.contains("stix-cyber-observable") {
        RecordFamily::StixCyberObservable
    } else if parent_types.contains("stix-meta-object") {
        RecordFamily::StixMetaObject
    } else if parent_types.contains("internal-object") {
        RecordFamily::InternalObject
    } else {
        let raw_type = non_empty_string(object, "type").unwrap_or_else(|| entity_type.to_owned());
        let normalized = normalized_type(&raw_type);
        if STIX_DOMAIN_TYPES.contains(&normalized.as_str()) {
            RecordFamily::StixDomainObject
        } else if STIX_OBSERVABLE_TYPES.contains(&normalized.as_str()) {
            RecordFamily::StixCyberObservable
        } else if STIX_META_TYPES.contains(&normalized.as_str()) {
            RecordFamily::StixMetaObject
        } else if INTERNAL_OBJECT_TYPES.contains(&normalized.as_str()) {
            RecordFamily::InternalObject
        } else {
            RecordFamily::UnknownObject
        }
    }
}

fn is_relationship_record(object: &serde_json::Map<String, Value>) -> bool {
    non_empty_string(object, "base_type")
        .is_some_and(|value| value.eq_ignore_ascii_case("RELATION"))
        || non_empty_string(object, "type")
            .is_some_and(|value| matches!(value.as_str(), "relationship" | "sighting"))
        || string_values(object.get("parent_types"))
            .iter()
            .any(|value| normalized_type(value).contains("relationship"))
        || (object.contains_key("source_ref") && object.contains_key("target_ref"))
}

fn is_abstract_relationship_type(value: &str) -> bool {
    matches!(
        normalized_type(value).as_str(),
        "basic-relationship"
            | "internal-relationship"
            | "stix-relationship"
            | "stix-core-relationship"
            | "stix-ref-relationship"
            | "stix-sighting-relationship"
    )
}

fn non_empty_string(object: &serde_json::Map<String, Value>, field: &str) -> Option<String> {
    object
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn extract_endpoint(object: &serde_json::Map<String, Value>, fields: &[&str]) -> Option<String> {
    for field in fields {
        let Some(value) = object.get(*field) else {
            continue;
        };
        if let Some(value) = value
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(value.to_owned());
        }
        if let Some(first) = value.as_array().and_then(|values| values.first())
            && let Some(value) = identifier_from_value(first)
        {
            return Some(value);
        }
        if let Some(value) = identifier_from_value(value) {
            return Some(value);
        }
    }
    None
}

fn extract_identifiers(
    object: &serde_json::Map<String, Value>,
    family: RecordFamily,
) -> Result<BTreeSet<Identifier>, MappingError> {
    let mut identifiers = BTreeSet::new();
    if let Some(internal_id) = non_empty_string(object, "internal_id") {
        identifiers.insert(Identifier::new(IdentifierKind::Internal, internal_id)?);
    } else if matches!(
        family,
        RecordFamily::InternalObject | RecordFamily::InternalRelationship
    ) && let Some(id) = non_empty_string(object, "id")
    {
        identifiers.insert(Identifier::new(IdentifierKind::Internal, id)?);
    }

    if let Some(standard_id) =
        non_empty_string(object, "standard_id").or_else(|| non_empty_string(object, "id"))
    {
        identifiers.insert(Identifier::new(IdentifierKind::Standard, standard_id)?);
    }

    if !matches!(
        family,
        RecordFamily::InternalObject | RecordFamily::InternalRelationship
    ) && let Some(id) = non_empty_string(object, "id")
    {
        identifiers.insert(Identifier::new(IdentifierKind::Stix, id)?);
    }
    insert_identifiers(
        &mut identifiers,
        IdentifierKind::Stix,
        string_values(object.get("x_opencti_stix_ids")),
    )?;

    let mut external_ids = string_values(object.get("external_id"));
    for field in ["external_references", "externalReferences"] {
        if let Some(values) = object.get(field).and_then(Value::as_array) {
            for value in values {
                if let Some(external_id) = value
                    .as_object()
                    .and_then(|reference| non_empty_string(reference, "external_id"))
                {
                    external_ids.push(external_id);
                }
            }
        }
    }
    insert_identifiers(&mut identifiers, IdentifierKind::External, external_ids)?;

    let mut aliases = string_values(object.get("aliases"));
    aliases.extend(string_values(object.get("x_opencti_aliases")));
    aliases.extend(string_values(object.get("i_aliases_ids")));
    insert_identifiers(&mut identifiers, IdentifierKind::Alias, aliases)?;

    let mut deduplication = string_values(object.get("i_aliases_ids"));
    deduplication.extend(string_values(object.get("deduplication_keys")));
    deduplication.extend(string_values(object.get("x_opencti_deduplication_keys")));
    deduplication.extend(string_values(object.get("deduplication_id")));
    deduplication.extend(string_values(object.get("x_opencti_deduplication_id")));
    insert_identifiers(
        &mut identifiers,
        IdentifierKind::Deduplication,
        deduplication,
    )?;

    if identifiers.is_empty() {
        return Err(MappingError::InvalidField {
            field: "identifiers".to_owned(),
            reason: "record has no indexable identifier".to_owned(),
        });
    }
    Ok(identifiers)
}

fn insert_identifiers(
    identifiers: &mut BTreeSet<Identifier>,
    kind: IdentifierKind,
    values: Vec<String>,
) -> Result<(), MappingError> {
    for value in values {
        identifiers.insert(Identifier::new(kind, value)?);
    }
    Ok(())
}

fn string_values(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::String(value)) if !value.trim().is_empty() => {
            vec![value.trim().to_owned()]
        }
        Some(Value::Array(values)) => values.iter().filter_map(identifier_from_value).collect(),
        Some(Value::Object(_)) => value.and_then(identifier_from_value).into_iter().collect(),
        _ => Vec::new(),
    }
}

fn identifier_from_value(value: &Value) -> Option<String> {
    if let Some(value) = value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(value.to_owned());
    }
    let object = value.as_object()?;
    ["standard_id", "internal_id", "id"]
        .into_iter()
        .find_map(|field| non_empty_string(object, field))
}

fn extract_references(object: &serde_json::Map<String, Value>) -> Vec<Reference> {
    let mut references = BTreeSet::new();
    for (field, value) in object {
        let is_reference = field.ends_with("_ref")
            || field.ends_with("_refs")
            || matches!(
                field.as_str(),
                "createdBy"
                    | "objectMarking"
                    | "objectOrganization"
                    | "externalReferences"
                    | "objects"
            );
        if is_reference {
            for target in string_values(Some(value)) {
                references.insert((field.clone(), target));
            }
        }
    }
    references
        .into_iter()
        .map(|(field, target)| Reference { field, target })
        .collect()
}

fn extract_access(object: &serde_json::Map<String, Value>) -> AccessMetadata {
    let mut marking_ids = collect_fields(
        object,
        &["object_marking_refs", "objectMarking", "object_marking_ids"],
    );
    let mut organization_ids = collect_fields(
        object,
        &[
            "objectOrganization",
            "granted_refs",
            "x_opencti_organization_refs",
            "organization_ids",
        ],
    );
    let mut tenant_ids = collect_fields(
        object,
        &["x_opencti_tenant_refs", "tenant_id", "tenant_ids"],
    );
    tenant_ids.extend(collect_fields(object, &["tenants"]));
    let mut creator_ids = collect_fields(object, &["created_by_ref", "createdBy", "creator_id"]);
    let mut owner_ids = collect_fields(
        object,
        &[
            "owner_id",
            "owners",
            "owner_ids",
            "object_owner_refs",
            "objectOwner",
        ],
    );
    for values in [
        &mut marking_ids,
        &mut organization_ids,
        &mut tenant_ids,
        &mut creator_ids,
        &mut owner_ids,
    ] {
        values.sort();
        values.dedup();
    }
    let authorized_members = ["authorized_members", "restricted_members"]
        .into_iter()
        .flat_map(|field| match object.get(field) {
            Some(Value::Array(values)) => values.clone(),
            Some(value) => vec![value.clone()],
            None => Vec::new(),
        })
        .collect();
    let sharing_policy = object
        .get("sharing_policy")
        .or_else(|| object.get("x_opencti_sharing_policy"))
        .cloned();
    let mut authorized_authorities = string_values(object.get("authorized_authorities"));
    authorized_authorities.sort();
    authorized_authorities.dedup();
    AccessMetadata {
        marking_ids,
        organization_ids,
        authorized_members,
        tenant_ids,
        creator_ids,
        owner_ids,
        sharing_policy,
        authorized_authorities,
    }
}

fn collect_fields(object: &serde_json::Map<String, Value>, fields: &[&str]) -> Vec<String> {
    fields
        .iter()
        .flat_map(|field| string_values(object.get(*field)))
        .collect()
}

fn extract_timestamps(
    object: &serde_json::Map<String, Value>,
) -> Result<OpenCtiTimestamps, MappingError> {
    Ok(OpenCtiTimestamps {
        created: timestamp(object, "created")?,
        modified: timestamp(object, "modified")?,
        created_at: timestamp(object, "created_at")?,
        updated_at: timestamp(object, "updated_at")?,
        refreshed_at: timestamp(object, "refreshed_at")?,
        valid_from: timestamp(object, "valid_from")?,
        valid_until: timestamp(object, "valid_until")?,
        first_seen: timestamp(object, "first_seen")?,
        last_seen: timestamp(object, "last_seen")?,
    })
}

fn timestamp(
    object: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<Option<String>, MappingError> {
    let Some(value) = object.get(field) else {
        return Ok(None);
    };
    let value = value.as_str().ok_or_else(|| MappingError::InvalidField {
        field: field.to_owned(),
        reason: "timestamp must be a string".to_owned(),
    })?;
    DateTime::parse_from_rfc3339(value).map_err(|error| MappingError::InvalidField {
        field: field.to_owned(),
        reason: format!("timestamp is not RFC 3339: {error}"),
    })?;
    Ok(Some(value.to_owned()))
}

fn extract_provenance(object: &serde_json::Map<String, Value>) -> Provenance {
    let external_references = ["external_references", "externalReferences"]
        .into_iter()
        .find_map(|field| object.get(field).and_then(Value::as_array).cloned())
        .unwrap_or_default();
    let source_references = collect_fields(
        object,
        &["source_refs", "source_reference", "source_references"],
    );
    let migration_keys = [
        "mapping_version",
        "schema_version",
        "from_schema",
        "to_schema",
        "migration_id",
        "migration_metadata",
    ];
    let migration = migration_keys
        .into_iter()
        .filter_map(|key| {
            object
                .get(key)
                .cloned()
                .map(|value| (key.to_owned(), value))
        })
        .collect::<serde_json::Map<_, _>>();
    Provenance {
        external_references,
        source_references,
        migration: (!migration.is_empty()).then_some(Value::Object(migration)),
    }
}

#[allow(clippy::too_many_arguments)]
fn canonical_properties(
    record_ref: &RecordRef,
    family: RecordFamily,
    raw: &Value,
    identifiers: &BTreeSet<Identifier>,
    references: &[Reference],
    access: &AccessMetadata,
    timestamps: &OpenCtiTimestamps,
    provenance: &Provenance,
    mapping_version: MappingVersion,
) -> Vec<(String, PropertyValue)> {
    let mut properties = vec![
        (
            "opencti.canonical_id".to_owned(),
            PropertyValue::String(record_ref.canonical_id.clone()),
        ),
        (
            "opencti.family".to_owned(),
            PropertyValue::String(family.as_str().to_owned()),
        ),
        (
            "opencti.mapping_version".to_owned(),
            PropertyValue::String(format!(
                "{}.{}",
                mapping_version.major, mapping_version.minor
            )),
        ),
        ("opencti.raw".to_owned(), PropertyValue::Json(raw.clone())),
        (
            "opencti.identifiers".to_owned(),
            PropertyValue::Json(
                serde_json::to_value(identifiers)
                    .expect("serializing identifier metadata cannot fail"),
            ),
        ),
        (
            "opencti.references".to_owned(),
            PropertyValue::Json(
                serde_json::to_value(references)
                    .expect("serializing reference metadata cannot fail"),
            ),
        ),
        (
            "opencti.access".to_owned(),
            PropertyValue::Json(
                serde_json::to_value(access).expect("serializing access metadata cannot fail"),
            ),
        ),
        (
            "opencti.timestamps".to_owned(),
            PropertyValue::Json(
                serde_json::to_value(timestamps)
                    .expect("serializing timestamp metadata cannot fail"),
            ),
        ),
        (
            "opencti.provenance".to_owned(),
            PropertyValue::Json(
                serde_json::to_value(provenance)
                    .expect("serializing provenance metadata cannot fail"),
            ),
        ),
    ];
    if let Some(object) = raw.as_object() {
        properties.extend(
            object
                .iter()
                .map(|(key, value)| (format!("opencti.field.{key}"), json_property_value(value))),
        );
    }
    properties
}

fn json_property_value(value: &Value) -> PropertyValue {
    match value {
        Value::Null => PropertyValue::Null,
        Value::Bool(value) => PropertyValue::Bool(*value),
        Value::Number(value) if value.is_i64() => {
            PropertyValue::Integer(value.as_i64().expect("checked integer"))
        }
        Value::Number(value) => {
            PropertyValue::Float(value.as_f64().expect("JSON number should convert to f64"))
        }
        Value::String(value) => PropertyValue::String(value.clone()),
        Value::Array(values) if values.iter().all(Value::is_string) => PropertyValue::StringList(
            values
                .iter()
                .map(|value| value.as_str().expect("checked string").to_owned())
                .collect(),
        ),
        Value::Array(values) if values.iter().all(Value::is_i64) => PropertyValue::IntegerList(
            values
                .iter()
                .map(|value| value.as_i64().expect("checked integer"))
                .collect(),
        ),
        Value::Array(values) if values.iter().all(Value::is_number) => PropertyValue::FloatList(
            values
                .iter()
                .map(|value| value.as_f64().expect("checked number"))
                .collect(),
        ),
        Value::Array(values) if values.iter().all(Value::is_boolean) => PropertyValue::BoolList(
            values
                .iter()
                .map(|value| value.as_bool().expect("checked boolean"))
                .collect(),
        ),
        Value::Array(_) | Value::Object(_) => PropertyValue::Json(value.clone()),
    }
}

fn family_label(family: RecordFamily) -> &'static str {
    match family {
        RecordFamily::StixDomainObject => "OpenCtiStixDomainObject",
        RecordFamily::StixCyberObservable => "OpenCtiStixCyberObservable",
        RecordFamily::StixMetaObject => "OpenCtiStixMetaObject",
        RecordFamily::InternalObject => "OpenCtiInternalObject",
        RecordFamily::UnknownObject => "OpenCtiUnknownObject",
        RecordFamily::StixCoreRelationship
        | RecordFamily::StixRefRelationship
        | RecordFamily::StixSightingRelationship
        | RecordFamily::InternalRelationship
        | RecordFamily::UnknownRelationship => "OpenCtiInvalidObjectFamily",
    }
}

fn entity_type_label(entity_type: &str) -> String {
    let suffix = entity_type
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    format!("OpenCtiType_{suffix}")
}

fn graph_raw(property: Option<&PropertyValue>) -> Result<Value, MappingError> {
    match property {
        Some(PropertyValue::Json(raw)) => Ok(raw.clone()),
        Some(_) => Err(MappingError::InvalidGraphPayload {
            reason: "opencti.raw must be typed JSON".to_owned(),
        }),
        None => Err(MappingError::InvalidGraphPayload {
            reason: "opencti.raw is missing".to_owned(),
        }),
    }
}

fn validate_graph_metadata(
    version: Option<&PropertyValue>,
    family: Option<&PropertyValue>,
    canonical_id: Option<&PropertyValue>,
    mapped: &MappedRecord,
) -> Result<(), MappingError> {
    let expected_version = format!(
        "{}.{}",
        mapped.mapping_version().major,
        mapped.mapping_version().minor
    );
    for (name, value, expected) in [
        (
            "opencti.mapping_version",
            version,
            expected_version.as_str(),
        ),
        ("opencti.family", family, mapped.family().as_str()),
        (
            "opencti.canonical_id",
            canonical_id,
            mapped.record_ref().canonical_id(),
        ),
    ] {
        match value {
            Some(PropertyValue::String(actual)) if actual == expected => {}
            Some(PropertyValue::String(actual)) => {
                return Err(MappingError::InvalidGraphPayload {
                    reason: format!("{name} is {actual}, expected {expected}"),
                });
            }
            Some(_) => {
                return Err(MappingError::InvalidGraphPayload {
                    reason: format!("{name} must be a string"),
                });
            }
            None => {
                return Err(MappingError::InvalidGraphPayload {
                    reason: format!("{name} is missing"),
                });
            }
        }
    }
    Ok(())
}

/// Stable mapping failures.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum MappingError {
    /// A required identity, type, or endpoint field is absent.
    #[error("missing required OpenCTI field {field}")]
    MissingRequiredField {
        /// Missing field.
        field: &'static str,
    },
    /// A present field has the wrong shape or an empty value.
    #[error("invalid OpenCTI field {field}: {reason}")]
    InvalidField {
        /// Invalid field.
        field: String,
        /// Safe validation detail.
        reason: String,
    },
    /// A generic graph record does not carry compatible mapping metadata.
    #[error("invalid OpenCTI graph payload: {reason}")]
    InvalidGraphPayload {
        /// Safe validation detail.
        reason: String,
    },
    /// Generic graph input construction failed.
    #[error("generic graph mapping failed: {0}")]
    Graph(String),
}

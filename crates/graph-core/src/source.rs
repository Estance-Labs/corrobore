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
//! Immutable `Source` record and store (Epic 0029, WS-A item 2).
//!
//! Module boundary:
//! this module owns the stable origin identity behind observations and
//! evidence: URI or file identity, source type, publisher, authority domain,
//! acquisition time, artifact hash, optional signature, optional parent source.
//! It does not weigh authority, cluster dependent sources, or store observed
//! payloads; those belong to WS-D and to the `Observation` record of item 3.
//!
//! Validation targets:
//! - the URI is non-blank; optional strings are non-blank when present;
//! - the artifact hash, when present, is 64 lowercase hexadecimal characters,
//!   the same rule `EvidenceInput` applies to `content_sha256`;
//! - a parent source is a different, already registered source identity;
//! - a source has no update path: an identical registration is a no-op, a
//!   changed artifact hash creates a superseding version and records a
//!   content-drift finding, any other change is a conflict error.
//!
//! Compatibility targets:
//! - legacy `EvidenceRecord`s lift into sources idempotently, keyed by their
//!   `source_ref`, and are marked `derived_from_legacy`;
//! - the graph-facing projection is additive and namespaced under `source_*`.
use serde::{Deserialize, Serialize};

use crate::{
    EvidenceRecord, EvidenceSourceType, GraphError, ImmutableRecordKind, PropertyMap,
    PropertyValue, SourceId, SourceVersionId, TemporalTimestamp, ValidationErrorRecord,
    ValidationErrorSeverity, ValidationTarget,
};

/// Validation code recorded when the artifact behind a source identity changes.
pub const SOURCE_CONTENT_DRIFT_CODE: &str = "source.content_drift";

/// Input builder for registering a source version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceInput {
    id: SourceId,
    uri: String,
    source_type: EvidenceSourceType,
    publisher: Option<String>,
    authority_domain: Option<String>,
    acquired_at: Option<TemporalTimestamp>,
    artifact_sha256: Option<String>,
    signature: Option<String>,
    parent_source: Option<SourceId>,
    #[serde(
        default,
        skip_serializing_if = "crate::SourceDependencySignals::is_empty"
    )]
    dependency_signals: crate::SourceDependencySignals,
}

impl SourceInput {
    /// Start an input from the three mandatory identity fields.
    pub fn new(id: SourceId, uri: impl Into<String>, source_type: EvidenceSourceType) -> Self {
        Self {
            id,
            uri: uri.into(),
            source_type,
            publisher: None,
            authority_domain: None,
            acquired_at: None,
            artifact_sha256: None,
            signature: None,
            parent_source: None,
            dependency_signals: crate::SourceDependencySignals::default(),
        }
    }

    /// Record explicit dependencies used by source clustering.
    pub fn with_dependency_signals(mut self, signals: crate::SourceDependencySignals) -> Self {
        self.dependency_signals = signals;
        self
    }

    /// Set the publisher or origin organisation.
    pub fn with_publisher(mut self, publisher: impl Into<String>) -> Self {
        self.publisher = Some(publisher.into());
        self
    }

    /// Set the authority domain used by later authority policies (WS-D).
    pub fn with_authority_domain(mut self, authority_domain: impl Into<String>) -> Self {
        self.authority_domain = Some(authority_domain.into());
        self
    }

    /// Set the acquisition timestamp.
    pub fn with_acquired_at(mut self, acquired_at: TemporalTimestamp) -> Self {
        self.acquired_at = Some(acquired_at);
        self
    }

    /// Set the SHA-256 digest of the acquired artifact.
    pub fn with_artifact_sha256(mut self, artifact_sha256: impl Into<String>) -> Self {
        self.artifact_sha256 = Some(artifact_sha256.into());
        self
    }

    /// Set an opaque signature or attestation reference.
    pub fn with_signature(mut self, signature: impl Into<String>) -> Self {
        self.signature = Some(signature.into());
        self
    }

    /// Set the parent source this source was derived or syndicated from.
    pub fn with_parent_source(mut self, parent_source: SourceId) -> Self {
        self.parent_source = Some(parent_source);
        self
    }

    /// Validate every field before registration.
    ///
    /// # Errors
    ///
    /// [`GraphError::InvalidPropertyValue`] naming the offending field.
    fn validate(&self) -> Result<(), GraphError> {
        self.dependency_signals.validate()?;
        if self.uri.trim().is_empty() {
            return Err(GraphError::InvalidPropertyValue(
                "source uri must not be empty".to_owned(),
            ));
        }

        for (field, value) in [
            ("publisher", &self.publisher),
            ("authority_domain", &self.authority_domain),
            ("signature", &self.signature),
        ] {
            if let Some(value) = value
                && value.trim().is_empty()
            {
                return Err(GraphError::InvalidPropertyValue(format!(
                    "source {field} must not be empty when provided"
                )));
            }
        }

        if let Some(digest) = self.artifact_sha256.as_deref()
            && !is_lowercase_sha256(digest)
        {
            return Err(GraphError::InvalidPropertyValue(
                "source artifact_sha256 must be 64 lowercase hexadecimal characters".to_owned(),
            ));
        }

        if self.parent_source.as_ref() == Some(&self.id) {
            return Err(GraphError::InvalidPropertyValue(
                "source parent_source must not reference the source itself".to_owned(),
            ));
        }

        Ok(())
    }
}

/// One immutable version of a source identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    id: SourceId,
    version_id: SourceVersionId,
    version: u64,
    uri: String,
    source_type: EvidenceSourceType,
    publisher: Option<String>,
    authority_domain: Option<String>,
    acquired_at: Option<TemporalTimestamp>,
    artifact_sha256: Option<String>,
    signature: Option<String>,
    parent_source: Option<SourceId>,
    #[serde(
        default,
        skip_serializing_if = "crate::SourceDependencySignals::is_empty"
    )]
    dependency_signals: crate::SourceDependencySignals,
    supersedes: Option<SourceVersionId>,
    derived_from_legacy: bool,
}

impl Source {
    /// Stable identity shared by every version.
    pub fn id(&self) -> &SourceId {
        &self.id
    }

    /// Identifier of this version.
    pub fn version_id(&self) -> &SourceVersionId {
        &self.version_id
    }

    /// Version number, starting at 1.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// URI or file identity.
    pub fn uri(&self) -> &str {
        self.uri.as_str()
    }

    /// Source type.
    pub fn source_type(&self) -> EvidenceSourceType {
        self.source_type
    }

    /// Publisher, when known.
    pub fn publisher(&self) -> Option<&str> {
        self.publisher.as_deref()
    }

    /// Authority domain, when known.
    pub fn authority_domain(&self) -> Option<&str> {
        self.authority_domain.as_deref()
    }

    /// Acquisition timestamp, when known.
    pub fn acquired_at(&self) -> Option<&TemporalTimestamp> {
        self.acquired_at.as_ref()
    }

    /// Artifact digest, when known.
    pub fn artifact_sha256(&self) -> Option<&str> {
        self.artifact_sha256.as_deref()
    }

    /// Signature reference, when present.
    pub fn signature(&self) -> Option<&str> {
        self.signature.as_deref()
    }

    /// Parent source, when present.
    pub fn parent_source(&self) -> Option<&SourceId> {
        self.parent_source.as_ref()
    }

    /// Version this one supersedes, when it is not the first.
    pub fn supersedes(&self) -> Option<&SourceVersionId> {
        self.supersedes.as_ref()
    }

    /// Whether this version was lifted from a legacy evidence record rather
    /// than registered explicitly.
    pub fn derived_from_legacy(&self) -> bool {
        self.derived_from_legacy
    }

    /// Project the source into additive, namespaced node properties.
    ///
    /// Keys are prefixed `source_`; optional fields are omitted rather than
    /// emitted as null.
    pub fn to_property_map(&self) -> PropertyMap {
        let mut properties = PropertyMap::new();
        let mut put = |key: &str, value: PropertyValue| {
            properties.insert(key.to_owned(), value);
        };

        put(
            "source_id",
            PropertyValue::String(self.id.as_str().to_owned()),
        );
        put(
            "source_version_id",
            PropertyValue::String(self.version_id.as_str().to_owned()),
        );
        put(
            "source_version",
            PropertyValue::Integer(i64::try_from(self.version).unwrap_or(i64::MAX)),
        );
        put("source_uri", PropertyValue::String(self.uri.clone()));
        put(
            "source_type",
            PropertyValue::String(self.source_type.as_str().to_owned()),
        );
        if let Some(publisher) = &self.publisher {
            put("source_publisher", PropertyValue::String(publisher.clone()));
        }
        if let Some(domain) = &self.authority_domain {
            put(
                "source_authority_domain",
                PropertyValue::String(domain.clone()),
            );
        }
        if let Some(acquired_at) = &self.acquired_at {
            put(
                "source_acquired_at",
                PropertyValue::String(acquired_at.as_str().to_owned()),
            );
        }
        if let Some(digest) = &self.artifact_sha256 {
            put(
                "source_artifact_sha256",
                PropertyValue::String(digest.clone()),
            );
        }
        if let Some(signature) = &self.signature {
            put("source_signature", PropertyValue::String(signature.clone()));
        }
        if let Some(parent) = &self.parent_source {
            put(
                "source_parent",
                PropertyValue::String(parent.as_str().to_owned()),
            );
        }
        if let Some(supersedes) = &self.supersedes {
            put(
                "source_supersedes",
                PropertyValue::String(supersedes.as_str().to_owned()),
            );
        }
        put(
            "source_derived_from_legacy",
            PropertyValue::Bool(self.derived_from_legacy),
        );

        properties
    }

    /// Recorded dependency signals for this immutable source version.
    pub fn dependency_signals(&self) -> &crate::SourceDependencySignals {
        &self.dependency_signals
    }

    /// Whether `input` describes the same descriptive content as this
    /// version, ignoring version bookkeeping and the legacy marker.
    fn matches_input(&self, input: &SourceInput) -> bool {
        self.id == input.id
            && self.uri == input.uri
            && self.source_type == input.source_type
            && self.publisher == input.publisher
            && self.authority_domain == input.authority_domain
            && self.acquired_at == input.acquired_at
            && self.artifact_sha256 == input.artifact_sha256
            && self.signature == input.signature
            && self.parent_source == input.parent_source
            && self.dependency_signals == input.dependency_signals
    }
}

/// Outcome of registering a source input against the store.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceRegistrationOutcome {
    /// First version of a new identity.
    Created,
    /// Identical to the current version; nothing stored.
    Unchanged,
    /// Artifact hash changed; a new version supersedes the current one.
    Superseded,
}

/// Result of one registration call.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRegistration {
    source_id: SourceId,
    version_id: SourceVersionId,
    version: u64,
    outcome: SourceRegistrationOutcome,
}

impl SourceRegistration {
    /// Source identity.
    pub fn source_id(&self) -> &SourceId {
        &self.source_id
    }

    /// Current version identifier after the call.
    pub fn version_id(&self) -> &SourceVersionId {
        &self.version_id
    }

    /// Current version number after the call.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// What the call did.
    pub fn outcome(&self) -> SourceRegistrationOutcome {
        self.outcome
    }
}

/// Durable record of one artifact change behind a source identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceContentDrift {
    source_id: SourceId,
    previous_version_id: SourceVersionId,
    new_version_id: SourceVersionId,
    previous_sha256: Option<String>,
    new_sha256: Option<String>,
}

impl SourceContentDrift {
    /// Source identity whose artifact changed.
    pub fn source_id(&self) -> &SourceId {
        &self.source_id
    }

    /// Version that was superseded.
    pub fn previous_version_id(&self) -> &SourceVersionId {
        &self.previous_version_id
    }

    /// Version that now carries the new artifact.
    pub fn new_version_id(&self) -> &SourceVersionId {
        &self.new_version_id
    }

    /// Digest before the change.
    pub fn previous_sha256(&self) -> Option<&str> {
        self.previous_sha256.as_deref()
    }

    /// Digest after the change.
    pub fn new_sha256(&self) -> Option<&str> {
        self.new_sha256.as_deref()
    }

    /// Render the drift as a validation finding targeting the source.
    pub fn to_validation_record(&self) -> ValidationErrorRecord {
        let render = |digest: Option<&str>| digest.unwrap_or("<none>").to_owned();

        ValidationErrorRecord::new(
            SOURCE_CONTENT_DRIFT_CODE,
            ValidationErrorSeverity::Warning,
            format!(
                "artifact behind source {} changed from sha256 {} ({}) to sha256 {} ({})",
                self.source_id.as_str(),
                render(self.previous_sha256.as_deref()),
                self.previous_version_id.as_str(),
                render(self.new_sha256.as_deref()),
                self.new_version_id.as_str(),
            ),
            ValidationTarget::source(self.source_id.as_str()),
        )
    }
}

/// Append-only store of source versions with content-drift tracking.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceStore {
    versions: Vec<Source>,
    drifts: Vec<SourceContentDrift>,
}

impl SourceStore {
    /// Creates an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a source input.
    ///
    /// Behaviour:
    /// - unknown identity: store version 1, `Created`;
    /// - identical to the current version: store nothing, `Unchanged`;
    /// - same identity, different artifact hash: store a superseding version,
    ///   record a [`SourceContentDrift`], `Superseded`;
    /// - same identity, same hash, other descriptive difference: conflict.
    ///
    /// # Errors
    ///
    /// [`GraphError::InvalidPropertyValue`] for invalid input or a conflicting
    /// re-registration; [`GraphError::SourceNotFound`] for an unknown parent.
    pub fn register_source(
        &mut self,
        input: SourceInput,
    ) -> Result<SourceRegistration, GraphError> {
        self.register(input, false)
    }

    /// Lift a legacy evidence record into a source.
    ///
    /// The source identity is the record's `source_ref`; the URI is the
    /// record's `source_url` when present, otherwise the `source_ref`; the
    /// type defaults to `Other`; the artifact hash is the record's
    /// `content_sha256`; the acquisition time is left unset because a record's
    /// `observed_at` belongs to its observation, not to the source. The
    /// resulting version is marked `derived_from_legacy`.
    ///
    /// # Errors
    ///
    /// Same as [`Self::register_source`].
    pub fn lift_from_evidence(
        &mut self,
        record: &EvidenceRecord,
    ) -> Result<SourceRegistration, GraphError> {
        let source_id = SourceId::new(record.source_ref())?;
        let uri = record.source_url().unwrap_or_else(|| record.source_ref());
        let mut input = SourceInput::new(
            source_id,
            uri,
            record.source_type().unwrap_or(EvidenceSourceType::Other),
        );
        if let Some(digest) = record.content_sha256() {
            input = input.with_artifact_sha256(digest);
        }
        // `observed_at` is a per-observation time and stays on the lifted
        // `Observation`; copying it here would make two spans of the same
        // document observed at different times conflict as sources.

        self.register(input, true)
    }

    fn register(
        &mut self,
        input: SourceInput,
        derived_from_legacy: bool,
    ) -> Result<SourceRegistration, GraphError> {
        input.validate()?;

        if let Some(parent) = &input.parent_source
            && self.current_source(parent).is_none()
        {
            return Err(GraphError::SourceNotFound(parent.clone()));
        }

        let Some(current) = self.current_source(&input.id).cloned() else {
            let version = Self::materialize(input, 1, None, derived_from_legacy)?;
            let registration = SourceRegistration {
                source_id: version.id.clone(),
                version_id: version.version_id.clone(),
                version: version.version,
                outcome: SourceRegistrationOutcome::Created,
            };
            self.versions.push(version);
            return Ok(registration);
        };

        if current.matches_input(&input) {
            return Ok(SourceRegistration {
                source_id: current.id,
                version_id: current.version_id,
                version: current.version,
                outcome: SourceRegistrationOutcome::Unchanged,
            });
        }

        if current.artifact_sha256 == input.artifact_sha256 {
            return Err(GraphError::ImmutableRecordConflict {
                kind: ImmutableRecordKind::Source,
                id: input.id.as_str().to_owned(),
            });
        }

        let next = Self::materialize(
            input,
            current.version + 1,
            Some(current.version_id.clone()),
            derived_from_legacy,
        )?;
        let drift = SourceContentDrift {
            source_id: next.id.clone(),
            previous_version_id: current.version_id,
            new_version_id: next.version_id.clone(),
            previous_sha256: current.artifact_sha256,
            new_sha256: next.artifact_sha256.clone(),
        };
        let registration = SourceRegistration {
            source_id: next.id.clone(),
            version_id: next.version_id.clone(),
            version: next.version,
            outcome: SourceRegistrationOutcome::Superseded,
        };
        self.versions.push(next);
        self.drifts.push(drift);

        Ok(registration)
    }

    fn materialize(
        input: SourceInput,
        version: u64,
        supersedes: Option<SourceVersionId>,
        derived_from_legacy: bool,
    ) -> Result<Source, GraphError> {
        let version_id = SourceVersionId::new(format!(
            "source-version--{}--{}",
            input.id.as_str(),
            version
        ))?;

        Ok(Source {
            id: input.id,
            version_id,
            version,
            uri: input.uri,
            source_type: input.source_type,
            publisher: input.publisher,
            authority_domain: input.authority_domain,
            acquired_at: input.acquired_at,
            artifact_sha256: input.artifact_sha256,
            signature: input.signature,
            parent_source: input.parent_source,
            dependency_signals: input.dependency_signals,
            supersedes,
            derived_from_legacy,
        })
    }

    /// Every distinct source identity, in first-registration order.
    pub fn source_ids(&self) -> Vec<&SourceId> {
        let mut seen = Vec::new();
        for version in &self.versions {
            if !seen.contains(&&version.id) {
                seen.push(&version.id);
            }
        }
        seen
    }

    /// Current (latest) version of a source identity.
    pub fn current_source(&self, source_id: &SourceId) -> Option<&Source> {
        self.versions
            .iter()
            .rev()
            .find(|source| &source.id == source_id)
    }

    /// Every stored version of a source identity, oldest first.
    pub fn source_versions(&self, source_id: &SourceId) -> Vec<&Source> {
        self.versions
            .iter()
            .filter(|source| &source.id == source_id)
            .collect()
    }

    /// One version by its version identifier.
    pub fn source_version(&self, version_id: &SourceVersionId) -> Option<&Source> {
        self.versions
            .iter()
            .find(|source| &source.version_id == version_id)
    }

    /// Recorded artifact changes, oldest first.
    pub fn content_drifts(&self) -> &[SourceContentDrift] {
        &self.drifts
    }

    /// Recorded artifact changes rendered as validation findings.
    pub fn content_drift_issues(&self) -> Vec<ValidationErrorRecord> {
        self.drifts
            .iter()
            .map(SourceContentDrift::to_validation_record)
            .collect()
    }

    /// Number of stored versions across all identities.
    pub fn len(&self) -> usize {
        self.versions.len()
    }

    /// Whether no version is stored.
    pub fn is_empty(&self) -> bool {
        self.versions.is_empty()
    }
}

/// Whether `digest` is a 64-character lowercase hexadecimal SHA-256 rendering,
/// the same shape `EvidenceInput` enforces for `content_sha256`.
fn is_lowercase_sha256(digest: &str) -> bool {
    digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

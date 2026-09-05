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
use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::{
    ClaimId, Confidence, EvidenceId, ExtractionRunId, GraphError, NodeId, ObservationId,
    ObservationStore, RelationshipId, SourceId, SourceRegistration, SourceStore, TemporalTimestamp,
};

/// Provenance source category for first-class evidence metadata.
///
/// Classifies the origin of an evidence record to support provenance tracking
/// and downstream trust evaluation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidenceSourceType {
    /// Evidence extracted from a structured or unstructured document.
    Document,
    /// Evidence retrieved from a web URL.
    Url,
    /// Evidence derived from a structured dataset.
    Dataset,
    /// Evidence produced by an automated extraction pipeline.
    Extraction,
    /// Evidence from a source that does not fit other categories.
    Other,
}

impl EvidenceSourceType {
    /// Canonical lowercase token used in projections.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Document => "document",
            Self::Url => "url",
            Self::Dataset => "dataset",
            Self::Extraction => "extraction",
            Self::Other => "other",
        }
    }
}

/// Bounded location of an evidence excerpt inside its source document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EvidenceLocator {
    /// One one-based document page.
    Page {
        /// One-based page number.
        page: u32,
    },
    /// One one-based paragraph, optionally scoped to a page.
    Paragraph {
        /// Optional one-based page number.
        page: Option<u32>,
        /// One-based paragraph number.
        paragraph: u32,
    },
    /// One table cell, using one-based table, row and column coordinates.
    TableCell {
        /// Optional one-based page number.
        page: Option<u32>,
        /// One-based table number.
        table: u32,
        /// One-based row number.
        row: u32,
        /// One-based column number.
        column: u32,
    },
    /// Half-open byte range in the source content.
    ByteRange {
        /// Inclusive byte offset.
        start: u64,
        /// Exclusive byte offset.
        end: u64,
    },
    /// Half-open range of Unicode scalar values in the source text.
    CharacterSpan {
        /// Inclusive character offset.
        start: u64,
        /// Exclusive character offset.
        end: u64,
    },
    /// Path to one field or record inside a structured source, for example a
    /// JSON pointer or a STIX object path.
    RecordPath {
        /// Non-blank path expression.
        path: String,
    },
}

impl EvidenceLocator {
    /// Validate the locator's shape: positive bounded ordinals, non-empty
    /// half-open ranges, and a non-blank record path.
    ///
    /// # Errors
    ///
    /// [`GraphError::InvalidPropertyValue`] when the shape is invalid.
    pub(crate) fn validate(&self) -> Result<(), GraphError> {
        const MAX_PAGE: u32 = 1_000_000;
        const MAX_ORDINAL: u32 = 1_000_000;
        let valid_ordinal = |value: u32| (1..=MAX_ORDINAL).contains(&value);
        let valid_page = |value: u32| (1..=MAX_PAGE).contains(&value);
        let valid = match self {
            Self::Page { page } => valid_page(*page),
            Self::Paragraph { page, paragraph } => {
                page.is_none_or(valid_page) && valid_ordinal(*paragraph)
            }
            Self::TableCell {
                page,
                table,
                row,
                column,
            } => {
                page.is_none_or(valid_page)
                    && valid_ordinal(*table)
                    && valid_ordinal(*row)
                    && valid_ordinal(*column)
            }
            Self::ByteRange { start, end } | Self::CharacterSpan { start, end } => start < end,
            Self::RecordPath { path } => !path.trim().is_empty(),
        };
        if !valid {
            return Err(GraphError::InvalidPropertyValue(
                "evidence locator must use positive bounded ordinals, a non-empty half-open \
                 range, or a non-blank record path"
                    .to_owned(),
            ));
        }
        Ok(())
    }

    /// Render the locator as one stable token for projections.
    pub fn render(&self) -> String {
        match self {
            Self::Page { page } => format!("page:{page}"),
            Self::Paragraph { page, paragraph } => match page {
                Some(page) => format!("paragraph:{page}/{paragraph}"),
                None => format!("paragraph:{paragraph}"),
            },
            Self::TableCell {
                page,
                table,
                row,
                column,
            } => match page {
                Some(page) => format!("table_cell:{page}/{table}/{row}/{column}"),
                None => format!("table_cell:{table}/{row}/{column}"),
            },
            Self::ByteRange { start, end } => format!("byte_range:{start}-{end}"),
            Self::CharacterSpan { start, end } => format!("character_span:{start}-{end}"),
            Self::RecordPath { path } => format!("record_path:{path}"),
        }
    }
}

/// First-class evidence record with full provenance metadata.
///
/// Captures the raw payload, source classification, extraction lineage, and
/// reliability assessments for a single piece of evidence. Evidence records
/// are attached to graph elements (nodes, relationships, claims) through
/// [`EvidenceAttachment`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EvidenceRecord {
    id: EvidenceId,
    source_ref: String,
    payload: String,
    source_type: Option<EvidenceSourceType>,
    chunk_id: Option<String>,
    offset_start: Option<u64>,
    offset_end: Option<u64>,
    source_url: Option<String>,
    extraction_run_id: Option<ExtractionRunId>,
    extractor_id: Option<String>,
    model_version: Option<String>,
    observed_at: Option<TemporalTimestamp>,
    language: Option<String>,
    source_reliability: Option<Confidence>,
    information_credibility: Option<Confidence>,
    content_sha256: Option<String>,
    locator: Option<EvidenceLocator>,
    /// Source identity behind this record, when known. Absent for records
    /// created before Epic 0029 WS-A; set explicitly through
    /// [`EvidenceInput::with_source_id`] or by [`EvidenceRecordStore::lift_sources`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_id: Option<SourceId>,
    /// Observation carrying this record's exact span, when known. Set through
    /// [`EvidenceInput::with_observation_id`] or by
    /// [`EvidenceRecordStore::lift_observations`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    observation_id: Option<ObservationId>,
}

impl EvidenceRecord {
    /// Returns the stable evidence identifier.
    pub fn id(&self) -> &EvidenceId {
        &self.id
    }

    /// Returns the external source reference string.
    pub fn source_ref(&self) -> &str {
        self.source_ref.as_str()
    }

    /// Returns the raw evidence payload.
    pub fn payload(&self) -> &str {
        self.payload.as_str()
    }

    /// Returns the provenance source category, if set.
    pub fn source_type(&self) -> Option<EvidenceSourceType> {
        self.source_type
    }

    /// Returns the chunk identifier within the source, if set.
    pub fn chunk_id(&self) -> Option<&str> {
        self.chunk_id.as_deref()
    }

    /// Returns the byte offset where the evidence starts in the source, if set.
    pub fn offset_start(&self) -> Option<u64> {
        self.offset_start
    }

    /// Returns the byte offset where the evidence ends in the source, if set.
    pub fn offset_end(&self) -> Option<u64> {
        self.offset_end
    }

    /// Returns the URL of the original source, if set.
    pub fn source_url(&self) -> Option<&str> {
        self.source_url.as_deref()
    }

    /// Returns the extraction run that produced this record, if set.
    pub fn extraction_run_id(&self) -> Option<&ExtractionRunId> {
        self.extraction_run_id.as_ref()
    }

    /// Returns the extractor identifier, if set.
    pub fn extractor_id(&self) -> Option<&str> {
        self.extractor_id.as_deref()
    }

    /// Returns the model version used for extraction, if set.
    pub fn model_version(&self) -> Option<&str> {
        self.model_version.as_deref()
    }

    /// Returns the timestamp when the evidence was observed, if set.
    pub fn observed_at(&self) -> Option<&TemporalTimestamp> {
        self.observed_at.as_ref()
    }

    /// Returns the language of the evidence payload, if set.
    pub fn language(&self) -> Option<&str> {
        self.language.as_deref()
    }

    /// Returns the source reliability confidence score, if set.
    pub fn source_reliability(&self) -> Option<Confidence> {
        self.source_reliability
    }

    /// Returns the information credibility confidence score, if set.
    pub fn information_credibility(&self) -> Option<Confidence> {
        self.information_credibility
    }

    /// Returns the lowercase SHA-256 digest of the complete source content.
    pub fn content_sha256(&self) -> Option<&str> {
        self.content_sha256.as_deref()
    }

    /// Source identity behind this record, when known.
    pub fn source_id(&self) -> Option<&SourceId> {
        self.source_id.as_ref()
    }

    /// Observation carrying this record's exact span, when known.
    pub fn observation_id(&self) -> Option<&ObservationId> {
        self.observation_id.as_ref()
    }

    /// Returns the bounded locator for this excerpt.
    pub const fn locator(&self) -> Option<&EvidenceLocator> {
        self.locator.as_ref()
    }
}

/// Input contract for creating an explicit evidence record.
///
/// Uses a builder pattern: construct with [`EvidenceInput::new`] and chain
/// optional `with_*` methods to set provenance metadata before passing to
/// [`EvidenceRecordStore::create_evidence`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EvidenceInput {
    id: EvidenceId,
    source_ref: String,
    payload: String,
    source_type: Option<EvidenceSourceType>,
    chunk_id: Option<String>,
    offset_start: Option<u64>,
    offset_end: Option<u64>,
    source_url: Option<String>,
    extraction_run_id: Option<ExtractionRunId>,
    extractor_id: Option<String>,
    model_version: Option<String>,
    observed_at: Option<TemporalTimestamp>,
    language: Option<String>,
    source_reliability: Option<Confidence>,
    information_credibility: Option<Confidence>,
    content_sha256: Option<String>,
    locator: Option<EvidenceLocator>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_id: Option<SourceId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    observation_id: Option<ObservationId>,
}

impl EvidenceInput {
    /// Creates a new evidence input with the required fields.
    pub fn new(id: EvidenceId, source_ref: impl Into<String>, payload: impl Into<String>) -> Self {
        Self {
            id,
            // Source ref.
            source_ref: source_ref.into(),
            // Payload.
            payload: payload.into(),
            // Source type.
            source_type: None,
            // Chunk id.
            chunk_id: None,
            // Offset start.
            offset_start: None,
            // Offset end.
            offset_end: None,
            // Source url.
            source_url: None,
            // Extraction run id.
            extraction_run_id: None,
            // Extractor id.
            extractor_id: None,
            model_version: None,
            // Observed at.
            observed_at: None,
            // Language.
            language: None,
            // Source reliability.
            source_reliability: None,
            // Information credibility.
            information_credibility: None,
            content_sha256: None,
            locator: None,
            source_id: None,
            observation_id: None,
        }
    }

    /// Sets the source identity behind this record.
    pub fn with_source_id(mut self, source_id: SourceId) -> Self {
        self.source_id = Some(source_id);
        self
    }

    /// Sets the observation carrying this record's exact span.
    pub fn with_observation_id(mut self, observation_id: ObservationId) -> Self {
        self.observation_id = Some(observation_id);
        self
    }

    /// Sets the provenance source category.
    pub fn with_source_type(mut self, source_type: EvidenceSourceType) -> Self {
        self.source_type = Some(source_type);
        self
    }

    /// Sets the chunk identifier within the source.
    pub fn with_chunk_id(mut self, chunk_id: impl Into<String>) -> Self {
        self.chunk_id = Some(chunk_id.into());
        self
    }

    /// Sets the byte offset range within the source.
    pub fn with_offsets(mut self, offset_start: u64, offset_end: u64) -> Self {
        self.offset_start = Some(offset_start);
        self.offset_end = Some(offset_end);
        self
    }

    /// Sets the URL of the original source.
    pub fn with_source_url(mut self, source_url: impl Into<String>) -> Self {
        self.source_url = Some(source_url.into());
        self
    }

    /// Sets the extraction run that produced this evidence.
    pub fn with_extraction_run_id(mut self, extraction_run_id: ExtractionRunId) -> Self {
        self.extraction_run_id = Some(extraction_run_id);
        self
    }

    /// Sets the extractor identifier.
    pub fn with_extractor_id(mut self, extractor_id: impl Into<String>) -> Self {
        self.extractor_id = Some(extractor_id.into());
        self
    }

    /// Sets the model version used for extraction.
    pub fn with_model_version(mut self, model_version: impl Into<String>) -> Self {
        self.model_version = Some(model_version.into());
        self
    }

    /// Sets the timestamp when the evidence was observed.
    pub fn with_observed_at(mut self, observed_at: TemporalTimestamp) -> Self {
        self.observed_at = Some(observed_at);
        self
    }

    /// Sets the language of the evidence payload.
    pub fn with_language(mut self, language: impl Into<String>) -> Self {
        self.language = Some(language.into());
        self
    }

    /// Sets the source reliability confidence score.
    pub fn with_source_reliability(mut self, source_reliability: Confidence) -> Self {
        self.source_reliability = Some(source_reliability);
        self
    }

    /// Sets the information credibility confidence score.
    pub fn with_information_credibility(mut self, information_credibility: Confidence) -> Self {
        self.information_credibility = Some(information_credibility);
        self
    }

    /// Sets the lowercase SHA-256 digest of the complete source content.
    pub fn with_content_sha256(mut self, content_sha256: impl Into<String>) -> Self {
        self.content_sha256 = Some(content_sha256.into());
        self
    }

    /// Sets the bounded source locator.
    pub fn with_locator(mut self, locator: EvidenceLocator) -> Self {
        self.locator = Some(locator);
        self
    }

    /// Validates all input fields before evidence creation.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::InvalidPropertyValue`] when:
    /// - `source_ref` or `payload` is empty or whitespace-only,
    /// - an optional string field (`chunk_id`, `source_url`, `extractor_id`,
    ///   `model_version`, `language`) is provided but empty,
    /// - `offset_start` exceeds `offset_end`.
    fn validate(&self) -> Result<(), GraphError> {
        if self.source_ref.trim().is_empty() {
            return Err(GraphError::InvalidPropertyValue(
                "evidence source_ref must not be empty".to_owned(),
            ));
        }

        if self.payload.trim().is_empty() {
            return Err(GraphError::InvalidPropertyValue(
                "evidence payload must not be empty".to_owned(),
            ));
        }

        if let Some(chunk_id) = &self.chunk_id
            && chunk_id.trim().is_empty()
        {
            return Err(GraphError::InvalidPropertyValue(
                "evidence chunk_id must not be empty when provided".to_owned(),
            ));
        }

        if let Some(source_url) = &self.source_url
            && source_url.trim().is_empty()
        {
            return Err(GraphError::InvalidPropertyValue(
                "evidence source_url must not be empty when provided".to_owned(),
            ));
        }

        if let Some(extractor_id) = &self.extractor_id
            && extractor_id.trim().is_empty()
        {
            return Err(GraphError::InvalidPropertyValue(
                "evidence extractor_id must not be empty when provided".to_owned(),
            ));
        }

        if let Some(model_version) = &self.model_version
            && model_version.trim().is_empty()
        {
            return Err(GraphError::InvalidPropertyValue(
                "evidence model_version must not be empty when provided".to_owned(),
            ));
        }

        if let Some(language) = &self.language
            && language.trim().is_empty()
        {
            return Err(GraphError::InvalidPropertyValue(
                "evidence language must not be empty when provided".to_owned(),
            ));
        }

        if let (Some(start), Some(end)) = (self.offset_start, self.offset_end)
            && start > end
        {
            return Err(GraphError::InvalidPropertyValue(
                "evidence offsets must satisfy offset_start <= offset_end".to_owned(),
            ));
        }

        self.validate_evidence_contract()?;

        Ok(())
    }

    fn validate_evidence_contract(&self) -> Result<(), GraphError> {
        if let Some(digest) = self.content_sha256.as_deref()
            && (digest.len() != 64
                || !digest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
        {
            return Err(GraphError::InvalidPropertyValue(
                "evidence content_sha256 must be 64 lowercase hexadecimal characters".to_owned(),
            ));
        }

        if let Some(locator) = self.locator.as_ref() {
            locator.validate()?;
        }

        Ok(())
    }
}

/// Graph element that can be the target of an evidence attachment.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EvidenceAttachmentTarget {
    /// A graph node.
    Node(NodeId),
    /// A graph relationship.
    Relationship(RelationshipId),
    /// An epistemic claim.
    Claim(ClaimId),
    /// An export record identified by a string key.
    ExportRecord(String),
    /// An observation record.
    Observation(ObservationId),
}

impl EvidenceAttachmentTarget {
    /// Creates a node attachment target.
    pub fn node(id: NodeId) -> Self {
        Self::Node(id)
    }

    /// Creates a relationship attachment target.
    pub fn relationship(id: RelationshipId) -> Self {
        Self::Relationship(id)
    }

    /// Creates a claim attachment target.
    pub fn claim(id: ClaimId) -> Self {
        Self::Claim(id)
    }

    /// Creates an export record attachment target.
    pub fn export_record(id: impl Into<String>) -> Self {
        Self::ExportRecord(id.into())
    }

    /// Observation target.
    pub fn observation(id: ObservationId) -> Self {
        Self::Observation(id)
    }
}

/// Links an evidence record to a specific graph element.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceAttachment {
    evidence_id: EvidenceId,
    target: EvidenceAttachmentTarget,
}

impl EvidenceAttachment {
    /// Returns the attached evidence identifier.
    pub fn evidence_id(&self) -> &EvidenceId {
        &self.evidence_id
    }

    /// Returns the attachment target.
    pub fn target(&self) -> &EvidenceAttachmentTarget {
        &self.target
    }
}

/// In-memory store for evidence records and their attachments to graph elements.
///
/// Manages the lifecycle of [`EvidenceRecord`] instances and validates that
/// attachment targets are registered before allowing an evidence link.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EvidenceRecordStore {
    records: Vec<EvidenceRecord>,
    attachments: Vec<EvidenceAttachment>,
    known_node_targets: HashSet<NodeId>,
    known_relationship_targets: HashSet<RelationshipId>,
    known_claim_targets: HashSet<ClaimId>,
    known_export_record_targets: HashSet<String>,
    #[serde(default, skip_serializing_if = "HashSet::is_empty")]
    known_observation_targets: HashSet<ObservationId>,
}

impl EvidenceRecordStore {
    /// Creates an empty evidence store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates and stores an evidence record from the given input.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::InvalidPropertyValue`] if input validation fails.
    pub fn create_evidence(&mut self, input: EvidenceInput) -> Result<EvidenceId, GraphError> {
        input.validate()?;

        let evidence_id = input.id;
        let record = EvidenceRecord {
            id: evidence_id.clone(),
            source_ref: input.source_ref,
            payload: input.payload,
            source_type: input.source_type,
            chunk_id: input.chunk_id,
            offset_start: input.offset_start,
            offset_end: input.offset_end,
            source_url: input.source_url,
            extraction_run_id: input.extraction_run_id,
            extractor_id: input.extractor_id,
            model_version: input.model_version,
            observed_at: input.observed_at,
            language: input.language,
            source_reliability: input.source_reliability,
            information_credibility: input.information_credibility,
            content_sha256: input.content_sha256,
            locator: input.locator,
            source_id: input.source_id,
            observation_id: input.observation_id,
        };

        if let Some(existing) = self
            .records
            .iter()
            .find(|existing| existing.id == evidence_id)
        {
            if existing == &record {
                return Ok(evidence_id);
            }
            return Err(GraphError::InvalidPropertyValue(format!(
                "conflicting evidence record for {}",
                evidence_id.as_str()
            )));
        }

        self.records.push(record);
        Ok(evidence_id)
    }

    /// Lift every record without a `source_id` into `sources` and bind it to
    /// the lifted identity. Records that already name a source are left as
    /// they are. Idempotent: a second call finds every record bound, lifts
    /// nothing, and returns an empty list; the source store keeps the same
    /// versions and records no drift.
    ///
    /// # Errors
    ///
    /// Propagates [`SourceStore::lift_from_evidence`] errors.
    pub fn lift_sources(
        &mut self,
        sources: &mut SourceStore,
    ) -> Result<Vec<SourceRegistration>, GraphError> {
        let mut registrations = Vec::new();

        for record in self.records.iter_mut() {
            if record.source_id.is_some() {
                continue;
            }

            let registration = sources.lift_from_evidence(record)?;
            record.source_id = Some(registration.source_id().clone());
            registrations.push(registration);
        }

        Ok(registrations)
    }

    /// Lift every record that has a `source_id` but no `observation_id` into
    /// `observations` and bind it. Records already naming an observation are
    /// left as they are. Idempotent: a second call lifts nothing and returns an
    /// empty list.
    ///
    /// # Errors
    ///
    /// [`GraphError::InvalidVersionState`] when a record has no `source_id`
    /// (run [`Self::lift_sources`] first); otherwise the errors of
    /// [`ObservationStore::lift_from_evidence`].
    pub fn lift_observations(
        &mut self,
        observations: &mut ObservationStore,
        sources: &SourceStore,
    ) -> Result<Vec<ObservationId>, GraphError> {
        let mut lifted = Vec::new();

        for record in self.records.iter_mut() {
            if record.observation_id.is_some() {
                continue;
            }

            let observation_id = observations.lift_from_evidence(record, sources)?;
            record.observation_id = Some(observation_id.clone());
            lifted.push(observation_id);
        }

        Ok(lifted)
    }

    /// Every evidence record, in creation order.
    pub fn records(&self) -> &[EvidenceRecord] {
        &self.records
    }

    /// Returns the evidence record for the given ID, if it exists.
    pub fn evidence_by_id(&self, evidence_id: &EvidenceId) -> Option<&EvidenceRecord> {
        self.records.iter().find(|record| &record.id == evidence_id)
    }

    /// Returns the number of unique durable evidence records.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Returns whether the store contains no evidence records.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Registers a node as a valid attachment target.
    pub fn register_node_target(&mut self, node_id: NodeId) {
        self.known_node_targets.insert(node_id);
    }

    /// Registers a relationship as a valid attachment target.
    pub fn register_relationship_target(&mut self, relationship_id: RelationshipId) {
        self.known_relationship_targets.insert(relationship_id);
    }

    /// Registers a claim as a valid attachment target.
    pub fn register_claim_target(&mut self, claim_id: ClaimId) {
        self.known_claim_targets.insert(claim_id);
    }

    /// Registers an export record as a valid attachment target.
    pub fn register_export_record_target(&mut self, export_record_id: impl Into<String>) {
        self.known_export_record_targets
            .insert(export_record_id.into());
    }

    /// Registers an observation as a valid attachment target.
    pub fn register_observation_target(&mut self, observation_id: ObservationId) {
        self.known_observation_targets.insert(observation_id);
    }

    /// Attaches an evidence record to a validated target.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::EvidenceNotFound`] if the evidence ID is unknown.
    /// Returns the appropriate `NotFound` variant if the target has not been
    /// registered.
    pub fn attach_evidence(
        &mut self,
        evidence_id: EvidenceId,
        target: EvidenceAttachmentTarget,
    ) -> Result<(), GraphError> {
        if self.evidence_by_id(&evidence_id).is_none() {
            return Err(GraphError::EvidenceNotFound(evidence_id));
        }

        match &target {
            EvidenceAttachmentTarget::Node(node_id) => {
                if !self.known_node_targets.contains(node_id) {
                    return Err(GraphError::NodeNotFound(node_id.clone()));
                }
            }
            EvidenceAttachmentTarget::Relationship(relationship_id) => {
                if !self.known_relationship_targets.contains(relationship_id) {
                    return Err(GraphError::RelationshipNotFound(relationship_id.clone()));
                }
            }
            EvidenceAttachmentTarget::Claim(claim_id) => {
                if !self.known_claim_targets.contains(claim_id) {
                    return Err(GraphError::ClaimNotFound(claim_id.clone()));
                }
            }
            EvidenceAttachmentTarget::ExportRecord(export_record_id) => {
                if !self.known_export_record_targets.contains(export_record_id) {
                    return Err(GraphError::InvalidPropertyValue(format!(
                        "export record target not found: {}",
                        export_record_id
                    )));
                }
            }
            EvidenceAttachmentTarget::Observation(observation_id) => {
                if !self.known_observation_targets.contains(observation_id) {
                    return Err(GraphError::ObservationNotFound(observation_id.clone()));
                }
            }
        }

        let attachment = EvidenceAttachment {
            evidence_id,
            target,
        };
        if !self.attachments.contains(&attachment) {
            self.attachments.push(attachment);
        }

        Ok(())
    }

    /// Returns all evidence attachments.
    pub fn attachments(&self) -> &[EvidenceAttachment] {
        self.attachments.as_slice()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evidence_id(value: &str) -> EvidenceId {
        EvidenceId::new(value).expect("test evidence ID should be valid")
    }

    fn node_id(value: &str) -> NodeId {
        NodeId::new(value).expect("test node ID should be valid")
    }

    // Verifies that empty source references are rejected.
    #[test]
    fn evidence_input_rejects_empty_source_ref() {
        let mut store = EvidenceRecordStore::new();
        let error = store
            .create_evidence(EvidenceInput::new(
                evidence_id("evidence--1"),
                " ",
                "payload",
            ))
            .expect_err("empty source_ref should be rejected");

        assert!(matches!(
        error,
        GraphError::InvalidPropertyValue(message)
        if message == "evidence source_ref must not be empty"
        ));
    }

    // Verifies that attachments require registered node targets.
    #[test]
    fn attachment_rejects_unknown_node_target() {
        let mut store = EvidenceRecordStore::new();
        let evidence_id = evidence_id("evidence--2");

        store
            .create_evidence(EvidenceInput::new(
                evidence_id.clone(),
                "source://test/2",
                "payload",
            ))
            .expect("evidence should be created");

        let missing_node = node_id("node--missing");
        let error = store
            .attach_evidence(
                evidence_id,
                EvidenceAttachmentTarget::node(missing_node.clone()),
            )
            .expect_err("unknown node should be rejected");

        assert!(matches!(
        error,
        GraphError::NodeNotFound(node_id) if node_id == missing_node
        ));
    }

    #[test]
    fn evidence_replay_is_idempotent_but_conflicting_duplicates_are_rejected() {
        let mut store = EvidenceRecordStore::new();
        let id = evidence_id("evidence--stable-1");
        let input = EvidenceInput::new(id.clone(), "document--1", "payload")
            .with_content_sha256("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
            .with_locator(EvidenceLocator::Paragraph {
                page: Some(7),
                paragraph: 2,
            });

        store
            .create_evidence(input.clone())
            .expect("first evidence write should succeed");
        store
            .create_evidence(input)
            .expect("identical evidence replay should be idempotent");
        assert_eq!(store.len(), 1);

        let error = store
            .create_evidence(
                EvidenceInput::new(id, "document--1", "conflicting payload")
                    .with_content_sha256(
                        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    )
                    .with_locator(EvidenceLocator::Paragraph {
                        page: Some(7),
                        paragraph: 2,
                    }),
            )
            .expect_err("conflicting duplicate evidence should fail");
        assert!(matches!(
            error,
            GraphError::InvalidPropertyValue(message)
                if message == "conflicting evidence record for evidence--stable-1"
        ));
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn evidence_locator_and_digest_validate_at_the_core_boundary() {
        let mut store = EvidenceRecordStore::new();
        let invalid_page = store
            .create_evidence(
                EvidenceInput::new(evidence_id("evidence--invalid-page"), "document--1", "x")
                    .with_content_sha256(
                        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    )
                    .with_locator(EvidenceLocator::Page { page: 0 }),
            )
            .expect_err("page zero should fail");
        assert!(matches!(invalid_page, GraphError::InvalidPropertyValue(_)));

        let invalid_digest = store
            .create_evidence(
                EvidenceInput::new(evidence_id("evidence--invalid-digest"), "document--1", "x")
                    .with_content_sha256("not-a-sha256")
                    .with_locator(EvidenceLocator::Page { page: 1 }),
            )
            .expect_err("invalid digest should fail");
        assert!(matches!(
            invalid_digest,
            GraphError::InvalidPropertyValue(_)
        ));
    }

    #[test]
    fn evidence_store_round_trips_through_serde() {
        let mut store = EvidenceRecordStore::new();
        let id = evidence_id("evidence--persisted-1");
        store
            .create_evidence(
                EvidenceInput::new(id.clone(), "document--persisted", "payload")
                    .with_content_sha256(
                        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                    )
                    .with_locator(EvidenceLocator::TableCell {
                        page: Some(9),
                        table: 1,
                        row: 3,
                        column: 2,
                    }),
            )
            .expect("evidence should be created");

        let encoded = serde_json::to_vec(&store).expect("store should serialize");
        let restored: EvidenceRecordStore =
            serde_json::from_slice(&encoded).expect("store should deserialize");

        assert_eq!(restored.len(), 1);
        assert_eq!(
            restored
                .evidence_by_id(&id)
                .expect("evidence should survive")
                .source_ref(),
            "document--persisted"
        );
    }
}

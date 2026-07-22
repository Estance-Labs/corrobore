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
use std::collections::{HashMap, HashSet};

use crate::{
    ClaimId, Confidence, EvidenceId, ExtractionRunId, GraphError, NodeId, RelationshipId,
    TemporalTimestamp,
};

/// Provenance source category for first-class evidence metadata.
///
/// Classifies the origin of an evidence record to support provenance tracking
/// and downstream trust evaluation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

/// First-class evidence record with full provenance metadata.
///
/// Captures the raw payload, source classification, extraction lineage, and
/// reliability assessments for a single piece of evidence. Evidence records
/// are attached to graph elements (nodes, relationships, claims) through
/// [`EvidenceAttachment`].
#[derive(Clone, Debug, PartialEq)]
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
}

/// Input contract for creating an explicit evidence record.
///
/// Uses a builder pattern: construct with [`EvidenceInput::new`] and chain
/// optional `with_*` methods to set provenance metadata before passing to
/// [`EvidenceRecordStore::create_evidence`].
#[derive(Clone, Debug, PartialEq)]
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
        }
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

        Ok(())
    }
}

/// Graph element that can be the target of an evidence attachment.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum EvidenceAttachmentTarget {
    /// A graph node.
    Node(NodeId),
    /// A graph relationship.
    Relationship(RelationshipId),
    /// An epistemic claim.
    Claim(ClaimId),
    /// An export record identified by a string key.
    ExportRecord(String),
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
}

/// Links an evidence record to a specific graph element.
#[derive(Clone, Debug, PartialEq, Eq)]
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
#[derive(Default)]
pub struct EvidenceRecordStore {
    records: HashMap<EvidenceId, EvidenceRecord>,
    attachments: Vec<EvidenceAttachment>,
    known_node_targets: HashSet<NodeId>,
    known_relationship_targets: HashSet<RelationshipId>,
    known_claim_targets: HashSet<ClaimId>,
    known_export_record_targets: HashSet<String>,
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
        };

        self.records.insert(evidence_id.clone(), record);
        Ok(evidence_id)
    }

    /// Returns the evidence record for the given ID, if it exists.
    pub fn evidence_by_id(&self, evidence_id: &EvidenceId) -> Option<&EvidenceRecord> {
        self.records.get(evidence_id)
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
        if !self.records.contains_key(&evidence_id) {
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
        }

        self.attachments.push(EvidenceAttachment {
            evidence_id,
            target,
        });

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
}

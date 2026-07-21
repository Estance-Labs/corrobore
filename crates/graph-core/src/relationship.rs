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
//! Relationship model, relationship input, and relationship patch types.
//!
//! Module boundary:
//! this module owns relationship record shape and relationship-specific input
//! builders. It does not own graph storage, endpoint existence checks, adjacency
//! indexes, traversal, semantic loading, CTI rules, FIMI rules, or crisis rules.

use serde::{Deserialize, Serialize};

use crate::{
    Confidence, EvidenceId, ExtractionRunId, GraphError, NodeId, PropertyMap, PropertyValue,
    RecordStatus, RelationshipId, RelationshipVersionId, TemporalMetadata, TemporalTimestamp,
    TransactionMetadata,
};

/// Validated semantic type attached to a relationship.
///
/// Relationship types are graph-core edge labels. This primitive validates that
/// the label is meaningful while preserving the caller-provided value exactly.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RelationshipType(String);

impl RelationshipType {
    /// Build a validated relationship type value.
    ///
    ///
    /// keep relationship type validation close to the domain primitive so graph
    /// write paths can accept only meaningful relationship labels.
    ///
    ///
    /// accept a non-empty relationship type and preserve its original value.
    ///
    /// # Errors
    ///
    /// empty or whitespace-only values should fail with
    /// `GraphError::InvalidRelationshipType`.
    pub fn new(value: impl Into<String>) -> Result<Self, GraphError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(GraphError::InvalidRelationshipType(value));
        }
        Ok(Self(value))
    }

    /// Return the relationship type as a string slice.
    ///
    ///
    /// expose the validated relationship type without leaking mutable access to
    /// the underlying storage value.
    ///
    ///
    /// return the exact type value accepted by `RelationshipType::new`.
    ///
    /// # Errors
    ///
    /// none expected because construction performs validation.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Versioned relationship record stored and returned by graph-core.
///
/// A `Relationship` is immutable from the public API perspective. Graph
/// operations own endpoint validation, version transitions, current-version
/// flags, and tombstone behavior.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Relationship {
    pub(crate) id: RelationshipId,
    pub(crate) version_id: RelationshipVersionId,
    pub(crate) version: u64,
    pub(crate) current: bool,
    pub(crate) previous_version_id: Option<RelationshipVersionId>,
    pub(crate) source: NodeId,
    pub(crate) target: NodeId,
    pub(crate) rel_type: RelationshipType,
    pub(crate) properties: PropertyMap,
    pub(crate) status: RecordStatus,
    pub(crate) confidence: Option<Confidence>,
    pub(crate) source_reliability: Option<Confidence>,
    pub(crate) information_credibility: Option<Confidence>,
    pub(crate) extraction_run_id: Option<ExtractionRunId>,
    pub(crate) evidence_refs: Vec<EvidenceId>,
    pub(crate) temporal: TemporalMetadata,
    pub(crate) transaction: TransactionMetadata,
}

impl Relationship {
    /// Id.
    pub fn id(&self) -> &RelationshipId {
        &self.id
    }

    /// Version id.
    pub fn version_id(&self) -> &RelationshipVersionId {
        &self.version_id
    }

    /// Version.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Returns `true` if current.
    pub fn is_current(&self) -> bool {
        self.current
    }

    /// Previous version id.
    pub fn previous_version_id(&self) -> Option<&RelationshipVersionId> {
        self.previous_version_id.as_ref()
    }

    /// Source.
    pub fn source(&self) -> &NodeId {
        &self.source
    }

    /// Target.
    pub fn target(&self) -> &NodeId {
        &self.target
    }

    /// Rel type.
    pub fn rel_type(&self) -> &RelationshipType {
        &self.rel_type
    }

    /// Property.
    pub fn property(&self, key: &str) -> Option<&PropertyValue> {
        self.properties.get(key)
    }

    /// Read-only properties attached to this relationship version.
    pub fn properties(&self) -> &PropertyMap {
        &self.properties
    }

    /// Status.
    pub fn status(&self) -> RecordStatus {
        self.status
    }

    /// Confidence.
    pub fn confidence(&self) -> Option<Confidence> {
        self.confidence
    }

    /// First seen.
    pub fn first_seen(&self) -> Option<&str> {
        self.temporal.first_seen.as_deref()
    }

    /// Last seen.
    pub fn last_seen(&self) -> Option<&str> {
        self.temporal.last_seen.as_deref()
    }

    /// Source reliability.
    pub fn source_reliability(&self) -> Option<Confidence> {
        self.source_reliability
    }

    /// Information credibility.
    pub fn information_credibility(&self) -> Option<Confidence> {
        self.information_credibility
    }

    /// Extraction run id.
    pub fn extraction_run_id(&self) -> Option<&ExtractionRunId> {
        self.extraction_run_id.as_ref()
    }

    /// Evidence refs.
    pub fn evidence_refs(&self) -> &[EvidenceId] {
        self.evidence_refs.as_slice()
    }
}

/// Public input builder used to create the first version of a relationship.
///
/// `RelationshipInput` collects endpoint IDs, type, properties, status,
/// confidence, evidence references, temporal metadata, and transaction metadata.
/// Relationship type validation stays here, while endpoint existence checks and
/// stable ID allocation stay in `Graph`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RelationshipInput {
    pub(crate) source: NodeId,
    pub(crate) target: NodeId,
    pub(crate) rel_type: RelationshipType,
    pub(crate) properties: PropertyMap,
    pub(crate) status: RecordStatus,
    pub(crate) confidence: Option<Confidence>,
    pub(crate) source_reliability: Option<Confidence>,
    pub(crate) information_credibility: Option<Confidence>,
    pub(crate) extraction_run_id: Option<ExtractionRunId>,
    pub(crate) evidence_refs: Vec<EvidenceId>,
    pub(crate) temporal: TemporalMetadata,
    pub(crate) transaction: TransactionMetadata,
}

impl RelationshipInput {
    /// Build the minimum input required to create a relationship.
    ///
    ///
    /// collect the stable source node ID, relationship type, and stable target
    /// node ID before the graph write path validates node existence.
    ///
    ///
    /// validate the relationship type, default optional metadata, and return an
    /// input object that can later be passed to `Graph::create_relationship`.
    ///
    /// # Errors
    ///
    /// invalid relationship types should fail with
    /// `GraphError::InvalidRelationshipType`; source and target existence are
    /// validated by the graph write path, not by this input builder.
    pub fn new(
        source: NodeId,
        rel_type: impl Into<String>,
        target: NodeId,
    ) -> Result<Self, GraphError> {
        Ok(Self {
            source,
            target,
            rel_type: RelationshipType::new(rel_type)?,
            properties: PropertyMap::new(),
            status: RecordStatus::Candidate,
            confidence: None,
            source_reliability: None,
            information_credibility: None,
            extraction_run_id: None,
            evidence_refs: Vec::new(),
            temporal: TemporalMetadata::default(),
            transaction: TransactionMetadata::default(),
        })
    }

    /// Attach or replace one property on the relationship input.
    ///
    ///
    /// let callers enrich the relationship payload before it is persisted as the
    /// first relationship version.
    ///
    ///
    /// insert the property value under the provided key and return the updated
    /// builder value.
    ///
    /// # Errors
    ///
    /// none expected because `PropertyValue` is already a typed domain value.
    pub fn with_property(mut self, key: impl Into<String>, value: PropertyValue) -> Self {
        self.properties.insert(key.into(), value);
        self
    }

    /// Set the initial relationship status.
    ///
    ///
    /// allow callers to choose the first lifecycle status before the relationship
    /// is stored.
    ///
    ///
    /// replace the default candidate status and return the updated builder value.
    ///
    /// # Errors
    ///
    /// none expected because `RecordStatus` is a closed enum.
    pub fn with_status(mut self, status: RecordStatus) -> Self {
        self.status = status;
        self
    }

    /// Set the initial relationship confidence.
    ///
    ///
    /// preserve analyst or extraction confidence on the first relationship
    /// version instead of forcing callers to patch it later.
    ///
    ///
    /// store the provided bounded confidence value and return the updated builder
    /// value.
    ///
    /// # Errors
    ///
    /// none expected because `Confidence` is validated before it reaches this
    /// builder method.
    pub fn with_confidence(mut self, confidence: Confidence) -> Self {
        self.confidence = Some(confidence);
        self
    }

    /// Sets the first seen.
    pub fn with_first_seen(mut self, first_seen: TemporalTimestamp) -> Self {
        self.temporal = self.temporal.with_first_seen(first_seen);
        self
    }

    /// Sets the last seen.
    pub fn with_last_seen(mut self, last_seen: TemporalTimestamp) -> Self {
        self.temporal = self.temporal.with_last_seen(last_seen);
        self
    }

    /// Sets the source reliability.
    pub fn with_source_reliability(mut self, source_reliability: Confidence) -> Self {
        self.source_reliability = Some(source_reliability);
        self
    }

    /// Sets the information credibility.
    pub fn with_information_credibility(mut self, information_credibility: Confidence) -> Self {
        self.information_credibility = Some(information_credibility);
        self
    }

    /// Sets the extraction run id.
    pub fn with_extraction_run_id(mut self, extraction_run_id: ExtractionRunId) -> Self {
        self.extraction_run_id = Some(extraction_run_id);
        self
    }

    /// Sets the evidence ref.
    pub fn with_evidence_ref(mut self, evidence_id: EvidenceId) -> Self {
        self.evidence_refs.push(evidence_id);
        self
    }
}

/// Public patch object used to describe relationship version transitions.
///
/// `RelationshipPatch` carries already typed field changes. Applying it belongs
/// to `Graph` because only graph storage can append a new version and move the
/// current relationship pointer safely.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RelationshipPatch {
    pub(crate) properties_to_set: PropertyMap,
    pub(crate) status: Option<RecordStatus>,
    pub(crate) confidence: Option<Confidence>,
}

impl RelationshipPatch {
    /// Sets the property.
    pub fn set_property(mut self, key: impl Into<String>, value: PropertyValue) -> Self {
        self.properties_to_set.insert(key.into(), value);
        self
    }

    /// Sets the status.
    pub fn set_status(mut self, status: RecordStatus) -> Self {
        self.status = Some(status);
        self
    }

    /// Sets the confidence.
    pub fn set_confidence(mut self, confidence: Confidence) -> Self {
        self.confidence = Some(confidence);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    //
    // Verify that a meaningful relationship type can be constructed and read
    // through the public primitive API. Relationship types are the semantic label
    // on graph edges, so the unit test must document the accepted happy path.
    //
    // Given a non-empty relationship type string,
    // when `RelationshipType::new` is called,
    // then construction should succeed and `as_str` should preserve the value.
    #[test]
    fn relationship_type_accepts_valid_value() {
        let rel_type =
            RelationshipType::new("indicates").expect("valid relationship type should be accepted");

        assert_eq!(rel_type.as_str(), "indicates");
    }

    //
    // Verify that empty relationship types are rejected before a relationship
    // input can reach graph persistence. This keeps invalid edge semantics out
    // of graph write paths.
    //
    // Given an empty relationship type string,
    // when `RelationshipType::new` is called,
    // then construction should fail with `GraphError::InvalidRelationshipType`.
    #[test]
    fn relationship_type_rejects_empty_value() {
        let error =
            RelationshipType::new("").expect_err("empty relationship type should be rejected");

        assert!(matches!(
        error,
        GraphError::InvalidRelationshipType(value) if value.is_empty()
        ));
    }

    //
    // Verify that whitespace-only relationship types are rejected and reported
    // with the same typed error branch as other invalid relationship type values.
    //
    // Given a relationship type containing only whitespace,
    // when `RelationshipType::new` is called,
    // then construction should fail with `GraphError::InvalidRelationshipType`.
    #[test]
    fn relationship_type_rejects_whitespace_only_value() {
        let error = RelationshipType::new(" \t\n")
            .expect_err("whitespace-only relationship type should be rejected");

        assert!(matches!(
        error,
        GraphError::InvalidRelationshipType(value) if value == " \t\n"
        ));
    }

    //
    // Verify the current normalization policy for relationship types. The
    // primitive validates that a type is meaningful but should not trim or rewrite
    // caller-provided values unless a later normalization layer is introduced.
    //
    // Given a relationship type with surrounding spaces and non-whitespace content,
    // when it is constructed,
    // then `as_str` should return the original value exactly as provided.
    #[test]
    fn relationship_type_preserves_original_value_without_normalization() {
        let rel_type = RelationshipType::new(" indicates ")
            .expect("non-empty relationship type should be accepted");

        assert_eq!(rel_type.as_str(), " indicates ");
    }
}

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
//! Node model, node input, and node patch types.
//!
//! Module boundary:
//! this module owns node record shape and node-specific input builders. It does
//! not own graph storage, identifier allocation, relationship validation,
//! traversal, persistence, semantic loading, CTI rules, FIMI rules, or crisis
//! rules.

use serde::{Deserialize, Serialize};

use crate::{
    Confidence, EvidenceId, ExtractionRunId, GraphError, LabelSet, NodeId, NodeVersionId,
    PropertyMap, PropertyValue, RecordStatus, TemporalMetadata, TemporalTimestamp,
    TransactionMetadata,
};

/// Versioned node record stored and returned by graph-core.
///
/// A `Node` is immutable from the public API perspective. Graph operations own
/// version transitions, current-version flags, and tombstone behavior so callers
/// cannot mutate historical node state directly.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub(crate) id: NodeId,
    pub(crate) version_id: NodeVersionId,
    pub(crate) version: u64,
    pub(crate) current: bool,
    pub(crate) previous_version_id: Option<NodeVersionId>,
    pub(crate) labels: LabelSet,
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

impl Node {
    /// Id.
    pub fn id(&self) -> &NodeId {
        &self.id
    }

    /// Version id.
    pub fn version_id(&self) -> &NodeVersionId {
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
    pub fn previous_version_id(&self) -> Option<&NodeVersionId> {
        self.previous_version_id.as_ref()
    }

    /// Returns `true` if has label.
    pub fn has_label(&self, label: &str) -> bool {
        self.labels.iter().any(|candidate| candidate == label)
    }

    /// Labels in their stored order.
    pub fn labels(&self) -> &[String] {
        self.labels.as_slice()
    }

    /// Property.
    pub fn property(&self, key: &str) -> Option<&PropertyValue> {
        self.properties.get(key)
    }

    /// Read-only properties attached to this node version.
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

/// Public input builder used to create the first version of a node.
///
/// `NodeInput` collects caller-provided labels, properties, status, confidence,
/// evidence references, temporal metadata, and transaction metadata. Validation
/// for node labels stays here, while stable ID and version allocation stay in
/// `Graph`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NodeInput {
    pub(crate) labels: LabelSet,
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

impl NodeInput {
    /// Creates a new instance.
    pub fn new<I, S>(labels: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Self {
            // Labels.
            labels: labels
                .into_iter()
                .map(|label| label.as_ref().to_owned())
                .collect(),
            // Properties.
            properties: PropertyMap::new(),
            // Status.
            status: RecordStatus::Candidate,
            // Confidence.
            confidence: None,
            // Source reliability.
            source_reliability: None,
            // Information credibility.
            information_credibility: None,
            // Extraction run id.
            extraction_run_id: None,
            // Evidence refs.
            evidence_refs: Vec::new(),
            // Temporal.
            temporal: TemporalMetadata::default(),
            // Transaction.
            transaction: TransactionMetadata::default(),
        }
    }

    /// Sets the property.
    pub fn with_property(mut self, key: impl Into<String>, value: PropertyValue) -> Self {
        self.properties.insert(key.into(), value);
        self
    }

    /// Sets the status.
    pub fn with_status(mut self, status: RecordStatus) -> Self {
        self.status = status;
        self
    }

    /// Sets the confidence.
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

    /// Validate.
    pub fn validate(&self) -> Result<(), GraphError> {
        if self.labels.is_empty() {
            return Err(GraphError::InvalidLabel(String::new()));
        }
        if let Some(invalid) = self.labels.iter().find(|label| label.trim().is_empty()) {
            return Err(GraphError::InvalidLabel(invalid.clone()));
        }
        Ok(())
    }
}

/// Public patch object used to describe node version transitions.
///
/// `NodePatch` carries already typed values. Applying it belongs to `Graph`
/// because only graph storage can append a new version and move the current node
/// pointer safely.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NodePatch {
    pub(crate) properties_to_set: PropertyMap,
    pub(crate) status: Option<RecordStatus>,
    pub(crate) confidence: Option<Confidence>,
}

impl NodePatch {
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
    // Verify that node input validation accepts a meaningful node label. This is
    // the direct unit-level contract behind graph node creation before the graph
    // allocates a stable ID or version ID.
    //
    // Given a `NodeInput` with at least one non-empty label,
    // when `validate` is called,
    // then validation should return `Ok(())`.
    #[test]
    fn node_input_validate_accepts_non_empty_labels() {
        let input = NodeInput::new(["ThreatActor"]);

        assert_eq!(input.validate(), Ok(()));
    }

    //
    // Verify that node input validation rejects nodes with no labels. A graph node
    // without labels cannot carry a useful domain type and should fail before it
    // is inserted into storage.
    //
    // Given a `NodeInput` with an empty label list,
    // when `validate` is called,
    // then validation should fail with `GraphError::InvalidLabel(String::new())`.
    #[test]
    fn node_input_validate_rejects_missing_labels() {
        let input = NodeInput::new(std::iter::empty::<&str>());
        let error = input
            .validate()
            .expect_err("node input without labels should be rejected");

        assert!(matches!(error, GraphError::InvalidLabel(label) if label.is_empty()));
    }

    //
    // Verify that node input validation rejects labels that contain only spaces.
    // This documents that validation checks semantic emptiness, not only string
    // length.
    //
    // Given a `NodeInput` containing a whitespace-only label,
    // when `validate` is called,
    // then validation should fail with `GraphError::InvalidLabel` carrying that label.
    #[test]
    fn node_input_validate_rejects_whitespace_only_label() {
        let input = NodeInput::new(["ThreatActor", " \t\n"]);
        let error = input
            .validate()
            .expect_err("node input with a whitespace-only label should be rejected");

        assert!(matches!(error, GraphError::InvalidLabel(label) if label == " \t\n"));
    }

    //
    // Verify that valid labels are preserved exactly as provided. Validation
    // should reject meaningless labels but must not normalize or rewrite accepted
    // values in phase-one graph-core storage.
    //
    // Given a `NodeInput` with a non-empty label that includes surrounding spaces,
    // when `validate` is called,
    // then validation should succeed and leave normalization to a future layer.
    #[test]
    fn node_input_validate_does_not_normalize_valid_labels() {
        let input = NodeInput::new([" ThreatActor "]);

        assert_eq!(input.validate(), Ok(()));
        assert_eq!(input.labels, vec![" ThreatActor ".to_owned()]);
    }
}

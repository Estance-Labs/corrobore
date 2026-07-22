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
use super::*;

/// Minimal statement payload for an epistemic claim.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimStatement {
    pub(crate) text: String,
}

impl ClaimStatement {
    /// Creates a new instance.
    pub fn new(value: impl Into<String>) -> Result<Self, GraphError> {
        let text = value.into();

        if text.trim().is_empty() {
            return Err(GraphError::InvalidPropertyValue(
                "claim statement must not be empty".to_owned(),
            ));
        }

        Ok(Self { text })
    }

    /// Returns the value as str.
    pub fn as_str(&self) -> &str {
        self.text.as_str()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Claim target kind.
pub enum ClaimTargetKind {
    /// Node.
    Node,
    /// Relationship.
    Relationship,
    /// Evidence.
    Evidence,
    /// Source.
    Source,
    /// Temporal assertion.
    TemporalAssertion,
    /// Confidence assertion.
    ConfidenceAssertion,
    /// Analytical assertion.
    AnalyticalAssertion,
    /// Unsupported.
    Unsupported,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Claim evidence target ref.
pub struct ClaimEvidenceTargetRef {
    pub(crate) stable_reference: String,
}

impl ClaimEvidenceTargetRef {
    /// Creates a new instance.
    pub fn new(stable_reference: impl Into<String>) -> Self {
        Self {
            // Stable reference.
            stable_reference: stable_reference.into(),
        }
    }

    /// Returns the value as str.
    pub fn as_str(&self) -> &str {
        self.stable_reference.as_str()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Claim source target ref.
pub struct ClaimSourceTargetRef {
    pub(crate) stable_reference: String,
}

impl ClaimSourceTargetRef {
    /// Creates a new instance.
    pub fn new(stable_reference: impl Into<String>) -> Self {
        Self {
            // Stable reference.
            stable_reference: stable_reference.into(),
        }
    }

    /// Returns the value as str.
    pub fn as_str(&self) -> &str {
        self.stable_reference.as_str()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Claim temporal target.
pub struct ClaimTemporalTarget {
    pub(crate) field: String,
    pub(crate) value: String,
}

impl ClaimTemporalTarget {
    /// Creates a new instance.
    pub fn new(field: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            // Field.
            field: field.into(),
            // Value.
            value: value.into(),
        }
    }

    /// Field.
    pub fn field(&self) -> &str {
        self.field.as_str()
    }

    /// Value.
    pub fn value(&self) -> &str {
        self.value.as_str()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
/// Claim confidence target.
pub struct ClaimConfidenceTarget {
    pub(crate) confidence_kind: String,
    pub(crate) confidence: f64,
}

impl ClaimConfidenceTarget {
    /// Creates a new instance.
    pub fn new(confidence_kind: impl Into<String>, confidence: f64) -> Self {
        Self {
            // Confidence kind.
            confidence_kind: confidence_kind.into(),
            confidence,
        }
    }

    /// Confidence kind.
    pub fn confidence_kind(&self) -> &str {
        self.confidence_kind.as_str()
    }

    /// Confidence.
    pub fn confidence(&self) -> f64 {
        self.confidence
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Claim analytical target.
pub struct ClaimAnalyticalTarget {
    pub(crate) summary: String,
    pub(crate) hypothesis_workspace_ref: Option<String>,
}

impl ClaimAnalyticalTarget {
    /// Creates a new instance.
    pub fn new(summary: impl Into<String>, hypothesis_workspace_ref: Option<String>) -> Self {
        Self {
            // Summary.
            summary: summary.into(),
            hypothesis_workspace_ref,
        }
    }

    /// Summary.
    pub fn summary(&self) -> &str {
        self.summary.as_str()
    }

    /// Hypothesis workspace ref.
    pub fn hypothesis_workspace_ref(&self) -> Option<&str> {
        self.hypothesis_workspace_ref.as_deref()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Claim target metadata.
pub struct ClaimTargetMetadata {
    /// Kind.
    pub kind: ClaimTargetKind,
    /// Stable reference.
    pub stable_reference: Option<String>,
}

#[derive(Clone, Debug, Default)]
/// Claim target validation context.
pub struct ClaimTargetValidationContext {
    pub(crate) known_nodes: HashSet<NodeId>,
    pub(crate) known_relationships: HashSet<RelationshipId>,
    pub(crate) known_evidence_refs: HashSet<String>,
    pub(crate) known_source_refs: HashSet<String>,
}

impl ClaimTargetValidationContext {
    /// Creates a new instance.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register node.
    pub fn register_node(&mut self, node_id: NodeId) {
        self.known_nodes.insert(node_id);
    }

    /// Register relationship.
    pub fn register_relationship(&mut self, relationship_id: RelationshipId) {
        self.known_relationships.insert(relationship_id);
    }

    /// Register evidence.
    pub fn register_evidence(&mut self, evidence_ref: impl Into<String>) {
        self.known_evidence_refs.insert(evidence_ref.into());
    }

    /// Register source.
    pub fn register_source(&mut self, source_ref: impl Into<String>) {
        self.known_source_refs.insert(source_ref.into());
    }
}

/// Explicit claim target model for graph, evidence, and analytical assertions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ClaimTarget {
    /// Node.
    Node(NodeId),
    /// Relationship.
    Relationship(RelationshipId),
    /// Evidence.
    Evidence(ClaimEvidenceTargetRef),
    /// Source.
    Source(ClaimSourceTargetRef),
    /// Temporal assertion.
    TemporalAssertion(ClaimTemporalTarget),
    /// Confidence assertion.
    ConfidenceAssertion(ClaimConfidenceTarget),
    /// Free-form analytical targets are accepted for bounded analyst
    /// hypotheses that are not yet normalized into concrete graph records.
    AnalyticalAssertion(ClaimAnalyticalTarget),
    /// Unsupported.
    Unsupported {
        /// Kind.
        kind: String,
        /// Raw reference.
        raw_reference: String,
    },
}

impl ClaimTarget {
    /// Kind.
    pub fn kind(&self) -> ClaimTargetKind {
        match self {
            Self::Node(_) => ClaimTargetKind::Node,
            Self::Relationship(_) => ClaimTargetKind::Relationship,
            Self::Evidence(_) => ClaimTargetKind::Evidence,
            Self::Source(_) => ClaimTargetKind::Source,
            Self::TemporalAssertion(_) => ClaimTargetKind::TemporalAssertion,
            Self::ConfidenceAssertion(_) => ClaimTargetKind::ConfidenceAssertion,
            Self::AnalyticalAssertion(_) => ClaimTargetKind::AnalyticalAssertion,
            Self::Unsupported { .. } => ClaimTargetKind::Unsupported,
        }
    }

    /// Validates the references.
    pub fn validate_references(
        &self,
        context: &ClaimTargetValidationContext,
    ) -> Result<(), GraphError> {
        match self {
            Self::Node(node_id) => {
                if context.known_nodes.contains(node_id) {
                    Ok(())
                } else {
                    Err(GraphError::ClaimTargetNotFound(self.clone()))
                }
            }
            Self::Relationship(relationship_id) => {
                if context.known_relationships.contains(relationship_id) {
                    Ok(())
                } else {
                    Err(GraphError::ClaimTargetNotFound(self.clone()))
                }
            }
            Self::Evidence(evidence_ref) => {
                if evidence_ref.as_str().trim().is_empty() {
                    return Err(GraphError::InvalidPropertyValue(
                        "evidence target reference must not be empty".to_owned(),
                    ));
                }

                if context.known_evidence_refs.contains(evidence_ref.as_str()) {
                    Ok(())
                } else {
                    Err(GraphError::ClaimTargetNotFound(self.clone()))
                }
            }
            Self::Source(source_ref) => {
                if source_ref.as_str().trim().is_empty() {
                    return Err(GraphError::InvalidPropertyValue(
                        "source target reference must not be empty".to_owned(),
                    ));
                }

                if context.known_source_refs.contains(source_ref.as_str()) {
                    Ok(())
                } else {
                    Err(GraphError::ClaimTargetNotFound(self.clone()))
                }
            }
            Self::TemporalAssertion(target) => {
                if target.field().trim().is_empty() || target.value().trim().is_empty() {
                    return Err(GraphError::InvalidPropertyValue(
                        "temporal target requires non-empty field and value".to_owned(),
                    ));
                }

                Ok(())
            }
            Self::ConfidenceAssertion(target) => {
                if target.confidence_kind().trim().is_empty() {
                    return Err(GraphError::InvalidPropertyValue(
                        "confidence target kind must not be empty".to_owned(),
                    ));
                }

                if !(0.0..=1.0).contains(&target.confidence()) {
                    return Err(GraphError::InvalidConfidence(target.confidence()));
                }

                Ok(())
            }
            Self::AnalyticalAssertion(target) => {
                if target.summary().trim().is_empty() {
                    return Err(GraphError::InvalidPropertyValue(
                        "analytical target summary must not be empty".to_owned(),
                    ));
                }

                Ok(())
            }
            Self::Unsupported { kind, .. } => {
                Err(GraphError::UnsupportedClaimTargetKind(kind.clone()))
            }
        }
    }

    /// Resolve target metadata.
    pub fn resolve_target_metadata(
        &self,
        context: &ClaimTargetValidationContext,
    ) -> Result<ClaimTargetMetadata, GraphError> {
        self.validate_references(context)?;

        let stable_reference = match self {
            Self::Node(node_id) => Some(node_id.as_str().to_owned()),
            Self::Relationship(relationship_id) => Some(relationship_id.as_str().to_owned()),
            Self::Evidence(evidence_ref) => Some(evidence_ref.as_str().to_owned()),
            Self::Source(source_ref) => Some(source_ref.as_str().to_owned()),
            Self::TemporalAssertion(_) => None,
            Self::ConfidenceAssertion(_) => None,
            Self::AnalyticalAssertion(target) => {
                target.hypothesis_workspace_ref().map(str::to_owned)
            }
            Self::Unsupported { kind, .. } => {
                return Err(GraphError::UnsupportedClaimTargetKind(kind.clone()));
            }
        };

        Ok(ClaimTargetMetadata {
            kind: self.kind(),
            stable_reference,
        })
    }
}

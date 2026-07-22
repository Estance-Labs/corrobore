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
//! Branch-local overlay records for hypothetical worlds (Epic 0021).
//!
//! This module owns hypotheses, derived relations, predictions, expected
//! evidence, and contradictions that exist only inside one hypothetical branch.

use serde::{Deserialize, Serialize};

use crate::{FactId, GraphError, RelationshipType};

macro_rules! overlay_id {
    ($name:ident) => {
        #[doc = concat!("Typed identifier for `", stringify!($name), "`.")]
        #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub struct $name {
            value: String,
        }

        impl $name {
            /// Creates a validated branch-overlay identifier.
            ///
            /// # Errors
            ///
            /// Rejects blank identifiers with [`GraphError::InvalidIdentifier`].
            pub fn new(value: impl Into<String>) -> Result<Self, GraphError> {
                let value = value.into();
                if value.trim().is_empty() {
                    return Err(GraphError::InvalidIdentifier(stringify!($name).to_owned()));
                }
                Ok(Self { value })
            }

            /// Returns the identifier as a string slice.
            #[must_use]
            pub fn as_str(&self) -> &str {
                self.value.as_str()
            }
        }
    };
}

overlay_id!(OverlayHypothesisId);
overlay_id!(BranchDerivedRelationId);
overlay_id!(BranchPredictionId);
overlay_id!(ExpectedEvidenceMarkerId);
overlay_id!(BranchContradictionId);

/// Typed reference to an assertion visible from one branch overlay.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BranchOverlayReference {
    /// Immutable canonical fact shared by all worlds.
    BaseFact(FactId),
    /// Hypothesis declared in the same branch.
    Hypothesis(OverlayHypothesisId),
    /// Derived relation declared in the same branch.
    DerivedRelation(BranchDerivedRelationId),
    /// Prediction declared in the same branch.
    Prediction(BranchPredictionId),
}

/// Branch-local analytical hypothesis.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverlayHypothesis {
    id: OverlayHypothesisId,
    statement: String,
}

impl OverlayHypothesis {
    /// Creates a validated hypothesis record.
    ///
    /// # Errors
    ///
    /// Rejects blank statements with [`GraphError::InvalidBranchOverlay`].
    pub fn new(id: OverlayHypothesisId, statement: impl Into<String>) -> Result<Self, GraphError> {
        let statement = statement.into();
        validate_non_blank("hypothesis statement", &statement)?;
        Ok(Self { id, statement })
    }

    /// Returns the hypothesis identifier.
    #[must_use]
    pub fn id(&self) -> &OverlayHypothesisId {
        &self.id
    }

    /// Returns the hypothesis statement.
    #[must_use]
    pub fn statement(&self) -> &str {
        self.statement.as_str()
    }
}

/// Relation inferred only within one branch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchDerivedRelation {
    id: BranchDerivedRelationId,
    source: BranchOverlayReference,
    target: BranchOverlayReference,
    relationship_type: RelationshipType,
}

impl BranchDerivedRelation {
    /// Creates a branch-local derived relation.
    #[must_use]
    pub fn new(
        id: BranchDerivedRelationId,
        source: BranchOverlayReference,
        target: BranchOverlayReference,
        relationship_type: RelationshipType,
    ) -> Self {
        Self {
            id,
            source,
            target,
            relationship_type,
        }
    }

    /// Returns the derived relation identifier.
    #[must_use]
    pub fn id(&self) -> &BranchDerivedRelationId {
        &self.id
    }

    /// Returns the source reference.
    #[must_use]
    pub fn source(&self) -> &BranchOverlayReference {
        &self.source
    }

    /// Returns the target reference.
    #[must_use]
    pub fn target(&self) -> &BranchOverlayReference {
        &self.target
    }

    /// Returns the relationship type.
    #[must_use]
    pub fn relationship_type(&self) -> &RelationshipType {
        &self.relationship_type
    }
}

/// Prediction made only within one branch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchPrediction {
    id: BranchPredictionId,
    statement: String,
}

impl BranchPrediction {
    /// Creates a validated branch-local prediction.
    ///
    /// # Errors
    ///
    /// Rejects blank statements.
    pub fn new(id: BranchPredictionId, statement: impl Into<String>) -> Result<Self, GraphError> {
        let statement = statement.into();
        validate_non_blank("prediction statement", &statement)?;
        Ok(Self { id, statement })
    }

    /// Returns the prediction identifier.
    #[must_use]
    pub fn id(&self) -> &BranchPredictionId {
        &self.id
    }

    /// Returns the prediction statement.
    #[must_use]
    pub fn statement(&self) -> &str {
        self.statement.as_str()
    }
}

/// Evidence expected if a branch assertion is correct.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchExpectedEvidence {
    id: ExpectedEvidenceMarkerId,
    description: String,
    target: BranchOverlayReference,
}

impl BranchExpectedEvidence {
    /// Creates a validated expected-evidence marker.
    ///
    /// # Errors
    ///
    /// Rejects blank descriptions.
    pub fn new(
        id: ExpectedEvidenceMarkerId,
        description: impl Into<String>,
        target: BranchOverlayReference,
    ) -> Result<Self, GraphError> {
        let description = description.into();
        validate_non_blank("expected-evidence description", &description)?;
        Ok(Self {
            id,
            description,
            target,
        })
    }

    /// Returns the marker identifier.
    #[must_use]
    pub fn id(&self) -> &ExpectedEvidenceMarkerId {
        &self.id
    }

    /// Returns the marker description.
    #[must_use]
    pub fn description(&self) -> &str {
        self.description.as_str()
    }

    /// Returns the assertion expected evidence would inform.
    #[must_use]
    pub fn target(&self) -> &BranchOverlayReference {
        &self.target
    }
}

/// Unresolved conflict between two assertions visible in one branch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchContradiction {
    id: BranchContradictionId,
    left: BranchOverlayReference,
    right: BranchOverlayReference,
    description: String,
}

impl BranchContradiction {
    /// Creates a validated contradiction marker.
    ///
    /// # Errors
    ///
    /// Rejects blank descriptions and self-contradictions.
    pub fn new(
        id: BranchContradictionId,
        left: BranchOverlayReference,
        right: BranchOverlayReference,
        description: impl Into<String>,
    ) -> Result<Self, GraphError> {
        let description = description.into();
        validate_non_blank("contradiction description", &description)?;
        if left == right {
            return Err(GraphError::InvalidBranchOverlay(
                "a branch assertion cannot contradict itself".to_owned(),
            ));
        }
        Ok(Self {
            id,
            left,
            right,
            description,
        })
    }

    /// Returns the contradiction identifier.
    #[must_use]
    pub fn id(&self) -> &BranchContradictionId {
        &self.id
    }

    /// Returns the left assertion reference.
    #[must_use]
    pub fn left(&self) -> &BranchOverlayReference {
        &self.left
    }

    /// Returns the right assertion reference.
    #[must_use]
    pub fn right(&self) -> &BranchOverlayReference {
        &self.right
    }

    /// Returns the contradiction description.
    #[must_use]
    pub fn description(&self) -> &str {
        self.description.as_str()
    }
}

/// Deterministically ordered state owned by one branch.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchOverlay {
    pub(crate) hypotheses: Vec<OverlayHypothesis>,
    pub(crate) derived_relations: Vec<BranchDerivedRelation>,
    pub(crate) predictions: Vec<BranchPrediction>,
    pub(crate) expected_evidence: Vec<BranchExpectedEvidence>,
    pub(crate) contradictions: Vec<BranchContradiction>,
}

impl BranchOverlay {
    /// Returns hypotheses in identifier order.
    #[must_use]
    pub fn hypotheses(&self) -> &[OverlayHypothesis] {
        self.hypotheses.as_slice()
    }

    /// Returns derived relations in identifier order.
    #[must_use]
    pub fn derived_relations(&self) -> &[BranchDerivedRelation] {
        self.derived_relations.as_slice()
    }

    /// Returns predictions in identifier order.
    #[must_use]
    pub fn predictions(&self) -> &[BranchPrediction] {
        self.predictions.as_slice()
    }

    /// Returns expected-evidence markers in identifier order.
    #[must_use]
    pub fn expected_evidence(&self) -> &[BranchExpectedEvidence] {
        self.expected_evidence.as_slice()
    }

    /// Returns contradiction markers in identifier order.
    #[must_use]
    pub fn contradictions(&self) -> &[BranchContradiction] {
        self.contradictions.as_slice()
    }

    pub(crate) fn contains_reference(&self, reference: &BranchOverlayReference) -> bool {
        match reference {
            BranchOverlayReference::BaseFact(_) => false,
            BranchOverlayReference::Hypothesis(id) => {
                self.hypotheses.iter().any(|record| record.id() == id)
            }
            BranchOverlayReference::DerivedRelation(id) => self
                .derived_relations
                .iter()
                .any(|record| record.id() == id),
            BranchOverlayReference::Prediction(id) => {
                self.predictions.iter().any(|record| record.id() == id)
            }
        }
    }
}

fn validate_non_blank(field: &str, value: &str) -> Result<(), GraphError> {
    if value.trim().is_empty() {
        return Err(GraphError::InvalidBranchOverlay(format!(
            "{field} must not be blank"
        )));
    }
    Ok(())
}

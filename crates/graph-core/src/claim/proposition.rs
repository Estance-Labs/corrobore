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
//! Structured claim proposition (Epic 0029, WS-A item 1).
//!
//! Module boundary:
//! this module owns the atomic, machine-readable form of a claim: subject,
//! predicate, object or literal value, polarity, modality, valid-time scope,
//! and extraction version. It sits beside the free-text [`ClaimStatement`] and
//! never replaces it. It does not compute verdicts, resolve entities, or
//! persist anything; those belong to later WS-A items and to WS-C and WS-D.
//!
//! Validation targets:
//! - subject and predicate are non-blank;
//! - a literal object carries a typed value, never an explicit null;
//! - an entity object resolves against the same [`ClaimTargetValidationContext`]
//!   claim targets use;
//! - a valid-time scope has at least one bound and is ordered;
//! - an extraction version, when present, is non-blank.
//!
//! Compatibility targets:
//! - the proposition is an optional component of a claim, serialized only when
//!   present, so payloads written before this module deserialize unchanged;
//! - the graph-facing projection is additive and namespaced under
//!   `proposition_*` so no existing node property changes meaning.
use super::*;
use crate::{PropertyMap, PropertyValue, TemporalTimestamp};

/// Polarity of a proposition: whether the claim affirms or negates the
/// relation between subject and object.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClaimPolarity {
    /// The relation is asserted to hold.
    Affirmed,
    /// The relation is asserted not to hold.
    Negated,
}

impl ClaimPolarity {
    /// Closed vocabulary in canonical order.
    pub const ALL: [Self; 2] = [Self::Affirmed, Self::Negated];

    /// Canonical lowercase token used in projections and explanations.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Affirmed => "affirmed",
            Self::Negated => "negated",
        }
    }
}

/// Modality of a proposition: the epistemic mode under which the relation is
/// put forward, independent of whether evidence supports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClaimModality {
    /// Stated directly by the source or actor.
    Asserted,
    /// Attributed by the source to a third party.
    Reported,
    /// Put forward as a hypothesis awaiting evidence.
    Hypothesized,
    /// Stated about a future or expected state.
    Predicted,
}

impl ClaimModality {
    /// Closed vocabulary in canonical order.
    pub const ALL: [Self; 4] = [
        Self::Asserted,
        Self::Reported,
        Self::Hypothesized,
        Self::Predicted,
    ];

    /// Canonical lowercase token used in projections and explanations.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Asserted => "asserted",
            Self::Reported => "reported",
            Self::Hypothesized => "hypothesized",
            Self::Predicted => "predicted",
        }
    }
}

/// Object of a proposition: either a reference to a graph entity or a typed
/// literal value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ClaimPropositionObject {
    /// The object is an entity identified by a stable node ID. Resolution
    /// against known nodes happens in
    /// [`ClaimProposition::validate_references`].
    Entity(NodeId),
    /// The object is a typed literal. [`PropertyValue::Null`] is rejected at
    /// construction because a claim cannot assert an explicit null.
    Literal(PropertyValue),
}

/// Valid-time scope of a proposition: the interval in the world during which
/// the proposition is claimed to hold. Distinct from system time, which the
/// claim's temporal metadata and the bitemporal store carry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimValidTimeScope {
    pub(crate) valid_from: Option<TemporalTimestamp>,
    pub(crate) valid_until: Option<TemporalTimestamp>,
}

/// One numeric component of an aggregate proposition.
///
/// The verifier compares the component value with the proposition's numeric
/// literal and checks an optional unit against the aggregate unit. Keeping the
/// declaration typed avoids conventions hidden in free-form JSON payloads.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClaimArithmeticPart {
    value: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    unit: Option<String>,
}

impl ClaimArithmeticPart {
    /// Declare one aggregate component.
    pub fn new(value: f64) -> Self {
        Self { value, unit: None }
    }

    /// Declare the component's unit.
    pub fn with_unit(mut self, unit: impl Into<String>) -> Self {
        self.unit = Some(unit.into());
        self
    }

    /// Numeric component value.
    pub fn value(&self) -> f64 {
        self.value
    }

    /// Declared unit, when present.
    pub fn unit(&self) -> Option<&str> {
        self.unit.as_deref()
    }
}

/// Optional arithmetic declarations attached to a numeric proposition.
///
/// Bounds, units, and parts are declarations rather than constructor
/// invariants: persisted or imported data may be inconsistent, and the
/// deterministic verifier must report that inconsistency instead of making the
/// invalid state impossible to inspect.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ClaimArithmeticConstraint {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    minimum: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    maximum: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    unit: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    parts: Vec<ClaimArithmeticPart>,
}

impl ClaimArithmeticConstraint {
    /// Start an empty arithmetic declaration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Declare the inclusive lower bound.
    pub fn with_minimum(mut self, minimum: f64) -> Self {
        self.minimum = Some(minimum);
        self
    }

    /// Declare the inclusive upper bound.
    pub fn with_maximum(mut self, maximum: f64) -> Self {
        self.maximum = Some(maximum);
        self
    }

    /// Declare the aggregate unit.
    pub fn with_unit(mut self, unit: impl Into<String>) -> Self {
        self.unit = Some(unit.into());
        self
    }

    /// Append one aggregate component.
    pub fn with_part(mut self, part: ClaimArithmeticPart) -> Self {
        self.parts.push(part);
        self
    }

    /// Inclusive lower bound, when declared.
    pub fn minimum(&self) -> Option<f64> {
        self.minimum
    }

    /// Inclusive upper bound, when declared.
    pub fn maximum(&self) -> Option<f64> {
        self.maximum
    }

    /// Aggregate unit, when declared.
    pub fn unit(&self) -> Option<&str> {
        self.unit.as_deref()
    }

    /// Aggregate components.
    pub fn parts(&self) -> &[ClaimArithmeticPart] {
        self.parts.as_slice()
    }

    /// Whether the declaration carries nothing to check.
    pub fn is_empty(&self) -> bool {
        self.minimum.is_none()
            && self.maximum.is_none()
            && self.unit.is_none()
            && self.parts.is_empty()
    }
}

impl ClaimValidTimeScope {
    /// Build a scope from optional bounds.
    ///
    /// Validation: at least one bound is present, and when both are present
    /// `valid_from <= valid_until`. An empty scope is expressed by omitting
    /// the scope on the proposition, not by two absent bounds.
    ///
    /// # Errors
    ///
    /// [`GraphError::InvalidPropertyValue`] when no bound is given or the
    /// bounds are inverted.
    pub fn new(
        valid_from: Option<TemporalTimestamp>,
        valid_until: Option<TemporalTimestamp>,
    ) -> Result<Self, GraphError> {
        if valid_from.is_none() && valid_until.is_none() {
            return Err(GraphError::InvalidPropertyValue(
                "claim valid-time scope requires at least one bound".to_owned(),
            ));
        }

        // RFC3339 UTC timestamps validated by `TemporalTimestamp` compare
        // correctly as strings, which is the same rule `TemporalMetadata`
        // applies to its own valid_from / valid_until pair.
        if let (Some(from), Some(until)) = (&valid_from, &valid_until)
            && from.as_str() > until.as_str()
        {
            return Err(GraphError::InvalidPropertyValue(
                "invalid claim valid-time scope: valid_from must be <= valid_until".to_owned(),
            ));
        }

        Ok(Self {
            valid_from,
            valid_until,
        })
    }

    /// Lower bound, when present.
    pub fn valid_from(&self) -> Option<&TemporalTimestamp> {
        self.valid_from.as_ref()
    }

    /// Upper bound, when present.
    pub fn valid_until(&self) -> Option<&TemporalTimestamp> {
        self.valid_until.as_ref()
    }
}

/// Atomic, machine-readable proposition carried by a claim beside its text
/// statement.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClaimProposition {
    pub(crate) subject: String,
    pub(crate) predicate: String,
    pub(crate) object: ClaimPropositionObject,
    pub(crate) polarity: ClaimPolarity,
    pub(crate) modality: ClaimModality,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) valid_time: Option<ClaimValidTimeScope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) extraction_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) arithmetic: Option<ClaimArithmeticConstraint>,
}

impl ClaimProposition {
    /// Build a proposition with affirmed polarity and asserted modality.
    ///
    /// Validation: subject and predicate are trimmed and must be non-blank; a
    /// literal object must not be [`PropertyValue::Null`]. Entity objects are
    /// accepted here and resolved later by [`Self::validate_references`].
    ///
    /// # Errors
    ///
    /// [`GraphError::InvalidPropertyValue`] naming the offending field.
    pub fn new(
        subject: impl Into<String>,
        predicate: impl Into<String>,
        object: ClaimPropositionObject,
    ) -> Result<Self, GraphError> {
        let subject = non_blank(subject.into(), "claim proposition subject")?;
        let predicate = non_blank(predicate.into(), "claim proposition predicate")?;

        if matches!(object, ClaimPropositionObject::Literal(PropertyValue::Null)) {
            return Err(GraphError::InvalidPropertyValue(
                "claim proposition literal object must carry a typed value, not null".to_owned(),
            ));
        }

        Ok(Self {
            subject,
            predicate,
            object,
            polarity: ClaimPolarity::Affirmed,
            modality: ClaimModality::Asserted,
            valid_time: None,
            extraction_version: None,
            arithmetic: None,
        })
    }

    /// Set the polarity.
    pub fn with_polarity(mut self, polarity: ClaimPolarity) -> Self {
        self.polarity = polarity;
        self
    }

    /// Set the modality.
    pub fn with_modality(mut self, modality: ClaimModality) -> Self {
        self.modality = modality;
        self
    }

    /// Set the valid-time scope.
    pub fn with_valid_time(mut self, valid_time: ClaimValidTimeScope) -> Self {
        self.valid_time = Some(valid_time);
        self
    }

    /// Attach bounds, unit, and optional aggregate parts for deterministic
    /// arithmetic verification.
    pub fn with_arithmetic_constraint(mut self, arithmetic: ClaimArithmeticConstraint) -> Self {
        self.arithmetic = Some(arithmetic);
        self
    }

    /// Set the extraction version.
    ///
    /// # Errors
    ///
    /// [`GraphError::InvalidPropertyValue`] when the version is blank.
    pub fn with_extraction_version(
        self,
        extraction_version: impl Into<String>,
    ) -> Result<Self, GraphError> {
        let extraction_version = non_blank(
            extraction_version.into(),
            "claim proposition extraction version",
        )?;

        Ok(Self {
            extraction_version: Some(extraction_version),
            ..self
        })
    }

    /// Subject reference.
    pub fn subject(&self) -> &str {
        self.subject.as_str()
    }

    /// Predicate.
    pub fn predicate(&self) -> &str {
        self.predicate.as_str()
    }

    /// Object.
    pub fn object(&self) -> &ClaimPropositionObject {
        &self.object
    }

    /// Polarity.
    pub fn polarity(&self) -> ClaimPolarity {
        self.polarity
    }

    /// Modality.
    pub fn modality(&self) -> ClaimModality {
        self.modality
    }

    /// Valid-time scope, when present.
    pub fn valid_time(&self) -> Option<&ClaimValidTimeScope> {
        self.valid_time.as_ref()
    }

    /// Extraction version, when present.
    pub fn extraction_version(&self) -> Option<&str> {
        self.extraction_version.as_deref()
    }

    /// Arithmetic declarations, when present.
    pub fn arithmetic_constraint(&self) -> Option<&ClaimArithmeticConstraint> {
        self.arithmetic.as_ref()
    }

    /// Resolve graph references carried by the proposition.
    ///
    /// An entity object must be a node registered in `context`; literal
    /// objects carry no reference and always pass. The subject is a caller
    /// reference string and is not resolved here: WS-C introduces entity
    /// mentions and reconciliation records for that purpose.
    ///
    /// # Errors
    ///
    /// [`GraphError::ClaimPropositionEntityNotFound`] naming the unknown node.
    pub fn validate_references(
        &self,
        context: &ClaimTargetValidationContext,
    ) -> Result<(), GraphError> {
        match &self.object {
            ClaimPropositionObject::Entity(node_id) => {
                if context.known_nodes.contains(node_id) {
                    Ok(())
                } else {
                    Err(GraphError::ClaimPropositionEntityNotFound(node_id.clone()))
                }
            }
            ClaimPropositionObject::Literal(_) => Ok(()),
        }
    }

    /// Project the proposition into additive, namespaced node properties.
    ///
    /// Keys are prefixed `proposition_` so a `Claim` node in the epistemic
    /// vocabulary can expose the proposition through Cypher reads without any
    /// existing property changing meaning. Optional fields are omitted rather
    /// than emitted as null.
    pub fn to_property_map(&self) -> PropertyMap {
        let mut properties = PropertyMap::new();
        properties.insert(
            "proposition_subject".to_owned(),
            PropertyValue::String(self.subject.clone()),
        );
        properties.insert(
            "proposition_predicate".to_owned(),
            PropertyValue::String(self.predicate.clone()),
        );

        let (object_kind, object_value) = match &self.object {
            ClaimPropositionObject::Entity(node_id) => {
                ("entity", PropertyValue::String(node_id.as_str().to_owned()))
            }
            ClaimPropositionObject::Literal(value) => ("literal", value.clone()),
        };
        properties.insert(
            "proposition_object_kind".to_owned(),
            PropertyValue::String(object_kind.to_owned()),
        );
        properties.insert("proposition_object".to_owned(), object_value);

        properties.insert(
            "proposition_polarity".to_owned(),
            PropertyValue::String(self.polarity.as_str().to_owned()),
        );
        properties.insert(
            "proposition_modality".to_owned(),
            PropertyValue::String(self.modality.as_str().to_owned()),
        );

        if let Some(scope) = &self.valid_time {
            if let Some(from) = &scope.valid_from {
                properties.insert(
                    "proposition_valid_from".to_owned(),
                    PropertyValue::String(from.as_str().to_owned()),
                );
            }
            if let Some(until) = &scope.valid_until {
                properties.insert(
                    "proposition_valid_until".to_owned(),
                    PropertyValue::String(until.as_str().to_owned()),
                );
            }
        }

        if let Some(version) = &self.extraction_version {
            properties.insert(
                "proposition_extraction_version".to_owned(),
                PropertyValue::String(version.clone()),
            );
        }

        properties
    }
}

/// Trim a caller-provided field and reject blank values with a message naming
/// the field, so validation errors point at the offending proposition part.
fn non_blank(value: String, field: &str) -> Result<String, GraphError> {
    let trimmed = value.trim();

    if trimmed.is_empty() {
        return Err(GraphError::InvalidPropertyValue(format!(
            "{field} must not be empty"
        )));
    }

    Ok(trimmed.to_owned())
}

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
//! Named confidence dimensions; no aggregation or actionability policy lives here.
use crate::Confidence;
use serde::{Deserialize, Serialize};

/// Closed vocabulary of confidence dimensions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfidenceDimension {
    /// Evidence sufficiency.
    EvidenceSufficiency,
    /// Source authority.
    SourceAuthority,
    /// Source independence.
    SourceIndependence,
    /// Extraction certainty.
    ExtractionCertainty,
    /// Entity resolution certainty.
    EntityResolutionCertainty,
    /// Temporal validity.
    TemporalValidity,
    /// Contradiction load.
    ContradictionLoad,
    /// Verifier strength.
    VerifierStrength,
    /// Epistemic uncertainty.
    EpistemicUncertainty,
    /// Actionability.
    Actionability,
}

/// Independently optional confidence dimensions. Absence means uncomputed, never zero.
/// `contradiction_load` and `epistemic_uncertainty` increase as evidence quality worsens.
/// Use `favorable_value` for comparisons in which higher must mean better.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfidenceDimensions {
    /// Evidence sufficiency.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_dimension"
    )]
    pub evidence_sufficiency: Option<Confidence>,
    /// Source authority.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_dimension"
    )]
    pub source_authority: Option<Confidence>,
    /// Source independence.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_dimension"
    )]
    pub source_independence: Option<Confidence>,
    /// Extraction certainty.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_dimension"
    )]
    pub extraction_certainty: Option<Confidence>,
    /// Entity resolution certainty.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_dimension"
    )]
    pub entity_resolution_certainty: Option<Confidence>,
    /// Temporal validity.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_dimension"
    )]
    pub temporal_validity: Option<Confidence>,
    /// Contradiction load (higher is worse).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_dimension"
    )]
    pub contradiction_load: Option<Confidence>,
    /// Verifier strength.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_dimension"
    )]
    pub verifier_strength: Option<Confidence>,
    /// Epistemic uncertainty (higher is worse).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_dimension"
    )]
    pub epistemic_uncertainty: Option<Confidence>,
    /// Actionability.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_dimension"
    )]
    pub actionability: Option<Confidence>,
}
impl ConfidenceDimension {
    /// Whether a larger raw value represents worse quality.
    pub fn is_inverted(self) -> bool {
        matches!(self, Self::ContradictionLoad | Self::EpistemicUncertainty)
    }
}
impl ConfidenceDimensions {
    /// Display-only projection; never an input to engine policies.
    pub fn display_confidence(&self) -> Option<Confidence> {
        Confidence::new(
            self.evidence_sufficiency?
                .value()
                .min(self.source_independence?.value())
                * self.verifier_strength.map_or(1.0, Confidence::value),
        )
        .ok()
    }
    /// Whether every dimension is uncomputed.
    pub fn is_empty(&self) -> bool {
        self.present_values().next().is_none()
    }
    /// Return a score oriented so higher is better, preserving absence.
    pub fn favorable_value(&self, dimension: ConfidenceDimension) -> Option<Confidence> {
        let raw = match dimension {
            ConfidenceDimension::EvidenceSufficiency => self.evidence_sufficiency,
            ConfidenceDimension::SourceAuthority => self.source_authority,
            ConfidenceDimension::SourceIndependence => self.source_independence,
            ConfidenceDimension::ExtractionCertainty => self.extraction_certainty,
            ConfidenceDimension::EntityResolutionCertainty => self.entity_resolution_certainty,
            ConfidenceDimension::TemporalValidity => self.temporal_validity,
            ConfidenceDimension::ContradictionLoad => self.contradiction_load,
            ConfidenceDimension::VerifierStrength => self.verifier_strength,
            ConfidenceDimension::EpistemicUncertainty => self.epistemic_uncertainty,
            ConfidenceDimension::Actionability => self.actionability,
        }?;
        if dimension.is_inverted() {
            Some(Confidence::new(1.0 - raw.value()).expect("complement of bounded confidence"))
        } else {
            Some(raw)
        }
    }
    /// Iterate over computed dimensions using their canonical property names.
    pub fn present_values(&self) -> impl Iterator<Item = (&'static str, Confidence)> + '_ {
        [
            ("evidence_sufficiency", self.evidence_sufficiency),
            ("source_authority", self.source_authority),
            ("source_independence", self.source_independence),
            ("extraction_certainty", self.extraction_certainty),
            (
                "entity_resolution_certainty",
                self.entity_resolution_certainty,
            ),
            ("temporal_validity", self.temporal_validity),
            ("contradiction_load", self.contradiction_load),
            ("verifier_strength", self.verifier_strength),
            ("epistemic_uncertainty", self.epistemic_uncertainty),
            ("actionability", self.actionability),
        ]
        .into_iter()
        .filter_map(|(name, value)| value.map(|value| (name, value)))
    }
    /// Consume known legacy keys; leave unknown keys for the verdict migration audit.
    pub(crate) fn take_legacy(
        values: &mut std::collections::BTreeMap<String, Option<Confidence>>,
    ) -> Result<Self, crate::GraphError> {
        let result = Self {
            evidence_sufficiency: values.remove("evidence_sufficiency").flatten(),
            source_authority: values.remove("source_authority").flatten(),
            source_independence: values.remove("source_independence").flatten(),
            extraction_certainty: values.remove("extraction_certainty").flatten(),
            entity_resolution_certainty: values.remove("entity_resolution_certainty").flatten(),
            temporal_validity: values.remove("temporal_validity").flatten(),
            contradiction_load: values.remove("contradiction_load").flatten(),
            verifier_strength: values.remove("verifier_strength").flatten(),
            epistemic_uncertainty: values.remove("epistemic_uncertainty").flatten(),
            actionability: values.remove("actionability").flatten(),
        };
        for (_, value) in result.present_values() {
            Confidence::new(value.value())?;
        }
        Ok(result)
    }
}

fn deserialize_dimension<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Confidence>, D::Error> {
    Option::<f64>::deserialize(deserializer)?
        .map(Confidence::new)
        .transpose()
        .map_err(serde::de::Error::custom)
}

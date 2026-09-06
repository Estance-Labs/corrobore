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
//! Immutable `Observation` record and store (Epic 0029, WS-A item 3).
//!
//! Module boundary:
//! this module owns the exact span, region, or structured record actually
//! observed in a registered `Source`: a verbatim payload, an optional selector,
//! a modality, an observation time, and an optional payload hash. It does not
//! link observations to claims, compute verdicts, or enforce reachability;
//! those belong to items 4 to 6.
//!
//! Validation targets:
//! - the source must be registered in the [`SourceStore`] handed to the call;
//! - the payload is non-blank; the payload hash, when present, is 64 lowercase
//!   hexadecimal characters; the selector, when present, passes the shared
//!   [`EvidenceLocator`] rules;
//! - no update path: an identical re-creation is a no-op, a differing payload
//!   under the same identifier is a conflict, and the only correction is a
//!   superseding observation that keeps the old one queryable.
//!
//! Compatibility targets:
//! - legacy `EvidenceRecord`s lift into observations idempotently once their
//!   source has been lifted, keeping payload and offsets unchanged;
//! - the graph-facing projection is additive and namespaced under
//!   `observation_*` and never duplicates the verbatim payload.
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{
    EvidenceLocator, EvidenceRecord, GraphError, ImmutableRecordKind, ObservationId, PropertyMap,
    PropertyValue, SourceId, SourceStore, TemporalTimestamp,
};

/// What kind of thing was observed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObservationModality {
    /// A text span.
    Text,
    /// A region of an image or video frame.
    ImageRegion,
    /// A structured record or field (JSON, STIX object, table row).
    StructuredRecord,
}

impl ObservationModality {
    /// Closed vocabulary in canonical order.
    pub const ALL: [Self; 3] = [Self::Text, Self::ImageRegion, Self::StructuredRecord];

    /// Canonical lowercase token used in projections.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::ImageRegion => "image_region",
            Self::StructuredRecord => "structured_record",
        }
    }
}

/// Input builder for creating an observation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservationInput {
    id: ObservationId,
    source_id: SourceId,
    selector: Option<EvidenceLocator>,
    payload: String,
    modality: ObservationModality,
    observed_at: Option<TemporalTimestamp>,
    payload_sha256: Option<String>,
}

impl ObservationInput {
    /// Start an input from the mandatory fields.
    pub fn new(
        id: ObservationId,
        source_id: SourceId,
        payload: impl Into<String>,
        modality: ObservationModality,
    ) -> Self {
        Self {
            id,
            source_id,
            selector: None,
            payload: payload.into(),
            modality,
            observed_at: None,
            payload_sha256: None,
        }
    }

    /// Set the selector locating the observation inside the source.
    pub fn with_selector(mut self, selector: EvidenceLocator) -> Self {
        self.selector = Some(selector);
        self
    }

    /// Set the observation timestamp.
    pub fn with_observed_at(mut self, observed_at: TemporalTimestamp) -> Self {
        self.observed_at = Some(observed_at);
        self
    }

    /// Set the SHA-256 digest of the verbatim payload.
    pub fn with_payload_sha256(mut self, payload_sha256: impl Into<String>) -> Self {
        self.payload_sha256 = Some(payload_sha256.into());
        self
    }

    /// Validate every field before creation.
    ///
    /// # Errors
    ///
    /// [`GraphError::InvalidPropertyValue`] naming the offending field.
    fn validate(&self) -> Result<(), GraphError> {
        if self.payload.trim().is_empty() {
            return Err(GraphError::InvalidPropertyValue(
                "observation payload must not be empty".to_owned(),
            ));
        }

        if let Some(digest) = self.payload_sha256.as_deref()
            && !(digest.len() == 64
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
        {
            return Err(GraphError::InvalidPropertyValue(
                "observation payload_sha256 must be 64 lowercase hexadecimal characters".to_owned(),
            ));
        }

        if let Some(selector) = &self.selector {
            selector.validate()?;
        }

        Ok(())
    }
}

/// One immutable observation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Observation {
    id: ObservationId,
    source_id: SourceId,
    selector: Option<EvidenceLocator>,
    payload: String,
    modality: ObservationModality,
    observed_at: Option<TemporalTimestamp>,
    payload_sha256: Option<String>,
    supersedes: Option<ObservationId>,
    derived_from_legacy: bool,
}

impl Observation {
    /// Identifier.
    pub fn id(&self) -> &ObservationId {
        &self.id
    }

    /// Source the observation was taken from.
    pub fn source_id(&self) -> &SourceId {
        &self.source_id
    }

    /// Selector inside the source, when known.
    pub fn selector(&self) -> Option<&EvidenceLocator> {
        self.selector.as_ref()
    }

    /// Verbatim payload.
    pub fn payload(&self) -> &str {
        self.payload.as_str()
    }

    /// Modality.
    pub fn modality(&self) -> ObservationModality {
        self.modality
    }

    /// Observation timestamp, when known.
    pub fn observed_at(&self) -> Option<&TemporalTimestamp> {
        self.observed_at.as_ref()
    }

    /// Payload digest, when known.
    pub fn payload_sha256(&self) -> Option<&str> {
        self.payload_sha256.as_deref()
    }

    /// Observation this one corrects, when it is a correction.
    pub fn supersedes(&self) -> Option<&ObservationId> {
        self.supersedes.as_ref()
    }

    /// Whether this observation was lifted from a legacy evidence record.
    pub fn derived_from_legacy(&self) -> bool {
        self.derived_from_legacy
    }

    /// Project the observation into additive, namespaced node properties.
    ///
    /// The verbatim payload is deliberately not projected: it stays on the
    /// observation record and is reached through the record, not copied onto
    /// nodes. The selector is rendered through [`EvidenceLocator::render`].
    pub fn to_property_map(&self) -> PropertyMap {
        let mut properties = PropertyMap::new();
        properties.insert(
            "observation_id".to_owned(),
            PropertyValue::String(self.id.as_str().to_owned()),
        );
        properties.insert(
            "observation_source".to_owned(),
            PropertyValue::String(self.source_id.as_str().to_owned()),
        );
        properties.insert(
            "observation_modality".to_owned(),
            PropertyValue::String(self.modality.as_str().to_owned()),
        );
        if let Some(selector) = &self.selector {
            properties.insert(
                "observation_selector".to_owned(),
                PropertyValue::String(selector.render()),
            );
        }
        if let Some(observed_at) = &self.observed_at {
            properties.insert(
                "observation_observed_at".to_owned(),
                PropertyValue::String(observed_at.as_str().to_owned()),
            );
        }
        if let Some(digest) = &self.payload_sha256 {
            properties.insert(
                "observation_payload_sha256".to_owned(),
                PropertyValue::String(digest.clone()),
            );
        }
        if let Some(supersedes) = &self.supersedes {
            properties.insert(
                "observation_supersedes".to_owned(),
                PropertyValue::String(supersedes.as_str().to_owned()),
            );
        }
        properties.insert(
            "observation_derived_from_legacy".to_owned(),
            PropertyValue::Bool(self.derived_from_legacy),
        );

        properties
    }

    /// Whether `input` describes the same content as this observation,
    /// ignoring supersession bookkeeping and the legacy marker.
    fn matches_input(&self, input: &ObservationInput) -> bool {
        self.id == input.id
            && self.source_id == input.source_id
            && self.selector == input.selector
            && self.payload == input.payload
            && self.modality == input.modality
            && self.observed_at == input.observed_at
            && self.payload_sha256 == input.payload_sha256
    }
}

/// Append-only store of observations with explicit supersession.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservationStore {
    observations: Vec<Observation>,
    superseded_by: HashMap<ObservationId, ObservationId>,
}

impl ObservationStore {
    /// Creates an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create an observation bound to a source registered in `sources`.
    ///
    /// Idempotent for an identical input; a differing input under the same
    /// identifier is a conflict because observations have no update path.
    ///
    /// # Errors
    ///
    /// [`GraphError::SourceNotFound`] when the source is unknown;
    /// [`GraphError::InvalidPropertyValue`] for invalid input or a conflict.
    pub fn create_observation(
        &mut self,
        input: ObservationInput,
        sources: &SourceStore,
    ) -> Result<ObservationId, GraphError> {
        self.insert(input, sources, None, false)
    }

    /// Correct an observation by creating a new one that supersedes it.
    ///
    /// # Errors
    ///
    /// [`GraphError::ObservationNotFound`] when `previous` is unknown;
    /// [`GraphError::InvalidVersionState`] when it is already superseded;
    /// otherwise the errors of [`Self::create_observation`].
    pub fn supersede_observation(
        &mut self,
        previous: &ObservationId,
        input: ObservationInput,
        sources: &SourceStore,
    ) -> Result<ObservationId, GraphError> {
        if self.observation_by_id(previous).is_none() {
            return Err(GraphError::ObservationNotFound(previous.clone()));
        }
        if let Some(existing) = self.superseded_by.get(previous) {
            return Err(GraphError::InvalidVersionState(format!(
                "observation {} is already superseded by {}",
                previous.as_str(),
                existing.as_str()
            )));
        }
        if self.observation_by_id(&input.id).is_some() {
            return Err(GraphError::InvalidVersionState(format!(
                "superseding observation {} already exists",
                input.id.as_str()
            )));
        }

        let new_id = self.insert(input, sources, Some(previous.clone()), false)?;
        self.superseded_by.insert(previous.clone(), new_id.clone());

        Ok(new_id)
    }

    /// Lift a legacy evidence record into an observation.
    ///
    /// The observation identifier is `observation--<evidence id>`; the source
    /// is the record's `source_id`, which must already be set by
    /// `EvidenceRecordStore::lift_sources`; the selector is the record's
    /// locator, else a byte range from its offsets, else absent; the payload
    /// and observation time are copied; the modality is `Text`.
    ///
    /// # Errors
    ///
    /// [`GraphError::InvalidVersionState`] when the record has no `source_id`;
    /// otherwise the errors of [`Self::create_observation`].
    pub fn lift_from_evidence(
        &mut self,
        record: &EvidenceRecord,
        sources: &SourceStore,
    ) -> Result<ObservationId, GraphError> {
        let Some(source_id) = record.source_id() else {
            return Err(GraphError::InvalidVersionState(format!(
                "evidence {} has no source_id; lift sources before observations",
                record.id().as_str()
            )));
        };

        let id = ObservationId::new(format!("observation--{}", record.id().as_str()))?;
        let mut input = ObservationInput::new(
            id,
            source_id.clone(),
            record.payload(),
            ObservationModality::Text,
        );
        if let Some(locator) = record.locator() {
            input = input.with_selector(locator.clone());
        } else if let (Some(start), Some(end)) = (record.offset_start(), record.offset_end())
            && start < end
        {
            input = input.with_selector(EvidenceLocator::ByteRange { start, end });
        }
        if let Some(observed_at) = record.observed_at() {
            input = input.with_observed_at(observed_at.clone());
        }

        self.insert(input, sources, None, true)
    }

    fn insert(
        &mut self,
        input: ObservationInput,
        sources: &SourceStore,
        supersedes: Option<ObservationId>,
        derived_from_legacy: bool,
    ) -> Result<ObservationId, GraphError> {
        input.validate()?;

        if sources.current_source(&input.source_id).is_none() {
            return Err(GraphError::SourceNotFound(input.source_id));
        }

        if let Some(existing) = self.observation_by_id(&input.id) {
            if existing.matches_input(&input) {
                return Ok(input.id);
            }
            return Err(GraphError::ImmutableRecordConflict {
                kind: ImmutableRecordKind::Observation,
                id: input.id.as_str().to_owned(),
            });
        }

        let id = input.id.clone();
        self.observations.push(Observation {
            id: input.id,
            source_id: input.source_id,
            selector: input.selector,
            payload: input.payload,
            modality: input.modality,
            observed_at: input.observed_at,
            payload_sha256: input.payload_sha256,
            supersedes,
            derived_from_legacy,
        });

        Ok(id)
    }

    /// Every observation, in creation order.
    pub fn observations(&self) -> &[Observation] {
        &self.observations
    }

    /// One observation by identifier.
    pub fn observation_by_id(&self, observation_id: &ObservationId) -> Option<&Observation> {
        self.observations
            .iter()
            .find(|observation| &observation.id == observation_id)
    }

    /// Whether the observation exists and has not been superseded.
    pub fn is_current(&self, observation_id: &ObservationId) -> bool {
        self.observation_by_id(observation_id).is_some()
            && !self.superseded_by.contains_key(observation_id)
    }

    /// The observation that supersedes `observation_id`, when any.
    pub fn superseded_by(&self, observation_id: &ObservationId) -> Option<&ObservationId> {
        self.superseded_by.get(observation_id)
    }

    /// Number of stored observations, superseded ones included.
    pub fn len(&self) -> usize {
        self.observations.len()
    }

    /// Whether no observation is stored.
    pub fn is_empty(&self) -> bool {
        self.observations.is_empty()
    }
}

impl ObservationStore {
    pub(crate) fn audit_subset(&self, ids: &std::collections::HashSet<ObservationId>) -> Self {
        Self {
            observations: self
                .observations
                .iter()
                .filter(|r| ids.contains(r.id()))
                .cloned()
                .collect(),
            superseded_by: self
                .superseded_by
                .iter()
                .filter(|(a, b)| ids.contains(*a) && ids.contains(*b))
                .map(|(a, b)| (a.clone(), b.clone()))
                .collect(),
        }
    }
}

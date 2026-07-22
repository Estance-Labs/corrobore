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
//! Multi-resolution graph model contracts (Epic 0022).
//!
//! This module defines tactical/operational/strategic resolution levels,
//! typed cross-level artifact references, and auditable derivation links that
//! preserve backward provenance to lower-level sources.

use serde::{Deserialize, Serialize};

use crate::{FactId, GraphError};

/// One explicit graph reasoning resolution level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ResolutionLevel {
    /// Fine-grained observations (posts, URLs, account actions, timestamps).
    Tactical,
    /// Mid-level operational structures (claims, narratives, campaigns).
    Operational,
    /// High-level strategic framing (actors, objectives, trends, regions).
    Strategic,
}

impl ResolutionLevel {
    /// Stable complete level vocabulary in deterministic order.
    pub const ALL: [ResolutionLevel; 3] = [
        ResolutionLevel::Tactical,
        ResolutionLevel::Operational,
        ResolutionLevel::Strategic,
    ];

    const fn rank(self) -> u8 {
        match self {
            ResolutionLevel::Tactical => 0,
            ResolutionLevel::Operational => 1,
            ResolutionLevel::Strategic => 2,
        }
    }
}

/// Metadata describing one reasoning level.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolutionLevelMetadata {
    level: ResolutionLevel,
    label: String,
    expected_scope: String,
}

impl ResolutionLevelMetadata {
    #[must_use]
    fn new(level: ResolutionLevel, label: &str, expected_scope: &str) -> Self {
        Self {
            level,
            label: label.to_owned(),
            expected_scope: expected_scope.to_owned(),
        }
    }

    /// Returns the typed resolution level.
    #[must_use]
    pub const fn level(&self) -> ResolutionLevel {
        self.level
    }
}

/// Typed identifier of one multi-resolution artifact.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ResolutionArtifactId {
    value: String,
}

impl ResolutionArtifactId {
    /// Creates a validated artifact identifier.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::InvalidIdentifier`] when `value` is blank.
    pub fn new(value: impl Into<String>) -> Result<Self, GraphError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(GraphError::InvalidIdentifier(
                "ResolutionArtifactId".to_owned(),
            ));
        }
        Ok(Self { value })
    }

    /// Returns the artifact identifier as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.value.as_str()
    }
}

/// Typed reference to one artifact within one explicit resolution level.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ResolutionRecordRef {
    level: ResolutionLevel,
    artifact_id: ResolutionArtifactId,
}

impl ResolutionRecordRef {
    /// Creates a typed artifact reference.
    #[must_use]
    pub fn new(level: ResolutionLevel, artifact_id: ResolutionArtifactId) -> Self {
        Self { level, artifact_id }
    }

    /// Returns the reference resolution level.
    #[must_use]
    pub const fn level(&self) -> ResolutionLevel {
        self.level
    }
}

/// One registered artifact at one resolution level.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolutionArtifact {
    record_ref: ResolutionRecordRef,
    title: String,
}

impl ResolutionArtifact {
    /// Creates one validated resolution artifact.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::InvalidResolutionModel`] when `title` is blank.
    pub fn new(
        record_ref: ResolutionRecordRef,
        title: impl Into<String>,
    ) -> Result<Self, GraphError> {
        let title = title.into();
        if title.trim().is_empty() {
            return Err(GraphError::InvalidResolutionModel(
                "resolution artifact title must not be blank".to_owned(),
            ));
        }
        Ok(Self { record_ref, title })
    }

    #[must_use]
    fn record_ref(&self) -> &ResolutionRecordRef {
        &self.record_ref
    }
}

/// Typed identifier of one auditable derivation link.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DerivationLinkId {
    value: String,
}

impl DerivationLinkId {
    /// Creates a validated derivation-link identifier.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::InvalidIdentifier`] when `value` is blank.
    pub fn new(value: impl Into<String>) -> Result<Self, GraphError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(GraphError::InvalidIdentifier("DerivationLinkId".to_owned()));
        }
        Ok(Self { value })
    }

    /// Returns the derivation-link identifier as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.value.as_str()
    }
}

/// Auditable derivation link from lower-level sources to one higher-level artifact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DerivationLink {
    id: DerivationLinkId,
    target: ResolutionRecordRef,
    supporting_sources: Vec<ResolutionRecordRef>,
    provenance_fact_refs: Vec<FactId>,
}

impl DerivationLink {
    /// Creates a derivation link with deterministic source/provenance ordering.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::InvalidResolutionModel`] when no supporting source
    /// or no provenance fact is supplied.
    pub fn new(
        id: DerivationLinkId,
        target: ResolutionRecordRef,
        supporting_sources: Vec<ResolutionRecordRef>,
        provenance_fact_refs: Vec<FactId>,
    ) -> Result<Self, GraphError> {
        if supporting_sources.is_empty() {
            return Err(GraphError::InvalidResolutionModel(
                "derivation link must include at least one supporting source".to_owned(),
            ));
        }
        if provenance_fact_refs.is_empty() {
            return Err(GraphError::InvalidResolutionModel(
                "derivation link must include at least one provenance fact".to_owned(),
            ));
        }

        let mut supporting_sources = supporting_sources;
        supporting_sources.sort();
        supporting_sources.dedup();

        let mut provenance_fact_refs = provenance_fact_refs;
        provenance_fact_refs.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        provenance_fact_refs.dedup_by(|left, right| left == right);

        Ok(Self {
            id,
            target,
            supporting_sources,
            provenance_fact_refs,
        })
    }

    #[must_use]
    fn id(&self) -> &DerivationLinkId {
        &self.id
    }

    #[must_use]
    fn target(&self) -> &ResolutionRecordRef {
        &self.target
    }

    /// Returns sorted supporting lower-level source references.
    #[must_use]
    pub fn supporting_sources(&self) -> &[ResolutionRecordRef] {
        self.supporting_sources.as_slice()
    }

    /// Returns sorted provenance fact references.
    #[must_use]
    pub fn provenance_fact_refs(&self) -> &[FactId] {
        self.provenance_fact_refs.as_slice()
    }
}

/// Multi-resolution model with auditable derivation links.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MultiResolutionModel {
    level_metadata: Vec<ResolutionLevelMetadata>,
    artifacts: Vec<ResolutionArtifact>,
    derivation_links: Vec<DerivationLink>,
}

impl MultiResolutionModel {
    /// Creates an empty model with the canonical level metadata catalog.
    #[must_use]
    pub fn new() -> Self {
        Self {
            level_metadata: vec![
                ResolutionLevelMetadata::new(
                    ResolutionLevel::Tactical,
                    "tactical",
                    "accounts, posts, urls, and timestamps",
                ),
                ResolutionLevelMetadata::new(
                    ResolutionLevel::Operational,
                    "operational",
                    "claims, narratives, and campaigns",
                ),
                ResolutionLevelMetadata::new(
                    ResolutionLevel::Strategic,
                    "strategic",
                    "actors, objectives, trends, and regions",
                ),
            ],
            artifacts: Vec::new(),
            derivation_links: Vec::new(),
        }
    }

    /// Returns the deterministic level metadata catalog.
    #[must_use]
    pub fn level_metadata(&self) -> &[ResolutionLevelMetadata] {
        self.level_metadata.as_slice()
    }

    /// Registers one resolution artifact.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::InvalidResolutionModel`] when the reference already exists.
    pub fn register_artifact(mut self, artifact: ResolutionArtifact) -> Result<Self, GraphError> {
        if self
            .artifacts
            .iter()
            .any(|candidate| candidate.record_ref() == artifact.record_ref())
        {
            return Err(GraphError::InvalidResolutionModel(format!(
                "duplicate resolution artifact reference: {:?}",
                artifact.record_ref()
            )));
        }
        self.artifacts.push(artifact);
        self.artifacts
            .sort_by(|left, right| left.record_ref().cmp(right.record_ref()));
        Ok(self)
    }

    /// Adds one auditable derivation link.
    ///
    /// # Errors
    ///
    /// Returns [`GraphError::InvalidResolutionModel`] for duplicate link IDs,
    /// unknown references, or invalid level transitions.
    pub fn add_derivation_link(mut self, link: DerivationLink) -> Result<Self, GraphError> {
        if self
            .derivation_links
            .iter()
            .any(|candidate| candidate.id() == link.id())
        {
            return Err(GraphError::InvalidResolutionModel(format!(
                "duplicate derivation link identifier: {}",
                link.id().as_str()
            )));
        }

        if !self
            .artifacts
            .iter()
            .any(|candidate| candidate.record_ref() == link.target())
        {
            return Err(GraphError::InvalidResolutionModel(format!(
                "unknown derivation target artifact reference: {:?}",
                link.target()
            )));
        }

        for source in link.supporting_sources() {
            if !self
                .artifacts
                .iter()
                .any(|candidate| candidate.record_ref() == source)
            {
                return Err(GraphError::InvalidResolutionModel(format!(
                    "unknown derivation source artifact reference: {:?}",
                    source
                )));
            }
            if source.level().rank() >= link.target().level().rank() {
                return Err(GraphError::InvalidResolutionModel(format!(
                    "invalid derivation level transition: {:?} -> {:?}",
                    source.level(),
                    link.target().level()
                )));
            }
        }

        self.derivation_links.push(link);
        self.derivation_links
            .sort_by(|left, right| left.id().cmp(right.id()));
        Ok(self)
    }

    /// Returns all derivation links for one target artifact, in deterministic order.
    #[must_use]
    pub fn derivation_links_for(&self, target: &ResolutionRecordRef) -> Vec<&DerivationLink> {
        self.derivation_links
            .iter()
            .filter(|link| link.target() == target)
            .collect()
    }
}

impl Default for MultiResolutionModel {
    fn default() -> Self {
        Self::new()
    }
}

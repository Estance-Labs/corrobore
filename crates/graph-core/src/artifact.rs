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
//! Investigation artifacts bound to live records.
//!
//! Module boundary: an artifact is a view, not a fact. This module retains what
//! an artifact is built from and who annotated it. It computes no verdict,
//! reads none while publishing, and holds no factual text.
//!
//! That last point is structural rather than a convention: the type has a
//! title, analyst annotations, and identifier bindings, and no field anywhere
//! for a factual statement. An artifact therefore cannot become a second,
//! stale copy of the evidence — regenerate it and it says what the records say
//! now, while every annotation an analyst wrote is carried forward.
//!
//! Publication is not an epistemic act. A publication state is appended as a
//! new version and never consults a verdict, so publishing a brief about a
//! refuted claim is a permissions decision that leaves the epistemic record
//! exactly as it was.
use crate::*;
use serde::{Deserialize, Serialize};

/// What an artifact presents.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    /// Events ordered in time.
    Timeline,
    /// Evidence and its sources around a question.
    EvidenceMap,
    /// Claims against dimensions.
    ClaimMatrix,
    /// A campaign and the collections it spans.
    CampaignGraph,
    /// A written brief for a human reader.
    AnalystBrief,
}

/// Whether an artifact may be shown, and to whom it has been released.
///
/// Independent of truth: a state change appends a version and reads no verdict.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicationState {
    /// Held by its authors.
    Draft,
    /// Released to its audience.
    Published,
    /// Released and then withdrawn; earlier versions remain.
    Withdrawn,
}

/// The records an artifact is built from, by identity only.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactBinding {
    /// Governed claims presented.
    pub claims: Vec<ClaimId>,
    /// Evidence records presented.
    pub evidence: Vec<EvidenceId>,
    /// Neutral narrative collections presented.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub narratives: Vec<NarrativeId>,
    /// Neutral campaign collections presented.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub campaigns: Vec<CampaignId>,
}

/// One analyst note, preserved across every regeneration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactAnnotation {
    id: String,
    author: ActorId,
    note: String,
    stamp: BitemporalStamp,
}

/// One immutable artifact version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactVersion {
    version: u64,
    binding: ArtifactBinding,
    publication: PublicationState,
    annotations: Vec<ArtifactAnnotation>,
    stamp: BitemporalStamp,
}

/// Creation request for an artifact and its first version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactInput {
    id: String,
    kind: ArtifactKind,
    title: String,
    binding: ArtifactBinding,
    annotations: Vec<ArtifactAnnotation>,
    stamp: BitemporalStamp,
}

/// One artifact and its complete version lineage.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    id: String,
    kind: ArtifactKind,
    title: String,
    versions: Vec<ArtifactVersion>,
}

/// Append-only artifacts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "StoredArtifacts")]
pub struct ArtifactStore {
    artifacts: Vec<Artifact>,
}

impl ArtifactKind {
    /// Closed vocabulary in canonical order.
    pub const ALL: [Self; 5] = [
        Self::Timeline,
        Self::EvidenceMap,
        Self::ClaimMatrix,
        Self::CampaignGraph,
        Self::AnalystBrief,
    ];

    /// Canonical snake_case token.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Timeline => "timeline",
            Self::EvidenceMap => "evidence_map",
            Self::ClaimMatrix => "claim_matrix",
            Self::CampaignGraph => "campaign_graph",
            Self::AnalystBrief => "analyst_brief",
        }
    }
}

impl PublicationState {
    /// Canonical snake_case token.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Published => "published",
            Self::Withdrawn => "withdrawn",
        }
    }
}

impl ArtifactBinding {
    /// Whether the binding names no record at all.
    pub fn is_empty(&self) -> bool {
        self.claims.is_empty()
            && self.evidence.is_empty()
            && self.narratives.is_empty()
            && self.campaigns.is_empty()
    }

    fn validate_structure(&self) -> Result<(), GraphError> {
        // An artifact that names nothing would have to carry its content some
        // other way, which is the copied-text failure this type exists to
        // prevent.
        if self.is_empty() {
            return Err(invalid(
                "an artifact must bind at least one governed record",
            ));
        }
        unique(self.claims.iter().map(ClaimId::as_str))?;
        unique(self.evidence.iter().map(EvidenceId::as_str))?;
        unique(self.narratives.iter().map(NarrativeId::as_str))?;
        unique(self.campaigns.iter().map(CampaignId::as_str))
    }

    fn validate_bindings(&self, stores: &EpistemicStores) -> Result<(), GraphError> {
        self.validate_structure()?;
        for claim in &self.claims {
            stores.claims.claim_by_id(claim)?;
        }
        for narrative in &self.narratives {
            if stores
                .narrative_campaigns
                .narrative_by_id(narrative)
                .is_none()
            {
                return Err(invalid(format!(
                    "artifact narrative {} is missing",
                    narrative.as_str()
                )));
            }
        }
        for campaign in &self.campaigns {
            if stores
                .narrative_campaigns
                .campaign_by_id(campaign)
                .is_none()
            {
                return Err(invalid(format!(
                    "artifact campaign {} is missing",
                    campaign.as_str()
                )));
            }
        }
        Ok(())
    }
}

impl ArtifactAnnotation {
    /// Record one analyst note.
    ///
    /// # Errors
    /// [`GraphError::InvalidPropertyValue`] for a blank identity or note.
    pub fn new(
        id: impl Into<String>,
        author: ActorId,
        note: impl Into<String>,
        stamp: BitemporalStamp,
    ) -> Result<Self, GraphError> {
        let annotation = Self {
            id: id.into(),
            author,
            note: note.into(),
            stamp,
        };
        annotation.validate_structure()?;
        Ok(annotation)
    }

    /// Stable annotation identity.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Analyst who wrote it.
    pub fn author(&self) -> &ActorId {
        &self.author
    }

    /// The note. Analyst commentary, never a factual record.
    pub fn note(&self) -> &str {
        &self.note
    }

    /// When it was written.
    pub fn stamp(&self) -> &BitemporalStamp {
        &self.stamp
    }

    fn validate_structure(&self) -> Result<(), GraphError> {
        if self.id.trim().is_empty() || self.note.trim().is_empty() {
            return Err(invalid("an annotation requires an identity and a note"));
        }
        validate_stamp(&self.stamp)
    }
}

impl ArtifactVersion {
    /// Monotonic version, starting at one.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Records this version presents.
    pub fn binding(&self) -> &ArtifactBinding {
        &self.binding
    }

    /// Publication state at this version.
    pub fn publication(&self) -> PublicationState {
        self.publication
    }

    /// Analyst notes carried at this version.
    pub fn annotations(&self) -> &[ArtifactAnnotation] {
        &self.annotations
    }

    /// When the version was recorded.
    pub fn stamp(&self) -> &BitemporalStamp {
        &self.stamp
    }
}

impl ArtifactInput {
    /// Prepare an artifact and its first version.
    pub fn new(
        id: impl Into<String>,
        kind: ArtifactKind,
        title: impl Into<String>,
        binding: ArtifactBinding,
        stamp: BitemporalStamp,
    ) -> Self {
        Self {
            id: id.into(),
            kind,
            title: title.into(),
            binding,
            annotations: Vec::new(),
            stamp,
        }
    }

    /// Attach one analyst note to the first version.
    pub fn with_annotation(mut self, annotation: ArtifactAnnotation) -> Self {
        self.annotations.push(annotation);
        self
    }
}

impl Artifact {
    /// Stable artifact identity.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// What the artifact presents.
    pub fn kind(&self) -> ArtifactKind {
        self.kind
    }

    /// Human title. Presentation, never a factual claim.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Every version, oldest first.
    pub fn versions(&self) -> &[ArtifactVersion] {
        &self.versions
    }

    /// The newest version.
    pub fn current(&self) -> &ArtifactVersion {
        self.versions
            .last()
            .expect("an artifact always has one version")
    }

    /// One version by number.
    pub fn version(&self, version: u64) -> Option<&ArtifactVersion> {
        self.versions
            .iter()
            .find(|candidate| candidate.version == version)
    }

    fn validate_structure(&self) -> Result<(), GraphError> {
        if self.id.trim().is_empty() || self.title.trim().is_empty() {
            return Err(invalid("an artifact requires an identity and a title"));
        }
        if self.versions.is_empty() {
            return Err(invalid("an artifact requires at least one version"));
        }
        for (position, version) in self.versions.iter().enumerate() {
            if version.version != position as u64 + 1 {
                return Err(invalid(format!(
                    "artifact {} versions must be consecutive from one",
                    self.id
                )));
            }
            version.binding.validate_structure()?;
            unique(version.annotations.iter().map(ArtifactAnnotation::id))?;
            for annotation in &version.annotations {
                annotation.validate_structure()?;
            }
            validate_stamp(&version.stamp)?;
        }
        Ok(())
    }
}

impl ArtifactStore {
    /// Whether no artifact exists.
    pub fn is_empty(&self) -> bool {
        self.artifacts.is_empty()
    }

    /// Every artifact in creation order.
    pub fn artifacts(&self) -> &[Artifact] {
        &self.artifacts
    }

    /// One artifact by identity.
    pub fn artifact_by_id(&self, id: &str) -> Option<&Artifact> {
        self.artifacts.iter().find(|artifact| artifact.id() == id)
    }

    fn validate_structure(&self) -> Result<(), GraphError> {
        unique(self.artifacts.iter().map(Artifact::id))?;
        for artifact in &self.artifacts {
            artifact.validate_structure()?;
        }
        Ok(())
    }

    /// Validate every record an artifact version binds.
    ///
    /// # Errors
    /// [`GraphError::ClaimNotFound`] for an unknown claim, and
    /// [`GraphError::InvalidPropertyValue`] for an unknown collection.
    pub(crate) fn validate_bindings(&self, stores: &EpistemicStores) -> Result<(), GraphError> {
        self.validate_structure()?;
        for artifact in &self.artifacts {
            for version in &artifact.versions {
                version.binding.validate_bindings(stores)?;
            }
        }
        Ok(())
    }
}

fn invalid(detail: impl Into<String>) -> GraphError {
    GraphError::InvalidPropertyValue(detail.into())
}

fn unique<'a>(values: impl IntoIterator<Item = &'a str>) -> Result<(), GraphError> {
    let mut seen = std::collections::BTreeSet::new();
    for value in values {
        if value.trim().is_empty() || !seen.insert(value) {
            return Err(invalid(
                "artifact references require nonblank distinct identities",
            ));
        }
    }
    Ok(())
}

fn validate_stamp(stamp: &BitemporalStamp) -> Result<(), GraphError> {
    let validated = BitemporalStamp::new(stamp.valid_from.clone(), stamp.transaction_time.clone())?;
    let _ = validated;
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredArtifacts {
    artifacts: Vec<Artifact>,
}

impl TryFrom<StoredArtifacts> for ArtifactStore {
    type Error = GraphError;
    fn try_from(stored: StoredArtifacts) -> Result<Self, Self::Error> {
        let store = Self {
            artifacts: stored.artifacts,
        };
        store.validate_structure()?;
        Ok(store)
    }
}

impl Graph {
    /// Create an artifact and its first version.
    ///
    /// Idempotent by content; reusing an identity for different content is a
    /// conflict, because an artifact's lineage is its history.
    ///
    /// # Errors
    /// [`GraphError::InvalidPropertyValue`] for invalid input or a reused
    /// identity, and a store error for an unknown bound record.
    pub fn create_artifact(&mut self, input: ArtifactInput) -> Result<String, GraphError> {
        let artifact = Artifact {
            id: input.id,
            kind: input.kind,
            title: input.title,
            versions: vec![ArtifactVersion {
                version: 1,
                binding: input.binding,
                publication: PublicationState::Draft,
                annotations: input.annotations,
                stamp: input.stamp,
            }],
        };
        artifact.validate_structure()?;
        artifact
            .current()
            .binding
            .validate_bindings(self.epistemic_stores())?;
        let store = &mut self.epistemic_stores_mut().artifacts;
        if let Some(existing) = store.artifact_by_id(artifact.id()) {
            if existing == &artifact {
                return Ok(artifact.id);
            }
            return Err(invalid(format!(
                "artifact {} already exists with different content",
                artifact.id
            )));
        }
        let id = artifact.id.clone();
        store.artifacts.push(artifact);
        Ok(id)
    }

    /// Append an analyst note to the current version's successor.
    ///
    /// # Errors
    /// [`GraphError::InvalidPropertyValue`] for an unknown artifact or a
    /// duplicate annotation identity.
    pub fn annotate_artifact(
        &mut self,
        id: &str,
        annotation: ArtifactAnnotation,
        stamp: BitemporalStamp,
    ) -> Result<u64, GraphError> {
        annotation.validate_structure()?;
        let artifact = self
            .epistemic_stores()
            .artifacts
            .artifact_by_id(id)
            .ok_or_else(|| invalid(format!("artifact {id} is missing")))?;
        let current = artifact.current();
        let mut annotations = current.annotations.clone();
        if annotations
            .iter()
            .any(|existing| existing.id() == annotation.id())
        {
            return Err(invalid(format!(
                "artifact {id} already carries annotation {}",
                annotation.id()
            )));
        }
        annotations.push(annotation);
        let next = ArtifactVersion {
            version: current.version + 1,
            binding: current.binding.clone(),
            publication: current.publication,
            annotations,
            stamp,
        };
        self.append_artifact_version(id, next)
    }

    /// Rebuild an artifact from the records as they stand now.
    ///
    /// Analyst annotations are carried forward and the publication state is
    /// preserved: regeneration refreshes what the artifact reads from the
    /// graph, and decides nothing about who may see it or whether the
    /// underlying claims hold.
    ///
    /// # Errors
    /// [`GraphError::InvalidPropertyValue`] for an unknown artifact, and a
    /// store error for an unknown bound record.
    pub fn regenerate_artifact(
        &mut self,
        id: &str,
        binding: ArtifactBinding,
        stamp: BitemporalStamp,
    ) -> Result<u64, GraphError> {
        binding.validate_bindings(self.epistemic_stores())?;
        let artifact = self
            .epistemic_stores()
            .artifacts
            .artifact_by_id(id)
            .ok_or_else(|| invalid(format!("artifact {id} is missing")))?;
        let current = artifact.current();
        let next = ArtifactVersion {
            version: current.version + 1,
            binding,
            publication: current.publication,
            annotations: current.annotations.clone(),
            stamp,
        };
        self.append_artifact_version(id, next)
    }

    /// Change an artifact's publication state.
    ///
    /// Publishing is a permissions decision, not an epistemic one: this reads
    /// no verdict and changes no claim, so an artifact about a refuted claim
    /// can be published and a supported one can stay a draft.
    ///
    /// # Errors
    /// [`GraphError::InvalidPropertyValue`] for an unknown artifact.
    pub fn set_artifact_publication(
        &mut self,
        id: &str,
        publication: PublicationState,
        stamp: BitemporalStamp,
    ) -> Result<u64, GraphError> {
        let artifact = self
            .epistemic_stores()
            .artifacts
            .artifact_by_id(id)
            .ok_or_else(|| invalid(format!("artifact {id} is missing")))?;
        let current = artifact.current();
        let next = ArtifactVersion {
            version: current.version + 1,
            binding: current.binding.clone(),
            publication,
            annotations: current.annotations.clone(),
            stamp,
        };
        self.append_artifact_version(id, next)
    }

    fn append_artifact_version(
        &mut self,
        id: &str,
        version: ArtifactVersion,
    ) -> Result<u64, GraphError> {
        validate_stamp(&version.stamp)?;
        let number = version.version;
        let store = &mut self.epistemic_stores_mut().artifacts;
        let artifact = store
            .artifacts
            .iter_mut()
            .find(|artifact| artifact.id == id)
            .ok_or_else(|| invalid(format!("artifact {id} is missing")))?;
        artifact.versions.push(version);
        artifact.validate_structure()?;
        Ok(number)
    }
}

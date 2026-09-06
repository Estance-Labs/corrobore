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
//! Observation-bound mentions; candidate references never constitute resolution.
use crate::{
    EntityMentionId, Graph, GraphError, NodeId, ObservationId, ObservationStore, PropertyMap,
    TemporalTimestamp,
};
use serde::{Deserialize, Serialize};
/// Half-open UTF-8 byte offsets relative to the observation payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MentionOffsets {
    /// Inclusive byte start.
    pub start: u64,
    /// Exclusive byte end.
    pub end: u64,
}
/// Direction of an observed relation around a surface mention.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MentionRelationDirection {
    /// Mention is the observed subject.
    Outgoing,
    /// Mention is the observed object.
    Incoming,
}
/// Observed neighbourhood, without any canonical entity resolution.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MentionRelationFeature {
    /// Observed predicate.
    pub predicate: String,
    /// Relation direction relative to this mention.
    pub direction: MentionRelationDirection,
    /// Verbatim or descriptive counterpart, not an entity ID.
    pub counterpart: String,
}
/// Retained evidence features for later reconciliation.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MentionFeatures {
    /// Context reported by the source; source identity belongs to the observation.
    pub source_context: Option<String>,
    /// Role expressed in the observation.
    pub role: Option<String>,
    /// Event or mention time when supplied by the source.
    pub time: Option<TemporalTimestamp>,
    /// Observed location description.
    pub location: Option<String>,
    /// Observed affiliation names, without implicit identity links.
    pub affiliations: Vec<String>,
    /// Observed neighbouring relations, without materialized graph edges.
    pub relation_neighbourhood: Vec<MentionRelationFeature>,
}
/// Input for one immutable surface mention.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityMentionInput {
    id: EntityMentionId,
    observation_id: ObservationId,
    offsets: MentionOffsets,
    surface_form: String,
    candidate_entities: Vec<NodeId>,
    features: MentionFeatures,
}
impl EntityMentionInput {
    /// Prepare a mention; creation verifies its span against the observation.
    pub fn new(
        id: EntityMentionId,
        observation_id: ObservationId,
        offsets: MentionOffsets,
        surface_form: impl Into<String>,
    ) -> Self {
        Self {
            id,
            observation_id,
            offsets,
            surface_form: surface_form.into(),
            candidate_entities: vec![],
            features: MentionFeatures::default(),
        }
    }
    /// Retain possible entity references, without resolving or linking them.
    pub fn with_candidate_entities(mut self, candidates: Vec<NodeId>) -> Self {
        self.candidate_entities = candidates;
        self
    }
    /// Retain source-grounded features for later reconciliation.
    pub fn with_features(mut self, features: MentionFeatures) -> Self {
        self.features = features;
        self
    }
}
/// An immutable mention, never a canonical entity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityMention {
    #[serde(flatten)]
    input: EntityMentionInput,
}
impl EntityMention {
    /// Stable mention identity.
    pub fn id(&self) -> &EntityMentionId {
        &self.input.id
    }
    /// Observation providing the surface form.
    pub fn observation_id(&self) -> &ObservationId {
        &self.input.observation_id
    }
    /// Verbatim observed text.
    pub fn surface_form(&self) -> &str {
        &self.input.surface_form
    }
    /// Half-open byte span in the observation payload.
    pub fn offsets(&self) -> MentionOffsets {
        self.input.offsets
    }
    /// Unresolved candidate references; these do not assert identity.
    pub fn candidate_entities(&self) -> &[NodeId] {
        &self.input.candidate_entities
    }
    /// Evidence features, unchanged from creation.
    pub fn features(&self) -> &MentionFeatures {
        &self.input.features
    }
    /// Additive namespaced projection, without entity-resolution properties.
    pub fn to_property_map(&self) -> Result<PropertyMap, GraphError> {
        self.input.validate_structure()?;
        let mut properties = PropertyMap::new();
        for (name, value) in [
            ("mention_id", self.id().as_str()),
            ("mention_observation", self.observation_id().as_str()),
            ("mention_surface_form", self.surface_form()),
        ] {
            properties.insert(name.into(), crate::PropertyValue::String(value.into()));
        }
        for (name, offset) in [
            ("mention_offset_start", self.offsets().start),
            ("mention_offset_end", self.offsets().end),
        ] {
            properties.insert(
                name.into(),
                crate::PropertyValue::Integer(
                    i64::try_from(offset)
                        .map_err(|_| invalid("mention offset exceeds projection range"))?,
                ),
            );
        }
        properties.insert(
            "mention_offset_unit".into(),
            crate::PropertyValue::String("utf8_bytes".into()),
        );
        properties.insert(
            "mention_candidate_entities".into(),
            crate::PropertyValue::StringList(
                self.candidate_entities()
                    .iter()
                    .map(|id| id.as_str().to_owned())
                    .collect(),
            ),
        );
        let features = self.features();
        for (name, value) in [
            ("mention_source_context", features.source_context.as_deref()),
            ("mention_role", features.role.as_deref()),
            (
                "mention_time",
                features.time.as_ref().map(TemporalTimestamp::as_str),
            ),
            ("mention_location", features.location.as_deref()),
        ] {
            if let Some(value) = value {
                properties.insert(name.into(), crate::PropertyValue::String(value.into()));
            }
        }
        properties.insert(
            "mention_affiliations".into(),
            crate::PropertyValue::StringList(features.affiliations.clone()),
        );
        properties.insert(
            "mention_relation_neighbourhood".into(),
            crate::PropertyValue::Json(
                serde_json::to_value(&features.relation_neighbourhood)
                    .map_err(|_| invalid("cannot project mention neighbourhood"))?,
            ),
        );
        Ok(properties)
    }
}
/// Append-only mention store, carried with the governed observation records.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "StoredMentions")]
pub struct EntityMentionStore {
    mentions: Vec<EntityMention>,
}
impl EntityMentionStore {
    /// Create a mention only when its observation and exact span are valid.
    pub fn create_mention(
        &mut self,
        input: EntityMentionInput,
        observations: &ObservationStore,
    ) -> Result<EntityMentionId, GraphError> {
        input.validate_binding(observations)?;
        if let Some(existing) = self.mention_by_id(&input.id) {
            if existing.input == input {
                return Ok(input.id);
            }
            return Err(GraphError::ImmutableRecordConflict {
                kind: crate::ImmutableRecordKind::EntityMention,
                id: input.id.as_str().into(),
            });
        }
        let id = input.id.clone();
        self.mentions.push(EntityMention { input });
        Ok(id)
    }
    pub(crate) fn validate_bindings(
        &self,
        observations: &ObservationStore,
    ) -> Result<(), GraphError> {
        for mention in &self.mentions {
            mention.input.validate_binding(observations)?;
        }
        Ok(())
    }
    /// Every retained mention in creation order.
    pub fn mentions(&self) -> &[EntityMention] {
        &self.mentions
    }
    /// Look up an immutable mention.
    pub fn mention_by_id(&self, id: &EntityMentionId) -> Option<&EntityMention> {
        self.mentions.iter().find(|m| m.id() == id)
    }
    /// All mentions bound to an observation, in creation order.
    pub fn mentions_for_observation(&self, id: &ObservationId) -> Vec<&EntityMention> {
        self.mentions
            .iter()
            .filter(|m| m.observation_id() == id)
            .collect()
    }
    /// Retained record count.
    pub fn len(&self) -> usize {
        self.mentions.len()
    }
    /// Whether no mention is stored.
    pub fn is_empty(&self) -> bool {
        self.mentions.is_empty()
    }
}
impl Graph {
    /// Append an observation-bound mention without materializing an entity.
    pub fn create_entity_mention(
        &mut self,
        input: EntityMentionInput,
    ) -> Result<EntityMentionId, GraphError> {
        let stores = self.epistemic_stores_mut();
        stores.mentions.create_mention(input, &stores.observations)
    }
}

fn invalid(message: &str) -> GraphError {
    GraphError::InvalidPropertyValue(message.into())
}
impl EntityMentionInput {
    fn validate_structure(&self) -> Result<(), GraphError> {
        EntityMentionId::new(self.id.as_str())?;
        ObservationId::new(self.observation_id.as_str())?;
        if self.surface_form.trim().is_empty()
            || self.offsets.start >= self.offsets.end
            || self.offsets.end > i64::MAX as u64
        {
            return Err(invalid(
                "mention requires a nonblank surface and a nonempty byte range",
            ));
        }
        let mut candidates = std::collections::HashSet::new();
        for id in &self.candidate_entities {
            NodeId::new(id.as_str())?;
            if !candidates.insert(id) {
                return Err(invalid("duplicate mention candidate entity reference"));
            }
        }
        let features = &self.features;
        if features
            .source_context
            .iter()
            .chain(features.role.iter())
            .chain(features.location.iter())
            .chain(features.affiliations.iter())
            .any(|s| s.trim().is_empty())
            || features
                .relation_neighbourhood
                .iter()
                .any(|r| r.predicate.trim().is_empty() || r.counterpart.trim().is_empty())
        {
            return Err(invalid("present mention features must not be blank"));
        }
        if let Some(time) = &features.time {
            TemporalTimestamp::new(time.as_str())?;
        }
        Ok(())
    }
    fn validate_binding(&self, observations: &ObservationStore) -> Result<(), GraphError> {
        self.validate_structure()?;
        let observation = observations
            .observation_by_id(&self.observation_id)
            .ok_or_else(|| GraphError::ObservationNotFound(self.observation_id.clone()))?;
        let start = usize::try_from(self.offsets.start)
            .map_err(|_| invalid("mention start out of range"))?;
        let end =
            usize::try_from(self.offsets.end).map_err(|_| invalid("mention end out of range"))?;
        if observation.payload().get(start..end) != Some(self.surface_form.as_str()) {
            return Err(invalid(
                "mention surface must equal its UTF-8 byte span in the observation payload",
            ));
        }
        Ok(())
    }
}
#[derive(Deserialize)]
struct StoredMentions {
    mentions: Vec<EntityMention>,
}
impl TryFrom<StoredMentions> for EntityMentionStore {
    type Error = GraphError;
    fn try_from(stored: StoredMentions) -> Result<Self, Self::Error> {
        let mut ids = std::collections::HashSet::new();
        for mention in &stored.mentions {
            mention.input.validate_structure()?;
            if !ids.insert(mention.id()) {
                return Err(invalid("duplicate stored mention ID"));
            }
        }
        Ok(Self {
            mentions: stored.mentions,
        })
    }
}

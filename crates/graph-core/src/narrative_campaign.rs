//! Neutral immutable collections; membership is context, never a factual judgment.
use crate::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
/// Typed contextual references; themes are caller-owned neutral labels.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextMembership {
    /// Governed claims included in this collection.
    pub claims: Vec<ClaimId>,
    /// Neutral thematic labels.
    pub themes: Vec<String>,
    /// Source identities of the included content.
    pub content: Vec<SourceId>,
    /// Canonical node references; these do not assert identity or ownership.
    pub infrastructure: Vec<NodeId>,
    /// Canonical node references; these do not assert responsibility.
    pub actors: Vec<NodeId>,
}
/// Immutable narrative creation request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NarrativeInput {
    id: NarrativeId,
    membership: ContextMembership,
    stamp: BitemporalStamp,
}
impl NarrativeInput {
    /// Prepare a collection; insertion validates memberships and temporal values.
    pub fn new(id: NarrativeId, membership: ContextMembership, stamp: BitemporalStamp) -> Self {
        Self {
            id,
            membership,
            stamp,
        }
    }
}
/// Immutable campaign creation request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignInput {
    id: CampaignId,
    narratives: Vec<NarrativeId>,
    membership: ContextMembership,
    stamp: BitemporalStamp,
}
impl CampaignInput {
    /// Prepare a collection with explicit existing narrative references.
    pub fn new(
        id: CampaignId,
        narratives: Vec<NarrativeId>,
        membership: ContextMembership,
        stamp: BitemporalStamp,
    ) -> Self {
        Self {
            id,
            narratives,
            membership,
            stamp,
        }
    }
}
/// Retained neutral narrative record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Narrative {
    input: NarrativeInput,
}
impl Narrative {
    /// Stable immutable identity.
    pub fn id(&self) -> &NarrativeId {
        &self.input.id
    }
    /// Retained membership without implicit factual interpretation.
    pub fn membership(&self) -> &ContextMembership {
        &self.input.membership
    }
    /// World interval and recorded time.
    pub fn stamp(&self) -> &BitemporalStamp {
        &self.input.stamp
    }
}
/// Retained neutral campaign record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Campaign {
    input: CampaignInput,
}
impl Campaign {
    /// Stable immutable identity.
    pub fn id(&self) -> &CampaignId {
        &self.input.id
    }
    /// Existing narrative identities retained in creation order.
    pub fn narratives(&self) -> &[NarrativeId] {
        &self.input.narratives
    }
    /// Retained contextual members.
    pub fn membership(&self) -> &ContextMembership {
        &self.input.membership
    }
    /// World interval and recorded time.
    pub fn stamp(&self) -> &BitemporalStamp {
        &self.input.stamp
    }
}
/// Append-only collections carried by native snapshots and durable stores.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "StoredCollections")]
pub struct NarrativeCampaignStore {
    narratives: Vec<Narrative>,
    campaigns: Vec<Campaign>,
}
impl NarrativeCampaignStore {
    /// Whether no collections exist.
    pub fn is_empty(&self) -> bool {
        self.narratives.is_empty() && self.campaigns.is_empty()
    }
    /// All narratives in creation order.
    pub fn narratives(&self) -> &[Narrative] {
        &self.narratives
    }
    /// All campaigns in creation order.
    pub fn campaigns(&self) -> &[Campaign] {
        &self.campaigns
    }
    /// Find one retained narrative.
    pub fn narrative_by_id(&self, id: &NarrativeId) -> Option<&Narrative> {
        self.narratives.iter().find(|r| r.id() == id)
    }
    /// Find one retained campaign.
    pub fn campaign_by_id(&self, id: &CampaignId) -> Option<&Campaign> {
        self.campaigns.iter().find(|r| r.id() == id)
    }
}
fn invalid(detail: impl Into<String>) -> GraphError {
    GraphError::InvalidPropertyValue(detail.into())
}
fn unique<'a>(values: impl IntoIterator<Item = &'a str>) -> Result<(), GraphError> {
    let mut seen = HashSet::new();
    for value in values {
        if value.trim().is_empty() || !seen.insert(value) {
            return Err(invalid(
                "Collection membership requires nonblank distinct references and themes",
            ));
        }
    }
    Ok(())
}
impl ContextMembership {
    fn validate_structure(&self) -> Result<(), GraphError> {
        unique(self.claims.iter().map(ClaimId::as_str))?;
        unique(self.themes.iter().map(String::as_str))?;
        unique(self.content.iter().map(SourceId::as_str))?;
        unique(self.infrastructure.iter().map(NodeId::as_str))?;
        unique(self.actors.iter().map(NodeId::as_str))
    }
    fn validate_bindings(&self, stores: &EpistemicStores) -> Result<(), GraphError> {
        self.validate_structure()?;
        for id in &self.claims {
            stores.claims.claim_by_id(id)?;
        }
        for id in &self.content {
            if stores.sources.current_source(id).is_none() {
                return Err(invalid(format!(
                    "Collection content source {} is missing",
                    id.as_str()
                )));
            }
        }
        // Node references are opaque canonical IDs, also usable in a paged view.
        // They never create entities, assign responsibility, or upgrade factual confidence.
        Ok(())
    }
}
fn validate_stamp(stamp: &BitemporalStamp) -> Result<(), GraphError> {
    let mut validated =
        BitemporalStamp::new(stamp.valid_from.clone(), stamp.transaction_time.clone())?;
    if let Some(end) = &stamp.valid_to {
        validated = validated.with_valid_to(end.clone())?;
    }
    for time in stamp
        .observation_time
        .iter()
        .chain(stamp.publication_time.iter())
    {
        BitemporalStamp::new(time.clone(), time.clone())?;
    }
    let _ = validated;
    Ok(())
}
impl NarrativeInput {
    fn validate_structure(&self) -> Result<(), GraphError> {
        NarrativeId::new(self.id.as_str())?;
        self.membership.validate_structure()?;
        validate_stamp(&self.stamp)
    }
}
impl CampaignInput {
    fn validate_structure(&self) -> Result<(), GraphError> {
        CampaignId::new(self.id.as_str())?;
        unique(self.narratives.iter().map(NarrativeId::as_str))?;
        self.membership.validate_structure()?;
        validate_stamp(&self.stamp)
    }
}
impl NarrativeCampaignStore {
    fn validate_structure(&self) -> Result<(), GraphError> {
        unique(self.narratives.iter().map(|r| r.id().as_str()))?;
        unique(self.campaigns.iter().map(|r| r.id().as_str()))?;
        for record in &self.narratives {
            record.input.validate_structure()?;
        }
        for record in &self.campaigns {
            record.input.validate_structure()?;
            self.validate_narratives(record.narratives())?;
        }
        Ok(())
    }
    fn validate_narratives(&self, ids: &[NarrativeId]) -> Result<(), GraphError> {
        for id in ids {
            if self.narrative_by_id(id).is_none() {
                return Err(invalid(format!(
                    "Campaign narrative {} is missing",
                    id.as_str()
                )));
            }
        }
        Ok(())
    }
    pub(crate) fn validate_bindings(&self, stores: &EpistemicStores) -> Result<(), GraphError> {
        self.validate_structure()?;
        for membership in self
            .narratives
            .iter()
            .map(Narrative::membership)
            .chain(self.campaigns.iter().map(Campaign::membership))
        {
            membership.validate_bindings(stores)?;
        }
        Ok(())
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredCollections {
    narratives: Vec<Narrative>,
    campaigns: Vec<Campaign>,
}
impl TryFrom<StoredCollections> for NarrativeCampaignStore {
    type Error = GraphError;
    fn try_from(stored: StoredCollections) -> Result<Self, Self::Error> {
        let store = Self {
            narratives: stored.narratives,
            campaigns: stored.campaigns,
        };
        store.validate_structure()?;
        Ok(store)
    }
}
impl Graph {
    /// Validate all references before an idempotent append; never rewrite a claim.
    pub fn create_narrative(&mut self, input: NarrativeInput) -> Result<NarrativeId, GraphError> {
        input.validate_structure()?;
        input
            .membership
            .validate_bindings(self.epistemic_stores())?;
        let store = &mut self.epistemic_stores_mut().narrative_campaigns;
        if let Some(existing) = store.narrative_by_id(&input.id) {
            if existing.input == input {
                return Ok(input.id);
            }
            return Err(GraphError::ImmutableRecordConflict {
                kind: ImmutableRecordKind::Narrative,
                id: input.id.as_str().into(),
            });
        }
        let id = input.id.clone();
        store.narratives.push(Narrative { input });
        Ok(id)
    }
    /// Validate existing narrative references before an idempotent append.
    pub fn create_campaign(&mut self, input: CampaignInput) -> Result<CampaignId, GraphError> {
        input.validate_structure()?;
        input
            .membership
            .validate_bindings(self.epistemic_stores())?;
        let store = &mut self.epistemic_stores_mut().narrative_campaigns;
        store.validate_narratives(&input.narratives)?;
        if let Some(existing) = store.campaign_by_id(&input.id) {
            if existing.input == input {
                return Ok(input.id);
            }
            return Err(GraphError::ImmutableRecordConflict {
                kind: ImmutableRecordKind::Campaign,
                id: input.id.as_str().into(),
            });
        }
        let id = input.id.clone();
        store.campaigns.push(Campaign { input });
        Ok(id)
    }
}
fn properties(
    prefix: &str,
    id: &str,
    membership: &ContextMembership,
    stamp: &BitemporalStamp,
) -> Result<PropertyMap, GraphError> {
    let mut properties = PropertyMap::new();
    properties.insert(format!("{prefix}_id"), PropertyValue::String(id.into()));
    properties.insert(
        format!("{prefix}_themes"),
        PropertyValue::StringList(membership.themes.clone()),
    );
    properties.insert(
        format!("{prefix}_membership"),
        PropertyValue::Json(serde_json::to_value(membership).map_err(|e| invalid(e.to_string()))?),
    );
    properties.insert(
        format!("{prefix}_stamp"),
        PropertyValue::Json(serde_json::to_value(stamp).map_err(|e| invalid(e.to_string()))?),
    );
    Ok(properties)
}
impl Narrative {
    /// Additive namespaced properties for read projections and exporters.
    pub fn to_property_map(&self) -> Result<PropertyMap, GraphError> {
        self.input.validate_structure()?;
        properties(
            "narrative",
            self.id().as_str(),
            self.membership(),
            self.stamp(),
        )
    }
}
impl Campaign {
    /// Additive namespaced properties, keeping narrative identities explicit.
    pub fn to_property_map(&self) -> Result<PropertyMap, GraphError> {
        self.input.validate_structure()?;
        let mut properties = properties(
            "campaign",
            self.id().as_str(),
            self.membership(),
            self.stamp(),
        )?;
        properties.insert(
            "campaign_narratives".into(),
            PropertyValue::StringList(
                self.narratives()
                    .iter()
                    .map(|id| id.as_str().to_owned())
                    .collect(),
            ),
        );
        Ok(properties)
    }
}
fn project_node(
    projection: &mut Graph,
    kind: EpistemicNodeKind,
    properties: PropertyMap,
) -> Result<NodeId, GraphError> {
    let mut input = NodeInput::new([kind.canonical_label()]);
    for (key, value) in properties {
        input = input.with_property(key, value);
    }
    projection.create_node(input)
}
fn member_edge(
    projection: &mut Graph,
    source: &NodeId,
    target: &NodeId,
    role: &str,
) -> Result<(), GraphError> {
    projection.create_relationship(
        RelationshipInput::new(
            source.clone(),
            EpistemicRelationKind::HasMember
                .canonical_relationship_type()
                .as_str(),
            target.clone(),
        )?
        .with_property("membership_role", PropertyValue::String(role.into())),
    )?;
    Ok(())
}
fn project_members(
    projection: &mut Graph,
    collection: &NodeId,
    members: &ContextMembership,
    claims: &HashMap<String, NodeId>,
    sources: &HashMap<String, NodeId>,
    references: &mut HashMap<NodeId, NodeId>,
) -> Result<(), GraphError> {
    for (ids, targets, role) in [
        (
            members
                .claims
                .iter()
                .map(ClaimId::as_str)
                .collect::<Vec<_>>(),
            claims,
            "claim",
        ),
        (
            members.content.iter().map(SourceId::as_str).collect(),
            sources,
            "content",
        ),
    ] {
        for id in ids {
            let target = targets
                .get(id)
                .ok_or_else(|| invalid("Collection member missing from governed projection"))?;
            member_edge(projection, collection, target, role)?;
        }
    }
    for (ids, role) in [
        (&members.actors, "actor"),
        (&members.infrastructure, "infrastructure"),
    ] {
        for id in ids {
            let target = if let Some(target) = references.get(id) {
                target.clone()
            } else {
                let target = projection.create_node(
                    NodeInput::new([EpistemicNodeKind::RecordReference.canonical_label()])
                        .with_property("record_id", PropertyValue::String(id.as_str().into()))
                        .with_property("record_kind", PropertyValue::String("node".into())),
                )?;
                references.insert(id.clone(), target.clone());
                target
            };
            member_edge(projection, collection, &target, role)?;
        }
    }
    Ok(())
}
impl Graph {
    pub(crate) fn project_context_collections(
        &self,
        projection: &mut Graph,
        claims: &HashMap<String, NodeId>,
        sources: &HashMap<String, NodeId>,
    ) -> Result<(), GraphError> {
        let stores = self.epistemic_stores();
        let records = &stores.narrative_campaigns;
        records.validate_bindings(stores)?;
        let mut narratives = HashMap::new();
        let mut references = HashMap::new();
        for record in records.narratives() {
            let id = project_node(
                projection,
                EpistemicNodeKind::Narrative,
                record.to_property_map()?,
            )?;
            project_members(
                projection,
                &id,
                record.membership(),
                claims,
                sources,
                &mut references,
            )?;
            narratives.insert(record.id().clone(), id);
        }
        for record in records.campaigns() {
            let id = project_node(
                projection,
                EpistemicNodeKind::Campaign,
                record.to_property_map()?,
            )?;
            project_members(
                projection,
                &id,
                record.membership(),
                claims,
                sources,
                &mut references,
            )?;
            for narrative in record.narratives() {
                let target = narratives
                    .get(narrative)
                    .ok_or_else(|| invalid("Campaign narrative missing from projection"))?;
                member_edge(projection, &id, target, "narrative")?;
            }
        }
        Ok(())
    }
}

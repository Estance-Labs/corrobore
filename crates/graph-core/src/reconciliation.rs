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
//! Append-only evidence-cited identity judgments. Merge execution belongs to WS-C 5.
use crate::{
    ActorId, Confidence, EntityMentionId, EntityMentionStore, Graph, GraphError, ObservationId,
    ObservationStore, PropertyMap, ReconciliationRecordId, SourceId, SourceStore, SourceVersionId,
    TemporalTimestamp,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
/// A decided identity outcome; abstention is a successful, retained judgment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReconciliationOutcome {
    /// The decider concludes that the pair identifies the same entity.
    Merge,
    /// The decider concludes that the pair identifies different entities.
    Distinct,
    /// Evidence does not justify committing to either identity judgment.
    Abstain,
}
/// Explicit deciding actor or versioned verifier.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReconciliationDecider {
    /// Named human or agent actor.
    Actor(ActorId),
    /// Versioned verifier responsible for the decision.
    Verifier {
        /// Stable verifier identity.
        id: String,
        /// Version of the deciding verifier.
        version: String,
    },
}
/// Evidence features available on a governed mention.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ReconciliationFeature {
    /// Verbatim surface form; never sufficient contextual evidence.
    SurfaceForm,
    /// Context retained on the mention from its source.
    SourceContext,
    /// Role described by the observation.
    Role,
    /// Mention or event time.
    Time,
    /// Observed location.
    Location,
    /// Observed affiliation names.
    Affiliations,
    /// Observed neighbouring predicates and counterparts.
    RelationNeighbourhood,
}
/// Reference to retained evidence; values are resolved by the store, not the caller.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ReconciliationEvidence {
    /// Cite an existing feature on one mention of the pair.
    Mention {
        /// Mention providing the feature.
        mention_id: EntityMentionId,
        /// Feature to resolve from retained mention data.
        feature: ReconciliationFeature,
    },
    /// Cite newly acquired observation content as source context.
    Observation {
        /// Registered observation whose verbatim payload is cited.
        observation_id: ObservationId,
    },
}
/// Similarity is only a hint, never evidence sufficient to decide identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReconciliationSimilarityKind {
    /// String or name similarity.
    Name,
    /// Embedding-space similarity.
    Embedding,
}
/// Optional retained retrieval hint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReconciliationSimilarity {
    /// Kind of similarity hint.
    pub kind: ReconciliationSimilarityKind,
    /// Bounded similarity score, not an identity confidence.
    pub score: Confidence,
}
/// One resolved citation, including the source version captured when recorded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReconciliationCitation {
    /// Original reference selected by the decider.
    pub evidence: ReconciliationEvidence,
    /// Resolved evidence feature category.
    pub feature: ReconciliationFeature,
    /// Immutable observation behind the cited value.
    pub observation_id: ObservationId,
    /// Origin identity of the observation.
    pub source_id: SourceId,
    /// Source metadata version captured when this judgment was recorded.
    pub source_version_id: SourceVersionId,
    /// Exact retained value, resolved by the store.
    pub value: Value,
}
/// Input for one immutable judgment on a pair of distinct mentions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReconciliationInput {
    id: ReconciliationRecordId,
    left: EntityMentionId,
    right: EntityMentionId,
    outcome: ReconciliationOutcome,
    decider: ReconciliationDecider,
    decided_at: TemporalTimestamp,
    rationale: String,
    evidence: Vec<ReconciliationEvidence>,
    similarity_hints: Vec<ReconciliationSimilarity>,
}
impl ReconciliationInput {
    /// Prepare a judgment; recording validates references and resolves citations.
    pub fn new(
        id: ReconciliationRecordId,
        left: EntityMentionId,
        right: EntityMentionId,
        outcome: ReconciliationOutcome,
        decider: ReconciliationDecider,
        decided_at: TemporalTimestamp,
        rationale: impl Into<String>,
    ) -> Self {
        Self {
            id,
            left,
            right,
            outcome,
            decider,
            decided_at,
            rationale: rationale.into(),
            evidence: vec![],
            similarity_hints: vec![],
        }
    }
    /// Cite feature references and any new observation evidence.
    pub fn with_evidence(mut self, evidence: Vec<ReconciliationEvidence>) -> Self {
        self.evidence = evidence;
        self
    }
    /// Retain optional similarity hints, without elevating them to evidence.
    pub fn with_similarity_hints(mut self, hints: Vec<ReconciliationSimilarity>) -> Self {
        self.similarity_hints = hints;
        self
    }
    /// Name the actor or verifier responsible for the judgment.
    pub fn with_decider(mut self, decider: ReconciliationDecider) -> Self {
        self.decider = decider;
        self
    }
}
/// Immutable reconciliation record, not a graph merge operation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReconciliationRecord {
    #[serde(flatten)]
    input: ReconciliationInput,
    citations: Vec<ReconciliationCitation>,
}
impl ReconciliationRecord {
    /// Stable record ID.
    pub fn id(&self) -> &ReconciliationRecordId {
        &self.input.id
    }
    /// First mention in the recorded pair.
    pub fn left(&self) -> &EntityMentionId {
        &self.input.left
    }
    /// Second mention in the recorded pair.
    pub fn right(&self) -> &EntityMentionId {
        &self.input.right
    }
    /// Recorded outcome, including successful abstention.
    pub fn outcome(&self) -> ReconciliationOutcome {
        self.input.outcome
    }
    /// Deciding actor or verifier.
    pub fn decider(&self) -> &ReconciliationDecider {
        &self.input.decider
    }
    /// Decision time supplied by the decider.
    pub fn decided_at(&self) -> &TemporalTimestamp {
        &self.input.decided_at
    }
    /// Explanation of the source-grounded judgment.
    pub fn rationale(&self) -> &str {
        &self.input.rationale
    }
    /// Immutable cited values and their provenance.
    pub fn citations(&self) -> &[ReconciliationCitation] {
        &self.citations
    }
    /// Retrieval hints that did not independently justify the outcome.
    pub fn similarity_hints(&self) -> &[ReconciliationSimilarity] {
        &self.input.similarity_hints
    }
    /// Namespaced read projection of the judgment and cited evidence.
    pub fn to_property_map(&self) -> Result<PropertyMap, GraphError> {
        self.validate_structure()?;
        let mut properties = PropertyMap::new();
        for (key, value) in [
            ("reconciliation_id", self.id().as_str()),
            ("reconciliation_left", self.left().as_str()),
            ("reconciliation_right", self.right().as_str()),
            ("reconciliation_decided_at", self.decided_at().as_str()),
            ("reconciliation_rationale", self.rationale()),
        ] {
            properties.insert(key.into(), crate::PropertyValue::String(value.into()));
        }
        let outcome = match self.outcome() {
            ReconciliationOutcome::Merge => "merge",
            ReconciliationOutcome::Distinct => "distinct",
            ReconciliationOutcome::Abstain => "abstain",
        };
        properties.insert(
            "reconciliation_outcome".into(),
            crate::PropertyValue::String(outcome.into()),
        );
        for (key, value) in [
            (
                "reconciliation_decider",
                serde_json::to_value(self.decider()),
            ),
            (
                "reconciliation_citations",
                serde_json::to_value(self.citations()),
            ),
            (
                "reconciliation_similarity_hints",
                serde_json::to_value(self.similarity_hints()),
            ),
        ] {
            properties.insert(
                key.into(),
                crate::PropertyValue::Json(
                    value.map_err(|_| invalid("cannot project reconciliation"))?,
                ),
            );
        }
        Ok(properties)
    }
}
/// Append-only, insertion-ordered reconciliation history.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "StoredReconciliations")]
pub struct ReconciliationStore {
    records: Vec<ReconciliationRecord>,
}
impl ReconciliationStore {
    /// Record a judgment with actual cited values and mandatory non-similarity evidence.
    pub fn create_record(
        &mut self,
        input: ReconciliationInput,
        mentions: &EntityMentionStore,
        observations: &ObservationStore,
        sources: &SourceStore,
    ) -> Result<ReconciliationRecordId, GraphError> {
        input.validate_structure()?;
        if let Some(existing) = self.record_by_id(&input.id) {
            if existing.input != input {
                return Err(GraphError::ImmutableRecordConflict {
                    kind: crate::ImmutableRecordKind::ReconciliationRecord,
                    id: input.id.as_str().into(),
                });
            }
            existing.validate_bindings(mentions, observations, sources)?;
            return Ok(input.id);
        }
        mentions.validate_bindings(observations)?;
        let citations = input
            .evidence
            .iter()
            .map(|evidence| {
                let (feature, observation_id, source_id, value) =
                    resolve(evidence, mentions, observations)?;
                let source = sources
                    .current_source(&source_id)
                    .ok_or_else(|| GraphError::SourceNotFound(source_id.clone()))?;
                Ok(ReconciliationCitation {
                    evidence: evidence.clone(),
                    feature,
                    observation_id,
                    source_id,
                    source_version_id: source.version_id().clone(),
                    value,
                })
            })
            .collect::<Result<Vec<_>, GraphError>>()?;
        let record = ReconciliationRecord { input, citations };
        let id = record.id().clone();
        self.records.push(record);
        Ok(id)
    }
    pub(crate) fn validate_bindings(
        &self,
        mentions: &EntityMentionStore,
        observations: &ObservationStore,
        sources: &SourceStore,
    ) -> Result<(), GraphError> {
        for record in &self.records {
            record.validate_bindings(mentions, observations, sources)?;
        }
        Ok(())
    }
    /// All retained judgments in creation order.
    pub fn records(&self) -> &[ReconciliationRecord] {
        &self.records
    }
    /// One immutable record by ID.
    pub fn record_by_id(&self, id: &ReconciliationRecordId) -> Option<&ReconciliationRecord> {
        self.records.iter().find(|r| r.id() == id)
    }
    /// Pair history, independent of query orientation; no outcome hides older records.
    pub fn records_for_pair(
        &self,
        left: &EntityMentionId,
        right: &EntityMentionId,
    ) -> Vec<&ReconciliationRecord> {
        self.records
            .iter()
            .filter(|r| {
                (r.left() == left && r.right() == right) || (r.left() == right && r.right() == left)
            })
            .collect()
    }
    /// All retained judgments of an outcome, including abstentions.
    pub fn records_by_outcome(&self, outcome: ReconciliationOutcome) -> Vec<&ReconciliationRecord> {
        self.records
            .iter()
            .filter(|r| r.outcome() == outcome)
            .collect()
    }
    /// Whether no judgment is stored.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}
impl Graph {
    /// Record a reconciliation decision without executing an identity merge.
    pub fn record_reconciliation(
        &mut self,
        input: ReconciliationInput,
    ) -> Result<ReconciliationRecordId, GraphError> {
        let stores = self.epistemic_stores_mut();
        stores.merges.validate_bindings(&stores.reconciliations)?;
        let existing = stores.reconciliations.record_by_id(&input.id).is_some();
        let id = stores.reconciliations.create_record(
            input,
            &stores.mentions,
            &stores.observations,
            &stores.sources,
        )?;
        if !existing {
            stores.merges.record_context(&id, &stores.reconciliations)?;
        }
        Ok(id)
    }
}

fn invalid(message: &str) -> GraphError {
    GraphError::InvalidPropertyValue(message.into())
}
impl ReconciliationInput {
    fn validate_structure(&self) -> Result<(), GraphError> {
        ReconciliationRecordId::new(self.id.as_str())?;
        EntityMentionId::new(self.left.as_str())?;
        EntityMentionId::new(self.right.as_str())?;
        TemporalTimestamp::new(self.decided_at.as_str())?;
        if self.left == self.right || self.rationale.trim().is_empty() {
            return Err(invalid(
                "reconciliation requires distinct mentions and a rationale",
            ));
        }
        match &self.decider {
            ReconciliationDecider::Actor(id) => {
                ActorId::new(id.as_str())?;
            }
            ReconciliationDecider::Verifier { id, version }
                if id.trim().is_empty() || version.trim().is_empty() =>
            {
                return Err(invalid("deciding verifier requires an ID and version"));
            }
            _ => {}
        }
        for hint in &self.similarity_hints {
            Confidence::new(hint.score.value())?;
        }
        let mut unique = std::collections::HashSet::new();
        let mut left = false;
        let mut right = false;
        let mut contextual = false;
        for evidence in &self.evidence {
            if !unique.insert(evidence) {
                return Err(invalid("duplicate reconciliation evidence reference"));
            }
            match evidence {
                ReconciliationEvidence::Mention {
                    mention_id,
                    feature,
                } => {
                    if mention_id == &self.left {
                        left = true;
                    } else if mention_id == &self.right {
                        right = true;
                    } else {
                        return Err(invalid("cited mention is outside the reconciled pair"));
                    }
                    contextual |= *feature != ReconciliationFeature::SurfaceForm;
                }
                ReconciliationEvidence::Observation { observation_id } => {
                    ObservationId::new(observation_id.as_str())?;
                    contextual = true;
                }
            }
        }
        if !left || !right || !contextual {
            return Err(invalid(
                "reconciliation requires citations for both mentions and contextual evidence; similarity alone is insufficient",
            ));
        }
        Ok(())
    }
}
fn resolve(
    evidence: &ReconciliationEvidence,
    mentions: &EntityMentionStore,
    observations: &ObservationStore,
) -> Result<(ReconciliationFeature, ObservationId, SourceId, Value), GraphError> {
    let (feature, observation_id, value) = match evidence {
        ReconciliationEvidence::Mention {
            mention_id,
            feature,
        } => {
            let mention = mentions
                .mention_by_id(mention_id)
                .ok_or_else(|| invalid("unknown reconciliation mention"))?;
            let features = mention.features();
            let value = match feature {
                ReconciliationFeature::SurfaceForm => {
                    Some(Value::String(mention.surface_form().into()))
                }
                ReconciliationFeature::SourceContext => {
                    features.source_context.clone().map(Value::String)
                }
                ReconciliationFeature::Role => features.role.clone().map(Value::String),
                ReconciliationFeature::Time => features
                    .time
                    .as_ref()
                    .map(|time| Value::String(time.as_str().into())),
                ReconciliationFeature::Location => features.location.clone().map(Value::String),
                ReconciliationFeature::Affiliations => {
                    if features.affiliations.is_empty() {
                        None
                    } else {
                        Some(
                            serde_json::to_value(&features.affiliations)
                                .map_err(|_| invalid("invalid affiliations"))?,
                        )
                    }
                }
                ReconciliationFeature::RelationNeighbourhood => {
                    if features.relation_neighbourhood.is_empty() {
                        None
                    } else {
                        Some(
                            serde_json::to_value(&features.relation_neighbourhood)
                                .map_err(|_| invalid("invalid neighbourhood"))?,
                        )
                    }
                }
            }
            .ok_or_else(|| invalid("cited mention feature is absent"))?;
            (*feature, mention.observation_id().clone(), value)
        }
        ReconciliationEvidence::Observation { observation_id } => {
            let observation = observations
                .observation_by_id(observation_id)
                .ok_or_else(|| GraphError::ObservationNotFound(observation_id.clone()))?;
            if observation.payload().trim().is_empty() {
                return Err(invalid("cited observation is empty"));
            }
            (
                ReconciliationFeature::SourceContext,
                observation_id.clone(),
                Value::String(observation.payload().into()),
            )
        }
    };
    let observation = observations
        .observation_by_id(&observation_id)
        .ok_or_else(|| GraphError::ObservationNotFound(observation_id.clone()))?;
    Ok((
        feature,
        observation_id,
        observation.source_id().clone(),
        value,
    ))
}
impl ReconciliationRecord {
    fn validate_structure(&self) -> Result<(), GraphError> {
        self.input.validate_structure()?;
        if self.citations.len() != self.input.evidence.len()
            || self
                .citations
                .iter()
                .zip(&self.input.evidence)
                .any(|(citation, evidence)| &citation.evidence != evidence)
        {
            return Err(invalid(
                "reconciliation citation references differ from input",
            ));
        }
        Ok(())
    }
    fn validate_bindings(
        &self,
        mentions: &EntityMentionStore,
        observations: &ObservationStore,
        sources: &SourceStore,
    ) -> Result<(), GraphError> {
        self.validate_structure()?;
        mentions.validate_bindings(observations)?;
        for citation in &self.citations {
            let (feature, observation, source, value) =
                resolve(&citation.evidence, mentions, observations)?;
            let version = sources
                .source_version(&citation.source_version_id)
                .ok_or_else(|| invalid("cited source version is missing"))?;
            if citation.feature != feature
                || citation.observation_id != observation
                || citation.source_id != source
                || citation.value != value
                || version.id() != &source
            {
                return Err(invalid(
                    "reconciliation citation differs from retained evidence",
                ));
            }
        }
        Ok(())
    }
}
#[derive(Deserialize)]
struct StoredReconciliations {
    records: Vec<ReconciliationRecord>,
}
impl TryFrom<StoredReconciliations> for ReconciliationStore {
    type Error = GraphError;
    fn try_from(stored: StoredReconciliations) -> Result<Self, Self::Error> {
        let mut ids = std::collections::HashSet::new();
        for record in &stored.records {
            record.validate_structure()?;
            if !ids.insert(record.id()) {
                return Err(invalid("duplicate reconciliation record ID"));
            }
        }
        Ok(Self {
            records: stored.records,
        })
    }
}

impl ReconciliationStore {
    pub(crate) fn audit_subset(
        &self,
        ids: &std::collections::HashSet<ReconciliationRecordId>,
    ) -> Self {
        Self {
            records: self
                .records
                .iter()
                .filter(|r| ids.contains(r.id()))
                .cloned()
                .collect(),
        }
    }
}

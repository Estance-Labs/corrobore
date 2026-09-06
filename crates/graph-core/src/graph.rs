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
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::EpistemicStores;
use crate::{
    EvidenceId, EvidenceInput, EvidenceRecord, EvidenceRecordStore, GraphError, Node, NodeId,
    NodeInput, NodePatch, NodeVersionId, RecordStatus, Relationship, RelationshipId,
    RelationshipInput, RelationshipPatch, RelationshipVersionId, adjacency::AdjacencyIndexes,
};

#[derive(Clone, Debug, Default)]
/// Graph.
pub struct Graph {
    nodes: HashMap<NodeId, Vec<Node>>,
    current_node_versions: HashMap<NodeId, NodeVersionId>,
    next_node_sequence: u64,
    next_node_version_sequence: u64,
    relationships: HashMap<RelationshipId, Vec<Relationship>>,
    current_relationship_versions: HashMap<RelationshipId, RelationshipVersionId>,
    adjacency: AdjacencyIndexes,
    next_relationship_sequence: u64,
    next_relationship_version_sequence: u64,
    evidence: EvidenceRecordStore,
    epistemic: EpistemicStores,
}

/// Serializable, version-preserving snapshot of one in-memory graph.
///
/// Storage adapters use this opaque value instead of depending on the private
/// map and adjacency representation of [`Graph`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphPersistenceSnapshot {
    nodes: Vec<Node>,
    relationships: Vec<Relationship>,
    next_node_sequence: u64,
    next_node_version_sequence: u64,
    next_relationship_sequence: u64,
    next_relationship_version_sequence: u64,
    #[serde(default)]
    evidence: EvidenceRecordStore,
    /// Epic 0029 governed stores. Skipped when empty so snapshots written
    /// before WS-A stay byte-identical.
    #[serde(default, skip_serializing_if = "EpistemicStores::is_empty")]
    epistemic: EpistemicStores,
}

/// Global identifier sequence floors carried by a paged graph projection.
///
/// A projection may contain only a subset of canonical records. These floors
/// keep newly generated IDs globally monotonic instead of deriving them from
/// the currently resident subset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GraphSequenceFloor {
    /// Highest allocated node sequence.
    pub node: u64,
    /// Highest allocated node-version sequence.
    pub node_version: u64,
    /// Highest allocated relationship sequence.
    pub relationship: u64,
    /// Highest allocated relationship-version sequence.
    pub relationship_version: u64,
}

impl Graph {
    /// Creates a new instance.
    pub fn new() -> Self {
        Self::default()
    }

    /// Captures every graph record version and identifier sequence for durable
    /// storage.
    pub fn persistence_snapshot(&self) -> GraphPersistenceSnapshot {
        let mut nodes: Vec<Node> = self.nodes.values().flatten().cloned().collect();
        nodes.sort_by(|left, right| {
            left.id
                .as_str()
                .cmp(right.id.as_str())
                .then(left.version.cmp(&right.version))
        });
        let mut relationships: Vec<Relationship> =
            self.relationships.values().flatten().cloned().collect();
        relationships.sort_by(|left, right| {
            left.id
                .as_str()
                .cmp(right.id.as_str())
                .then(left.version.cmp(&right.version))
        });
        GraphPersistenceSnapshot {
            nodes,
            relationships,
            next_node_sequence: self.next_node_sequence,
            next_node_version_sequence: self.next_node_version_sequence,
            next_relationship_sequence: self.next_relationship_sequence,
            next_relationship_version_sequence: self.next_relationship_version_sequence,
            evidence: self.evidence.clone(),
            epistemic: self.epistemic.clone(),
        }
    }

    /// Reconstructs a graph from a validated durable snapshot.
    pub fn from_persistence_snapshot(
        snapshot: GraphPersistenceSnapshot,
    ) -> Result<Self, GraphError> {
        snapshot
            .epistemic
            .mentions
            .validate_bindings(&snapshot.epistemic.observations)?;
        snapshot.epistemic.reconciliations.validate_bindings(
            &snapshot.epistemic.mentions,
            &snapshot.epistemic.observations,
            &snapshot.epistemic.sources,
        )?;
        snapshot
            .epistemic
            .merges
            .validate_bindings(&snapshot.epistemic.reconciliations)?;
        snapshot.epistemic.ingestion_evaluations.validate_bindings(
            &snapshot.epistemic.candidates,
            &snapshot.epistemic.reconciliations,
        )?;
        snapshot
            .epistemic
            .audit_bindings
            .validate(&snapshot.epistemic)?;
        snapshot
            .epistemic
            .analyst_decisions
            .validate(&snapshot.epistemic.claims)?;
        snapshot.epistemic.claims.validate_link_indices()?;
        let mut graph = Graph {
            next_node_sequence: snapshot.next_node_sequence,
            next_node_version_sequence: snapshot.next_node_version_sequence,
            next_relationship_sequence: snapshot.next_relationship_sequence,
            next_relationship_version_sequence: snapshot.next_relationship_version_sequence,
            evidence: snapshot.evidence,
            epistemic: snapshot.epistemic,
            ..Graph::default()
        };

        for node in snapshot.nodes {
            if node.current
                && graph
                    .current_node_versions
                    .insert(node.id.clone(), node.version_id.clone())
                    .is_some()
            {
                return Err(GraphError::InternalInvariantViolation(format!(
                    "multiple current node versions in persistence snapshot for {}",
                    node.id.as_str()
                )));
            }
            graph.nodes.entry(node.id.clone()).or_default().push(node);
        }
        for node_id in graph.nodes.keys() {
            if !graph.current_node_versions.contains_key(node_id) {
                return Err(GraphError::InternalInvariantViolation(format!(
                    "missing current node version in persistence snapshot for {}",
                    node_id.as_str()
                )));
            }
        }

        for relationship in snapshot.relationships {
            if !graph.nodes.contains_key(&relationship.source)
                || !graph.nodes.contains_key(&relationship.target)
            {
                return Err(GraphError::InternalInvariantViolation(format!(
                    "relationship {} references a missing endpoint in persistence snapshot",
                    relationship.id.as_str()
                )));
            }
            if relationship.current {
                if graph
                    .current_relationship_versions
                    .insert(relationship.id.clone(), relationship.version_id.clone())
                    .is_some()
                {
                    return Err(GraphError::InternalInvariantViolation(format!(
                        "multiple current relationship versions in persistence snapshot for {}",
                        relationship.id.as_str()
                    )));
                }
                graph.adjacency.record_created_relationship(
                    &relationship.id,
                    &relationship.source,
                    &relationship.target,
                )?;
            }
            graph
                .relationships
                .entry(relationship.id.clone())
                .or_default()
                .push(relationship);
        }
        for relationship_id in graph.relationships.keys() {
            if !graph
                .current_relationship_versions
                .contains_key(relationship_id)
            {
                return Err(GraphError::InternalInvariantViolation(format!(
                    "missing current relationship version in persistence snapshot for {}",
                    relationship_id.as_str()
                )));
            }
        }
        Ok(graph)
    }

    /// Reconstructs an operational graph projection from current records only.
    ///
    /// Persistent paged stores use this boundary after selecting and paging the
    /// records needed for one request. Historical versions remain canonical in
    /// append-only storage; the projection retains current version pointers and
    /// advances generated identifier sequences beyond every loaded identifier.
    pub fn from_current_records(
        nodes: Vec<Node>,
        relationships: Vec<Relationship>,
        sequence_floor: GraphSequenceFloor,
    ) -> Result<Self, GraphError> {
        let snapshot = GraphPersistenceSnapshot {
            nodes,
            relationships,
            next_node_sequence: sequence_floor.node,
            next_node_version_sequence: sequence_floor.node_version,
            next_relationship_sequence: sequence_floor.relationship,
            next_relationship_version_sequence: sequence_floor.relationship_version,
            evidence: EvidenceRecordStore::new(),
            epistemic: EpistemicStores::default(),
        };
        Self::from_persistence_snapshot(snapshot)
    }

    /// Returns every current node record, including tombstones.
    ///
    /// The durable transition layer needs tombstones because they are canonical
    /// record versions even though normal graph reads intentionally hide them.
    pub fn current_node_records(&self) -> Result<Vec<Node>, GraphError> {
        let mut records = self
            .nodes
            .keys()
            .map(|node_id| self.current_node_version(node_id))
            .collect::<Result<Vec<_>, _>>()?;
        records.sort_by(|left, right| left.id().as_str().cmp(right.id().as_str()));
        Ok(records)
    }

    /// Returns every current relationship record, including tombstones.
    pub fn current_relationship_records(&self) -> Result<Vec<Relationship>, GraphError> {
        let mut records = self
            .relationships
            .keys()
            .map(|relationship_id| self.current_relationship_version(relationship_id))
            .collect::<Result<Vec<_>, _>>()?;
        records.sort_by(|left, right| left.id().as_str().cmp(right.id().as_str()));
        Ok(records)
    }

    /// Returns every node version in stable identifier/version order.
    pub fn all_node_records(&self) -> Vec<Node> {
        let mut records: Vec<Node> = self.nodes.values().flatten().cloned().collect();
        records.sort_by(|left, right| {
            left.id()
                .as_str()
                .cmp(right.id().as_str())
                .then(left.version().cmp(&right.version()))
        });
        records
    }

    /// Returns every relationship version in stable identifier/version order.
    pub fn all_relationship_records(&self) -> Vec<Relationship> {
        let mut records: Vec<Relationship> =
            self.relationships.values().flatten().cloned().collect();
        records.sort_by(|left, right| {
            left.id()
                .as_str()
                .cmp(right.id().as_str())
                .then(left.version().cmp(&right.version()))
        });
        records
    }

    /// Creates one durable first-class evidence record.
    pub fn create_evidence(&mut self, input: EvidenceInput) -> Result<EvidenceId, GraphError> {
        self.evidence.create_evidence(input)
    }

    /// Returns one durable evidence record by its caller-owned identifier.
    pub fn evidence_by_id(&self, evidence_id: &EvidenceId) -> Option<&EvidenceRecord> {
        self.evidence.evidence_by_id(evidence_id)
    }

    /// Returns the number of unique durable evidence records.
    pub fn evidence_count(&self) -> usize {
        self.evidence.len()
    }

    /// Returns the complete first-class evidence store for durable adapters.
    pub fn evidence_store(&self) -> &EvidenceRecordStore {
        &self.evidence
    }

    /// Epic 0029 governed stores carried by this graph.
    pub fn epistemic_stores(&self) -> &EpistemicStores {
        &self.epistemic
    }

    /// Mutable access to the governed stores for engine-side resolution and
    /// ingestion. Not a client-facing surface.
    pub fn epistemic_stores_mut(&mut self) -> &mut EpistemicStores {
        &mut self.epistemic
    }

    /// Replaces the governed stores loaded by a durable adapter.
    pub fn replace_epistemic_stores(&mut self, epistemic: EpistemicStores) {
        self.epistemic = epistemic;
    }

    /// Build a read-only graph rendering every governed record as nodes and
    /// relationships of the epistemic vocabulary, so Cypher reads can
    /// traverse claims, sources, observations, entity mentions, evidence, verdicts,
    /// reconciliation judgments, verification records, and state transitions.
    ///
    /// Node identifiers are generated; record identifiers are properties
    /// (`claim_id`, `source_id`, ...). Labels: `Source`, `Observation`,
    /// `EntityMention`, `Claim`, `Evidence`, `Verdict` + `Assessment`,
    /// `VerificationRecord` + `Assessment`, `StateTransition` + `Decision`,
    /// `ReconciliationRecord` + `Decision`.
    /// Relationships: `REPORTS` (source to observation), `HAS_MENTION`
    /// (observation to mention, never an entity-resolution link), the evidence-link
    /// kinds (link source to claim), `ASSESSES` (verdict and verification
    /// record to claim), `DECIDES` (transition to claim or reconciliation to mentions).
    /// The source graph is
    /// not mutated and the projection carries no stores.
    ///
    /// # Errors
    ///
    /// Propagates graph construction errors.
    pub fn epistemic_projection(&self) -> Result<Graph, GraphError> {
        use std::collections::HashMap;

        use crate::{
            ClaimLinkSource, ClaimTarget, EpistemicNodeKind, EpistemicRelationKind, NodeInput,
            PropertyMap, PropertyValue, RelationshipInput, SourceId, lifecycle_token,
            project_verdict_state,
        };

        let stores = &self.epistemic;
        stores.mentions.validate_bindings(&stores.observations)?;
        stores.reconciliations.validate_bindings(
            &stores.mentions,
            &stores.observations,
            &stores.sources,
        )?;
        stores.merges.validate_bindings(&stores.reconciliations)?;
        let mut projection = Graph::new();
        let with_properties = |labels: &[&str], properties: PropertyMap| {
            let mut input = NodeInput::new(labels.iter().copied());
            for (key, value) in properties {
                input = input.with_property(key, value);
            }
            input
        };

        // Sources: every version is a node; the current version anchors REPORTS.
        let mut source_nodes: HashMap<String, NodeId> = HashMap::new();
        let mut source_ids: Vec<&SourceId> = stores.sources.source_ids().into_iter().collect();
        source_ids.sort();
        for source_id in source_ids {
            for version in stores.sources.source_versions(source_id) {
                let node_id = projection.create_node(with_properties(
                    &[EpistemicNodeKind::Source.canonical_label()],
                    version.to_property_map(),
                ))?;
                if stores
                    .sources
                    .current_source(source_id)
                    .map(|current| current.version_id())
                    == Some(version.version_id())
                {
                    source_nodes.insert(source_id.as_str().to_owned(), node_id);
                }
            }
        }

        // Observations, with the verbatim payload available to reads.
        let mut observation_nodes: HashMap<String, NodeId> = HashMap::new();
        for observation in stores.observations.observations() {
            let mut properties = observation.to_property_map();
            properties.insert(
                "observation_payload".to_owned(),
                PropertyValue::String(observation.payload().to_owned()),
            );
            let node_id = projection.create_node(with_properties(
                &[EpistemicNodeKind::Observation.canonical_label()],
                properties,
            ))?;
            observation_nodes.insert(observation.id().as_str().to_owned(), node_id.clone());
            if let Some(source_node) = source_nodes.get(observation.source_id().as_str()) {
                projection.create_relationship(RelationshipInput::new(
                    source_node.clone(),
                    EpistemicRelationKind::Reports
                        .canonical_relationship_type()
                        .as_str(),
                    node_id,
                )?)?;
            }
        }

        // Quotient view: group identity without rewriting evidence. Every
        // observation edge retains its original mention ID; full member records
        // preserve offsets, candidate hints and relation neighbourhood features.
        let mut mention_nodes: HashMap<String, NodeId> = HashMap::new();
        let mut groups: std::collections::BTreeMap<String, Vec<&crate::EntityMention>> =
            std::collections::BTreeMap::new();
        let representatives = self.resolved_mentions()?;
        for mention in stores.mentions.mentions() {
            groups
                .entry(representatives[mention.id()].as_str().to_owned())
                .or_default()
                .push(mention);
        }
        for (representative, members) in groups {
            let mention = stores
                .mentions
                .mention_by_id(&crate::EntityMentionId::new(&representative)?)
                .ok_or_else(|| GraphError::InvalidPropertyValue("missing representative".into()))?;
            let mut properties = mention.to_property_map()?;
            if members.len() > 1 {
                properties.insert(
                    "mention_members".into(),
                    PropertyValue::Json(
                        serde_json::to_value(&members)
                            .map_err(|e| GraphError::InvalidPropertyValue(e.to_string()))?,
                    ),
                );
            }
            let node_id = projection.create_node(with_properties(
                &[EpistemicNodeKind::EntityMention.canonical_label()],
                properties,
            ))?;
            for member in members {
                mention_nodes.insert(member.id().as_str().into(), node_id.clone());
                let observation_node = observation_nodes
                    .get(member.observation_id().as_str())
                    .ok_or_else(|| {
                        GraphError::ObservationNotFound(member.observation_id().clone())
                    })?;
                projection.create_relationship(
                    RelationshipInput::new(
                        observation_node.clone(),
                        EpistemicRelationKind::HasMention
                            .canonical_relationship_type()
                            .as_str(),
                        node_id.clone(),
                    )?
                    .with_property(
                        "mention_id",
                        PropertyValue::String(member.id().as_str().to_owned()),
                    ),
                )?;
            }
        }

        // Reconciliation judgments retain all outcomes, including abstention.
        for undo in stores.merges.undos() {
            let decision = projection.create_node(with_properties(
                &[
                    "ReconciliationUndo",
                    EpistemicNodeKind::Decision.canonical_label(),
                ],
                undo.to_property_map(),
            ))?;
            let record = stores
                .reconciliations
                .record_by_id(undo.reconciliation_id())
                .ok_or_else(|| GraphError::InvalidPropertyValue("undo judgment missing".into()))?;
            for mention in [record.left(), record.right()] {
                let target = mention_nodes.get(mention.as_str()).ok_or_else(|| {
                    GraphError::InvalidPropertyValue("undo mention missing".into())
                })?;
                projection.create_relationship(RelationshipInput::new(
                    decision.clone(),
                    EpistemicRelationKind::Decides
                        .canonical_relationship_type()
                        .as_str(),
                    target.clone(),
                )?)?;
            }
        }

        // These edges explain a decision about mentions; they do not merge entities.
        for record in stores.reconciliations.records() {
            let decision = projection.create_node(with_properties(
                &[
                    "ReconciliationRecord",
                    EpistemicNodeKind::Decision.canonical_label(),
                ],
                record.to_property_map()?,
            ))?;
            for mention in [record.left(), record.right()] {
                let target = mention_nodes.get(mention.as_str()).ok_or_else(|| {
                    GraphError::InvalidPropertyValue(
                        "reconciliation mention missing from projection".into(),
                    )
                })?;
                projection.create_relationship(RelationshipInput::new(
                    decision.clone(),
                    EpistemicRelationKind::Decides
                        .canonical_relationship_type()
                        .as_str(),
                    target.clone(),
                )?)?;
            }
        }

        // Evidence records.
        let mut evidence_nodes: HashMap<String, NodeId> = HashMap::new();
        for record in self.evidence.records() {
            let mut properties = PropertyMap::new();
            properties.insert(
                "evidence_id".to_owned(),
                PropertyValue::String(record.id().as_str().to_owned()),
            );
            properties.insert(
                "evidence_source_ref".to_owned(),
                PropertyValue::String(record.source_ref().to_owned()),
            );
            if let Some(source_id) = record.source_id() {
                properties.insert(
                    "evidence_source".to_owned(),
                    PropertyValue::String(source_id.as_str().to_owned()),
                );
            }
            if let Some(observation_id) = record.observation_id() {
                properties.insert(
                    "evidence_observation".to_owned(),
                    PropertyValue::String(observation_id.as_str().to_owned()),
                );
            }
            let node_id = projection.create_node(with_properties(
                &[EpistemicNodeKind::Evidence.canonical_label()],
                properties,
            ))?;
            evidence_nodes.insert(record.id().as_str().to_owned(), node_id);
        }

        // Claims with their current verdict.
        let mut claim_nodes: HashMap<String, NodeId> = HashMap::new();
        for claim in stores.claims.claims() {
            let mut properties = PropertyMap::new();
            properties.insert(
                "claim_id".to_owned(),
                PropertyValue::String(claim.id().as_str().to_owned()),
            );
            properties.insert(
                "claim_version".to_owned(),
                PropertyValue::Integer(i64::try_from(claim.version()).unwrap_or(i64::MAX)),
            );
            properties.insert(
                "claim_status".to_owned(),
                PropertyValue::String(lifecycle_token(claim.status())),
            );
            properties.insert(
                "claim_statement".to_owned(),
                PropertyValue::String(claim.statement().as_str().to_owned()),
            );
            if let ClaimTarget::Node(target) = claim.target() {
                properties.insert(
                    "claim_target_node".to_owned(),
                    PropertyValue::String(target.as_str().to_owned()),
                );
            }
            if let Some(proposition) = claim.proposition() {
                properties.extend(proposition.to_property_map());
            }
            let verification_coverage =
                crate::VerificationCoverage::derive(claim, &stores.verifications);
            properties.extend(verification_coverage.to_property_map());
            if let Some(verdict) = stores.verdicts.current_verdict(claim.id()) {
                properties.insert(
                    "verdict_state".to_owned(),
                    PropertyValue::String(verdict.state().as_str().to_owned()),
                );
                properties.insert(
                    "verdict_lifecycle_projection".to_owned(),
                    PropertyValue::String(lifecycle_token(project_verdict_state(
                        verdict.state(),
                        false,
                    ))),
                );
                properties.insert(
                    "verdict_id".to_owned(),
                    PropertyValue::String(verdict.id().as_str().to_owned()),
                );
            }
            let node_id = projection.create_node(with_properties(
                &[EpistemicNodeKind::Claim.canonical_label()],
                properties,
            ))?;
            claim_nodes.insert(claim.id().as_str().to_owned(), node_id);
        }

        // Evidence links as vocabulary relationships.
        for link in stores.claims.claim_links() {
            let source_node = match link.source() {
                ClaimLinkSource::Observation(id) => observation_nodes.get(id.as_str()),
                ClaimLinkSource::Evidence(id) => evidence_nodes.get(id.as_str()),
                ClaimLinkSource::Claim(id) => claim_nodes.get(id.as_str()),
            };
            let Some(source_node) = source_node else {
                continue;
            };
            let Some(target_node) = claim_nodes.get(link.target_claim_id().as_str()) else {
                continue;
            };
            let mut input = RelationshipInput::new(
                source_node.clone(),
                EpistemicRelationKind::from(link.kind())
                    .canonical_relationship_type()
                    .as_str(),
                target_node.clone(),
            )?;
            for (key, value) in link.to_property_map() {
                input = input.with_property(key, value);
            }
            projection.create_relationship(input)?;
        }

        // Verdicts and verification records assess claims; transitions decide.
        let assesses = EpistemicRelationKind::Assesses.canonical_relationship_type();
        let decides = EpistemicRelationKind::Decides.canonical_relationship_type();
        for claim in stores.claims.claims() {
            let claim_id = claim.id();
            let Some(target_node) = claim_nodes.get(claim_id.as_str()).cloned() else {
                continue;
            };
            let verification_coverage =
                crate::VerificationCoverage::derive(claim, &stores.verifications);
            for verdict in stores.verdicts.verdicts_for_claim(claim_id) {
                let node_id = projection.create_node(with_properties(
                    &["Verdict", EpistemicNodeKind::Assessment.canonical_label()],
                    verdict.to_property_map(),
                ))?;
                projection.create_relationship(RelationshipInput::new(
                    node_id,
                    assesses.as_str(),
                    target_node.clone(),
                )?)?;
            }
            for record in stores.verifications.records_for_claim(claim_id) {
                let mut properties = record.to_property_map();
                properties.insert(
                    "verification_coverage_class".to_owned(),
                    PropertyValue::String(record.coverage_class().as_str().to_owned()),
                );
                properties.insert(
                    "verification_coverage_target".to_owned(),
                    PropertyValue::String(verification_coverage.target().as_str().to_owned()),
                );
                properties.insert(
                    "verification_coverage_current".to_owned(),
                    PropertyValue::Bool(
                        verification_coverage
                            .entries()
                            .iter()
                            .any(|entry| entry.record_id() == Some(record.id().as_str())),
                    ),
                );
                let node_id = projection.create_node(with_properties(
                    &[
                        "VerificationRecord",
                        EpistemicNodeKind::Assessment.canonical_label(),
                    ],
                    properties,
                ))?;
                projection.create_relationship(RelationshipInput::new(
                    node_id,
                    assesses.as_str(),
                    target_node.clone(),
                )?)?;
            }
            for transition in stores.verdicts.transitions_for_claim(claim_id) {
                let node_id = projection.create_node(with_properties(
                    &[
                        "StateTransition",
                        EpistemicNodeKind::Decision.canonical_label(),
                    ],
                    transition.to_property_map(),
                ))?;
                projection.create_relationship(RelationshipInput::new(
                    node_id,
                    decides.as_str(),
                    target_node.clone(),
                )?)?;
            }
        }

        Ok(projection)
    }

    /// Replaces the evidence projection loaded by a durable adapter.
    pub fn replace_evidence_store(&mut self, evidence: EvidenceRecordStore) {
        self.evidence = evidence;
    }

    /// Creates the node.
    pub fn create_node(&mut self, input: NodeInput) -> Result<NodeId, GraphError> {
        input.validate()?;

        let id = self.next_node_id()?;
        let version_id = self.next_node_version_id()?;
        let node = Node {
            id: id.clone(),
            version_id: version_id.clone(),
            version: 1,
            current: true,
            previous_version_id: None,
            labels: input.labels,
            properties: input.properties,
            status: input.status,
            confidence: input.confidence,
            source_reliability: input.source_reliability,
            information_credibility: input.information_credibility,
            extraction_run_id: input.extraction_run_id,
            evidence_refs: input.evidence_refs,
            temporal: input.temporal,
            transaction: input.transaction,
        };

        self.nodes.insert(id.clone(), vec![node]);
        self.current_node_versions.insert(id.clone(), version_id);
        Ok(id)
    }

    /// Returns the node.
    pub fn get_node(&self, id: &NodeId) -> Result<Option<Node>, GraphError> {
        let Some(versions) = self.nodes.get(id) else {
            return Ok(None);
        };
        let current_version_id = self.current_node_versions.get(id).ok_or_else(|| {
            GraphError::InternalInvariantViolation(format!(
                "missing current node version pointer for {}",
                id.as_str()
            ))
        })?;
        let current = versions
            .iter()
            .find(|version| &version.version_id == current_version_id)
            .ok_or_else(|| {
                GraphError::InternalInvariantViolation(format!(
                    "current node version {} is missing for {}",
                    current_version_id.as_str(),
                    id.as_str()
                ))
            })?;

        if current.status == RecordStatus::Tombstoned {
            Ok(None)
        } else {
            Ok(Some(current.clone()))
        }
    }

    /// Update node.
    pub fn update_node(&mut self, id: &NodeId, patch: NodePatch) -> Result<NodeId, GraphError> {
        let current = self.current_node_version(id)?;
        if current.status == RecordStatus::Tombstoned {
            return Err(GraphError::RecordAlreadyTombstoned(id.as_str().to_owned()));
        }

        let version_id = self.next_node_version_id()?;
        let mut next = current.clone();
        next.version_id = version_id.clone();
        next.version += 1;
        next.current = true;
        next.previous_version_id = Some(current.version_id.clone());
        Self::apply_node_patch(&mut next, patch)?;
        self.append_node_version(id, &current.version_id, version_id, next)?;
        Ok(id.clone())
    }

    /// Replace the complete current node payload with a new canonical version.
    ///
    /// Synchronization adapters use this operation when a source record may
    /// remove labels or properties. Patch semantics cannot represent removal,
    /// while replacement preserves graph identity and version history.
    pub fn replace_node(&mut self, id: &NodeId, input: NodeInput) -> Result<NodeId, GraphError> {
        input.validate()?;
        let current = self.current_node_version(id)?;
        if current.status == RecordStatus::Tombstoned {
            return Err(GraphError::RecordAlreadyTombstoned(id.as_str().to_owned()));
        }
        let version_id = self.next_node_version_id()?;
        let next = Node {
            id: id.clone(),
            version_id: version_id.clone(),
            version: current.version + 1,
            current: true,
            previous_version_id: Some(current.version_id.clone()),
            labels: input.labels,
            properties: input.properties,
            status: input.status,
            confidence: input.confidence,
            source_reliability: input.source_reliability,
            information_credibility: input.information_credibility,
            extraction_run_id: input.extraction_run_id,
            evidence_refs: input.evidence_refs,
            temporal: input.temporal,
            transaction: input.transaction,
        };
        self.append_node_version(id, &current.version_id, version_id, next)?;
        Ok(id.clone())
    }

    /// Tombstone node.
    pub fn tombstone_node(&mut self, id: &NodeId) -> Result<NodeId, GraphError> {
        let current = self.current_node_version(id)?;
        if current.status == RecordStatus::Tombstoned {
            return Err(GraphError::RecordAlreadyTombstoned(id.as_str().to_owned()));
        }

        let version_id = self.next_node_version_id()?;
        let mut tombstone = current.clone();
        tombstone.version_id = version_id.clone();
        tombstone.version += 1;
        tombstone.current = true;
        tombstone.previous_version_id = Some(current.version_id.clone());
        tombstone.status = RecordStatus::Tombstoned;
        self.append_node_version(id, &current.version_id, version_id, tombstone)?;
        Ok(id.clone())
    }

    /// List nodes.
    pub fn list_nodes(&self) -> Result<Vec<Node>, GraphError> {
        let mut nodes = Vec::new();

        for node_id in self.nodes.keys() {
            if let Some(node) = self.get_node(node_id)? {
                nodes.push(node);
            }
        }

        nodes.sort_by(|left, right| left.id().as_str().cmp(right.id().as_str()));
        Ok(nodes)
    }

    /// List relationships.
    pub fn list_relationships(&self) -> Result<Vec<Relationship>, GraphError> {
        let mut relationships = Vec::new();

        for relationship_id in self.relationships.keys() {
            if let Some(relationship) = self.get_relationship(relationship_id)? {
                relationships.push(relationship);
            }
        }

        relationships.sort_by(|left, right| left.id().as_str().cmp(right.id().as_str()));
        Ok(relationships)
    }

    /// Creates the relationship.
    pub fn create_relationship(
        &mut self,
        input: RelationshipInput,
    ) -> Result<RelationshipId, GraphError> {
        if self.get_node(&input.source)?.is_none() {
            return Err(GraphError::SourceNodeNotFound(input.source));
        }
        if self.get_node(&input.target)?.is_none() {
            return Err(GraphError::TargetNodeNotFound(input.target));
        }

        let id = self.next_relationship_id()?;
        let version_id = self.next_relationship_version_id()?;
        let source = input.source.clone();
        let target = input.target.clone();
        let relationship = Relationship {
            id: id.clone(),
            version_id: version_id.clone(),
            version: 1,
            current: true,
            previous_version_id: None,
            source: source.clone(),
            target: target.clone(),
            rel_type: input.rel_type,
            properties: input.properties,
            status: input.status,
            confidence: input.confidence,
            source_reliability: input.source_reliability,
            information_credibility: input.information_credibility,
            extraction_run_id: input.extraction_run_id,
            evidence_refs: input.evidence_refs,
            temporal: input.temporal,
            transaction: input.transaction,
        };

        self.relationships.insert(id.clone(), vec![relationship]);
        self.current_relationship_versions
            .insert(id.clone(), version_id);
        self.adjacency
            .record_created_relationship(&id, &source, &target)?;
        Ok(id)
    }

    /// Returns the relationship.
    pub fn get_relationship(
        &self,
        id: &RelationshipId,
    ) -> Result<Option<Relationship>, GraphError> {
        let Some(versions) = self.relationships.get(id) else {
            return Ok(None);
        };
        let current_version_id = self.current_relationship_versions.get(id).ok_or_else(|| {
            GraphError::InternalInvariantViolation(format!(
                "missing current relationship version pointer for {}",
                id.as_str()
            ))
        })?;
        let current = versions
            .iter()
            .find(|version| &version.version_id == current_version_id)
            .ok_or_else(|| {
                GraphError::InternalInvariantViolation(format!(
                    "current relationship version {} is missing for {}",
                    current_version_id.as_str(),
                    id.as_str()
                ))
            })?;

        if current.status == RecordStatus::Tombstoned {
            Ok(None)
        } else {
            Ok(Some(current.clone()))
        }
    }

    /// Update relationship.
    pub fn update_relationship(
        &mut self,
        id: &RelationshipId,
        patch: RelationshipPatch,
    ) -> Result<RelationshipId, GraphError> {
        let current = self.current_relationship_version(id)?;
        if current.status == RecordStatus::Tombstoned {
            return Err(GraphError::RecordAlreadyTombstoned(id.as_str().to_owned()));
        }

        let version_id = self.next_relationship_version_id()?;
        let mut next = current.clone();
        next.version_id = version_id.clone();
        next.version += 1;
        next.current = true;
        next.previous_version_id = Some(current.version_id.clone());
        Self::apply_relationship_patch(&mut next, patch)?;
        self.append_relationship_version(id, &current.version_id, version_id, next)?;
        Ok(id.clone())
    }

    /// Replace the complete current relationship payload with a new version.
    ///
    /// Endpoint changes update both adjacency directions atomically in the
    /// in-memory projection while the stable relationship identity is retained.
    pub fn replace_relationship(
        &mut self,
        id: &RelationshipId,
        input: RelationshipInput,
    ) -> Result<RelationshipId, GraphError> {
        if self.get_node(&input.source)?.is_none() {
            return Err(GraphError::SourceNodeNotFound(input.source));
        }
        if self.get_node(&input.target)?.is_none() {
            return Err(GraphError::TargetNodeNotFound(input.target));
        }
        let current = self.current_relationship_version(id)?;
        if current.status == RecordStatus::Tombstoned {
            return Err(GraphError::RecordAlreadyTombstoned(id.as_str().to_owned()));
        }
        let version_id = self.next_relationship_version_id()?;
        let source = input.source.clone();
        let target = input.target.clone();
        let next = Relationship {
            id: id.clone(),
            version_id: version_id.clone(),
            version: current.version + 1,
            current: true,
            previous_version_id: Some(current.version_id.clone()),
            source: source.clone(),
            target: target.clone(),
            rel_type: input.rel_type,
            properties: input.properties,
            status: input.status,
            confidence: input.confidence,
            source_reliability: input.source_reliability,
            information_credibility: input.information_credibility,
            extraction_run_id: input.extraction_run_id,
            evidence_refs: input.evidence_refs,
            temporal: input.temporal,
            transaction: input.transaction,
        };
        self.adjacency.record_replaced_relationship(
            id,
            &current.source,
            &current.target,
            &source,
            &target,
        )?;
        self.append_relationship_version(id, &current.version_id, version_id, next)?;
        Ok(id.clone())
    }

    /// Tombstone relationship.
    pub fn tombstone_relationship(
        &mut self,
        id: &RelationshipId,
    ) -> Result<RelationshipId, GraphError> {
        let current = self.current_relationship_version(id)?;
        if current.status == RecordStatus::Tombstoned {
            return Err(GraphError::RecordAlreadyTombstoned(id.as_str().to_owned()));
        }

        let version_id = self.next_relationship_version_id()?;
        let mut tombstone = current.clone();
        tombstone.version_id = version_id.clone();
        tombstone.version += 1;
        tombstone.current = true;
        tombstone.previous_version_id = Some(current.version_id.clone());
        tombstone.status = RecordStatus::Tombstoned;
        self.append_relationship_version(id, &current.version_id, version_id, tombstone)?;
        Ok(id.clone())
    }

    /// Outgoing.
    pub fn outgoing(&self, node_id: &NodeId) -> Result<Vec<Relationship>, GraphError> {
        let relationship_ids = self.adjacency.outgoing_ids(node_id)?;
        self.visible_current_relationships(relationship_ids)
    }

    /// Incoming.
    pub fn incoming(&self, node_id: &NodeId) -> Result<Vec<Relationship>, GraphError> {
        let relationship_ids = self.adjacency.incoming_ids(node_id)?;
        self.visible_current_relationships(relationship_ids)
    }

    /// Relationships between.
    pub fn relationships_between(
        &self,
        source: &NodeId,
        target: &NodeId,
    ) -> Result<Vec<Relationship>, GraphError> {
        let relationship_ids = self.adjacency.between_ids(source, target)?;
        self.visible_current_relationships(relationship_ids)
    }

    /// Returns the node version.
    pub fn get_node_version(
        &self,
        node_id: &NodeId,
        version_id: &NodeVersionId,
    ) -> Result<Option<Node>, GraphError> {
        Ok(self.nodes.get(node_id).and_then(|versions| {
            versions
                .iter()
                .find(|version| &version.version_id == version_id)
                .cloned()
        }))
    }

    /// Returns the relationship version.
    pub fn get_relationship_version(
        &self,
        relationship_id: &RelationshipId,
        version_id: &RelationshipVersionId,
    ) -> Result<Option<Relationship>, GraphError> {
        Ok(self
            .relationships
            .get(relationship_id)
            .and_then(|versions| {
                versions
                    .iter()
                    .find(|version| &version.version_id == version_id)
                    .cloned()
            }))
    }

    /// List node versions.
    pub fn list_node_versions(&self, node_id: &NodeId) -> Result<Vec<Node>, GraphError> {
        let mut versions = self.nodes.get(node_id).cloned().unwrap_or_default();
        versions.sort_by_key(|node| node.version);
        Ok(versions)
    }

    /// List relationship versions.
    pub fn list_relationship_versions(
        &self,
        relationship_id: &RelationshipId,
    ) -> Result<Vec<Relationship>, GraphError> {
        let mut versions = self
            .relationships
            .get(relationship_id)
            .cloned()
            .unwrap_or_default();
        versions.sort_by_key(|relationship| relationship.version);
        Ok(versions)
    }

    fn next_node_id(&mut self) -> Result<NodeId, GraphError> {
        self.next_node_sequence += 1;
        NodeId::new(format!("node--{}", self.next_node_sequence))
    }

    fn next_node_version_id(&mut self) -> Result<NodeVersionId, GraphError> {
        self.next_node_version_sequence += 1;
        NodeVersionId::new(format!("node-version--{}", self.next_node_version_sequence))
    }

    fn next_relationship_id(&mut self) -> Result<RelationshipId, GraphError> {
        self.next_relationship_sequence += 1;
        RelationshipId::new(format!("relationship--{}", self.next_relationship_sequence))
    }

    fn next_relationship_version_id(&mut self) -> Result<RelationshipVersionId, GraphError> {
        self.next_relationship_version_sequence += 1;
        RelationshipVersionId::new(format!(
            "relationship-version--{}",
            self.next_relationship_version_sequence
        ))
    }

    fn current_node_version(&self, id: &NodeId) -> Result<Node, GraphError> {
        let versions = self
            .nodes
            .get(id)
            .ok_or_else(|| GraphError::NodeNotFound(id.clone()))?;
        let current_version_id = self.current_node_versions.get(id).ok_or_else(|| {
            GraphError::InternalInvariantViolation(format!(
                "missing current node version pointer for {}",
                id.as_str()
            ))
        })?;
        versions
            .iter()
            .find(|version| &version.version_id == current_version_id)
            .cloned()
            .ok_or_else(|| {
                GraphError::InternalInvariantViolation(format!(
                    "current node version {} is missing for {}",
                    current_version_id.as_str(),
                    id.as_str()
                ))
            })
    }

    fn current_relationship_version(
        &self,
        id: &RelationshipId,
    ) -> Result<Relationship, GraphError> {
        let versions = self
            .relationships
            .get(id)
            .ok_or_else(|| GraphError::RelationshipNotFound(id.clone()))?;
        let current_version_id = self.current_relationship_versions.get(id).ok_or_else(|| {
            GraphError::InternalInvariantViolation(format!(
                "missing current relationship version pointer for {}",
                id.as_str()
            ))
        })?;
        versions
            .iter()
            .find(|version| &version.version_id == current_version_id)
            .cloned()
            .ok_or_else(|| {
                GraphError::InternalInvariantViolation(format!(
                    "current relationship version {} is missing for {}",
                    current_version_id.as_str(),
                    id.as_str()
                ))
            })
    }

    fn visible_current_relationships(
        &self,
        relationship_ids: Vec<RelationshipId>,
    ) -> Result<Vec<Relationship>, GraphError> {
        let mut relationships = Vec::new();
        for relationship_id in relationship_ids {
            let relationship = self.current_relationship_version(&relationship_id)?;
            if relationship.status != RecordStatus::Tombstoned {
                relationships.push(relationship);
            }
        }
        Ok(relationships)
    }

    fn apply_node_patch(node: &mut Node, patch: NodePatch) -> Result<(), GraphError> {
        let requests_exportable = patch.status == Some(RecordStatus::Exportable);
        for (key, value) in patch.properties_to_set {
            node.properties.insert(key, value);
        }
        if let Some(status) = patch.status {
            node.status = status;
        }
        if let Some(confidence) = patch.confidence {
            node.confidence = Some(confidence);
            node.properties.remove("confidence");
        }
        if let Some(evidence_refs) = patch.evidence_refs {
            node.evidence_refs = evidence_refs;
            node.properties.remove("evidence_refs");
        }
        if patch.status.is_some() {
            node.properties.remove("status");
        }
        validate_exportable_transition(
            requests_exportable,
            node.confidence.is_some(),
            !node.evidence_refs.is_empty(),
        )
    }

    fn apply_relationship_patch(
        relationship: &mut Relationship,
        patch: RelationshipPatch,
    ) -> Result<(), GraphError> {
        let requests_exportable = patch.status == Some(RecordStatus::Exportable);
        for (key, value) in patch.properties_to_set {
            relationship.properties.insert(key, value);
        }
        if let Some(status) = patch.status {
            relationship.status = status;
        }
        if let Some(confidence) = patch.confidence {
            relationship.confidence = Some(confidence);
            relationship.properties.remove("confidence");
        }
        if let Some(evidence_refs) = patch.evidence_refs {
            relationship.evidence_refs = evidence_refs;
            relationship.properties.remove("evidence_refs");
        }
        if patch.status.is_some() {
            relationship.properties.remove("status");
        }
        validate_exportable_transition(
            requests_exportable,
            relationship.confidence.is_some(),
            !relationship.evidence_refs.is_empty(),
        )
    }

    fn append_node_version(
        &mut self,
        id: &NodeId,
        previous_version_id: &NodeVersionId,
        current_version_id: NodeVersionId,
        next: Node,
    ) -> Result<(), GraphError> {
        let versions = self
            .nodes
            .get_mut(id)
            .ok_or_else(|| GraphError::NodeNotFound(id.clone()))?;
        let previous = versions
            .iter_mut()
            .find(|version| &version.version_id == previous_version_id)
            .ok_or_else(|| {
                GraphError::InternalInvariantViolation(format!(
                    "previous node version {} is missing for {}",
                    previous_version_id.as_str(),
                    id.as_str()
                ))
            })?;
        previous.current = false;
        versions.push(next);
        self.current_node_versions
            .insert(id.clone(), current_version_id);
        Ok(())
    }

    fn append_relationship_version(
        &mut self,
        id: &RelationshipId,
        previous_version_id: &RelationshipVersionId,
        current_version_id: RelationshipVersionId,
        next: Relationship,
    ) -> Result<(), GraphError> {
        let versions = self
            .relationships
            .get_mut(id)
            .ok_or_else(|| GraphError::RelationshipNotFound(id.clone()))?;
        let previous = versions
            .iter_mut()
            .find(|version| &version.version_id == previous_version_id)
            .ok_or_else(|| {
                GraphError::InternalInvariantViolation(format!(
                    "previous relationship version {} is missing for {}",
                    previous_version_id.as_str(),
                    id.as_str()
                ))
            })?;
        previous.current = false;
        versions.push(next);
        self.current_relationship_versions
            .insert(id.clone(), current_version_id);
        Ok(())
    }
}

fn validate_exportable_transition(
    requests_exportable: bool,
    has_confidence: bool,
    has_evidence: bool,
) -> Result<(), GraphError> {
    if requests_exportable && (!has_confidence || !has_evidence) {
        return Err(GraphError::InvalidRecordStatusTransition(
            "exportable requires native confidence and at least one evidence reference".to_owned(),
        ));
    }
    Ok(())
}

impl Graph {
    pub(crate) fn scoped_audit_snapshot(
        &self,
        stores: crate::EpistemicStores,
        evidence: &std::collections::HashSet<EvidenceId>,
        roots: &[crate::ClaimId],
    ) -> GraphPersistenceSnapshot {
        let mut snapshot = self.persistence_snapshot();
        let mut nodes = std::collections::HashSet::new();
        let mut relationships = std::collections::HashSet::new();
        for id in roots {
            if let Ok(claim) = stores.claims.claim_by_id(id) {
                match claim.target() {
                    crate::ClaimTarget::Node(id) => {
                        nodes.insert(id.clone());
                    }
                    crate::ClaimTarget::Relationship(id) => {
                        relationships.insert(id.clone());
                    }
                    _ => {}
                }
            }
        }
        snapshot
            .relationships
            .retain(|r| relationships.contains(r.id()));
        for r in &snapshot.relationships {
            nodes.insert(r.source().clone());
            nodes.insert(r.target().clone());
        }
        snapshot.nodes.retain(|r| nodes.contains(r.id()));
        snapshot.evidence = self.evidence.audit_subset(evidence);
        snapshot.epistemic = stores;
        snapshot
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EvidenceLocator, PropertyValue};

    fn setup_graph_with_relationship() -> (Graph, NodeId, NodeId, RelationshipId) {
        let mut graph = Graph::new();
        let source = graph
            .create_node(NodeInput::new(["Actor"]))
            .expect("source node creation should succeed");
        let target = graph
            .create_node(NodeInput::new(["Indicator"]))
            .expect("target node creation should succeed");
        let relationship = graph
            .create_relationship(
                RelationshipInput::new(source.clone(), "USES", target.clone())
                    .expect("relationship input should be valid"),
            )
            .expect("relationship creation should succeed");

        (graph, source, target, relationship)
    }

    #[test]
    fn native_evidence_reference_patches_are_versioned_for_nodes_and_relationships() {
        let (mut graph, source, _target, relationship) = setup_graph_with_relationship();
        let evidence_id = EvidenceId::new("span--123").expect("evidence id should be valid");

        graph
            .update_node(
                &source,
                NodePatch::default().set_evidence_refs(vec![evidence_id.clone()]),
            )
            .expect("node evidence patch should succeed");
        graph
            .update_relationship(
                &relationship,
                RelationshipPatch::default().set_evidence_refs(vec![evidence_id.clone()]),
            )
            .expect("relationship evidence patch should succeed");

        assert_eq!(
            graph
                .get_node(&source)
                .expect("node lookup should succeed")
                .expect("node should exist")
                .evidence_refs(),
            std::slice::from_ref(&evidence_id)
        );
        assert_eq!(
            graph
                .get_relationship(&relationship)
                .expect("relationship lookup should succeed")
                .expect("relationship should exist")
                .evidence_refs(),
            std::slice::from_ref(&evidence_id)
        );

        let restored = Graph::from_persistence_snapshot(graph.persistence_snapshot())
            .expect("native metadata snapshot should restore");
        assert_eq!(
            restored
                .get_node(&source)
                .expect("restored node lookup should succeed")
                .expect("restored node should exist")
                .evidence_refs(),
            std::slice::from_ref(&evidence_id)
        );
        assert_eq!(
            restored
                .get_relationship(&relationship)
                .expect("restored relationship lookup should succeed")
                .expect("restored relationship should exist")
                .evidence_refs(),
            std::slice::from_ref(&evidence_id)
        );
    }

    #[test]
    fn replace_node_removes_stale_labels_and_properties() {
        let mut graph = Graph::new();
        let node_id = graph
            .create_node(
                NodeInput::new(["Legacy"])
                    .with_property("removed", PropertyValue::String("stale".to_owned())),
            )
            .expect("node creation should succeed");

        graph
            .replace_node(
                &node_id,
                NodeInput::new(["Current"])
                    .with_property("retained", PropertyValue::String("fresh".to_owned())),
            )
            .expect("node replacement should succeed");

        let current = graph
            .get_node(&node_id)
            .expect("node lookup should succeed")
            .expect("node should remain visible");
        assert_eq!(current.version(), 2);
        assert_eq!(current.labels(), &["Current"]);
        assert!(current.property("removed").is_none());
        assert_eq!(
            current.property("retained"),
            Some(&PropertyValue::String("fresh".to_owned()))
        );
    }

    #[test]
    fn replace_relationship_moves_adjacency_to_new_endpoints() {
        let (mut graph, old_source, old_target, relationship_id) = setup_graph_with_relationship();
        let new_source = graph
            .create_node(NodeInput::new(["Actor"]))
            .expect("new source creation should succeed");
        let new_target = graph
            .create_node(NodeInput::new(["Indicator"]))
            .expect("new target creation should succeed");

        graph
            .replace_relationship(
                &relationship_id,
                RelationshipInput::new(new_source.clone(), "USES", new_target.clone())
                    .expect("replacement input should be valid"),
            )
            .expect("relationship replacement should succeed");

        assert!(
            graph
                .outgoing(&old_source)
                .expect("old outgoing lookup should succeed")
                .is_empty()
        );
        assert!(
            graph
                .incoming(&old_target)
                .expect("old incoming lookup should succeed")
                .is_empty()
        );
        assert_eq!(
            graph
                .outgoing(&new_source)
                .expect("new outgoing lookup should succeed")
                .len(),
            1
        );
        assert_eq!(
            graph
                .incoming(&new_target)
                .expect("new incoming lookup should succeed")
                .len(),
            1
        );
        let current = graph
            .get_relationship(&relationship_id)
            .expect("relationship lookup should succeed")
            .expect("relationship should remain visible");
        assert_eq!(current.version(), 2);
        assert_eq!(current.source(), &new_source);
        assert_eq!(current.target(), &new_target);
    }

    #[test]
    fn get_node_reports_missing_current_pointer_invariant() {
        let mut graph = Graph::new();
        let node_id = graph
            .create_node(NodeInput::new(["Campaign"]))
            .expect("node creation should succeed");
        graph.current_node_versions.remove(&node_id);

        let error = graph
            .get_node(&node_id)
            .expect_err("missing current node pointer should return invariant error");

        assert!(matches!(
        error,
        GraphError::InternalInvariantViolation(message)
        if message.contains("missing current node version pointer")
        && message.contains(node_id.as_str())
        ));
    }

    #[test]
    fn get_node_reports_missing_current_version_invariant() {
        let mut graph = Graph::new();
        let node_id = graph
            .create_node(NodeInput::new(["Campaign"]))
            .expect("node creation should succeed");
        let fake_version =
            NodeVersionId::new("node-version--999").expect("fake node version ID should be valid");
        graph
            .current_node_versions
            .insert(node_id.clone(), fake_version.clone());

        let error = graph
            .get_node(&node_id)
            .expect_err("unknown current node version should return invariant error");

        assert!(matches!(
        error,
        GraphError::InternalInvariantViolation(message)
        if message.contains("current node version")
        && message.contains(fake_version.as_str())
        && message.contains(node_id.as_str())
        ));
    }

    #[test]
    fn get_relationship_reports_missing_current_pointer_invariant() {
        let (mut graph, _, _, relationship_id) = setup_graph_with_relationship();
        graph.current_relationship_versions.remove(&relationship_id);

        let error = graph
            .get_relationship(&relationship_id)
            .expect_err("missing current relationship pointer should return invariant error");

        assert!(matches!(
        error,
        GraphError::InternalInvariantViolation(message)
        if message.contains("missing current relationship version pointer")
        && message.contains(relationship_id.as_str())
        ));
    }

    #[test]
    fn get_relationship_reports_missing_current_version_invariant() {
        let (mut graph, _, _, relationship_id) = setup_graph_with_relationship();
        let fake_version = RelationshipVersionId::new("relationship-version--999")
            .expect("fake relationship version ID should be valid");
        graph
            .current_relationship_versions
            .insert(relationship_id.clone(), fake_version.clone());

        let error = graph
            .get_relationship(&relationship_id)
            .expect_err("unknown current relationship version should return invariant error");

        assert!(matches!(
        error,
        GraphError::InternalInvariantViolation(message)
        if message.contains("current relationship version")
        && message.contains(fake_version.as_str())
        && message.contains(relationship_id.as_str())
        ));
    }

    #[test]
    fn append_node_version_reports_missing_previous_version_invariant() {
        let mut graph = Graph::new();
        let node_id = graph
            .create_node(NodeInput::new(["Campaign"]))
            .expect("node creation should succeed");
        let current = graph
            .get_node(&node_id)
            .expect("get_node should succeed")
            .expect("current node should exist");
        let new_version_id = graph
            .next_node_version_id()
            .expect("new node version ID should be generated");
        let mut next = current.clone();
        next.version_id = new_version_id.clone();
        next.version += 1;
        next.current = true;
        let fake_previous =
            NodeVersionId::new("node-version--404").expect("fake node version ID should be valid");

        let error = graph
            .append_node_version(&node_id, &fake_previous, new_version_id, next)
            .expect_err("missing previous version should return invariant error");

        assert!(matches!(
        error,
        GraphError::InternalInvariantViolation(message)
        if message.contains("previous node version")
        && message.contains(fake_previous.as_str())
        && message.contains(node_id.as_str())
        ));
    }

    #[test]
    fn append_relationship_version_reports_missing_previous_version_invariant() {
        let (mut graph, source, target, relationship_id) = setup_graph_with_relationship();
        let current = graph
            .get_relationship(&relationship_id)
            .expect("get_relationship should succeed")
            .expect("current relationship should exist");
        let new_version_id = graph
            .next_relationship_version_id()
            .expect("new relationship version ID should be generated");
        let mut next = current.clone();
        next.version_id = new_version_id.clone();
        next.version += 1;
        next.current = true;
        next.source = source;
        next.target = target;
        let fake_previous = RelationshipVersionId::new("relationship-version--404")
            .expect("fake relationship version ID should be valid");

        let error = graph
            .append_relationship_version(&relationship_id, &fake_previous, new_version_id, next)
            .expect_err("missing previous relationship version should return invariant error");

        assert!(matches!(
        error,
        GraphError::InternalInvariantViolation(message)
        if message.contains("previous relationship version")
        && message.contains(fake_previous.as_str())
        && message.contains(relationship_id.as_str())
        ));
    }

    #[test]
    fn visible_current_relationships_skips_tombstoned_records() {
        let (mut graph, _, _, relationship_id) = setup_graph_with_relationship();
        graph
            .tombstone_relationship(&relationship_id)
            .expect("tombstoning relationship should succeed");

        let visible = graph
            .visible_current_relationships(vec![relationship_id])
            .expect("visibility lookup should succeed");

        assert!(visible.is_empty());
    }

    #[test]
    fn node_and_relationship_lifecycle_paths_cover_create_update_tombstone_and_lists() {
        let mut graph = Graph::new();
        let source = graph
            .create_node(NodeInput::new(["Actor"]))
            .expect("source node creation should succeed");
        let target = graph
            .create_node(NodeInput::new(["Indicator"]))
            .expect("target node creation should succeed");

        let node_patch = NodePatch::default().set_status(RecordStatus::Validated);
        let _ = graph
            .update_node(&source, node_patch)
            .expect("node update should succeed");

        let relationship_id = graph
            .create_relationship(
                RelationshipInput::new(source.clone(), "USES", target.clone())
                    .expect("relationship input should be valid"),
            )
            .expect("relationship creation should succeed");

        let relationship_patch = RelationshipPatch::default().set_status(RecordStatus::Validated);
        let _ = graph
            .update_relationship(&relationship_id, relationship_patch)
            .expect("relationship update should succeed");

        let outgoing = graph
            .outgoing(&source)
            .expect("outgoing lookup should succeed");
        let incoming = graph
            .incoming(&target)
            .expect("incoming lookup should succeed");
        let between = graph
            .relationships_between(&source, &target)
            .expect("between lookup should succeed");
        assert_eq!(outgoing.len(), 1);
        assert_eq!(incoming.len(), 1);
        assert_eq!(between.len(), 1);

        let node_versions = graph
            .list_node_versions(&source)
            .expect("node versions should be readable");
        let relationship_versions = graph
            .list_relationship_versions(&relationship_id)
            .expect("relationship versions should be readable");
        assert_eq!(node_versions.len(), 2);
        assert_eq!(relationship_versions.len(), 2);

        graph
            .tombstone_node(&target)
            .expect("node tombstone should succeed");
        graph
            .tombstone_relationship(&relationship_id)
            .expect("relationship tombstone should succeed");

        assert!(
            graph
                .get_node(&target)
                .expect("node lookup should succeed")
                .is_none()
        );
        assert!(
            graph
                .get_relationship(&relationship_id)
                .expect("relationship lookup should succeed")
                .is_none()
        );

        let listed_nodes = graph.list_nodes().expect("node listing should succeed");
        let listed_relationships = graph
            .list_relationships()
            .expect("relationship listing should succeed");
        assert_eq!(listed_nodes.len(), 1);
        assert!(listed_relationships.is_empty());
    }

    #[test]
    fn relationship_creation_rejects_missing_source_or_target_nodes() {
        let mut graph = Graph::new();
        let existing = graph
            .create_node(NodeInput::new(["Actor"]))
            .expect("existing node creation should succeed");
        let missing = NodeId::new("node--missing").expect("missing node id should be valid");

        let missing_source = graph
            .create_relationship(
                RelationshipInput::new(missing.clone(), "USES", existing.clone())
                    .expect("relationship input should be valid"),
            )
            .expect_err("missing source should be rejected");
        assert!(matches!(
        missing_source,
        GraphError::SourceNodeNotFound(node_id) if node_id == missing
        ));

        let missing_target = graph
            .create_relationship(
                RelationshipInput::new(existing, "USES", missing.clone())
                    .expect("relationship input should be valid"),
            )
            .expect_err("missing target should be rejected");
        assert!(matches!(
        missing_target,
        GraphError::TargetNodeNotFound(node_id) if node_id == missing
        ));
    }

    #[test]
    fn get_specific_version_helpers_return_none_when_version_is_unknown() {
        let (graph, source, _, relationship_id) = setup_graph_with_relationship();
        let missing_node_version =
            NodeVersionId::new("node-version--unknown").expect("node version id should be valid");
        let missing_relationship_version =
            RelationshipVersionId::new("relationship-version--unknown")
                .expect("relationship version id should be valid");

        assert!(
            graph
                .get_node_version(&source, &missing_node_version)
                .expect("node version lookup should succeed")
                .is_none()
        );
        assert!(
            graph
                .get_relationship_version(&relationship_id, &missing_relationship_version)
                .expect("relationship version lookup should succeed")
                .is_none()
        );
    }

    #[test]
    fn persistence_snapshot_preserves_evidence_records() {
        let mut graph = Graph::new();
        let evidence_id =
            EvidenceId::new("evidence--snapshot-1").expect("evidence id should be valid");
        graph
            .create_evidence(
                EvidenceInput::new(evidence_id.clone(), "document--snapshot", "payload")
                    .with_content_sha256(
                        "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                    )
                    .with_locator(EvidenceLocator::ByteRange { start: 10, end: 20 }),
            )
            .expect("evidence should be created");
        graph
            .create_node(NodeInput::new(["OpenCtiObject"]).with_evidence_ref(evidence_id.clone()))
            .expect("node should be created");

        let restored = Graph::from_persistence_snapshot(graph.persistence_snapshot())
            .expect("snapshot should restore");

        assert_eq!(restored.evidence_count(), 1);
        assert_eq!(
            restored
                .evidence_by_id(&evidence_id)
                .expect("evidence should survive snapshot")
                .source_ref(),
            "document--snapshot"
        );
        assert_eq!(
            restored.list_nodes().expect("nodes should load")[0].evidence_refs(),
            &[evidence_id]
        );
    }
}

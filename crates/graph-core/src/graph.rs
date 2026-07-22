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

use crate::{
    GraphError, Node, NodeId, NodeInput, NodePatch, NodeVersionId, RecordStatus, Relationship,
    RelationshipId, RelationshipInput, RelationshipPatch, RelationshipVersionId,
    adjacency::AdjacencyIndexes,
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
}

impl Graph {
    /// Creates a new instance.
    pub fn new() -> Self {
        Self::default()
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
        Self::apply_node_patch(&mut next, patch);
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

    fn apply_node_patch(node: &mut Node, patch: NodePatch) {
        for (key, value) in patch.properties_to_set {
            node.properties.insert(key, value);
        }
        if let Some(status) = patch.status {
            node.status = status;
        }
        if let Some(confidence) = patch.confidence {
            node.confidence = Some(confidence);
        }
    }

    fn apply_relationship_patch(
        relationship: &mut Relationship,
        patch: RelationshipPatch,
    ) -> Result<(), GraphError> {
        for (key, value) in patch.properties_to_set {
            relationship.properties.insert(key, value);
        }
        if let Some(status) = patch.status {
            relationship.status = status;
        }
        if let Some(confidence) = patch.confidence {
            relationship.confidence = Some(confidence);
        }
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

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
}

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

//! Batches of record edits published as one commit (issues #352 and #366).
//!
//! A viewer collects drafts (property changes, new nodes and relationships,
//! deletions) and publishes them together. Every edit applies to the head of
//! one branch; the batch is validated in full before anything is written, then
//! published as a single commit: through the canonical WAL on `main`, through
//! branch payloads elsewhere. Several property edits of one record merge into
//! one new version. A relationship created in the batch may name a node created
//! earlier in the same batch as `$n`, the n-th `create_node` (from 0).

use std::collections::{BTreeMap, BTreeSet};

use graph_core::{
    Node, NodeId, NodeInput, NodePatch, PropertyValue, Relationship, RelationshipId,
    RelationshipInput, RelationshipPatch,
};

use super::{CanonicalEngineStore, TransitionCommitContext};
use crate::graph_history::{GraphCommitId, HistoryRecordKind, MAIN_BRANCH, hex_sha256};
use crate::{DurableTransactionId, GraphStorageError, GraphStorageResult};

/// Most edits one batch may hold.
pub const MAX_GRAPH_EDITS: usize = 500;

/// One edit of a batch.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub enum GraphEdit {
    /// Set and remove properties of a live node.
    SetProperties {
        /// Node to change.
        node_id: String,
        /// Properties to set.
        set: BTreeMap<String, PropertyValue>,
        /// Properties to remove, after the sets.
        remove: Vec<String>,
    },
    /// Create a node.
    CreateNode {
        /// Labels, at least one.
        labels: Vec<String>,
        /// Initial properties.
        properties: BTreeMap<String, PropertyValue>,
    },
    /// Tombstone a live node; its live relationships must be deleted too,
    /// in this batch or before.
    DeleteNode {
        /// Node to delete.
        node_id: String,
    },
    /// Create a relationship between live nodes or nodes created earlier in
    /// the batch (`$n`).
    CreateRelationship {
        /// Source node id, or `$n`.
        source_id: String,
        /// Target node id, or `$n`.
        target_id: String,
        /// Relationship type.
        rel_type: String,
        /// Initial properties.
        properties: BTreeMap<String, PropertyValue>,
    },
    /// Set and remove properties of a live relationship.
    SetRelationshipProperties {
        /// Relationship to change.
        relationship_id: String,
        /// Properties to set.
        set: BTreeMap<String, PropertyValue>,
        /// Properties to remove, after the sets.
        remove: Vec<String>,
    },
    /// Tombstone a live relationship.
    DeleteRelationship {
        /// Relationship to delete.
        relationship_id: String,
    },
}

/// A batch of edits for one branch.
#[derive(Clone, Debug)]
pub struct GraphEditRequest {
    /// Branch to write; `main` goes through the canonical WAL.
    pub branch: String,
    /// Head the caller read; the batch is refused if the branch moved.
    pub expected_head: Option<GraphCommitId>,
    /// Who edited.
    pub actor: Option<String>,
    /// Commit message.
    pub message: Option<String>,
    /// The edits, in order.
    pub edits: Vec<GraphEdit>,
}

/// The commit a batch published.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphEditReport {
    /// Commit holding the batch.
    pub commit_id: GraphCommitId,
    /// Branch written.
    pub branch: String,
    /// Ids of the created nodes, in the order of the create edits.
    pub created: Vec<String>,
    /// Ids of the created relationships, in the order of their create edits.
    pub created_relationships: Vec<String>,
    /// Number of records the commit changed, created ones included.
    pub changed: usize,
}

#[derive(Default)]
struct NodeChange {
    patch: Option<NodePatch>,
    delete: bool,
}

#[derive(Default)]
struct RelationshipChange {
    patch: Option<RelationshipPatch>,
    delete: bool,
}

/// A node a new relationship points at.
#[derive(Clone)]
enum Endpoint {
    Existing(String),
    Created(usize),
}

struct NewRelationship {
    source: Endpoint,
    target: Endpoint,
    rel_type: String,
    properties: BTreeMap<String, PropertyValue>,
}

fn invalid(index: usize, reason: impl Into<String>) -> GraphStorageError {
    GraphStorageError::GraphEditInvalid {
        index,
        reason: reason.into(),
    }
}

fn check_key(index: usize, key: &str) -> GraphStorageResult<()> {
    if key.trim().is_empty() {
        return Err(invalid(index, "property keys must not be blank"));
    }
    Ok(())
}

fn endpoint(index: usize, value: &str, created_so_far: usize) -> GraphStorageResult<Endpoint> {
    match value.strip_prefix('$') {
        Some(position) => {
            let position: usize = position
                .parse()
                .map_err(|_| invalid(index, format!("{value} is not a $n reference")))?;
            if position >= created_so_far {
                return Err(invalid(
                    index,
                    format!("{value} names no create_node before this edit"),
                ));
            }
            Ok(Endpoint::Created(position))
        }
        None if value.trim().is_empty() => Err(invalid(index, "endpoint ids must not be blank")),
        None => Ok(Endpoint::Existing(value.to_owned())),
    }
}

impl CanonicalEngineStore {
    /// Apply a batch of edits to a branch head as one commit.
    ///
    /// # Errors
    ///
    /// Returns [`GraphStorageError::GraphEditInvalid`] for an empty or
    /// oversized batch, blank keys or types, invalid labels, a record both
    /// edited and deleted, or an endpoint deleted in the same batch;
    /// [`GraphStorageError::GraphEditNodeNotFound`] and
    /// [`GraphStorageError::GraphEditRelationshipNotFound`];
    /// [`GraphStorageError::GraphEditBlocked`] when a deleted node keeps live
    /// relationships; [`GraphStorageError::UnknownGraphBranch`]; and
    /// [`GraphStorageError::GraphHeadMoved`] when `expected_head` is stale.
    /// Nothing is written when any edit fails.
    #[allow(clippy::too_many_lines)]
    pub fn apply_graph_edits(
        &mut self,
        request: GraphEditRequest,
    ) -> GraphStorageResult<GraphEditReport> {
        if request.edits.is_empty() || request.edits.len() > MAX_GRAPH_EDITS {
            return Err(invalid(
                0,
                format!("a batch holds 1 to {MAX_GRAPH_EDITS} edits"),
            ));
        }
        let branch = request.branch.clone();
        self.reconcile_history()?;
        let head = self.history.head(&branch)?.clone();
        if let Some(expected) = &request.expected_head
            && expected != &head
        {
            return Err(GraphStorageError::GraphHeadMoved {
                branch,
                expected: expected.to_string(),
                actual: head.to_string(),
            });
        }
        let view = self.commit_view(&head)?;

        // Validate the whole batch and fold it per record before writing.
        let mut node_changes: BTreeMap<String, NodeChange> = BTreeMap::new();
        let mut relationship_changes: BTreeMap<String, RelationshipChange> = BTreeMap::new();
        let mut node_creations: Vec<NodeInput> = Vec::new();
        let mut relationship_creations: Vec<NewRelationship> = Vec::new();
        for (index, edit) in request.edits.iter().enumerate() {
            match edit {
                GraphEdit::SetProperties {
                    node_id,
                    set,
                    remove,
                } => {
                    if set.is_empty() && remove.is_empty() {
                        return Err(invalid(index, "set_properties changes nothing"));
                    }
                    let change = node_changes.entry(node_id.clone()).or_default();
                    if change.delete {
                        return Err(invalid(index, format!("node {node_id} is also deleted")));
                    }
                    let mut patch = change.patch.take().unwrap_or_default();
                    for (key, value) in set {
                        check_key(index, key)?;
                        patch = patch.set_property(key.clone(), value.clone());
                    }
                    for key in remove {
                        check_key(index, key)?;
                        patch = patch.remove_property(key.clone());
                    }
                    change.patch = Some(patch);
                }
                GraphEdit::CreateNode { labels, properties } => {
                    let mut input = NodeInput::new(labels.iter().cloned());
                    for (key, value) in properties {
                        check_key(index, key)?;
                        input = input.with_property(key.clone(), value.clone());
                    }
                    input
                        .validate()
                        .map_err(|error| invalid(index, error.to_string()))?;
                    node_creations.push(input);
                }
                GraphEdit::DeleteNode { node_id } => {
                    let change = node_changes.entry(node_id.clone()).or_default();
                    if change.patch.is_some() {
                        return Err(invalid(index, format!("node {node_id} is also edited")));
                    }
                    change.delete = true;
                }
                GraphEdit::CreateRelationship {
                    source_id,
                    target_id,
                    rel_type,
                    properties,
                } => {
                    if rel_type.trim().is_empty() {
                        return Err(invalid(index, "relationship types must not be blank"));
                    }
                    for key in properties.keys() {
                        check_key(index, key)?;
                    }
                    relationship_creations.push(NewRelationship {
                        source: endpoint(index, source_id, node_creations.len())?,
                        target: endpoint(index, target_id, node_creations.len())?,
                        rel_type: rel_type.clone(),
                        properties: properties.clone(),
                    });
                }
                GraphEdit::SetRelationshipProperties {
                    relationship_id,
                    set,
                    remove,
                } => {
                    if set.is_empty() && remove.is_empty() {
                        return Err(invalid(
                            index,
                            "set_relationship_properties changes nothing",
                        ));
                    }
                    let change = relationship_changes
                        .entry(relationship_id.clone())
                        .or_default();
                    if change.delete {
                        return Err(invalid(
                            index,
                            format!("relationship {relationship_id} is also deleted"),
                        ));
                    }
                    let mut patch = change.patch.take().unwrap_or_default();
                    for (key, value) in set {
                        check_key(index, key)?;
                        patch = patch.set_property(key.clone(), value.clone());
                    }
                    for key in remove {
                        check_key(index, key)?;
                        patch = patch.remove_property(key.clone());
                    }
                    change.patch = Some(patch);
                }
                GraphEdit::DeleteRelationship { relationship_id } => {
                    let change = relationship_changes
                        .entry(relationship_id.clone())
                        .or_default();
                    if change.patch.is_some() {
                        return Err(invalid(
                            index,
                            format!("relationship {relationship_id} is also edited"),
                        ));
                    }
                    change.delete = true;
                }
            }
        }

        // Every named record must be live at the head.
        let live_node = |id: &str| {
            view.entries(HistoryRecordKind::Node)
                .get(id)
                .filter(|entry| !entry.deleted)
        };
        let mut touched_relationships: Vec<Relationship> = Vec::new();
        let mut endpoint_ids: BTreeSet<String> = BTreeSet::new();
        for relationship_id in relationship_changes.keys() {
            let entry = view
                .entries(HistoryRecordKind::Relationship)
                .get(relationship_id)
                .filter(|entry| !entry.deleted)
                .ok_or_else(|| GraphStorageError::GraphEditRelationshipNotFound {
                    branch: branch.clone(),
                    relationship_id: relationship_id.clone(),
                })?;
            endpoint_ids.extend(entry.source.iter().cloned());
            endpoint_ids.extend(entry.target.iter().cloned());
            touched_relationships.push(self.decode_relationship(entry)?);
        }
        for (position, creation) in relationship_creations.iter().enumerate() {
            for end in [&creation.source, &creation.target] {
                if let Endpoint::Existing(id) = end {
                    if live_node(id).is_none() {
                        return Err(GraphStorageError::GraphEditNodeNotFound {
                            branch: branch.clone(),
                            node_id: id.clone(),
                        });
                    }
                    if node_changes.get(id).is_some_and(|change| change.delete) {
                        return Err(invalid(
                            position,
                            format!("relationship endpoint {id} is deleted in this batch"),
                        ));
                    }
                    endpoint_ids.insert(id.clone());
                }
            }
        }
        let deleted_relationships: BTreeSet<&String> = relationship_changes
            .iter()
            .filter(|(_, change)| change.delete)
            .map(|(id, _)| id)
            .collect();
        let mut touched_nodes: BTreeMap<String, Node> = BTreeMap::new();
        for (node_id, change) in &node_changes {
            let entry =
                live_node(node_id).ok_or_else(|| GraphStorageError::GraphEditNodeNotFound {
                    branch: branch.clone(),
                    node_id: node_id.clone(),
                })?;
            if change.delete {
                let attached: Vec<&String> = view
                    .relationships
                    .iter()
                    .filter(|(id, relationship)| {
                        !relationship.deleted
                            && !deleted_relationships.contains(id)
                            && (relationship.source.as_deref() == Some(node_id.as_str())
                                || relationship.target.as_deref() == Some(node_id.as_str()))
                    })
                    .map(|(id, _)| id)
                    .collect();
                if !attached.is_empty() {
                    let shown: Vec<&str> = attached.iter().take(5).map(|id| id.as_str()).collect();
                    return Err(GraphStorageError::GraphEditBlocked {
                        branch,
                        node_id: node_id.clone(),
                        reason: format!(
                            "{} live relationship(s) still use it ({}); delete them first or in the same batch",
                            attached.len(),
                            shown.join(", ")
                        ),
                    });
                }
            }
            touched_nodes.insert(node_id.clone(), self.decode_node(entry)?);
        }
        for id in &endpoint_ids {
            if !touched_nodes.contains_key(id)
                && let Some(entry) = live_node(id)
            {
                touched_nodes.insert(id.clone(), self.decode_node(entry)?);
            }
        }

        let previous = self.graph_from_records(
            touched_nodes.values().cloned().collect(),
            touched_relationships.clone(),
        )?;
        let mut current = previous.clone();
        let edit_error = |error: graph_core::GraphError| invalid(0, error.to_string());
        for (relationship_id, change) in relationship_changes {
            let id = RelationshipId::new(relationship_id).map_err(edit_error)?;
            if change.delete {
                current.tombstone_relationship(&id).map_err(edit_error)?;
            } else if let Some(patch) = change.patch {
                current
                    .update_relationship(&id, patch)
                    .map_err(edit_error)?;
            }
        }
        for (node_id, change) in node_changes {
            let id = NodeId::new(node_id).map_err(edit_error)?;
            if change.delete {
                current.tombstone_node(&id).map_err(edit_error)?;
            } else if let Some(patch) = change.patch {
                current.update_node(&id, patch).map_err(edit_error)?;
            }
        }
        let mut created: Vec<NodeId> = Vec::with_capacity(node_creations.len());
        for input in node_creations {
            created.push(current.create_node(input).map_err(edit_error)?);
        }
        let resolve = |end: &Endpoint| -> GraphStorageResult<NodeId> {
            match end {
                Endpoint::Existing(id) => NodeId::new(id.clone()).map_err(edit_error),
                Endpoint::Created(position) => Ok(created[*position].clone()),
            }
        };
        let mut created_relationships = Vec::with_capacity(relationship_creations.len());
        for creation in &relationship_creations {
            let mut input = RelationshipInput::new(
                resolve(&creation.source)?,
                creation.rel_type.clone(),
                resolve(&creation.target)?,
            )
            .map_err(edit_error)?;
            for (key, value) in &creation.properties {
                input = input.with_property(key.clone(), value.clone());
            }
            created_relationships.push(
                current
                    .create_relationship(input)
                    .map_err(edit_error)?
                    .as_str()
                    .to_owned(),
            );
        }
        let changed = node_changes_len(&request.edits)
            + touched_relationships.len()
            + created.len()
            + created_relationships.len();
        let context = TransitionCommitContext {
            actor: request.actor.clone(),
            message: request.message.clone(),
            expected_head: Some(head.clone()),
            ..TransitionCommitContext::default()
        };

        let commit_id = if branch == MAIN_BRANCH {
            let fingerprint = serde_json::to_vec(&(&head, &request.edits)).map_err(|error| {
                GraphStorageError::OperationFailed {
                    operation: "apply_graph_edits",
                    message: error.to_string(),
                }
            })?;
            let transaction_id = DurableTransactionId::new(format!(
                "tx--graph-edits-{}",
                &hex_sha256(&fingerprint)[..40]
            ))
            .map_err(|error| GraphStorageError::OperationFailed {
                operation: "apply_graph_edits",
                message: error.to_string(),
            })?;
            self.commit_transition_with_context(
                &previous,
                &current,
                transaction_id,
                context,
                None,
            )?;
            let published = self.graph_head(MAIN_BRANCH)?;
            if published == head {
                return Err(GraphStorageError::OperationFailed {
                    operation: "apply_graph_edits",
                    message: "the edit transaction did not publish a main commit".to_owned(),
                });
            }
            published
        } else {
            self.commit_branch_transition(&branch, &previous, &current, context, Some(&head))?
                .ok_or_else(|| invalid(0, "the batch changes nothing"))?
                .commit_id
                .clone()
        };
        Ok(GraphEditReport {
            commit_id,
            branch,
            created: created.iter().map(|id| id.as_str().to_owned()).collect(),
            created_relationships,
            changed,
        })
    }
}

/// Distinct nodes the batch edits or deletes.
fn node_changes_len(edits: &[GraphEdit]) -> usize {
    edits
        .iter()
        .filter_map(|edit| match edit {
            GraphEdit::SetProperties { node_id, .. } | GraphEdit::DeleteNode { node_id } => {
                Some(node_id.as_str())
            }
            _ => None,
        })
        .collect::<BTreeSet<_>>()
        .len()
}

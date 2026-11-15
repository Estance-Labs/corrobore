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

//! Batches of record edits published as one commit (issue #352).
//!
//! A viewer collects drafts (property changes, new nodes, deletions) and
//! publishes them together. Every edit applies to the head of one branch; the
//! batch is validated in full before anything is written, then published as a
//! single commit: through the canonical WAL on `main`, through branch payloads
//! elsewhere. Several property edits of one node merge into one new version.

use std::collections::{BTreeMap, BTreeSet};

use graph_core::{Node, NodeId, NodeInput, NodePatch, PropertyValue};

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
    /// Tombstone a live node without live relationships.
    DeleteNode {
        /// Node to delete.
        node_id: String,
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
    /// Number of nodes the commit changed, created ones included.
    pub changed: usize,
}

#[derive(Default)]
struct NodeChange {
    patch: Option<NodePatch>,
    delete: bool,
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

impl CanonicalEngineStore {
    /// Apply a batch of edits to a branch head as one commit.
    ///
    /// # Errors
    ///
    /// Returns [`GraphStorageError::GraphEditInvalid`] for an empty or
    /// oversized batch, blank keys, invalid labels or a node both edited and
    /// deleted; [`GraphStorageError::GraphEditNodeNotFound`];
    /// [`GraphStorageError::GraphEditBlocked`] when a deleted node still has
    /// live relationships; [`GraphStorageError::UnknownGraphBranch`]; and
    /// [`GraphStorageError::GraphHeadMoved`] when `expected_head` is stale.
    /// Nothing is written when any edit fails.
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

        // Validate the whole batch and fold it per node before writing.
        let mut changes: BTreeMap<String, NodeChange> = BTreeMap::new();
        let mut creations: Vec<NodeInput> = Vec::new();
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
                    let change = changes.entry(node_id.clone()).or_default();
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
                    creations.push(input);
                }
                GraphEdit::DeleteNode { node_id } => {
                    let change = changes.entry(node_id.clone()).or_default();
                    if change.patch.is_some() {
                        return Err(invalid(index, format!("node {node_id} is also edited")));
                    }
                    change.delete = true;
                }
            }
        }
        let mut touched: Vec<Node> = Vec::new();
        for (node_id, change) in &changes {
            let entry = view
                .entries(HistoryRecordKind::Node)
                .get(node_id)
                .filter(|entry| !entry.deleted)
                .ok_or_else(|| GraphStorageError::GraphEditNodeNotFound {
                    branch: branch.clone(),
                    node_id: node_id.clone(),
                })?;
            if change.delete {
                let attached: BTreeSet<&String> = view
                    .relationships
                    .iter()
                    .filter(|(_, relationship)| {
                        !relationship.deleted
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
                            "{} live relationship(s) still use it ({}); delete them first",
                            attached.len(),
                            shown.join(", ")
                        ),
                    });
                }
            }
            touched.push(self.decode_node(entry)?);
        }

        let previous = self.graph_from_records(touched.clone(), Vec::new())?;
        let mut current = previous.clone();
        let edit_error = |error: graph_core::GraphError| invalid(0, error.to_string());
        for (node_id, change) in changes {
            let id = NodeId::new(node_id).map_err(edit_error)?;
            if change.delete {
                current.tombstone_node(&id).map_err(edit_error)?;
            } else if let Some(patch) = change.patch {
                current.update_node(&id, patch).map_err(edit_error)?;
            }
        }
        let mut created = Vec::with_capacity(creations.len());
        for input in creations {
            created.push(
                current
                    .create_node(input)
                    .map_err(edit_error)?
                    .as_str()
                    .to_owned(),
            );
        }
        let changed = touched.len() + created.len();
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
            created,
            changed,
        })
    }
}

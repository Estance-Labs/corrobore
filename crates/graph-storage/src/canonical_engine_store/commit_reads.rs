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

//! Commit-pinned reads (issue #318).
//!
//! A commit view names the record versions visible at a commit; these reads
//! page exactly those versions from the payload logs, so a read at an old
//! commit keeps returning the old versions after later writes and paging stays
//! stable while new commits land.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;

use graph_core::{Graph, Node, NodeId, Relationship};

use super::{CanonicalEngineStore, CanonicalProjectionRequest, graph_error};
use crate::graph_history::{
    CommitView, CommitViewEntry, GraphCommitId, GraphHistory, HistoryRecordKind, MAIN_BRANCH,
};
use crate::{GraphStorageError, GraphStorageResult};

/// Direction of a relationship group around a node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NeighbourDirection {
    /// The node is the relationship source.
    Outgoing,
    /// The node is the relationship target.
    Incoming,
}

/// One page of records of a commit view, in record-id order.
#[derive(Clone, Debug)]
pub struct CommitRecordPage {
    /// Commit the page was read at.
    pub commit_id: GraphCommitId,
    /// Nodes of the page (for `HistoryRecordKind::Node`).
    pub nodes: Vec<Node>,
    /// Relationships of the page (for `HistoryRecordKind::Relationship`).
    pub relationships: Vec<Relationship>,
    /// Record id to pass as `cursor` for the next page.
    pub next_cursor: Option<String>,
    /// Number of records matching the filter at this commit.
    pub total: usize,
}

/// A bounded window of a commit view for visual projections.
#[derive(Clone, Debug)]
pub struct CommitWindow {
    /// Commit the window was read at.
    pub commit_id: GraphCommitId,
    /// Graph holding the selected live records.
    pub graph: Graph,
    /// Live nodes at the commit.
    pub total_nodes: usize,
    /// Live relationships at the commit.
    pub total_relationships: usize,
}

/// Relationships of one type and direction around a node.
#[derive(Clone, Debug)]
pub struct NeighbourGroup {
    /// Relationship type.
    pub relationship_type: String,
    /// Direction relative to the node.
    pub direction: NeighbourDirection,
    /// Live relationships in the group.
    pub count: usize,
    /// First relationships of the group with their neighbour, in id order.
    pub items: Vec<(Relationship, Node)>,
}

/// A node with its relationship groups at a commit.
#[derive(Clone, Debug)]
pub struct NodeNeighbourhood {
    /// Commit the node was read at.
    pub commit_id: GraphCommitId,
    /// The node.
    pub node: Node,
    /// Relationship groups ordered by type then direction.
    pub groups: Vec<NeighbourGroup>,
}

impl CanonicalEngineStore {
    /// Resolve a read target: an explicit commit wins over a branch head;
    /// neither means the head of `main`.
    pub fn resolve_graph_ref(
        &mut self,
        branch: Option<&str>,
        commit: Option<&GraphCommitId>,
    ) -> GraphStorageResult<GraphCommitId> {
        self.reconcile_history()?;
        match commit {
            Some(commit) => {
                self.history.commit(commit)?;
                Ok(commit.clone())
            }
            None => self.history.head(branch.unwrap_or(MAIN_BRANCH)).cloned(),
        }
    }

    /// The `main` commit that recorded a canonical transaction, if retained.
    pub fn commit_for_transaction(
        &mut self,
        transaction_id: &str,
    ) -> GraphStorageResult<Option<GraphCommitId>> {
        self.reconcile_history()?;
        Ok(self
            .history
            .commits_in_append_order()
            .filter(|commit| commit.transaction_id.as_deref() == Some(transaction_id))
            .map(|commit| commit.commit_id.clone())
            .last())
    }

    /// The newest first-parent commit of `branch` created at or before `at`
    /// (RFC 3339). Returns `None` when `at` predates the history.
    pub fn commit_at(
        &mut self,
        branch: &str,
        at: &str,
    ) -> GraphStorageResult<Option<GraphCommitId>> {
        let at = parse_time(at)?;
        self.reconcile_history()?;
        let head = self.history.head(branch)?.clone();
        let mut cursor = None;
        loop {
            let page = self
                .history
                .first_parent_page(&head, cursor.as_ref(), 256)?;
            for commit in &page.commits {
                if parse_time(&commit.created_at)? <= at {
                    return Ok(Some(commit.commit_id.clone()));
                }
            }
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => return Ok(None),
            }
        }
    }

    /// Build the graph a Cypher read needs at a commit.
    ///
    /// The selection is a superset of what the request asks for: label and
    /// relationship-type keys narrow it, identifiers and property predicates
    /// are evaluated by the executor afterwards. Tombstoned records are not
    /// visible. Hot-record budgets apply exactly as for `main` projections.
    pub fn load_commit_projection(
        &mut self,
        commit_id: &GraphCommitId,
        request: &CanonicalProjectionRequest,
    ) -> GraphStorageResult<Graph> {
        let view = self.commit_view(commit_id)?;
        let mut node_ids: BTreeSet<String> = BTreeSet::new();
        if request.include_nodes || request.adjacency.is_some() {
            node_ids.extend(
                live(&view.nodes)
                    .filter(|(_, entry)| {
                        request
                            .node_label
                            .as_ref()
                            .is_none_or(|label| entry.labels.iter().any(|value| value == label))
                    })
                    .map(|(id, _)| id.clone()),
            );
        }
        let mut relationship_ids: BTreeSet<String> = BTreeSet::new();
        if let Some(adjacency) = &request.adjacency {
            let depth = adjacency.max_depth.max(1);
            let mut frontier: HashSet<String> = node_ids.iter().cloned().collect();
            for _ in 0..depth {
                let mut next = HashSet::new();
                for (id, entry) in live(&view.relationships) {
                    if !adjacency.relationship_types.is_empty()
                        && !adjacency
                            .relationship_types
                            .iter()
                            .any(|kind| entry.labels.first() == Some(kind))
                    {
                        continue;
                    }
                    let (Some(source), Some(target)) = (&entry.source, &entry.target) else {
                        continue;
                    };
                    let outgoing = adjacency.outgoing && frontier.contains(source);
                    let incoming = adjacency.incoming && frontier.contains(target);
                    if (outgoing || incoming) && relationship_ids.insert(id.clone()) {
                        next.insert(source.clone());
                        next.insert(target.clone());
                    }
                }
                if next.is_empty() {
                    break;
                }
                frontier = next;
            }
        } else if request.include_relationships {
            relationship_ids.extend(
                live(&view.relationships)
                    .filter(|(_, entry)| {
                        request
                            .relationship_type
                            .as_ref()
                            .is_none_or(|kind| entry.labels.first() == Some(kind))
                    })
                    .filter(|(_, entry)| {
                        request.node_label.is_none()
                            || entry
                                .source
                                .as_ref()
                                .is_some_and(|id| node_ids.contains(id))
                            || entry
                                .target
                                .as_ref()
                                .is_some_and(|id| node_ids.contains(id))
                    })
                    .map(|(id, _)| id.clone()),
            );
        }
        self.enforce_relationship_budget(relationship_ids.len())?;
        let relationships = self.decode_relationships(&view, relationship_ids.iter())?;
        for relationship in &relationships {
            node_ids.insert(relationship.source().as_str().to_owned());
            node_ids.insert(relationship.target().as_str().to_owned());
        }
        self.enforce_node_budget(node_ids.len())?;
        let nodes = self.decode_nodes(&view, node_ids.iter())?;
        self.graph_from_records(nodes, relationships)
    }

    /// Up to `max_nodes` live nodes (in id order) and the live relationships
    /// between them, up to `max_relationships`, with the commit's totals.
    pub fn load_commit_window(
        &mut self,
        commit_id: &GraphCommitId,
        max_nodes: usize,
        max_relationships: usize,
    ) -> GraphStorageResult<CommitWindow> {
        let view = self.commit_view(commit_id)?;
        let total_nodes = view.live_count(HistoryRecordKind::Node);
        let total_relationships = view.live_count(HistoryRecordKind::Relationship);
        let node_ids: BTreeSet<String> = live(&view.nodes)
            .take(max_nodes)
            .map(|(id, _)| id.clone())
            .collect();
        let relationship_ids: Vec<String> = live(&view.relationships)
            .filter(|(_, entry)| {
                entry
                    .source
                    .as_ref()
                    .is_some_and(|id| node_ids.contains(id))
                    && entry
                        .target
                        .as_ref()
                        .is_some_and(|id| node_ids.contains(id))
            })
            .take(max_relationships)
            .map(|(id, _)| id.clone())
            .collect();
        let nodes = self.decode_nodes(&view, node_ids.iter())?;
        let relationships = self.decode_relationships(&view, relationship_ids.iter())?;
        Ok(CommitWindow {
            commit_id: commit_id.clone(),
            graph: self.graph_from_records(nodes, relationships)?,
            total_nodes,
            total_relationships,
        })
    }

    /// One keyset page of records at a commit.
    ///
    /// `label` filters node labels or the relationship type. `cursor` is the
    /// last record id of the previous page.
    #[allow(clippy::too_many_arguments)]
    pub fn list_commit_records(
        &mut self,
        commit_id: &GraphCommitId,
        kind: HistoryRecordKind,
        label: Option<&str>,
        cursor: Option<&str>,
        limit: usize,
        include_deleted: bool,
    ) -> GraphStorageResult<CommitRecordPage> {
        let view = self.commit_view(commit_id)?;
        let limit = limit.max(1);
        let matches = |entry: &CommitViewEntry| {
            (include_deleted || !entry.deleted)
                && label.is_none_or(|label| entry.labels.iter().any(|value| value == label))
        };
        let entries = view.entries(kind);
        let total = entries.values().filter(|entry| matches(entry)).count();
        let range = match cursor {
            Some(cursor) => entries.range::<str, _>((
                std::ops::Bound::Excluded(cursor),
                std::ops::Bound::Unbounded,
            )),
            None => entries.range::<str, _>(..),
        };
        let mut ids = Vec::with_capacity(limit);
        let mut next_cursor = None;
        for (id, entry) in range {
            if !matches(entry) {
                continue;
            }
            if ids.len() == limit {
                next_cursor = ids.last().cloned();
                break;
            }
            ids.push(id.clone());
        }
        let (nodes, relationships) = match kind {
            HistoryRecordKind::Node => (self.decode_nodes(&view, ids.iter())?, Vec::new()),
            HistoryRecordKind::Relationship => {
                (Vec::new(), self.decode_relationships(&view, ids.iter())?)
            }
        };
        Ok(CommitRecordPage {
            commit_id: commit_id.clone(),
            nodes,
            relationships,
            next_cursor,
            total,
        })
    }

    /// A node and its relationship groups at a commit; `None` when the node
    /// is not visible there.
    pub fn commit_node(
        &mut self,
        commit_id: &GraphCommitId,
        node_id: &str,
        items_per_group: usize,
    ) -> GraphStorageResult<Option<NodeNeighbourhood>> {
        let view = self.commit_view(commit_id)?;
        let Some(entry) = view.nodes.get(node_id).filter(|entry| !entry.deleted) else {
            return Ok(None);
        };
        let node = self.decode_node(entry)?;
        let mut groups: BTreeMap<(String, NeighbourDirection), Vec<String>> = BTreeMap::new();
        for (id, entry) in live(&view.relationships) {
            let kind = entry.labels.first().cloned().unwrap_or_default();
            if entry.source.as_deref() == Some(node_id) {
                groups
                    .entry((kind.clone(), NeighbourDirection::Outgoing))
                    .or_default()
                    .push(id.clone());
            }
            if entry.target.as_deref() == Some(node_id) {
                groups
                    .entry((kind, NeighbourDirection::Incoming))
                    .or_default()
                    .push(id.clone());
            }
        }
        let mut result = Vec::with_capacity(groups.len());
        for ((relationship_type, direction), ids) in groups {
            let mut items = Vec::new();
            for id in ids.iter().take(items_per_group) {
                let relationship = self.decode_relationship(&view.relationships[id])?;
                let neighbour_id = match direction {
                    NeighbourDirection::Outgoing => relationship.target().as_str().to_owned(),
                    NeighbourDirection::Incoming => relationship.source().as_str().to_owned(),
                };
                let Some(neighbour) = view.nodes.get(&neighbour_id) else {
                    continue;
                };
                items.push((relationship, self.decode_node(neighbour)?));
            }
            result.push(NeighbourGroup {
                relationship_type,
                direction,
                count: ids.len(),
                items,
            });
        }
        Ok(Some(NodeNeighbourhood {
            commit_id: commit_id.clone(),
            node,
            groups: result,
        }))
    }

    pub(crate) fn commit_view(
        &mut self,
        commit_id: &GraphCommitId,
    ) -> GraphStorageResult<Arc<CommitView>> {
        self.reconcile_history()?;
        self.history.view(commit_id)
    }

    pub(crate) fn decode_node(&self, entry: &CommitViewEntry) -> GraphStorageResult<Node> {
        let bytes =
            GraphHistory::read_payload(&self.root, HistoryRecordKind::Node, &entry.payload)?;
        let node: Node = serde_json::from_slice(&bytes).map_err(graph_error)?;
        if node.version_id().as_str() != entry.version_id {
            return Err(GraphStorageError::OperationFailed {
                operation: "decode_commit_node",
                message: format!(
                    "payload holds version {} but the commit view expects {}",
                    node.version_id().as_str(),
                    entry.version_id
                ),
            });
        }
        Ok(node)
    }

    pub(crate) fn decode_relationship(
        &self,
        entry: &CommitViewEntry,
    ) -> GraphStorageResult<Relationship> {
        let bytes = GraphHistory::read_payload(
            &self.root,
            HistoryRecordKind::Relationship,
            &entry.payload,
        )?;
        let relationship: Relationship = serde_json::from_slice(&bytes).map_err(graph_error)?;
        if relationship.version_id().as_str() != entry.version_id {
            return Err(GraphStorageError::OperationFailed {
                operation: "decode_commit_relationship",
                message: format!(
                    "payload holds version {} but the commit view expects {}",
                    relationship.version_id().as_str(),
                    entry.version_id
                ),
            });
        }
        Ok(relationship)
    }

    fn decode_nodes<'a>(
        &self,
        view: &CommitView,
        ids: impl Iterator<Item = &'a String>,
    ) -> GraphStorageResult<Vec<Node>> {
        ids.filter_map(|id| view.nodes.get(id))
            .map(|entry| self.decode_node(entry))
            .collect()
    }

    fn decode_relationships<'a>(
        &self,
        view: &CommitView,
        ids: impl Iterator<Item = &'a String>,
    ) -> GraphStorageResult<Vec<Relationship>> {
        ids.filter_map(|id| view.relationships.get(id))
            .map(|entry| self.decode_relationship(entry))
            .collect()
    }

    pub(crate) fn graph_from_records(
        &self,
        nodes: Vec<Node>,
        relationships: Vec<Relationship>,
    ) -> GraphStorageResult<Graph> {
        let present: HashSet<&NodeId> = nodes.iter().map(Node::id).collect();
        let relationships = relationships
            .into_iter()
            .filter(|relationship| {
                present.contains(relationship.source()) && present.contains(relationship.target())
            })
            .collect();
        Graph::from_current_records(nodes, relationships, self.sequence_floor())
            .map_err(graph_error)
    }
}

fn live(
    entries: &BTreeMap<String, CommitViewEntry>,
) -> impl Iterator<Item = (&String, &CommitViewEntry)> {
    entries.iter().filter(|(_, entry)| !entry.deleted)
}

fn parse_time(value: &str) -> GraphStorageResult<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|time| time.with_timezone(&chrono::Utc))
        .map_err(|error| GraphStorageError::OperationFailed {
            operation: "parse_commit_time",
            message: format!("{value:?} is not an RFC 3339 time: {error}"),
        })
}

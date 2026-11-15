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

//! Persisted branches with isolated writes (issue #319).
//!
//! A branch write appends its record versions to the branch payload log,
//! which the canonical catalog never indexes, then publishes a branch commit.
//! `main` keeps going through the canonical WAL, so recovery, indexes and
//! metrics of `main` do not change. Merging into `main` (issue #321) replays
//! the merged versions through that WAL.

use std::collections::HashMap;
use std::sync::Arc;

use graph_core::{Graph, Node, NodeId, RecordStatus, Relationship, RelationshipId};

use super::{CanonicalEngineStore, TransitionCommitContext, graph_error};
use crate::graph_history::{
    CommitPayloadRef, CommitRecordChange, GraphBranchInfo, GraphCommit, GraphCommitDraft,
    GraphCommitId, HistoryRecordKind, MAIN_BRANCH,
};
use crate::{GraphStorageError, GraphStorageResult};

/// A branch with its merge state relative to `main`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphBranchStatus {
    /// Branch name, head and origin.
    pub info: GraphBranchInfo,
    /// Whether the branch head is reachable from the `main` head.
    pub merged: bool,
    /// Commits on the branch's first-parent chain since it was created.
    pub commits_ahead: usize,
}

impl CanonicalEngineStore {
    /// All branches, `main` first, then by name.
    pub fn list_graph_branches(&mut self) -> GraphStorageResult<Vec<GraphBranchStatus>> {
        self.reconcile_history()?;
        let main_head = self.history.head(MAIN_BRANCH)?.clone();
        let infos: Vec<GraphBranchInfo> = self.history.branches().cloned().collect();
        let mut statuses = Vec::with_capacity(infos.len());
        for info in infos {
            let merged =
                info.name == MAIN_BRANCH || self.history.is_ancestor(&info.head, &main_head)?;
            let commits_ahead = if info.name == MAIN_BRANCH {
                0
            } else {
                self.commits_since(&info.head, &info.created_from)?
            };
            statuses.push(GraphBranchStatus {
                info,
                merged,
                commits_ahead,
            });
        }
        statuses.sort_by(|left, right| {
            (left.info.name != MAIN_BRANCH, &left.info.name)
                .cmp(&(right.info.name != MAIN_BRANCH, &right.info.name))
        });
        Ok(statuses)
    }

    /// Create `name` at an explicit commit, else at the head of `from_branch`
    /// (default `main`).
    pub fn create_graph_branch(
        &mut self,
        name: &str,
        from_branch: Option<&str>,
        from_commit: Option<&GraphCommitId>,
        created_by: Option<String>,
    ) -> GraphStorageResult<GraphBranchInfo> {
        let from = self.resolve_graph_ref(from_branch, from_commit)?;
        self.history.create_branch(name, &from, created_by)
    }

    /// Delete a branch. An unmerged branch is refused unless `force` is set.
    pub fn delete_graph_branch(
        &mut self,
        name: &str,
        force: bool,
        deleted_by: Option<String>,
    ) -> GraphStorageResult<()> {
        self.reconcile_history()?;
        if name != MAIN_BRANCH && !force {
            let head = self.history.head(name)?.clone();
            let main_head = self.history.head(MAIN_BRANCH)?.clone();
            if !self.history.is_ancestor(&head, &main_head)? {
                return Err(GraphStorageError::GraphBranchConflict {
                    branch: name.to_owned(),
                    reason: "the branch has commits that main does not contain; merge it or delete with force".to_owned(),
                });
            }
        }
        self.history.delete_branch(name, deleted_by)
    }

    /// Publish a graph transition on a branch other than `main`.
    ///
    /// `previous` must be the projection the transition started from (read at
    /// the branch head); only records whose version changed are published.
    /// When `expected_head` is given and the branch moved since, the write is
    /// refused with [`GraphStorageError::GraphHeadMoved`] instead of silently
    /// overwriting the newer commit.
    pub fn commit_branch_transition(
        &mut self,
        branch: &str,
        previous: &Graph,
        current: &Graph,
        context: TransitionCommitContext,
        expected_head: Option<&GraphCommitId>,
    ) -> GraphStorageResult<Option<Arc<GraphCommit>>> {
        if branch == MAIN_BRANCH {
            return Err(GraphStorageError::GraphBranchConflict {
                branch: branch.to_owned(),
                reason: "main is written through the canonical WAL".to_owned(),
            });
        }
        self.reconcile_history()?;
        let head = self.history.head(branch)?.clone();
        if let Some(expected) = expected_head
            && expected != &head
        {
            return Err(GraphStorageError::GraphHeadMoved {
                branch: branch.to_owned(),
                expected: expected.to_string(),
                actual: head.to_string(),
            });
        }
        let (nodes, relationships) = changed_records(previous, current)?;
        if nodes.is_empty() && relationships.is_empty() {
            return Ok(None);
        }
        let mut payloads = Vec::with_capacity(nodes.len() + relationships.len());
        for node in &nodes {
            payloads.push(encode_payload_bytes(node)?);
        }
        for relationship in &relationships {
            payloads.push(encode_payload_bytes(relationship)?);
        }
        let references = self.history.append_branch_payloads(&payloads)?;
        let mut references = references.into_iter();
        let mut changes = Vec::with_capacity(payloads.len());
        for node in &nodes {
            let payload = references.next().ok_or_else(missing_reference)?;
            changes.push(branch_node_change(node, payload));
        }
        for relationship in &relationships {
            let payload = references.next().ok_or_else(missing_reference)?;
            changes.push(branch_relationship_change(relationship, payload));
        }
        let commit = self.history.record_commit(GraphCommitDraft {
            branch: branch.to_owned(),
            extra_parents: Vec::new(),
            transaction_id: None,
            mutation_sequence_number: None,
            actor: context.actor,
            message: context.message,
            created_at: None,
            changes,
        })?;
        Ok(Some(commit))
    }

    fn commits_since(
        &self,
        head: &GraphCommitId,
        origin: &GraphCommitId,
    ) -> GraphStorageResult<usize> {
        let mut count = 0;
        let mut current = Some(self.history.commit(head)?);
        while let Some(commit) = current {
            if &commit.commit_id == origin {
                break;
            }
            count += 1;
            current = commit
                .parents
                .first()
                .map(|parent| self.history.commit(parent))
                .transpose()?;
        }
        Ok(count)
    }
}

/// Current records of `current` whose version differs from `previous`.
pub(crate) fn changed_records(
    previous: &Graph,
    current: &Graph,
) -> GraphStorageResult<(Vec<Node>, Vec<Relationship>)> {
    let previous_nodes: HashMap<NodeId, Node> = previous
        .current_node_records()
        .map_err(graph_error)?
        .into_iter()
        .map(|node| (node.id().clone(), node))
        .collect();
    let previous_relationships: HashMap<RelationshipId, Relationship> = previous
        .current_relationship_records()
        .map_err(graph_error)?
        .into_iter()
        .map(|relationship| (relationship.id().clone(), relationship))
        .collect();
    let nodes = current
        .current_node_records()
        .map_err(graph_error)?
        .into_iter()
        .filter(|node| {
            previous_nodes
                .get(node.id())
                .is_none_or(|previous| previous.version_id() != node.version_id())
        })
        .collect();
    let relationships = current
        .current_relationship_records()
        .map_err(graph_error)?
        .into_iter()
        .filter(|relationship| {
            previous_relationships
                .get(relationship.id())
                .is_none_or(|previous| previous.version_id() != relationship.version_id())
        })
        .collect();
    Ok((nodes, relationships))
}

/// Byte-identical to the canonical payload encoding, so a branch payload
/// decodes exactly like a canonical one.
fn encode_payload_bytes(value: &impl serde::Serialize) -> GraphStorageResult<Vec<u8>> {
    let mut bytes = serde_json::to_vec(value).map_err(graph_error)?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn branch_node_change(node: &Node, payload: CommitPayloadRef) -> CommitRecordChange {
    CommitRecordChange {
        kind: HistoryRecordKind::Node,
        record_id: node.id().as_str().to_owned(),
        version_id: node.version_id().as_str().to_owned(),
        deleted: node.status() == RecordStatus::Tombstoned,
        labels: node.labels().to_vec(),
        source: None,
        target: None,
        payload,
    }
}

fn branch_relationship_change(
    relationship: &Relationship,
    payload: CommitPayloadRef,
) -> CommitRecordChange {
    CommitRecordChange {
        kind: HistoryRecordKind::Relationship,
        record_id: relationship.id().as_str().to_owned(),
        version_id: relationship.version_id().as_str().to_owned(),
        deleted: relationship.status() == RecordStatus::Tombstoned,
        labels: vec![relationship.rel_type().as_str().to_owned()],
        source: Some(relationship.source().as_str().to_owned()),
        target: Some(relationship.target().as_str().to_owned()),
        payload,
    }
}

fn missing_reference() -> GraphStorageError {
    GraphStorageError::OperationFailed {
        operation: "commit_branch_transition",
        message: "branch payload append returned fewer references than payloads".to_owned(),
    }
}

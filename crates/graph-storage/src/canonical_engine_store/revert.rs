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

//! Revert one change of a branch (issue #348).
//!
//! The record goes back to the version it had where the branch split from
//! `main`: a changed record gets its base version, a record the branch added
//! is tombstoned, a record the branch deleted is recreated. The revert is one
//! branch commit, published only if the branch head is still the one the
//! caller saw. A revert that would leave a live relationship without an
//! endpoint is refused; the relationship has to be reverted first.

use graph_core::{Node, NodeId, Relationship, RelationshipId};

use super::{CanonicalEngineStore, TransitionCommitContext};
use crate::graph_history::{
    CommitView, CommitViewEntry, GraphCommitId, HistoryRecordKind, MAIN_BRANCH,
};
use crate::{GraphStorageError, GraphStorageResult};

/// What a revert wrote.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphRevertRestoration {
    /// The record existed at the merge base and was changed: its base version is back.
    BaseVersion,
    /// The branch added the record: it is tombstoned.
    Removed,
    /// The branch deleted the record: its base version is back.
    Recreated,
}

/// One record to put back to its merge-base version.
#[derive(Clone, Debug)]
pub struct GraphRevertRequest {
    /// Branch to write on; never `main`.
    pub branch: String,
    /// Node or relationship.
    pub record_kind: HistoryRecordKind,
    /// Logical record id.
    pub record_id: String,
    /// Head the caller reviewed; the revert is refused if the branch moved.
    pub expected_head: Option<GraphCommitId>,
    /// Who reverted.
    pub actor: Option<String>,
}

/// The commit a revert published.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphRevertReport {
    /// Branch commit holding the revert.
    pub commit_id: GraphCommitId,
    /// Branch written.
    pub branch: String,
    /// Node or relationship.
    pub record_kind: HistoryRecordKind,
    /// Reverted record.
    pub record_id: String,
    /// What the revert wrote.
    pub restored: GraphRevertRestoration,
}

const BLOCKERS_SHOWN: usize = 5;

fn live<'a>(
    view: &'a CommitView,
    kind: HistoryRecordKind,
    id: &str,
) -> Option<&'a CommitViewEntry> {
    view.entries(kind).get(id).filter(|entry| !entry.deleted)
}

impl CanonicalEngineStore {
    /// Revert one record of a branch to its merge-base version, as one commit.
    ///
    /// # Errors
    ///
    /// Returns [`GraphStorageError::GraphBranchConflict`] for `main` or a
    /// branch without a merge base, [`GraphStorageError::UnknownGraphBranch`],
    /// [`GraphStorageError::GraphHeadMoved`] when `expected_head` is stale,
    /// [`GraphStorageError::GraphRevertNoChange`] when the record does not
    /// differ from the base, and [`GraphStorageError::GraphRevertBlocked`]
    /// when the revert would leave a relationship without an endpoint.
    pub fn revert_graph_change(
        &mut self,
        request: GraphRevertRequest,
    ) -> GraphStorageResult<GraphRevertReport> {
        let branch = request.branch.clone();
        let record_id = request.record_id.clone();
        if branch == MAIN_BRANCH {
            return Err(GraphStorageError::GraphBranchConflict {
                branch,
                reason: "main has no merge base to revert to; revert changes on a branch"
                    .to_owned(),
            });
        }
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
        let main_head = self.history.head(MAIN_BRANCH)?.clone();
        let base = self.history.merge_base(&main_head, &head)?.ok_or_else(|| {
            GraphStorageError::GraphBranchConflict {
                branch: branch.clone(),
                reason: "the branch shares no commit with main".to_owned(),
            }
        })?;
        let base_view = self.commit_view(&base)?;
        let head_view = self.commit_view(&head)?;
        let kind = request.record_kind;
        let base_entry = live(&base_view, kind, &record_id);
        let head_entry = live(&head_view, kind, &record_id);
        let no_change = || GraphStorageError::GraphRevertNoChange {
            branch: branch.clone(),
            record_id: record_id.clone(),
        };
        let blocked = |reason: String| GraphStorageError::GraphRevertBlocked {
            branch: branch.clone(),
            record_id: record_id.clone(),
            reason,
        };
        let restored = match (base_entry, head_entry) {
            (None, None) => return Err(no_change()),
            (Some(base), Some(head)) if base.version_id == head.version_id => {
                return Err(no_change());
            }
            (Some(_), Some(_)) => GraphRevertRestoration::BaseVersion,
            (None, Some(_)) => GraphRevertRestoration::Removed,
            (Some(_), None) => GraphRevertRestoration::Recreated,
        };

        let (previous, current) = match kind {
            HistoryRecordKind::Node => {
                if restored == GraphRevertRestoration::Removed {
                    let attached: Vec<&String> = head_view
                        .relationships
                        .iter()
                        .filter(|(_, entry)| {
                            !entry.deleted
                                && (entry.source.as_deref() == Some(record_id.as_str())
                                    || entry.target.as_deref() == Some(record_id.as_str()))
                        })
                        .map(|(id, _)| id)
                        .collect();
                    if !attached.is_empty() {
                        let shown: Vec<&str> = attached
                            .iter()
                            .take(BLOCKERS_SHOWN)
                            .map(|id| id.as_str())
                            .collect();
                        return Err(blocked(format!(
                            "{} live relationship(s) still use it ({}); revert them first",
                            attached.len(),
                            shown.join(", ")
                        )));
                    }
                }
                let head_node = head_entry
                    .map(|entry| self.decode_node(entry))
                    .transpose()?;
                let base_node = base_entry
                    .map(|entry| self.decode_node(entry))
                    .transpose()?;
                let previous =
                    self.graph_from_records(head_node.clone().into_iter().collect(), Vec::new())?;
                let current = match (restored, base_node, head_node) {
                    (GraphRevertRestoration::Removed, _, Some(node)) => {
                        let mut graph = self.graph_from_records(vec![node], Vec::new())?;
                        graph
                            .tombstone_node(&node_id(&record_id)?)
                            .map_err(revert_error)?;
                        graph
                    }
                    (_, Some(node), _) => self.graph_from_records(vec![node], Vec::new())?,
                    _ => return Err(no_change()),
                };
                (previous, current)
            }
            HistoryRecordKind::Relationship => {
                let endpoints_of = |entry: &CommitViewEntry| -> Vec<String> {
                    entry
                        .source
                        .iter()
                        .chain(entry.target.iter())
                        .cloned()
                        .collect()
                };
                let mut endpoint_ids: Vec<String> = Vec::new();
                for entry in [base_entry, head_entry].into_iter().flatten() {
                    for id in endpoints_of(entry) {
                        if !endpoint_ids.contains(&id) {
                            endpoint_ids.push(id);
                        }
                    }
                }
                let mut endpoints: Vec<Node> = Vec::new();
                for id in &endpoint_ids {
                    match live(&head_view, HistoryRecordKind::Node, id) {
                        Some(entry) => endpoints.push(self.decode_node(entry)?),
                        None if restored != GraphRevertRestoration::Removed => {
                            return Err(blocked(format!(
                                "its endpoint {id} is not on the branch; revert that node first"
                            )));
                        }
                        None => {}
                    }
                }
                let head_relationship: Option<Relationship> = head_entry
                    .map(|entry| self.decode_relationship(entry))
                    .transpose()?;
                let base_relationship: Option<Relationship> = base_entry
                    .map(|entry| self.decode_relationship(entry))
                    .transpose()?;
                let previous = self.graph_from_records(
                    endpoints.clone(),
                    head_relationship.clone().into_iter().collect(),
                )?;
                let current = match (restored, base_relationship, head_relationship) {
                    (GraphRevertRestoration::Removed, _, Some(relationship)) => {
                        let mut graph = self.graph_from_records(endpoints, vec![relationship])?;
                        graph
                            .tombstone_relationship(&relationship_id(&record_id)?)
                            .map_err(revert_error)?;
                        graph
                    }
                    (_, Some(relationship), _) => {
                        self.graph_from_records(endpoints, vec![relationship])?
                    }
                    _ => return Err(no_change()),
                };
                (previous, current)
            }
        };

        let kind_name = match kind {
            HistoryRecordKind::Node => "node",
            HistoryRecordKind::Relationship => "relationship",
        };
        let commit = self
            .commit_branch_transition(
                &branch,
                &previous,
                &current,
                TransitionCommitContext {
                    actor: request.actor.clone(),
                    message: Some(format!(
                        "revert {kind_name} {record_id} to its version at {base}"
                    )),
                    ..TransitionCommitContext::default()
                },
                Some(&head),
            )?
            .ok_or_else(no_change)?;
        Ok(GraphRevertReport {
            commit_id: commit.commit_id.clone(),
            branch,
            record_kind: kind,
            record_id,
            restored,
        })
    }
}

fn node_id(value: &str) -> GraphStorageResult<NodeId> {
    NodeId::new(value).map_err(revert_error)
}

fn relationship_id(value: &str) -> GraphStorageResult<RelationshipId> {
    RelationshipId::new(value).map_err(revert_error)
}

fn revert_error(error: impl std::fmt::Display) -> GraphStorageError {
    GraphStorageError::OperationFailed {
        operation: "revert_graph_change",
        message: error.to_string(),
    }
}

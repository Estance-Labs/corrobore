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

//! Three-way branch merge with typed conflicts (issue #321).
//!
//! The merge base is the best common ancestor of the two heads. A record the
//! source changed since the base and the target did not is taken from the
//! source; a record both sides changed to different versions is a conflict.
//! Conflicts are never resolved automatically. Merging into `main` replays
//! the merged versions through the canonical WAL with the source head as a
//! second parent; merging into another branch publishes a commit whose
//! changes reference the source payloads directly.

use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

use graph_core::{Node, Relationship};

use super::{CanonicalEngineStore, TransitionCommitContext};
use crate::graph_history::{
    CommitRecordChange, CommitView, CommitViewEntry, GraphCommitDraft, GraphCommitId,
    HistoryRecordKind, MAIN_BRANCH,
};
use crate::{DurableTransactionId, GraphStorageError, GraphStorageResult};

/// Why two sides of a merge cannot be combined automatically.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MergeConflictKind {
    /// Both sides changed the same record to different versions.
    ConcurrentUpdate,
    /// One side changed a record the other side deleted.
    UpdateDelete,
    /// Both sides created the same record id with different content.
    AddAdd,
    /// The merged graph would hold a live relationship whose endpoint is
    /// missing or deleted.
    DanglingRelationship,
}

/// One conflict of a merge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeConflict {
    /// Conflict kind.
    pub kind: MergeConflictKind,
    /// Node or relationship.
    pub record_kind: HistoryRecordKind,
    /// Logical record id.
    pub record_id: String,
    /// Version at the merge base.
    pub base_version: Option<String>,
    /// Version on the source head.
    pub source_version: Option<String>,
    /// Version on the target head.
    pub target_version: Option<String>,
    /// Human-readable explanation.
    pub detail: String,
}

/// How a merge ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeOutcomeKind {
    /// The target already contains the source head.
    AlreadyUpToDate,
    /// The target head is an ancestor of the source head.
    FastForward,
    /// Both sides changed since the base and the changes were combined.
    Merged,
    /// Conflicts prevent the merge; nothing was written.
    Conflicted,
}

/// Result (or preview, with `dry_run`) of a merge.
#[derive(Clone, Debug)]
pub struct GraphMergeReport {
    /// Outcome.
    pub outcome: MergeOutcomeKind,
    /// Source branch.
    pub source: String,
    /// Target branch.
    pub target: String,
    /// Source head merged.
    pub source_head: GraphCommitId,
    /// Target head before the merge.
    pub target_head: GraphCommitId,
    /// Merge base.
    pub base: Option<GraphCommitId>,
    /// Records the merge takes from the source.
    pub applied: Vec<CommitRecordChange>,
    /// Conflicts; non-empty only for `Conflicted`.
    pub conflicts: Vec<MergeConflict>,
    /// Commit published on the target, unless dry run, up to date or conflicted.
    pub commit: Option<GraphCommitId>,
    /// Whether this was a preview.
    pub dry_run: bool,
}

/// Options of a merge request.
#[derive(Clone, Debug, Default)]
pub struct GraphMergeRequest {
    /// Branch to merge from.
    pub source: String,
    /// Branch to merge into (`main` when empty).
    pub target: String,
    /// Compute the merge without writing.
    pub dry_run: bool,
    /// Refuse when the target head is no longer this commit.
    pub expected_target_head: Option<GraphCommitId>,
    /// Delete the source branch after a successful merge.
    pub delete_source: bool,
    /// Actor recorded on the merge commit.
    pub actor: Option<String>,
    /// Commit message; defaults to "merge <source> into <target>".
    pub message: Option<String>,
}

fn version(entry: Option<&CommitViewEntry>) -> Option<&str> {
    entry.map(|entry| entry.version_id.as_str())
}

fn live(entry: Option<&CommitViewEntry>) -> bool {
    entry.is_some_and(|entry| !entry.deleted)
}

impl CanonicalEngineStore {
    /// Merge `request.source` into `request.target`.
    ///
    /// Conflicts return a `Conflicted` report and write nothing. A dry run
    /// returns the report it would produce without writing.
    pub fn merge_graph_branch(
        &mut self,
        request: GraphMergeRequest,
    ) -> GraphStorageResult<GraphMergeReport> {
        let target_branch = if request.target.is_empty() {
            MAIN_BRANCH.to_owned()
        } else {
            request.target.clone()
        };
        if request.source == target_branch {
            return Err(GraphStorageError::GraphBranchConflict {
                branch: request.source.clone(),
                reason: "a branch cannot be merged into itself".to_owned(),
            });
        }
        self.reconcile_history()?;
        let source_head = self.history.head(&request.source)?.clone();
        let target_head = self.history.head(&target_branch)?.clone();
        if let Some(expected) = &request.expected_target_head
            && expected != &target_head
        {
            return Err(GraphStorageError::GraphHeadMoved {
                branch: target_branch,
                expected: expected.to_string(),
                actual: target_head.to_string(),
            });
        }
        let base = self.history.merge_base(&target_head, &source_head)?;
        let mut report = GraphMergeReport {
            outcome: MergeOutcomeKind::AlreadyUpToDate,
            source: request.source.clone(),
            target: target_branch.clone(),
            source_head: source_head.clone(),
            target_head: target_head.clone(),
            base: base.clone(),
            applied: Vec::new(),
            conflicts: Vec::new(),
            commit: None,
            dry_run: request.dry_run,
        };
        if self.history.is_ancestor(&source_head, &target_head)? {
            return Ok(report);
        }
        let fast_forward = self.history.is_ancestor(&target_head, &source_head)?;
        let base_view = match &base {
            Some(base) => self.commit_view(base)?,
            None => Arc::new(CommitView::default()),
        };
        let source_view = self.commit_view(&source_head)?;
        let target_view = self.commit_view(&target_head)?;
        let (applied, mut conflicts) = plan_merge(&base_view, &source_view, &target_view);
        conflicts.extend(dangling_relationships(&target_view, &source_view, &applied));
        report.applied = applied;
        if !conflicts.is_empty() {
            conflicts.sort_by(|left, right| {
                (left.kind, left.record_kind, &left.record_id).cmp(&(
                    right.kind,
                    right.record_kind,
                    &right.record_id,
                ))
            });
            report.conflicts = conflicts;
            report.outcome = MergeOutcomeKind::Conflicted;
            return Ok(report);
        }
        report.outcome = if fast_forward {
            MergeOutcomeKind::FastForward
        } else {
            MergeOutcomeKind::Merged
        };
        if request.dry_run || report.applied.is_empty() {
            return Ok(report);
        }
        let message = request
            .message
            .clone()
            .or_else(|| Some(format!("merge {} into {target_branch}", request.source)));
        let commit = if target_branch == MAIN_BRANCH {
            self.apply_merge_to_main(&report, &target_view, request.actor.clone(), message)?
        } else {
            self.history
                .record_commit(GraphCommitDraft {
                    branch: target_branch.clone(),
                    extra_parents: vec![source_head.clone()],
                    transaction_id: None,
                    mutation_sequence_number: None,
                    actor: request.actor.clone(),
                    message,
                    created_at: None,
                    changes: report.applied.clone(),
                })?
                .commit_id
                .clone()
        };
        report.commit = Some(commit);
        if request.delete_source {
            self.history
                .delete_branch(&request.source, request.actor.clone())?;
        }
        Ok(report)
    }

    fn apply_merge_to_main(
        &mut self,
        report: &GraphMergeReport,
        target_view: &CommitView,
        actor: Option<String>,
        message: Option<String>,
    ) -> GraphStorageResult<GraphCommitId> {
        let mut previous_nodes: Vec<Node> = Vec::new();
        let mut previous_relationships: Vec<Relationship> = Vec::new();
        let mut current_nodes: Vec<Node> = Vec::new();
        let mut current_relationships: Vec<Relationship> = Vec::new();
        let mut node_ids: HashSet<String> = HashSet::new();
        for change in &report.applied {
            let incoming = CommitViewEntry {
                version_id: change.version_id.clone(),
                deleted: change.deleted,
                labels: change.labels.clone(),
                source: change.source.clone(),
                target: change.target.clone(),
                payload: change.payload.clone(),
                published_by: report.source_head.clone(),
            };
            let existing = target_view.entries(change.kind).get(&change.record_id);
            match change.kind {
                HistoryRecordKind::Node => {
                    node_ids.insert(change.record_id.clone());
                    current_nodes.push(self.decode_node(&incoming)?);
                    if let Some(existing) = existing {
                        previous_nodes.push(self.decode_node(existing)?);
                    }
                }
                HistoryRecordKind::Relationship => {
                    current_relationships.push(self.decode_relationship(&incoming)?);
                    if let Some(existing) = existing {
                        previous_relationships.push(self.decode_relationship(existing)?);
                    }
                }
            }
        }
        // Relationships need their endpoints in both graphs; unchanged endpoints
        // come from the target and therefore never enter the WAL batch.
        let endpoint_ids: BTreeSet<String> = current_relationships
            .iter()
            .chain(previous_relationships.iter())
            .flat_map(|relationship| {
                [
                    relationship.source().as_str().to_owned(),
                    relationship.target().as_str().to_owned(),
                ]
            })
            .filter(|id| !node_ids.contains(id))
            .collect();
        for id in endpoint_ids {
            if let Some(entry) = target_view.nodes.get(&id) {
                let node = self.decode_node(entry)?;
                previous_nodes.push(node.clone());
                current_nodes.push(node);
            }
        }
        for change in &report.applied {
            if change.kind == HistoryRecordKind::Node
                && let Some(existing) = target_view.nodes.get(&change.record_id)
                && !previous_nodes
                    .iter()
                    .any(|node| node.id().as_str() == change.record_id)
            {
                previous_nodes.push(self.decode_node(existing)?);
            }
        }
        let previous = self.graph_from_records(previous_nodes, previous_relationships)?;
        let current = self.graph_from_records(current_nodes, current_relationships)?;
        let transaction_id = DurableTransactionId::new(format!(
            "tx--graph-merge-{}-{}",
            report.source_head, report.target_head
        ))
        .map_err(|error| GraphStorageError::OperationFailed {
            operation: "merge_graph_branch",
            message: error.to_string(),
        })?;
        self.commit_transition_with_context(
            &previous,
            &current,
            transaction_id,
            TransitionCommitContext {
                audit_events: vec![format!("graph branch {} merged into main", report.source)],
                actor,
                message,
                merge_parents: vec![report.source_head.clone()],
            },
            None,
        )?;
        let head = self.graph_head(MAIN_BRANCH)?;
        if head == report.target_head {
            return Err(GraphStorageError::OperationFailed {
                operation: "merge_graph_branch",
                message: "the merge transaction did not publish a main commit".to_owned(),
            });
        }
        Ok(head)
    }
}

/// Records to take from the source, and version conflicts.
fn plan_merge(
    base: &CommitView,
    source: &CommitView,
    target: &CommitView,
) -> (Vec<CommitRecordChange>, Vec<MergeConflict>) {
    let mut applied = Vec::new();
    let mut conflicts = Vec::new();
    for kind in [HistoryRecordKind::Node, HistoryRecordKind::Relationship] {
        let ids: BTreeSet<&String> = source.entries(kind).keys().collect();
        for id in ids {
            let b = base.entries(kind).get(id);
            let s = source.entries(kind).get(id);
            let t = target.entries(kind).get(id);
            let source_changed = version(s) != version(b);
            if !source_changed {
                continue;
            }
            let target_changed = version(t) != version(b);
            if !target_changed {
                if let Some(entry) = s {
                    applied.push(CommitRecordChange {
                        kind,
                        record_id: id.clone(),
                        version_id: entry.version_id.clone(),
                        deleted: entry.deleted,
                        labels: entry.labels.clone(),
                        source: entry.source.clone(),
                        target: entry.target.clone(),
                        payload: entry.payload.clone(),
                    });
                }
                continue;
            }
            if version(s) == version(t) {
                continue;
            }
            let (conflict_kind, detail) = if b.is_none() {
                (
                    MergeConflictKind::AddAdd,
                    "both sides created this record with different content",
                )
            } else if live(s) != live(t) {
                (
                    MergeConflictKind::UpdateDelete,
                    "one side deleted this record while the other changed it",
                )
            } else {
                (
                    MergeConflictKind::ConcurrentUpdate,
                    "both sides changed this record since the merge base",
                )
            };
            conflicts.push(MergeConflict {
                kind: conflict_kind,
                record_kind: kind,
                record_id: id.clone(),
                base_version: version(b).map(str::to_owned),
                source_version: version(s).map(str::to_owned),
                target_version: version(t).map(str::to_owned),
                detail: detail.to_owned(),
            });
        }
    }
    (applied, conflicts)
}

/// A relationship whose endpoints must stay live after a merge.
struct DanglingCandidate<'a> {
    record_id: String,
    from: Option<String>,
    to: Option<String>,
    source_version: Option<&'a str>,
    target_version: Option<&'a str>,
}

/// Live relationships of the merged graph whose endpoint would not be live.
fn dangling_relationships(
    target: &CommitView,
    source: &CommitView,
    applied: &[CommitRecordChange],
) -> Vec<MergeConflict> {
    let node_live = |id: &str| -> bool {
        match applied
            .iter()
            .find(|change| change.kind == HistoryRecordKind::Node && change.record_id == id)
        {
            Some(change) => !change.deleted,
            None => live(target.nodes.get(id)),
        }
    };
    let mut candidates: Vec<DanglingCandidate<'_>> = Vec::new();
    for change in applied {
        if change.kind == HistoryRecordKind::Relationship && !change.deleted {
            candidates.push(DanglingCandidate {
                record_id: change.record_id.clone(),
                from: change.source.clone(),
                to: change.target.clone(),
                source_version: Some(change.version_id.as_str()),
                target_version: version(target.relationships.get(&change.record_id)),
            });
        }
    }
    let deleted_nodes: HashSet<&str> = applied
        .iter()
        .filter(|change| change.kind == HistoryRecordKind::Node && change.deleted)
        .map(|change| change.record_id.as_str())
        .collect();
    if !deleted_nodes.is_empty() {
        for (id, entry) in &target.relationships {
            if entry.deleted
                || applied.iter().any(|change| {
                    change.kind == HistoryRecordKind::Relationship && &change.record_id == id
                })
            {
                continue;
            }
            let touches = [entry.source.as_deref(), entry.target.as_deref()]
                .into_iter()
                .flatten()
                .any(|endpoint| deleted_nodes.contains(endpoint));
            if touches {
                candidates.push(DanglingCandidate {
                    record_id: id.clone(),
                    from: entry.source.clone(),
                    to: entry.target.clone(),
                    source_version: version(source.relationships.get(id)),
                    target_version: Some(entry.version_id.as_str()),
                });
            }
        }
    }
    candidates
        .into_iter()
        .filter(|candidate| {
            !(candidate.from.as_deref().is_some_and(node_live)
                && candidate.to.as_deref().is_some_and(node_live))
        })
        .map(
            |DanglingCandidate {
                 record_id,
                 from,
                 to,
                 source_version,
                 target_version,
             }| MergeConflict {
                kind: MergeConflictKind::DanglingRelationship,
                record_kind: HistoryRecordKind::Relationship,
                record_id,
                base_version: None,
                source_version: source_version.map(str::to_owned),
                target_version: target_version.map(str::to_owned),
                detail: format!(
                    "the merged graph would keep this relationship between {} and {} while an endpoint is missing or deleted",
                    from.unwrap_or_default(),
                    to.unwrap_or_default()
                ),
            },
        )
        .collect()
}

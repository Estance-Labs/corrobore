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

//! Graph diffs, merge bases and the change feed (issue #320).

use std::collections::BTreeSet;
use std::sync::Arc;

use graph_core::{Node, Relationship};

use super::CanonicalEngineStore;
use crate::graph_history::{
    CommitView, CommitViewEntry, GraphCommit, GraphCommitId, HistoryRecordKind,
};
use crate::{GraphStorageError, GraphStorageResult};

/// How a record differs between two commits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordChangeKind {
    /// Not visible at `from`, visible at `to`.
    Added,
    /// Visible at both with a different version.
    Changed,
    /// Visible at `from`, not visible (absent or tombstoned) at `to`.
    Removed,
}

/// Payloads of one record on both sides of a diff.
#[derive(Clone, Debug)]
pub enum DiffValues {
    /// Node versions.
    Node {
        /// Version visible at `from`.
        before: Option<Box<Node>>,
        /// Version visible at `to`.
        after: Option<Box<Node>>,
    },
    /// Relationship versions.
    Relationship {
        /// Version visible at `from`.
        before: Option<Box<Relationship>>,
        /// Version visible at `to`.
        after: Option<Box<Relationship>>,
    },
}

/// One record that differs between two commits.
#[derive(Clone, Debug)]
pub struct GraphDiffEntry {
    /// Node or relationship.
    pub kind: HistoryRecordKind,
    /// Logical record id.
    pub record_id: String,
    /// Added, changed or removed.
    pub change: RecordChangeKind,
    /// Version visible at `from`.
    pub before_version: Option<String>,
    /// Version visible at `to`.
    pub after_version: Option<String>,
    /// Labels (or relationship type) of the latest visible side.
    pub labels: Vec<String>,
    /// Relationship endpoints, for relationships.
    pub endpoints: Option<(String, String)>,
    /// Decoded payloads when requested.
    pub values: Option<DiffValues>,
}

/// Counts of a full diff.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GraphDiffTotals {
    /// Added records.
    pub added: usize,
    /// Changed records.
    pub changed: usize,
    /// Removed records.
    pub removed: usize,
}

/// One page of a diff in (kind, record id) order.
#[derive(Clone, Debug)]
pub struct GraphDiffPage {
    /// Diff base.
    pub from: GraphCommitId,
    /// Diff target.
    pub to: GraphCommitId,
    /// Differences of this page.
    pub entries: Vec<GraphDiffEntry>,
    /// Totals over the whole diff, all kinds included.
    pub totals: GraphDiffTotals,
    /// Cursor (`node:<id>` or `relationship:<id>`) for the next page.
    pub next_cursor: Option<String>,
}

/// Commits published on a branch after a cursor, oldest first.
#[derive(Clone, Debug)]
pub struct GraphChangeFeedPage {
    /// Branch read.
    pub branch: String,
    /// Head of the branch when read.
    pub head: GraphCommitId,
    /// Commits after the cursor along the first-parent chain, oldest first.
    pub commits: Vec<Arc<GraphCommit>>,
    /// Cursor to resume after this page: the last commit returned.
    pub next_cursor: Option<GraphCommitId>,
    /// Whether commits newer than this page remain.
    pub has_more: bool,
}

fn visible(entry: Option<&CommitViewEntry>) -> Option<&CommitViewEntry> {
    entry.filter(|entry| !entry.deleted)
}

fn classify(
    before: Option<&CommitViewEntry>,
    after: Option<&CommitViewEntry>,
) -> Option<RecordChangeKind> {
    match (visible(before), visible(after)) {
        (None, Some(_)) => Some(RecordChangeKind::Added),
        (Some(_), None) => Some(RecordChangeKind::Removed),
        (Some(before), Some(after)) if before.version_id != after.version_id => {
            Some(RecordChangeKind::Changed)
        }
        _ => None,
    }
}

fn parse_cursor(cursor: Option<&str>) -> GraphStorageResult<Option<(HistoryRecordKind, String)>> {
    let Some(cursor) = cursor else {
        return Ok(None);
    };
    let parsed = match cursor.split_once(':') {
        Some(("node", id)) => Some((HistoryRecordKind::Node, id.to_owned())),
        Some(("relationship", id)) => Some((HistoryRecordKind::Relationship, id.to_owned())),
        _ => None,
    };
    parsed
        .map(Some)
        .ok_or_else(|| GraphStorageError::OperationFailed {
            operation: "graph_diff",
            message: format!("diff cursor {cursor:?} must be node:<id> or relationship:<id>"),
        })
}

impl CanonicalEngineStore {
    /// Records that differ between two commits, paged in (kind, id) order.
    ///
    /// `kind` restricts the page to nodes or relationships; totals always
    /// cover both. `include_values` decodes both versions of each entry.
    pub fn graph_diff(
        &mut self,
        from: &GraphCommitId,
        to: &GraphCommitId,
        kind: Option<HistoryRecordKind>,
        cursor: Option<&str>,
        limit: usize,
        include_values: bool,
    ) -> GraphStorageResult<GraphDiffPage> {
        let before = self.commit_view(from)?;
        let after = self.commit_view(to)?;
        let cursor = parse_cursor(cursor)?;
        let limit = limit.max(1);
        let mut totals = GraphDiffTotals::default();
        let mut entries = Vec::new();
        let mut next_cursor = None;
        for record_kind in [HistoryRecordKind::Node, HistoryRecordKind::Relationship] {
            let ids: BTreeSet<&String> = before
                .entries(record_kind)
                .keys()
                .chain(after.entries(record_kind).keys())
                .collect();
            for id in ids {
                let left = before.entries(record_kind).get(id);
                let right = after.entries(record_kind).get(id);
                let Some(change) = classify(left, right) else {
                    continue;
                };
                match change {
                    RecordChangeKind::Added => totals.added += 1,
                    RecordChangeKind::Changed => totals.changed += 1,
                    RecordChangeKind::Removed => totals.removed += 1,
                }
                if kind.is_some_and(|wanted| wanted != record_kind) {
                    continue;
                }
                if let Some((cursor_kind, cursor_id)) = &cursor
                    && (record_kind, id.as_str()) <= (*cursor_kind, cursor_id.as_str())
                {
                    continue;
                }
                if entries.len() == limit {
                    if next_cursor.is_none() {
                        next_cursor = entries.last().map(|entry: &GraphDiffEntry| {
                            format!("{}:{}", kind_prefix(entry.kind), entry.record_id)
                        });
                    }
                    continue;
                }
                entries.push(self.diff_entry(
                    &before,
                    &after,
                    record_kind,
                    id,
                    change,
                    include_values,
                )?);
            }
        }
        Ok(GraphDiffPage {
            from: from.clone(),
            to: to.clone(),
            entries,
            totals,
            next_cursor,
        })
    }

    /// Best common ancestor of two commits.
    pub fn graph_merge_base(
        &mut self,
        a: &GraphCommitId,
        b: &GraphCommitId,
    ) -> GraphStorageResult<Option<GraphCommitId>> {
        self.reconcile_history()?;
        self.history.merge_base(a, b)
    }

    /// Commits published on `branch` after `after`, oldest first.
    ///
    /// `after = None` starts at the genesis commit. A cursor that is no
    /// longer retained answers [`GraphStorageError::ChangeFeedGap`]; a
    /// retained commit off the branch's first-parent chain is refused too.
    pub fn graph_changes(
        &mut self,
        branch: &str,
        after: Option<&GraphCommitId>,
        limit: usize,
    ) -> GraphStorageResult<GraphChangeFeedPage> {
        self.reconcile_history()?;
        let head = self.history.head(branch)?.clone();
        if let Some(after) = after
            && !self.history.contains(after)
        {
            return Err(GraphStorageError::ChangeFeedGap {
                cursor: after.to_string(),
            });
        }
        let mut chain = Vec::new();
        let mut current = Some(self.history.commit(&head)?);
        let mut found = after.is_none();
        while let Some(commit) = current {
            if Some(&commit.commit_id) == after {
                found = true;
                break;
            }
            current = commit
                .parents
                .first()
                .map(|parent| self.history.commit(parent))
                .transpose()?;
            chain.push(commit);
        }
        if !found {
            return Err(GraphStorageError::GraphBranchConflict {
                branch: branch.to_owned(),
                reason: format!(
                    "change feed cursor {} is not on the first-parent history of {branch}",
                    after.map(ToString::to_string).unwrap_or_default()
                ),
            });
        }
        chain.reverse();
        let limit = limit.max(1);
        let has_more = chain.len() > limit;
        chain.truncate(limit);
        let next_cursor = chain
            .last()
            .map(|commit| commit.commit_id.clone())
            .or_else(|| after.cloned());
        Ok(GraphChangeFeedPage {
            branch: branch.to_owned(),
            head,
            commits: chain,
            next_cursor,
            has_more,
        })
    }

    fn diff_entry(
        &self,
        before: &CommitView,
        after: &CommitView,
        kind: HistoryRecordKind,
        id: &str,
        change: RecordChangeKind,
        include_values: bool,
    ) -> GraphStorageResult<GraphDiffEntry> {
        let left = visible(before.entries(kind).get(id));
        let right = visible(after.entries(kind).get(id));
        let latest = right.or(left);
        let values = if include_values {
            Some(match kind {
                HistoryRecordKind::Node => DiffValues::Node {
                    before: left
                        .map(|entry| self.decode_node(entry).map(Box::new))
                        .transpose()?,
                    after: right
                        .map(|entry| self.decode_node(entry).map(Box::new))
                        .transpose()?,
                },
                HistoryRecordKind::Relationship => DiffValues::Relationship {
                    before: left
                        .map(|entry| self.decode_relationship(entry).map(Box::new))
                        .transpose()?,
                    after: right
                        .map(|entry| self.decode_relationship(entry).map(Box::new))
                        .transpose()?,
                },
            })
        } else {
            None
        };
        Ok(GraphDiffEntry {
            kind,
            record_id: id.to_owned(),
            change,
            before_version: left.map(|entry| entry.version_id.clone()),
            after_version: right.map(|entry| entry.version_id.clone()),
            labels: latest.map(|entry| entry.labels.clone()).unwrap_or_default(),
            endpoints: latest.and_then(|entry| entry.source.clone().zip(entry.target.clone())),
            values,
        })
    }
}

fn kind_prefix(kind: HistoryRecordKind) -> &'static str {
    match kind {
        HistoryRecordKind::Node => "node",
        HistoryRecordKind::Relationship => "relationship",
    }
}

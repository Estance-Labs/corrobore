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

//! History retention and payload compaction (issue #324).

use super::CanonicalEngineStore;
use crate::graph_history::HistoryCompaction;
use crate::{
    AtomicPersistentCompactionRequest, AtomicPersistentCompactionScope, GraphStorageError,
    GraphStorageResult, compact_atomic_persistent_segments,
};

/// What to keep when cleaning up history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryCleanupRequest {
    /// Newest first-parent commits kept on every branch (at least 1).
    pub keep_commits: usize,
    /// Also keep every commit created less than this many seconds ago.
    pub keep_seconds: Option<u64>,
    /// Report what would be pruned without writing.
    pub dry_run: bool,
}

impl Default for HistoryCleanupRequest {
    fn default() -> Self {
        Self {
            keep_commits: 100,
            keep_seconds: None,
            dry_run: false,
        }
    }
}

/// Outcome of a history cleanup.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct HistoryCleanupReport {
    /// Whether nothing was written.
    pub dry_run: bool,
    /// Commits pruned (or that would be).
    pub pruned_commits: usize,
    /// Commits retained.
    pub retained_commits: usize,
    /// Retained commits turned into baselines.
    pub baselines: usize,
    /// History and branch payload compaction figures.
    pub compaction: HistoryCompaction,
    /// WAL and journal bytes reclaimed behind the latest checkpoint.
    pub wal_reclaimed_bytes: u64,
    /// Why WAL compaction was skipped, when it was.
    pub wal_skipped: Option<String>,
}

/// Creation-time cutoff of `keep_seconds`, as an RFC 3339 string.
fn retention_cutoff(request: &HistoryCleanupRequest) -> Option<String> {
    request.keep_seconds.map(|seconds| {
        let seconds = i64::try_from(seconds).unwrap_or(i64::MAX);
        (chrono::Utc::now() - chrono::Duration::seconds(seconds))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    })
}

impl CanonicalEngineStore {
    /// The `main` commits `request` would prune, oldest first, while their
    /// views are still readable: what a cold tier archives before cleanup
    /// (ADR-0023). Commits of other branches are not listed.
    pub fn retention_pruned_commits(
        &mut self,
        request: &HistoryCleanupRequest,
    ) -> GraphStorageResult<Vec<std::sync::Arc<crate::GraphCommit>>> {
        self.reconcile_history()?;
        let plan = self
            .history
            .plan_retention(request.keep_commits, retention_cutoff(request).as_deref());
        let retained: std::collections::HashSet<&crate::GraphCommitId> =
            plan.retained.iter().collect();
        Ok(self
            .history
            .commits_in_append_order()
            .filter(|commit| {
                commit.branch == crate::MAIN_BRANCH && !retained.contains(&commit.commit_id)
            })
            .collect())
    }

    /// Prune graph history outside the retention window, compact the branch
    /// payloads it no longer references, and compact the WAL and transaction
    /// journals behind the latest checkpoint.
    ///
    /// Commits reachable as the newest `keep_commits` of any branch, or newer
    /// than `keep_seconds`, keep their exact views. Older commits become
    /// unreadable: reads and change-feed cursors on them answer as unknown
    /// commits or `ChangeFeedGap`.
    pub fn cleanup_graph_history(
        &mut self,
        request: &HistoryCleanupRequest,
    ) -> GraphStorageResult<HistoryCleanupReport> {
        self.reconcile_history()?;
        let plan = self
            .history
            .plan_retention(request.keep_commits, retention_cutoff(request).as_deref());
        let mut report = HistoryCleanupReport {
            dry_run: request.dry_run,
            pruned_commits: plan.pruned,
            retained_commits: plan.retained.len(),
            baselines: plan.baselines.len(),
            ..HistoryCleanupReport::default()
        };
        if request.dry_run {
            return Ok(report);
        }
        let root = self.root.clone();
        report.compaction = self.history.compact(&root, &plan)?;
        match compact_atomic_persistent_segments(
            &root,
            AtomicPersistentCompactionRequest {
                scope: AtomicPersistentCompactionScope::TransactionsAndIndexes,
                snapshot_protected_sequences: Vec::new(),
                retention_protected_sequences: Vec::new(),
            },
        ) {
            Ok(outcome) => report.wal_reclaimed_bytes = outcome.reclaimed_bytes,
            Err(GraphStorageError::OperationFailed { message, .. }) => {
                report.wal_skipped = Some(message);
            }
            Err(error) => return Err(error),
        }
        Ok(report)
    }
}

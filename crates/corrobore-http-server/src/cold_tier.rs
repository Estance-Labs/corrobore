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

//! Lance cold tier on the server (issue #392, ADR-0023).
//!
//! With `CORROBORE_STORAGE_COLD_TIER_URL` set, history cleanup first
//! archives the `main` commits it is about to prune, then prunes them, under
//! one hold of the store lock so the archived set is exactly the pruned one.
//! Reads that name a pruned commit (Cypher reads and the commit route) are
//! then answered from the archive. Builds without the `lance-cold-tier`
//! feature refuse the setting at startup.

use graph_storage::{CanonicalEngineStore, HistoryCleanupReport, HistoryCleanupRequest};
use serde::Serialize;

use crate::config::ColdTierConfig;

/// Cleanup outcome with the archive run that preceded it.
#[derive(Debug, Serialize)]
pub struct CleanupOutcome {
    #[serde(flatten)]
    pub report: HistoryCleanupReport,
    /// What the cold tier archived; absent without a cold tier or on a dry run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cold_tier: Option<ArchiveSummary>,
}

#[derive(Debug, Serialize)]
pub struct ArchiveSummary {
    pub uri: String,
    pub archived: usize,
    pub already_archived: usize,
    pub node_rows: usize,
    pub relationship_rows: usize,
}

/// An archived commit, as the commit route reports it.
#[derive(Clone, Debug, Serialize)]
pub struct ArchivedCommit {
    pub commit_id: String,
    pub branch: String,
    pub generation: u64,
    pub parents: Vec<String>,
    pub actor: Option<String>,
    pub message: Option<String>,
    pub created_at: String,
}

#[cfg(feature = "lance-cold-tier")]
mod lance_tier {
    use std::sync::Arc;

    use super::*;
    use corrobore_cold_tier::{ColdCommit, ColdTier};
    use graph_storage::{CanonicalProjectionRequest, GraphCommitId};

    /// The configured archive.
    #[derive(Clone, Debug)]
    pub struct ColdTierHandle(Arc<ColdTier>);

    impl ColdTierHandle {
        pub fn open(config: &ColdTierConfig) -> Self {
            Self(Arc::new(ColdTier::new(
                config.uri.clone(),
                config.options.clone(),
            )))
        }

        pub fn uri(&self) -> &str {
            self.0.uri()
        }

        /// Archive what `request` prunes, then prune it. Runs on a blocking
        /// thread that holds the store lock; the archive's async I/O runs on
        /// the current runtime.
        pub fn archive_then_cleanup(
            &self,
            store: &mut CanonicalEngineStore,
            request: &HistoryCleanupRequest,
        ) -> Result<CleanupOutcome, String> {
            if request.dry_run {
                let report = store
                    .cleanup_graph_history(request)
                    .map_err(|e| e.to_string())?;
                return Ok(CleanupOutcome {
                    report,
                    cold_tier: None,
                });
            }
            let pruned = store
                .retention_pruned_commits(request)
                .map_err(|error| error.to_string())?;
            let commits: Vec<ColdCommit> = pruned
                .iter()
                .map(|commit| ColdCommit {
                    commit_id: commit.commit_id.to_string(),
                    branch: commit.branch.clone(),
                    generation: commit.generation,
                    parents: commit.parents.iter().map(ToString::to_string).collect(),
                    actor: commit.actor.clone(),
                    message: commit.message.clone(),
                    created_at: commit.created_at.clone(),
                })
                .collect();
            let full = CanonicalProjectionRequest::all();
            let tier = Arc::clone(&self.0);
            let report = tokio::runtime::Handle::current()
                .block_on(tier.archive(commits, |commit| {
                    let id = GraphCommitId::new(&commit.commit_id).map_err(|e| e.to_string())?;
                    store
                        .load_commit_projection(&id, &full)
                        .map_err(|error| error.to_string())
                }))
                .map_err(|error| error.to_string())?;
            // Archived (or already archived) first: only then is pruning safe.
            let cleanup = store
                .cleanup_graph_history(request)
                .map_err(|e| e.to_string())?;
            Ok(CleanupOutcome {
                report: cleanup,
                cold_tier: Some(ArchiveSummary {
                    uri: self.0.uri().to_owned(),
                    archived: report.archived,
                    already_archived: report.already_archived,
                    node_rows: report.node_rows,
                    relationship_rows: report.relationship_rows,
                }),
            })
        }

        pub fn graph_at(&self, commit_id: &str) -> Result<Option<graph_core::Graph>, String> {
            let tier = Arc::clone(&self.0);
            tokio::runtime::Handle::current()
                .block_on(tier.graph_at(commit_id))
                .map_err(|error| error.to_string())
        }

        pub async fn commit(&self, commit_id: &str) -> Result<Option<ArchivedCommit>, String> {
            Ok(self
                .0
                .commit(commit_id)
                .await
                .map_err(|error| error.to_string())?
                .map(archived))
        }

        pub async fn commits(&self) -> Result<Vec<ArchivedCommit>, String> {
            Ok(self
                .0
                .commits()
                .await
                .map_err(|error| error.to_string())?
                .into_iter()
                .map(archived)
                .collect())
        }
    }

    fn archived(commit: ColdCommit) -> ArchivedCommit {
        ArchivedCommit {
            commit_id: commit.commit_id,
            branch: commit.branch,
            generation: commit.generation,
            parents: commit.parents,
            actor: commit.actor,
            message: commit.message,
            created_at: commit.created_at,
        }
    }
}

#[cfg(feature = "lance-cold-tier")]
pub use lance_tier::ColdTierHandle;

/// Without the feature there is never a cold tier: configuration refuses the
/// setting, so this type is never built.
#[cfg(not(feature = "lance-cold-tier"))]
#[derive(Clone, Debug)]
pub enum ColdTierHandle {}

#[cfg(not(feature = "lance-cold-tier"))]
impl ColdTierHandle {
    pub fn open(_config: &ColdTierConfig) -> Self {
        unreachable!("configuration refuses a cold tier in builds without lance-cold-tier")
    }
    pub fn uri(&self) -> &str {
        match *self {}
    }
    pub fn archive_then_cleanup(
        &self,
        _store: &mut CanonicalEngineStore,
        _request: &HistoryCleanupRequest,
    ) -> Result<CleanupOutcome, String> {
        match *self {}
    }
    pub fn graph_at(&self, _commit_id: &str) -> Result<Option<graph_core::Graph>, String> {
        match *self {}
    }
    pub async fn commit(&self, _commit_id: &str) -> Result<Option<ArchivedCommit>, String> {
        match *self {}
    }
    pub async fn commits(&self) -> Result<Vec<ArchivedCommit>, String> {
        match *self {}
    }
}

/// Cleanup without a cold tier.
pub fn cleanup(
    cold: Option<&ColdTierHandle>,
    store: &mut CanonicalEngineStore,
    request: &HistoryCleanupRequest,
) -> Result<CleanupOutcome, String> {
    match cold {
        Some(cold) => cold.archive_then_cleanup(store, request),
        None => Ok(CleanupOutcome {
            report: store
                .cleanup_graph_history(request)
                .map_err(|error| error.to_string())?,
            cold_tier: None,
        }),
    }
}

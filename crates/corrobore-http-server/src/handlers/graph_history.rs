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

//! Graph history routes: addressable commits of the persistent store.

use std::sync::{Arc, Mutex};

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use graph_storage::{
    CanonicalEngineStore, GraphCommit, GraphCommitId, GraphStorageError, HistoryRecordKind,
    MAIN_BRANCH,
};
use serde::{Deserialize, Serialize};

use crate::{
    app::{AppState, RuntimeStoreProvider},
    error::ApiError,
};

const DEFAULT_COMMIT_PAGE: usize = 50;
const MAX_COMMIT_PAGE: usize = 500;

/// Canonical store of a persistent runtime, or 409 in ephemeral mode.
pub(crate) fn canonical_store(
    state: &AppState,
) -> Result<Arc<Mutex<CanonicalEngineStore>>, ApiError> {
    match &state.runtime_store {
        RuntimeStoreProvider::Persistent(runtime) => Ok(Arc::clone(&runtime.canonical_store)),
        RuntimeStoreProvider::Ephemeral => Err(ApiError {
            status: StatusCode::CONFLICT,
            code: "PERSISTENT_STORAGE_REQUIRED",
            message:
                "graph history, branches and commit reads need CORROBORE_STORAGE_MODE=persistent"
                    .to_owned(),
        }),
    }
}

/// Run a blocking closure against the locked canonical store.
pub(crate) async fn with_store<T, F>(state: &AppState, operation: F) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: FnOnce(&mut CanonicalEngineStore) -> Result<T, ApiError> + Send + 'static,
{
    let store = canonical_store(state)?;
    tokio::task::spawn_blocking(move || {
        let mut locked = store.lock().map_err(|_| {
            ApiError::internal("STATE_LOCK_FAILED", "canonical store lock poisoned")
        })?;
        operation(&mut locked)
    })
    .await
    .map_err(|error| ApiError::internal("TASK_JOIN_FAILED", error.to_string()))?
}

/// Map history errors to stable API codes.
pub(crate) fn map_history_error(error: GraphStorageError) -> ApiError {
    match error {
        GraphStorageError::UnknownGraphCommit { commit_id } => ApiError::not_found(
            "GRAPH_COMMIT_NOT_FOUND",
            format!("graph commit {commit_id} is not retained"),
        ),
        GraphStorageError::UnknownGraphBranch { branch } => ApiError::not_found(
            "GRAPH_BRANCH_NOT_FOUND",
            format!("graph branch {branch} does not exist"),
        ),
        GraphStorageError::GraphBranchConflict { branch, reason } => ApiError {
            status: StatusCode::CONFLICT,
            code: "GRAPH_BRANCH_CONFLICT",
            message: format!("branch {branch}: {reason}"),
        },
        GraphStorageError::GraphHeadMoved {
            branch,
            expected,
            actual,
        } => ApiError {
            status: StatusCode::CONFLICT,
            code: "GRAPH_COMMIT_PRECONDITION_FAILED",
            message: format!(
                "branch {branch} moved from {expected} to {actual}; read the new head and retry"
            ),
        },
        GraphStorageError::ChangeFeedGap { cursor } => ApiError {
            status: StatusCode::GONE,
            code: "CHANGE_FEED_GAP",
            message: format!(
                "change feed cursor {cursor} is no longer retained; read the branch at a commit and resume from its head"
            ),
        },
        GraphStorageError::GraphRevertNoChange { branch, record_id } => ApiError::unprocessable(
            "GRAPH_REVERT_NO_CHANGE",
            format!("record {record_id} on branch {branch} does not differ from its merge base"),
        ),
        GraphStorageError::GraphRevertBlocked {
            branch,
            record_id,
            reason,
        } => ApiError {
            status: StatusCode::CONFLICT,
            code: "GRAPH_REVERT_BLOCKED",
            message: format!("record {record_id} on branch {branch} cannot be reverted: {reason}"),
        },
        GraphStorageError::InvalidGraphBranchName { branch, reason } => ApiError::bad_request(
            "INVALID_BRANCH_NAME",
            format!("branch name {branch:?} is invalid: {reason}"),
        ),
        other => ApiError::internal("GRAPH_HISTORY_FAILED", other.to_string()),
    }
}

/// Parse a commit id from a path or query value; malformed ids are unknown commits.
pub(crate) fn parse_commit_id(value: &str) -> Result<GraphCommitId, ApiError> {
    GraphCommitId::new(value).map_err(map_history_error)
}

#[derive(Debug, Default, Deserialize)]
pub struct CommitListQuery {
    /// Branch whose first-parent history is listed. Defaults to `main`.
    pub branch: Option<String>,
    /// Resume after this commit (the `next_cursor` of the previous page).
    pub cursor: Option<String>,
    /// Page size, 1-500. Defaults to 50.
    pub limit: Option<usize>,
}

/// Commit summary returned by listings.
#[derive(Debug, Serialize)]
pub struct CommitSummaryView {
    pub commit_id: String,
    pub parents: Vec<String>,
    pub branch: String,
    pub generation: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mutation_sequence_number: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub created_at: String,
    pub node_changes: usize,
    pub relationship_changes: usize,
    pub deletions: usize,
}

impl From<&GraphCommit> for CommitSummaryView {
    fn from(commit: &GraphCommit) -> Self {
        Self {
            commit_id: commit.commit_id.to_string(),
            parents: commit.parents.iter().map(ToString::to_string).collect(),
            branch: commit.branch.clone(),
            generation: commit.generation,
            transaction_id: commit.transaction_id.clone(),
            mutation_sequence_number: commit.mutation_sequence_number,
            actor: commit.actor.clone(),
            message: commit.message.clone(),
            created_at: commit.created_at.clone(),
            node_changes: commit.node_change_count(),
            relationship_changes: commit.relationship_change_count(),
            deletions: commit.deletion_count(),
        }
    }
}

/// One record version published by a commit. Storage references stay internal.
#[derive(Debug, Serialize)]
pub struct CommitChangeView {
    pub kind: &'static str,
    pub record_id: String,
    pub version_id: String,
    pub deleted: bool,
    pub labels: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

pub(crate) fn kind_name(kind: HistoryRecordKind) -> &'static str {
    match kind {
        HistoryRecordKind::Node => "node",
        HistoryRecordKind::Relationship => "relationship",
    }
}

#[derive(Debug, Serialize)]
pub struct CommitDetailView {
    #[serde(flatten)]
    pub summary: CommitSummaryView,
    pub changes: Vec<CommitChangeView>,
}

#[derive(Debug, Serialize)]
pub struct CommitListResult {
    pub branch: String,
    pub head: String,
    pub commits: Vec<CommitSummaryView>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Envelope<T> {
    pub ok: bool,
    pub result: T,
}

pub(crate) fn ok<T>(result: T) -> Json<Envelope<T>> {
    Json(Envelope { ok: true, result })
}

/// `GET /v1/graph/commits`: first-parent history of a branch, newest first.
pub async fn list_commits(
    State(state): State<AppState>,
    Query(query): Query<CommitListQuery>,
) -> Result<Json<Envelope<CommitListResult>>, ApiError> {
    let branch = query.branch.unwrap_or_else(|| MAIN_BRANCH.to_owned());
    let limit = query
        .limit
        .unwrap_or(DEFAULT_COMMIT_PAGE)
        .clamp(1, MAX_COMMIT_PAGE);
    let cursor = query.cursor.as_deref().map(parse_commit_id).transpose()?;
    with_store(&state, move |store| {
        let head = store.graph_head(&branch).map_err(map_history_error)?;
        let page = store
            .list_graph_commits(&branch, cursor.as_ref(), limit)
            .map_err(map_history_error)?;
        Ok(ok(CommitListResult {
            branch,
            head: head.to_string(),
            commits: page
                .commits
                .iter()
                .map(|commit| CommitSummaryView::from(commit.as_ref()))
                .collect(),
            next_cursor: page.next_cursor.map(|cursor| cursor.to_string()),
        }))
    })
    .await
}

/// `GET /v1/graph/commits/{commit_id}`: one commit with its record changes.
pub async fn get_commit(
    State(state): State<AppState>,
    Path(commit_id): Path<String>,
) -> Result<Json<Envelope<CommitDetailView>>, ApiError> {
    let commit_id = parse_commit_id(&commit_id)?;
    with_store(&state, move |store| {
        let commit = store.graph_commit(&commit_id).map_err(map_history_error)?;
        Ok(ok(CommitDetailView {
            summary: CommitSummaryView::from(commit.as_ref()),
            changes: commit
                .changes
                .iter()
                .map(|change| CommitChangeView {
                    kind: kind_name(change.kind),
                    record_id: change.record_id.clone(),
                    version_id: change.version_id.clone(),
                    deleted: change.deleted,
                    labels: change.labels.clone(),
                    source: change.source.clone(),
                    target: change.target.clone(),
                })
                .collect(),
        }))
    })
    .await
}

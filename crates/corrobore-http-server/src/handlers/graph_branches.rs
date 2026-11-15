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

//! Branch routes of the persistent graph (issue #319).

use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use graph_storage::{GraphBranchInfo, GraphBranchStatus};
use serde::{Deserialize, Serialize};

use crate::{
    app::AppState,
    auth::{RequestIdentity, resolve_write_actor},
    error::ApiError,
    handlers::graph_history::{Envelope, map_history_error, ok, parse_commit_id, with_store},
};

/// One branch with its head, origin and merge state.
#[derive(Debug, Serialize)]
pub struct BranchView {
    pub name: String,
    pub head: String,
    pub created_from: String,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_by: Option<String>,
    pub merged: bool,
    pub commits_ahead: usize,
}

impl From<&GraphBranchStatus> for BranchView {
    fn from(status: &GraphBranchStatus) -> Self {
        Self {
            merged: status.merged,
            commits_ahead: status.commits_ahead,
            ..Self::from(&status.info)
        }
    }
}

impl From<&GraphBranchInfo> for BranchView {
    fn from(info: &GraphBranchInfo) -> Self {
        Self {
            name: info.name.clone(),
            head: info.head.to_string(),
            created_from: info.created_from.to_string(),
            created_at: info.created_at.clone(),
            created_by: info.created_by.clone(),
            merged: false,
            commits_ahead: 0,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct BranchListResult {
    pub branches: Vec<BranchView>,
}

#[derive(Debug, Deserialize)]
pub struct CreateBranchRequest {
    /// New branch name: 1-128 characters from `[A-Za-z0-9._/-]`.
    pub name: String,
    /// Start at this branch's head (default `main`).
    pub from_branch: Option<String>,
    /// Start at this commit; wins over `from_branch`.
    pub from_commit: Option<String>,
    /// Actor recorded on the branch.
    pub actor: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct DeleteBranchQuery {
    /// Delete even when the branch holds commits `main` does not contain.
    pub force: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct DeleteBranchResult {
    pub deleted: String,
}

/// `GET /v1/graph/branches`: every branch, `main` first.
pub async fn list_branches(
    State(state): State<AppState>,
) -> Result<Json<Envelope<BranchListResult>>, ApiError> {
    with_store(&state, |store| {
        let statuses = store.list_graph_branches().map_err(map_history_error)?;
        Ok(ok(BranchListResult {
            branches: statuses.iter().map(BranchView::from).collect(),
        }))
    })
    .await
}

/// `POST /v1/graph/branches`: create a branch at a branch head or commit.
pub async fn create_branch(
    State(state): State<AppState>,
    identity: Option<Extension<RequestIdentity>>,
    Json(request): Json<CreateBranchRequest>,
) -> Result<Json<Envelope<BranchView>>, ApiError> {
    let actor = resolve_write_actor(identity.as_deref(), request.actor.clone())?;
    let from_commit = request
        .from_commit
        .as_deref()
        .map(parse_commit_id)
        .transpose()?;
    with_store(&state, move |store| {
        let info = store
            .create_graph_branch(
                request.name.trim(),
                request.from_branch.as_deref(),
                from_commit.as_ref(),
                actor,
            )
            .map_err(map_history_error)?;
        Ok(ok(BranchView::from(&info)))
    })
    .await
}

/// `DELETE /v1/graph/branches/{*name}`: delete a merged branch, or any with `force`.
pub async fn delete_branch(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(query): Query<DeleteBranchQuery>,
) -> Result<Json<Envelope<DeleteBranchResult>>, ApiError> {
    let name = name.trim_start_matches('/').to_owned();
    with_store(&state, move |store| {
        store
            .delete_graph_branch(&name, query.force.unwrap_or(false), None)
            .map_err(map_history_error)?;
        Ok(ok(DeleteBranchResult { deleted: name }))
    })
    .await
}

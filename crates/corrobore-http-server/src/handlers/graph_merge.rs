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

//! Branch merge route (issue #321).

use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use graph_storage::{
    GraphMergeReport, GraphMergeRequest, MAIN_BRANCH, MergeConflictKind, MergeOutcomeKind,
};
use serde::{Deserialize, Serialize};

use crate::{
    app::AppState,
    error::ApiError,
    handlers::graph_history::{kind_name, map_history_error, parse_commit_id, with_store},
};

#[derive(Debug, Deserialize)]
pub struct MergeRequestBody {
    /// Branch to merge from.
    pub source: String,
    /// Branch to merge into. Defaults to `main`.
    pub target: Option<String>,
    /// Compute the merge without writing.
    #[serde(default)]
    pub dry_run: bool,
    /// Refuse with 409 when the target head is no longer this commit.
    pub expected_target_head: Option<String>,
    /// Delete the source branch after a successful merge.
    #[serde(default)]
    pub delete_source: bool,
    pub actor: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MergeConflictView {
    pub kind: &'static str,
    pub record_kind: &'static str,
    pub record_id: String,
    pub base_version: Option<String>,
    pub source_version: Option<String>,
    pub target_version: Option<String>,
    pub detail: String,
}

#[derive(Debug, Serialize)]
pub struct MergeChangeCounts {
    pub nodes: usize,
    pub relationships: usize,
    pub deletions: usize,
}

#[derive(Debug, Serialize)]
pub struct MergeReportView {
    pub outcome: &'static str,
    pub dry_run: bool,
    pub source: String,
    pub target: String,
    pub source_head: String,
    pub target_head: String,
    pub merge_base: Option<String>,
    pub commit_id: Option<String>,
    pub changes: MergeChangeCounts,
    pub conflicts: Vec<MergeConflictView>,
}

fn conflict_kind(kind: MergeConflictKind) -> &'static str {
    match kind {
        MergeConflictKind::ConcurrentUpdate => "concurrent_update",
        MergeConflictKind::UpdateDelete => "update_delete",
        MergeConflictKind::AddAdd => "add_add",
        MergeConflictKind::DanglingRelationship => "dangling_relationship",
    }
}

fn outcome_name(outcome: MergeOutcomeKind) -> &'static str {
    match outcome {
        MergeOutcomeKind::AlreadyUpToDate => "already_up_to_date",
        MergeOutcomeKind::FastForward => "fast_forward",
        MergeOutcomeKind::Merged => "merged",
        MergeOutcomeKind::Conflicted => "conflicted",
    }
}

fn report_view(report: &GraphMergeReport) -> MergeReportView {
    MergeReportView {
        outcome: outcome_name(report.outcome),
        dry_run: report.dry_run,
        source: report.source.clone(),
        target: report.target.clone(),
        source_head: report.source_head.to_string(),
        target_head: report.target_head.to_string(),
        merge_base: report.base.as_ref().map(ToString::to_string),
        commit_id: report.commit.as_ref().map(ToString::to_string),
        changes: MergeChangeCounts {
            nodes: report
                .applied
                .iter()
                .filter(|change| change.kind == graph_storage::HistoryRecordKind::Node)
                .count(),
            relationships: report
                .applied
                .iter()
                .filter(|change| change.kind == graph_storage::HistoryRecordKind::Relationship)
                .count(),
            deletions: report
                .applied
                .iter()
                .filter(|change| change.deleted)
                .count(),
        },
        conflicts: report
            .conflicts
            .iter()
            .map(|conflict| MergeConflictView {
                kind: conflict_kind(conflict.kind),
                record_kind: kind_name(conflict.record_kind),
                record_id: conflict.record_id.clone(),
                base_version: conflict.base_version.clone(),
                source_version: conflict.source_version.clone(),
                target_version: conflict.target_version.clone(),
                detail: conflict.detail.clone(),
            })
            .collect(),
    }
}

#[derive(Serialize)]
struct ConflictError {
    code: &'static str,
    message: String,
}

#[derive(Serialize)]
struct MergeEnvelope {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<ConflictError>,
    result: MergeReportView,
}

/// `POST /v1/graph/merge`: three-way merge of one branch into another.
///
/// A conflicted merge answers `409 MERGE_CONFLICT` with the full report, so
/// the caller sees every conflict at once; nothing is written.
pub async fn merge(
    State(state): State<AppState>,
    Json(body): Json<MergeRequestBody>,
) -> Result<Response, ApiError> {
    let expected_target_head = body
        .expected_target_head
        .as_deref()
        .map(parse_commit_id)
        .transpose()?;
    let report = with_store(&state, move |store| {
        store
            .merge_graph_branch(GraphMergeRequest {
                source: body.source,
                target: body.target.unwrap_or_else(|| MAIN_BRANCH.to_owned()),
                dry_run: body.dry_run,
                expected_target_head,
                delete_source: body.delete_source,
                actor: body.actor,
                message: body.message,
            })
            .map_err(map_history_error)
    })
    .await?;
    let view = report_view(&report);
    if report.outcome == MergeOutcomeKind::Conflicted {
        let message = format!(
            "{} conflict(s) block merging {} into {}",
            view.conflicts.len(),
            view.source,
            view.target
        );
        return Ok((
            StatusCode::CONFLICT,
            Json(MergeEnvelope {
                ok: false,
                error: Some(ConflictError {
                    code: "MERGE_CONFLICT",
                    message,
                }),
                result: view,
            }),
        )
            .into_response());
    }
    Ok(Json(MergeEnvelope {
        ok: true,
        error: None,
        result: view,
    })
    .into_response())
}

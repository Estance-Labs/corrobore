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

//! Review routes (issue #348): revert one change of a branch, and record or
//! list review decisions on exact branch heads.
//!
//! Authentication is a shared bearer token with no per-user identity, so a
//! review names its reviewer in the body and defaults to the server's trusted
//! actor (`CORROBORE_MEMORY_ACTOR_ID`); a revert names its actor the same way.

use axum::{
    Extension, Json,
    extract::{Query, State},
    http::StatusCode,
};
use graph_storage::{
    GraphRevertRequest, GraphRevertRestoration, GraphReview, GraphReviewDecision,
    HistoryRecordKind, MAIN_BRANCH,
};
use serde::{Deserialize, Serialize};

use crate::{
    app::AppState,
    auth::{RequestIdentity, resolve_write_actor},
    error::ApiError,
    handlers::graph_history::{Envelope, map_history_error, ok, parse_commit_id, with_store},
};

/// Longest review comment, in characters.
pub const MAX_COMMENT_CHARS: usize = 4_000;

#[derive(Debug, Deserialize)]
pub struct RevertBody {
    pub branch: String,
    pub record_kind: String,
    pub record_id: String,
    pub expected_head: Option<String>,
    pub actor: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RevertResult {
    pub commit_id: String,
    pub branch: String,
    pub record_kind: HistoryRecordKind,
    pub record_id: String,
    pub restored: GraphRevertRestoration,
}

fn invalid_revert(message: impl Into<String>) -> ApiError {
    ApiError::bad_request("INVALID_REVERT", message)
}

/// `POST /v1/graph/revert`: put one record of a branch back to its version
/// at the merge base with `main`, as one branch commit.
pub async fn revert(
    State(state): State<AppState>,
    identity: Option<Extension<RequestIdentity>>,
    Json(body): Json<RevertBody>,
) -> Result<Json<Envelope<RevertResult>>, ApiError> {
    if body.branch.trim().is_empty() || body.branch == MAIN_BRANCH {
        return Err(invalid_revert("revert changes on a branch other than main"));
    }
    let record_kind = match body.record_kind.as_str() {
        "node" => HistoryRecordKind::Node,
        "relationship" => HistoryRecordKind::Relationship,
        other => {
            return Err(invalid_revert(format!(
                "record_kind must be node or relationship, not {other:?}"
            )));
        }
    };
    if body.record_id.trim().is_empty() {
        return Err(invalid_revert("record_id must not be blank"));
    }
    let expected_head = body
        .expected_head
        .as_deref()
        .map(parse_commit_id)
        .transpose()?;
    let actor = resolve_write_actor(identity.as_deref(), body.actor)?
        .unwrap_or_else(|| state.config.memory_actor_id.clone());
    with_store(&state, move |store| {
        let report = store
            .revert_graph_change(GraphRevertRequest {
                branch: body.branch,
                record_kind,
                record_id: body.record_id,
                expected_head,
                actor: Some(actor),
            })
            .map_err(map_history_error)?;
        Ok(ok(RevertResult {
            commit_id: report.commit_id.to_string(),
            branch: report.branch,
            record_kind: report.record_kind,
            record_id: report.record_id,
            restored: report.restored,
        }))
    })
    .await
}

#[derive(Debug, Deserialize)]
pub struct ReviewBody {
    pub branch: String,
    pub head: String,
    pub decision: String,
    pub comment: Option<String>,
    pub reviewer: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ReviewView {
    pub branch: String,
    pub head: String,
    pub reviewer: String,
    pub decision: GraphReviewDecision,
    pub comment: Option<String>,
    pub at_ms: u64,
}

impl From<GraphReview> for ReviewView {
    fn from(review: GraphReview) -> Self {
        Self {
            branch: review.branch,
            head: review.head.to_string(),
            reviewer: review.reviewer,
            decision: review.decision,
            comment: review.comment,
            at_ms: review.at_ms,
        }
    }
}

fn invalid_review(message: impl Into<String>) -> ApiError {
    ApiError::bad_request("INVALID_REVIEW", message)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

/// `POST /v1/graph/reviews`: record a decision on the branch's current head.
pub async fn submit(
    State(state): State<AppState>,
    identity: Option<Extension<RequestIdentity>>,
    Json(body): Json<ReviewBody>,
) -> Result<(StatusCode, Json<Envelope<ReviewView>>), ApiError> {
    if body.branch.trim().is_empty() || body.branch == MAIN_BRANCH {
        return Err(invalid_review("review a branch other than main"));
    }
    let decision = match body.decision.as_str() {
        "approved" => GraphReviewDecision::Approved,
        "changes_requested" => GraphReviewDecision::ChangesRequested,
        other => {
            return Err(invalid_review(format!(
                "decision must be approved or changes_requested, not {other:?}"
            )));
        }
    };
    if body
        .comment
        .as_ref()
        .is_some_and(|comment| comment.chars().count() > MAX_COMMENT_CHARS)
    {
        return Err(invalid_review(format!(
            "comment must hold at most {MAX_COMMENT_CHARS} characters"
        )));
    }
    let head = parse_commit_id(&body.head)?;
    let reviewer = resolve_write_actor(identity.as_deref(), body.reviewer)?
        .unwrap_or_else(|| state.config.memory_actor_id.clone());
    let at_ms = now_ms();
    with_store(&state, move |store| {
        let review = store
            .record_graph_review(
                &body.branch,
                &head,
                &reviewer,
                decision,
                body.comment,
                at_ms,
            )
            .map_err(map_history_error)?;
        Ok((StatusCode::CREATED, ok(ReviewView::from(review))))
    })
    .await
}

#[derive(Debug, Deserialize)]
pub struct ReviewListQuery {
    pub branch: String,
}

#[derive(Debug, Serialize)]
pub struct ListedReview {
    #[serde(flatten)]
    pub review: ReviewView,
    /// Whether the review looked at the branch's current head.
    pub current: bool,
}

#[derive(Debug, Serialize)]
pub struct ReviewList {
    pub reviews: Vec<ListedReview>,
}

/// `GET /v1/graph/reviews?branch=`: a branch's reviews, newest first.
pub async fn list(
    State(state): State<AppState>,
    Query(query): Query<ReviewListQuery>,
) -> Result<Json<Envelope<ReviewList>>, ApiError> {
    with_store(&state, move |store| {
        let reviews = store
            .list_graph_reviews(&query.branch)
            .map_err(map_history_error)?
            .into_iter()
            .map(|status| ListedReview {
                review: ReviewView::from(status.review),
                current: status.current,
            })
            .collect();
        Ok(ok(ReviewList { reviews }))
    })
    .await
}

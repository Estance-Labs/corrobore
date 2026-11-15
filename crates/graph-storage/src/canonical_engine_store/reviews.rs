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

//! Review decisions on branch heads (issue #348).
//!
//! A review names the exact head it looked at, so an approval does not carry
//! over to commits added after it. Reviews live in `history/reviews.log`,
//! append-only like the graph history: one JSON line per review with the
//! SHA-256 of its body, appended with `sync_data`. A torn final line is
//! ignored when reading and truncated before the next append; any other
//! unreadable line is corruption.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use super::CanonicalEngineStore;
use crate::graph_history::{GraphCommitId, MAIN_BRANCH, hex_sha256};
use crate::{GraphStorageError, GraphStorageResult};

const REVIEW_LOG: &str = "reviews.log";

/// A reviewer's decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphReviewDecision {
    /// The head may be merged.
    Approved,
    /// The head needs changes first.
    ChangesRequested,
}

/// One review of one branch head.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphReview {
    /// Reviewed branch.
    pub branch: String,
    /// Exact head the review looked at.
    pub head: GraphCommitId,
    /// Who reviewed.
    pub reviewer: String,
    /// The decision.
    pub decision: GraphReviewDecision,
    /// Optional free-text comment.
    pub comment: Option<String>,
    /// Milliseconds since the Unix epoch.
    pub at_ms: u64,
}

/// A listed review and whether it looked at the branch's current head.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphReviewStatus {
    /// The review.
    pub review: GraphReview,
    /// Whether it looked at the branch's current head.
    pub current: bool,
}

#[derive(Serialize, Deserialize)]
struct ReviewLine {
    review: GraphReview,
    sha256: String,
}

fn log_path(root: &Path) -> PathBuf {
    root.join("history").join(REVIEW_LOG)
}

fn io_error(operation: &'static str, path: &Path, error: std::io::Error) -> GraphStorageError {
    GraphStorageError::OperationFailed {
        operation,
        message: format!("{}: {error}", path.display()),
    }
}

fn digest(review: &GraphReview) -> GraphStorageResult<String> {
    let bytes = serde_json::to_vec(review).map_err(|error| GraphStorageError::OperationFailed {
        operation: "encode_graph_review",
        message: error.to_string(),
    })?;
    Ok(hex_sha256(&bytes))
}

/// Reviews in log order and the length of the valid prefix of the log.
fn read_reviews(path: &Path) -> GraphStorageResult<(Vec<GraphReview>, u64)> {
    if !path.is_file() {
        return Ok((Vec::new(), 0));
    }
    let bytes = fs::read(path).map_err(|error| io_error("read_graph_reviews", path, error))?;
    let mut reviews = Vec::new();
    let mut valid = 0_usize;
    let mut cursor = 0_usize;
    let mut line_number = 0_usize;
    while cursor < bytes.len() {
        line_number += 1;
        let Some(relative_end) = bytes[cursor..].iter().position(|byte| *byte == b'\n') else {
            break;
        };
        let end = cursor + relative_end;
        let is_last = end + 1 >= bytes.len();
        let decoded = serde_json::from_slice::<ReviewLine>(&bytes[cursor..end])
            .map_err(|error| error.to_string())
            .and_then(|line| match digest(&line.review) {
                Ok(sha) if sha == line.sha256 => Ok(line.review),
                Ok(_) => Err("checksum mismatch".to_owned()),
                Err(error) => Err(error.to_string()),
            });
        match decoded {
            Ok(review) => {
                reviews.push(review);
                valid = end + 1;
            }
            Err(_) if is_last => break,
            Err(reason) => {
                return Err(GraphStorageError::GraphHistoryCorrupted {
                    line: line_number,
                    reason: format!("{REVIEW_LOG}: {reason}"),
                });
            }
        }
        cursor = end + 1;
    }
    Ok((reviews, valid as u64))
}

impl CanonicalEngineStore {
    /// Record a review of `head`, which must be the branch's current head.
    ///
    /// # Errors
    ///
    /// Returns [`GraphStorageError::GraphBranchConflict`] for `main`,
    /// [`GraphStorageError::UnknownGraphBranch`],
    /// [`GraphStorageError::GraphHeadMoved`] when `head` is not current, and
    /// an operation error for a blank reviewer or a failed append.
    pub fn record_graph_review(
        &mut self,
        branch: &str,
        head: &GraphCommitId,
        reviewer: &str,
        decision: GraphReviewDecision,
        comment: Option<String>,
        at_ms: u64,
    ) -> GraphStorageResult<GraphReview> {
        if branch == MAIN_BRANCH {
            return Err(GraphStorageError::GraphBranchConflict {
                branch: branch.to_owned(),
                reason: "main is not reviewed; review a branch before merging it".to_owned(),
            });
        }
        let reviewer = reviewer.trim();
        if reviewer.is_empty() {
            return Err(GraphStorageError::OperationFailed {
                operation: "record_graph_review",
                message: "a review names its reviewer".to_owned(),
            });
        }
        self.reconcile_history()?;
        let current = self.history.head(branch)?;
        if current != head {
            return Err(GraphStorageError::GraphHeadMoved {
                branch: branch.to_owned(),
                expected: head.to_string(),
                actual: current.to_string(),
            });
        }
        let review = GraphReview {
            branch: branch.to_owned(),
            head: head.clone(),
            reviewer: reviewer.to_owned(),
            decision,
            comment: comment
                .map(|comment| comment.trim().to_owned())
                .filter(|comment| !comment.is_empty()),
            at_ms,
        };
        let path = log_path(self.root.path());
        let (_, valid) = read_reviews(&path)?;
        let mut line = serde_json::to_vec(&ReviewLine {
            sha256: digest(&review)?,
            review: review.clone(),
        })
        .map_err(|error| GraphStorageError::OperationFailed {
            operation: "encode_graph_review",
            message: error.to_string(),
        })?;
        line.push(b'\n');
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| io_error("append_graph_review", parent, error))?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|error| io_error("append_graph_review", &path, error))?;
        let length = file
            .metadata()
            .map_err(|error| io_error("append_graph_review", &path, error))?
            .len();
        if length > valid {
            // Drop a torn final line before appending after it.
            file.set_len(valid)
                .map_err(|error| io_error("repair_graph_reviews_tail", &path, error))?;
        }
        file.write_all(&line)
            .and_then(|()| file.sync_data())
            .map_err(|error| io_error("append_graph_review", &path, error))?;
        Ok(review)
    }

    /// Reviews of a branch, newest first, each marked when it looked at the
    /// branch's current head. A deleted or unknown branch lists its past
    /// reviews, none of them current.
    ///
    /// # Errors
    ///
    /// Returns [`GraphStorageError::GraphHistoryCorrupted`] for a damaged log.
    pub fn list_graph_reviews(
        &mut self,
        branch: &str,
    ) -> GraphStorageResult<Vec<GraphReviewStatus>> {
        self.reconcile_history()?;
        let current = self.history.head(branch).ok().cloned();
        let (reviews, _) = read_reviews(&log_path(self.root.path()))?;
        let mut listed: Vec<(usize, GraphReviewStatus)> = reviews
            .into_iter()
            .enumerate()
            .filter(|(_, review)| review.branch == branch)
            .map(|(position, review)| {
                let is_current = current.as_ref() == Some(&review.head);
                (
                    position,
                    GraphReviewStatus {
                        review,
                        current: is_current,
                    },
                )
            })
            .collect();
        listed.sort_by(|(left_position, left), (right_position, right)| {
            right
                .review
                .at_ms
                .cmp(&left.review.at_ms)
                .then(right_position.cmp(left_position))
        });
        Ok(listed.into_iter().map(|(_, status)| status).collect())
    }
}

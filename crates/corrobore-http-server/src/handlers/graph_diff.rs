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

//! Diff, merge-base and change-feed routes (issue #320).

use std::collections::BTreeSet;

use axum::{
    Json,
    extract::{Query, State},
};
use graph_core::PropertyMap;
use graph_storage::{
    CanonicalEngineStore, DiffValues, GraphCommitId, GraphDiffEntry, HistoryRecordKind,
    MAIN_BRANCH, RecordChangeKind,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    app::AppState,
    error::ApiError,
    handlers::{
        graph_history::{
            CommitChangeView, CommitDetailView, CommitSummaryView, Envelope, kind_name,
            map_history_error, ok, with_store,
        },
        graph_records::{
            NodeRecordView, RelationshipRecordView, node_view, property_json, relationship_view,
        },
    },
};

const DEFAULT_DIFF_PAGE: usize = 200;
const MAX_DIFF_PAGE: usize = 2_000;
const DEFAULT_FEED_PAGE: usize = 50;
const MAX_FEED_PAGE: usize = 500;

/// Resolve `gc…` to that commit and anything else to a branch head.
pub(crate) fn resolve_ref(
    store: &mut CanonicalEngineStore,
    value: &str,
) -> Result<GraphCommitId, ApiError> {
    let value = value.trim();
    match GraphCommitId::new(value) {
        Ok(commit) => store
            .resolve_graph_ref(None, Some(&commit))
            .map_err(map_history_error),
        Err(_) => store.graph_head(value).map_err(map_history_error),
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct DiffQuery {
    /// Base: a commit id or a branch name.
    pub from: String,
    /// Target: a commit id or a branch name.
    pub to: String,
    /// `node` or `relationship`; both when absent.
    pub kind: Option<String>,
    /// `node:<id>` or `relationship:<id>` from the previous page.
    pub cursor: Option<String>,
    /// Page size, 1-2000. Defaults to 200.
    pub limit: Option<usize>,
    /// Include both record versions and their property changes.
    pub include_values: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct PropertyChangeView {
    pub key: String,
    pub before: Option<Value>,
    pub after: Option<Value>,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum DiffRecordView {
    Node(NodeRecordView),
    Relationship(RelationshipRecordView),
}

#[derive(Debug, Serialize)]
pub struct DiffEntryView {
    pub kind: &'static str,
    pub record_id: String,
    pub change: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_version: Option<String>,
    pub labels: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<DiffRecordView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<DiffRecordView>,
    /// Properties whose value differs; present with `include_values`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub property_changes: Option<Vec<PropertyChangeView>>,
    /// Native fields (`labels`, `status`, `confidence`) whose value differs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_changes: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct DiffTotalsView {
    pub added: usize,
    pub changed: usize,
    pub removed: usize,
}

#[derive(Debug, Serialize)]
pub struct DiffResult {
    pub from: String,
    pub to: String,
    pub totals: DiffTotalsView,
    pub entries: Vec<DiffEntryView>,
    pub next_cursor: Option<String>,
}

fn change_name(change: RecordChangeKind) -> &'static str {
    match change {
        RecordChangeKind::Added => "added",
        RecordChangeKind::Changed => "changed",
        RecordChangeKind::Removed => "removed",
    }
}

pub(crate) fn property_changes(
    before: Option<&PropertyMap>,
    after: Option<&PropertyMap>,
) -> Vec<PropertyChangeView> {
    let keys: BTreeSet<&String> = before
        .into_iter()
        .flat_map(|map| map.keys())
        .chain(after.into_iter().flat_map(|map| map.keys()))
        .collect();
    keys.into_iter()
        .filter_map(|key| {
            let left = before.and_then(|map| map.get(key));
            let right = after.and_then(|map| map.get(key));
            (left != right).then(|| PropertyChangeView {
                key: key.clone(),
                before: left.map(property_json),
                after: right.map(property_json),
            })
        })
        .collect()
}

pub(crate) fn diff_entry_view(entry: &GraphDiffEntry) -> DiffEntryView {
    let (source, target) = entry
        .endpoints
        .clone()
        .map_or((None, None), |(source, target)| {
            (Some(source), Some(target))
        });
    let mut view = DiffEntryView {
        kind: kind_name(entry.kind),
        record_id: entry.record_id.clone(),
        change: change_name(entry.change),
        before_version: entry.before_version.clone(),
        after_version: entry.after_version.clone(),
        labels: entry.labels.clone(),
        source,
        target,
        before: None,
        after: None,
        property_changes: None,
        field_changes: None,
    };
    match &entry.values {
        Some(DiffValues::Node { before, after }) => {
            view.property_changes = Some(property_changes(
                before.as_deref().map(graph_core::Node::properties),
                after.as_deref().map(graph_core::Node::properties),
            ));
            let mut fields = Vec::new();
            if let (Some(left), Some(right)) = (before.as_deref(), after.as_deref()) {
                if left.labels() != right.labels() {
                    fields.push("labels".to_owned());
                }
                if left.status() != right.status() {
                    fields.push("status".to_owned());
                }
                if left.confidence() != right.confidence() {
                    fields.push("confidence".to_owned());
                }
            }
            view.field_changes = Some(fields);
            view.before = before
                .as_deref()
                .map(|node| DiffRecordView::Node(node_view(node)));
            view.after = after
                .as_deref()
                .map(|node| DiffRecordView::Node(node_view(node)));
        }
        Some(DiffValues::Relationship { before, after }) => {
            view.property_changes = Some(property_changes(
                before.as_deref().map(graph_core::Relationship::properties),
                after.as_deref().map(graph_core::Relationship::properties),
            ));
            let mut fields = Vec::new();
            if let (Some(left), Some(right)) = (before.as_deref(), after.as_deref()) {
                if left.status() != right.status() {
                    fields.push("status".to_owned());
                }
                if left.confidence() != right.confidence() {
                    fields.push("confidence".to_owned());
                }
            }
            view.field_changes = Some(fields);
            view.before = before
                .as_deref()
                .map(|relationship| DiffRecordView::Relationship(relationship_view(relationship)));
            view.after = after
                .as_deref()
                .map(|relationship| DiffRecordView::Relationship(relationship_view(relationship)));
        }
        None => {}
    }
    view
}

fn parse_kind(kind: Option<&str>) -> Result<Option<HistoryRecordKind>, ApiError> {
    match kind.map(str::trim) {
        None | Some("") => Ok(None),
        Some("node" | "nodes") => Ok(Some(HistoryRecordKind::Node)),
        Some("relationship" | "relationships") => Ok(Some(HistoryRecordKind::Relationship)),
        Some(other) => Err(ApiError::bad_request(
            "INVALID_RECORD_KIND",
            format!("kind must be node or relationship, not {other:?}"),
        )),
    }
}

fn validate_cursor(cursor: Option<&str>) -> Result<(), ApiError> {
    match cursor {
        None => Ok(()),
        Some(cursor) if cursor.starts_with("node:") || cursor.starts_with("relationship:") => {
            Ok(())
        }
        Some(cursor) => Err(ApiError::bad_request(
            "INVALID_DIFF_CURSOR",
            format!("diff cursor {cursor:?} must be node:<id> or relationship:<id>"),
        )),
    }
}

/// `GET /v1/graph/diff`: records that differ between two commits or branch heads.
pub async fn diff(
    State(state): State<AppState>,
    Query(query): Query<DiffQuery>,
) -> Result<Json<Envelope<DiffResult>>, ApiError> {
    let kind = parse_kind(query.kind.as_deref())?;
    validate_cursor(query.cursor.as_deref())?;
    let limit = query
        .limit
        .unwrap_or(DEFAULT_DIFF_PAGE)
        .clamp(1, MAX_DIFF_PAGE);
    with_store(&state, move |store| {
        let from = resolve_ref(store, &query.from)?;
        let to = resolve_ref(store, &query.to)?;
        let page = store
            .graph_diff(
                &from,
                &to,
                kind,
                query.cursor.as_deref(),
                limit,
                query.include_values.unwrap_or(false),
            )
            .map_err(map_history_error)?;
        Ok(ok(DiffResult {
            from: page.from.to_string(),
            to: page.to.to_string(),
            totals: DiffTotalsView {
                added: page.totals.added,
                changed: page.totals.changed,
                removed: page.totals.removed,
            },
            entries: page.entries.iter().map(diff_entry_view).collect(),
            next_cursor: page.next_cursor,
        }))
    })
    .await
}

#[derive(Debug, Deserialize)]
pub struct MergeBaseQuery {
    pub a: String,
    pub b: String,
}

#[derive(Debug, Serialize)]
pub struct MergeBaseResult {
    pub a: String,
    pub b: String,
    pub merge_base: Option<String>,
}

/// `GET /v1/graph/merge-base`: best common ancestor of two commits or branch heads.
pub async fn merge_base(
    State(state): State<AppState>,
    Query(query): Query<MergeBaseQuery>,
) -> Result<Json<Envelope<MergeBaseResult>>, ApiError> {
    with_store(&state, move |store| {
        let a = resolve_ref(store, &query.a)?;
        let b = resolve_ref(store, &query.b)?;
        let base = store.graph_merge_base(&a, &b).map_err(map_history_error)?;
        Ok(ok(MergeBaseResult {
            a: a.to_string(),
            b: b.to_string(),
            merge_base: base.map(|base| base.to_string()),
        }))
    })
    .await
}

#[derive(Debug, Default, Deserialize)]
pub struct ChangesQuery {
    /// Branch to follow. Defaults to `main`.
    pub branch: Option<String>,
    /// Last commit already processed; absent starts at the genesis commit.
    pub after: Option<String>,
    /// Commits per page, 1-500. Defaults to 50.
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct ChangesResult {
    pub branch: String,
    pub head: String,
    pub commits: Vec<CommitDetailView>,
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

/// `GET /v1/graph/changes`: commits published on a branch after a cursor.
pub async fn changes(
    State(state): State<AppState>,
    Query(query): Query<ChangesQuery>,
) -> Result<Json<Envelope<ChangesResult>>, ApiError> {
    let branch = query.branch.unwrap_or_else(|| MAIN_BRANCH.to_owned());
    let limit = query
        .limit
        .unwrap_or(DEFAULT_FEED_PAGE)
        .clamp(1, MAX_FEED_PAGE);
    let after = match query.after.as_deref() {
        None | Some("") => None,
        Some(value) => Some(GraphCommitId::new(value).map_err(|_| {
            ApiError::bad_request(
                "INVALID_CHANGE_FEED_CURSOR",
                format!("change feed cursor {value:?} is not a commit id"),
            )
        })?),
    };
    with_store(&state, move |store| {
        let page = store
            .graph_changes(&branch, after.as_ref(), limit)
            .map_err(map_history_error)?;
        Ok(ok(ChangesResult {
            branch: page.branch,
            head: page.head.to_string(),
            commits: page
                .commits
                .iter()
                .map(|commit| CommitDetailView {
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
                    archived: false,
                })
                .collect(),
            next_cursor: page.next_cursor.map(|cursor| cursor.to_string()),
            has_more: page.has_more,
        }))
    })
    .await
}

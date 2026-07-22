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
//! Persisted, read-only snapshot and timeshot lineage for the graph explorer.
//!
//! Snapshot metadata enters this store only through graph-core `Snapshot`
//! records. Timeshots are derived analytical boundaries and never mutate the
//! authoritative snapshot lifecycle.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use chrono::DateTime;
use graph_core::Snapshot;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{session_runtime::SessionHealthView, visualization::VisualizationTemporalBoundary};

/// Stable kind discriminator for explorer temporal records.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExplorerBoundaryKind {
    /// Authoritative graph-core snapshot metadata.
    Snapshot,
    /// Derived read-only analytical point in time.
    Timeshot,
}

/// Persistable temporal boundary read model returned to explorer clients.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExplorerTemporalBoundary {
    /// Stable snapshot or timeshot identifier.
    pub boundary_id: String,
    /// Record kind.
    pub kind: ExplorerBoundaryKind,
    /// Owning session identifier.
    pub session_id: String,
    /// Owning workspace identifier.
    pub workspace_id: String,
    /// Parent snapshot or timeshot identifier.
    pub parent_id: Option<String>,
    /// Optional transaction anchor.
    pub transaction_id: Option<String>,
    /// RFC 3339 temporal boundary.
    pub at: String,
    /// Compact analyst-facing label.
    pub label: String,
}

/// Recursive, acyclic node returned by the explorer timeline endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExplorerTimelineNode {
    /// Temporal boundary represented by this tree node.
    pub boundary: ExplorerTemporalBoundary,
    /// Deterministically ordered children.
    pub children: Vec<ExplorerTimelineNode>,
}

/// Forest of roots for one session/workspace lineage.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExplorerTimeline {
    /// Session whose lineage is represented.
    pub session_id: String,
    /// Workspace owning the session.
    pub workspace_id: String,
    /// Deterministically ordered root boundaries.
    pub roots: Vec<ExplorerTimelineNode>,
}

/// Validated timeshot registration input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExplorerTimeshotInput {
    boundary_id: String,
    parent_id: String,
    transaction_id: Option<String>,
    at: String,
    label: String,
}

impl ExplorerTimeshotInput {
    /// Build an analytical boundary attached to an existing lineage parent.
    pub fn new(
        boundary_id: impl Into<String>,
        parent_id: impl Into<String>,
        transaction_id: Option<impl Into<String>>,
        at: impl Into<String>,
        label: impl Into<String>,
    ) -> Result<Self, ExplorerTimelineError> {
        let boundary_id = required_value("boundary_id", boundary_id.into())?;
        let parent_id = required_value("parent_id", parent_id.into())?;
        let transaction_id = transaction_id
            .map(Into::into)
            .map(|value| required_value("transaction_id", value))
            .transpose()?;
        let at = validated_timestamp("at", at.into())?;
        let label = required_value("label", label.into())?;
        Ok(Self {
            boundary_id,
            parent_id,
            transaction_id,
            at,
            label,
        })
    }
}

/// Selection requested by the graph projection endpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExplorerBoundarySelection {
    /// Current graph state.
    Current,
    /// Named snapshot.
    Snapshot(String),
    /// Named timeshot.
    Timeshot(String),
}

impl ExplorerBoundarySelection {
    /// Select current graph state.
    pub fn current() -> Self {
        Self::Current
    }

    /// Select a snapshot by identifier.
    pub fn snapshot(boundary_id: impl Into<String>) -> Self {
        Self::Snapshot(boundary_id.into())
    }

    /// Select a timeshot by identifier.
    pub fn timeshot(boundary_id: impl Into<String>) -> Self {
        Self::Timeshot(boundary_id.into())
    }
}

/// Typed timeline persistence and lineage failures.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ExplorerTimelineError {
    /// Boundary exists but belongs to another session.
    #[error("boundary {boundary_id} does not belong to requested session {requested_session_id}")]
    BoundarySessionMismatch {
        /// Requested boundary identifier.
        boundary_id: String,
        /// Session used for the lookup.
        requested_session_id: String,
    },
    /// Requested boundary does not exist.
    #[error("temporal boundary not found: {boundary_id}")]
    BoundaryNotFound {
        /// Requested boundary identifier.
        boundary_id: String,
    },
    /// Parent must exist before a child can be registered.
    #[error("parent temporal boundary not found: {parent_id}")]
    ParentBoundaryNotFound {
        /// Requested parent identifier.
        parent_id: String,
    },
    /// Child timestamp precedes its parent timestamp.
    #[error("temporal child {boundary_id} precedes parent {parent_id}")]
    InvalidTemporalOrder {
        /// Child identifier.
        boundary_id: String,
        /// Parent identifier.
        parent_id: String,
    },
    /// Boundary identifier is already registered.
    #[error("temporal boundary already exists: {boundary_id}")]
    BoundaryAlreadyExists {
        /// Duplicate identifier.
        boundary_id: String,
    },
    /// Input field is missing or malformed.
    #[error("invalid explorer timeline field {field}: {message}")]
    InvalidInput {
        /// Invalid field name.
        field: String,
        /// Actionable validation detail.
        message: String,
    },
    /// Timeline persistence failed.
    #[error("explorer timeline persistence failed: {0}")]
    Persistence(String),
    /// Persisted lineage contains a parent cycle.
    #[error("temporal lineage contains a cycle at {boundary_id}")]
    CycleDetected {
        /// Boundary participating in the cycle.
        boundary_id: String,
    },
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct PersistedExplorerTimeline {
    boundaries: Vec<ExplorerTemporalBoundary>,
}

/// Durable temporal navigation read model.
pub struct ExplorerTimelineStore {
    store_file: PathBuf,
    boundaries: BTreeMap<String, ExplorerTemporalBoundary>,
    load_error: Option<String>,
}

impl ExplorerTimelineStore {
    /// Open the store rooted beside session persistence.
    pub fn new(store_dir: impl AsRef<Path>) -> Self {
        let mut store = Self {
            store_file: store_dir.as_ref().join("explorer-timeline.json"),
            boundaries: BTreeMap::new(),
            load_error: None,
        };
        if let Err(error) = store.load_from_disk() {
            store.load_error = Some(error.to_string());
        }
        store
    }

    /// Return the persistence path used by this store.
    pub fn store_file(&self) -> &Path {
        &self.store_file
    }

    /// Record authoritative snapshot metadata in a session lineage.
    pub fn record_snapshot(
        &mut self,
        session: &SessionHealthView,
        snapshot: &Snapshot,
        parent_id: Option<&str>,
    ) -> Result<ExplorerTemporalBoundary, ExplorerTimelineError> {
        self.ensure_ready()?;
        let boundary = ExplorerTemporalBoundary {
            boundary_id: snapshot.id().as_str().to_owned(),
            kind: ExplorerBoundaryKind::Snapshot,
            session_id: session.session_id.clone(),
            workspace_id: session.workspace_id.clone(),
            parent_id: parent_id.map(str::to_owned),
            transaction_id: Some(snapshot.transaction_id().as_str().to_owned()),
            at: validated_timestamp("created_at", snapshot.created_at().as_str().to_owned())?,
            label: required_value("label", snapshot.label().to_owned())?,
        };
        self.insert_boundary(boundary)
    }

    /// Record a derived timeshot under an existing session lineage parent.
    pub fn record_timeshot(
        &mut self,
        session: &SessionHealthView,
        input: ExplorerTimeshotInput,
    ) -> Result<ExplorerTemporalBoundary, ExplorerTimelineError> {
        self.ensure_ready()?;
        let boundary = ExplorerTemporalBoundary {
            boundary_id: input.boundary_id,
            kind: ExplorerBoundaryKind::Timeshot,
            session_id: session.session_id.clone(),
            workspace_id: session.workspace_id.clone(),
            parent_id: Some(input.parent_id),
            transaction_id: input.transaction_id,
            at: input.at,
            label: input.label,
        };
        self.insert_boundary(boundary)
    }

    /// Build the deterministic acyclic tree for a session.
    pub fn timeline_for_session(
        &self,
        session: &SessionHealthView,
    ) -> Result<ExplorerTimeline, ExplorerTimelineError> {
        self.ensure_ready()?;
        let session_boundaries = self
            .boundaries
            .values()
            .filter(|boundary| {
                boundary.session_id == session.session_id
                    && boundary.workspace_id == session.workspace_id
            })
            .map(|boundary| (boundary.boundary_id.clone(), boundary))
            .collect::<BTreeMap<_, _>>();

        let mut children = BTreeMap::<Option<String>, Vec<String>>::new();
        for boundary in session_boundaries.values() {
            if let Some(parent_id) = boundary.parent_id.as_deref() {
                let parent = self.parent_for_session(parent_id, session)?;
                ensure_temporal_order(boundary, parent)?;
            }
            children
                .entry(boundary.parent_id.clone())
                .or_default()
                .push(boundary.boundary_id.clone());
        }
        for child_ids in children.values_mut() {
            child_ids.sort();
        }

        validate_acyclic(&session_boundaries)?;
        let roots = children
            .get(&None)
            .into_iter()
            .flatten()
            .map(|boundary_id| build_tree_node(boundary_id, &session_boundaries, &children))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ExplorerTimeline {
            session_id: session.session_id.clone(),
            workspace_id: session.workspace_id.clone(),
            roots,
        })
    }

    /// Resolve a selection to the exact projection boundary DTO.
    pub fn resolve_boundary(
        &self,
        session: &SessionHealthView,
        selection: &ExplorerBoundarySelection,
    ) -> Result<VisualizationTemporalBoundary, ExplorerTimelineError> {
        self.ensure_ready()?;
        let (expected_kind, boundary_id) = match selection {
            ExplorerBoundarySelection::Current => {
                return Ok(VisualizationTemporalBoundary::current());
            }
            ExplorerBoundarySelection::Snapshot(boundary_id) => {
                (ExplorerBoundaryKind::Snapshot, boundary_id)
            }
            ExplorerBoundarySelection::Timeshot(boundary_id) => {
                (ExplorerBoundaryKind::Timeshot, boundary_id)
            }
        };
        let boundary = self.boundary_for_session(boundary_id, session)?;
        if boundary.kind != expected_kind {
            return Err(ExplorerTimelineError::BoundaryNotFound {
                boundary_id: boundary_id.clone(),
            });
        }

        match boundary.kind {
            ExplorerBoundaryKind::Snapshot => {
                let transaction_id = boundary.transaction_id.as_deref().ok_or_else(|| {
                    ExplorerTimelineError::InvalidInput {
                        field: "transaction_id".to_owned(),
                        message: "snapshot boundaries require a transaction id".to_owned(),
                    }
                })?;
                VisualizationTemporalBoundary::snapshot(
                    &boundary.boundary_id,
                    transaction_id,
                    &boundary.at,
                )
                .map_err(map_projection_boundary_error)
            }
            ExplorerBoundaryKind::Timeshot => VisualizationTemporalBoundary::timeshot(
                &boundary.boundary_id,
                boundary.transaction_id.as_deref(),
                &boundary.at,
            )
            .map_err(map_projection_boundary_error),
        }
    }

    fn insert_boundary(
        &mut self,
        boundary: ExplorerTemporalBoundary,
    ) -> Result<ExplorerTemporalBoundary, ExplorerTimelineError> {
        if self.boundaries.contains_key(&boundary.boundary_id) {
            return Err(ExplorerTimelineError::BoundaryAlreadyExists {
                boundary_id: boundary.boundary_id,
            });
        }
        if let Some(parent_id) = boundary.parent_id.as_deref() {
            let parent =
                self.parent_for_identity(parent_id, &boundary.session_id, &boundary.workspace_id)?;
            ensure_temporal_order(&boundary, parent)?;
        }

        let boundary_id = boundary.boundary_id.clone();
        self.boundaries
            .insert(boundary_id.clone(), boundary.clone());
        if let Err(error) = self.persist_to_disk() {
            self.boundaries.remove(&boundary_id);
            return Err(error);
        }
        Ok(boundary)
    }

    fn boundary_for_session<'a>(
        &'a self,
        boundary_id: &str,
        session: &SessionHealthView,
    ) -> Result<&'a ExplorerTemporalBoundary, ExplorerTimelineError> {
        let boundary = self.boundaries.get(boundary_id).ok_or_else(|| {
            ExplorerTimelineError::BoundaryNotFound {
                boundary_id: boundary_id.to_owned(),
            }
        })?;
        if boundary.session_id != session.session_id
            || boundary.workspace_id != session.workspace_id
        {
            return Err(ExplorerTimelineError::BoundarySessionMismatch {
                boundary_id: boundary_id.to_owned(),
                requested_session_id: session.session_id.clone(),
            });
        }
        Ok(boundary)
    }

    fn parent_for_session<'a>(
        &'a self,
        parent_id: &str,
        session: &SessionHealthView,
    ) -> Result<&'a ExplorerTemporalBoundary, ExplorerTimelineError> {
        match self.boundary_for_session(parent_id, session) {
            Err(ExplorerTimelineError::BoundaryNotFound { .. }) => {
                Err(ExplorerTimelineError::ParentBoundaryNotFound {
                    parent_id: parent_id.to_owned(),
                })
            }
            result => result,
        }
    }

    fn parent_for_identity<'a>(
        &'a self,
        parent_id: &str,
        session_id: &str,
        workspace_id: &str,
    ) -> Result<&'a ExplorerTemporalBoundary, ExplorerTimelineError> {
        let parent = self.boundaries.get(parent_id).ok_or_else(|| {
            ExplorerTimelineError::ParentBoundaryNotFound {
                parent_id: parent_id.to_owned(),
            }
        })?;
        if parent.session_id != session_id || parent.workspace_id != workspace_id {
            return Err(ExplorerTimelineError::BoundarySessionMismatch {
                boundary_id: parent_id.to_owned(),
                requested_session_id: session_id.to_owned(),
            });
        }
        Ok(parent)
    }

    fn ensure_ready(&self) -> Result<(), ExplorerTimelineError> {
        match &self.load_error {
            Some(message) => Err(ExplorerTimelineError::Persistence(message.clone())),
            None => Ok(()),
        }
    }

    fn load_from_disk(&mut self) -> Result<(), ExplorerTimelineError> {
        if !self.store_file.exists() {
            return Ok(());
        }
        let payload = fs::read(&self.store_file)
            .map_err(|error| ExplorerTimelineError::Persistence(error.to_string()))?;
        let persisted: PersistedExplorerTimeline = serde_json::from_slice(&payload)
            .map_err(|error| ExplorerTimelineError::Persistence(error.to_string()))?;
        for boundary in persisted.boundaries {
            if self
                .boundaries
                .insert(boundary.boundary_id.clone(), boundary.clone())
                .is_some()
            {
                return Err(ExplorerTimelineError::BoundaryAlreadyExists {
                    boundary_id: boundary.boundary_id,
                });
            }
        }
        Ok(())
    }

    fn persist_to_disk(&self) -> Result<(), ExplorerTimelineError> {
        let parent = self.store_file.parent().ok_or_else(|| {
            ExplorerTimelineError::Persistence("timeline store path has no parent".to_owned())
        })?;
        fs::create_dir_all(parent)
            .map_err(|error| ExplorerTimelineError::Persistence(error.to_string()))?;
        let persisted = PersistedExplorerTimeline {
            boundaries: self.boundaries.values().cloned().collect(),
        };
        let payload = serde_json::to_vec_pretty(&persisted)
            .map_err(|error| ExplorerTimelineError::Persistence(error.to_string()))?;
        let temporary_file = self.store_file.with_extension("json.tmp");
        fs::write(&temporary_file, payload)
            .map_err(|error| ExplorerTimelineError::Persistence(error.to_string()))?;
        fs::rename(&temporary_file, &self.store_file)
            .map_err(|error| ExplorerTimelineError::Persistence(error.to_string()))
    }
}

fn required_value(field: &str, value: String) -> Result<String, ExplorerTimelineError> {
    if value.trim().is_empty() {
        return Err(ExplorerTimelineError::InvalidInput {
            field: field.to_owned(),
            message: format!("provide a non-empty {field}"),
        });
    }
    Ok(value)
}

fn validated_timestamp(field: &str, value: String) -> Result<String, ExplorerTimelineError> {
    let value = required_value(field, value)?;
    DateTime::parse_from_rfc3339(&value).map_err(|_| ExplorerTimelineError::InvalidInput {
        field: field.to_owned(),
        message: "provide an RFC 3339 timestamp".to_owned(),
    })?;
    Ok(value)
}

fn ensure_temporal_order(
    child: &ExplorerTemporalBoundary,
    parent: &ExplorerTemporalBoundary,
) -> Result<(), ExplorerTimelineError> {
    let child_at = DateTime::parse_from_rfc3339(&child.at).map_err(|_| {
        ExplorerTimelineError::InvalidInput {
            field: "at".to_owned(),
            message: "provide an RFC 3339 timestamp".to_owned(),
        }
    })?;
    let parent_at = DateTime::parse_from_rfc3339(&parent.at).map_err(|_| {
        ExplorerTimelineError::InvalidInput {
            field: "parent.at".to_owned(),
            message: "persisted parent must contain an RFC 3339 timestamp".to_owned(),
        }
    })?;
    if child_at < parent_at {
        return Err(ExplorerTimelineError::InvalidTemporalOrder {
            boundary_id: child.boundary_id.clone(),
            parent_id: parent.boundary_id.clone(),
        });
    }
    Ok(())
}

fn validate_acyclic(
    boundaries: &BTreeMap<String, &ExplorerTemporalBoundary>,
) -> Result<(), ExplorerTimelineError> {
    for boundary_id in boundaries.keys() {
        let mut visited = BTreeSet::new();
        let mut cursor = Some(boundary_id.as_str());
        while let Some(current_id) = cursor {
            if !visited.insert(current_id.to_owned()) {
                return Err(ExplorerTimelineError::CycleDetected {
                    boundary_id: current_id.to_owned(),
                });
            }
            cursor = boundaries
                .get(current_id)
                .and_then(|boundary| boundary.parent_id.as_deref());
        }
    }
    Ok(())
}

fn build_tree_node(
    boundary_id: &str,
    boundaries: &BTreeMap<String, &ExplorerTemporalBoundary>,
    children: &BTreeMap<Option<String>, Vec<String>>,
) -> Result<ExplorerTimelineNode, ExplorerTimelineError> {
    let boundary =
        boundaries
            .get(boundary_id)
            .ok_or_else(|| ExplorerTimelineError::BoundaryNotFound {
                boundary_id: boundary_id.to_owned(),
            })?;
    let child_nodes = children
        .get(&Some(boundary_id.to_owned()))
        .into_iter()
        .flatten()
        .map(|child_id| build_tree_node(child_id, boundaries, children))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ExplorerTimelineNode {
        boundary: (*boundary).clone(),
        children: child_nodes,
    })
}

fn map_projection_boundary_error(
    error: crate::visualization::VisualizationProjectionError,
) -> ExplorerTimelineError {
    ExplorerTimelineError::InvalidInput {
        field: "boundary".to_owned(),
        message: error.to_string(),
    }
}

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
use std::collections::HashMap;

use crate::{ActorId, GraphError, SnapshotId, TemporalTimestamp, TransactionId};

/// Logical snapshot checkpoint metadata captured at a transaction boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    id: SnapshotId,
    transaction_id: TransactionId,
    created_at: TemporalTimestamp,
    created_by: ActorId,
    reason: String,
    label: String,
    exporter_version: Option<String>,
    profile: Option<String>,
    metadata: HashMap<String, String>,
}

impl Snapshot {
    /// Id.
    pub fn id(&self) -> &SnapshotId {
        &self.id
    }

    /// Transaction id.
    pub fn transaction_id(&self) -> &TransactionId {
        &self.transaction_id
    }

    /// Created at.
    pub fn created_at(&self) -> &TemporalTimestamp {
        &self.created_at
    }

    /// Created by.
    pub fn created_by(&self) -> &ActorId {
        &self.created_by
    }

    /// Reason.
    pub fn reason(&self) -> &str {
        self.reason.as_str()
    }

    /// Label.
    pub fn label(&self) -> &str {
        self.label.as_str()
    }

    /// Exporter version.
    pub fn exporter_version(&self) -> Option<&str> {
        self.exporter_version.as_deref()
    }

    /// Profile.
    pub fn profile(&self) -> Option<&str> {
        self.profile.as_deref()
    }

    /// Metadata.
    pub fn metadata(&self) -> &HashMap<String, String> {
        &self.metadata
    }
}

/// Input contract used to create a snapshot record in the manager.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotCreateRequest {
    id: SnapshotId,
    transaction_id: TransactionId,
    created_by: ActorId,
    reason: String,
    label: String,
    exporter_version: Option<String>,
    profile: Option<String>,
    metadata: HashMap<String, String>,
}

impl SnapshotCreateRequest {
    /// Creates a new instance.
    pub fn new(
        id: SnapshotId,
        transaction_id: TransactionId,
        created_by: ActorId,
        reason: impl Into<String>,
        label: impl Into<String>,
    ) -> Result<Self, GraphError> {
        let reason = reason.into();
        if reason.trim().is_empty() {
            return Err(GraphError::InvalidPropertyValue(
                "snapshot create request field must not be empty: reason".to_owned(),
            ));
        }

        let label = label.into();
        if label.trim().is_empty() {
            return Err(GraphError::InvalidPropertyValue(
                "snapshot create request field must not be empty: label".to_owned(),
            ));
        }

        if created_by.as_str().trim().is_empty() {
            return Err(GraphError::InvalidPropertyValue(
                "snapshot create request field must not be empty: created_by".to_owned(),
            ));
        }

        Ok(Self {
            id,
            transaction_id,
            created_by,
            reason,
            label,
            exporter_version: None,
            profile: None,
            metadata: HashMap::new(),
        })
    }

    /// Sets the export context.
    pub fn with_export_context(
        mut self,
        exporter_version: impl Into<String>,
        profile: impl Into<String>,
    ) -> Self {
        self.exporter_version = Some(exporter_version.into());
        self.profile = Some(profile.into());
        self
    }

    /// Sets the metadata.
    pub fn with_metadata(mut self, metadata: HashMap<String, String>) -> Self {
        self.metadata = metadata;
        self
    }
}

/// In-memory snapshot lifecycle manager for logical checkpoint metadata.
#[derive(Default)]
pub struct SnapshotManager {
    snapshots: Vec<Snapshot>,
    snapshot_positions: HashMap<String, usize>,
}

impl SnapshotManager {
    /// Creates a new instance.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates the snapshot.
    pub fn create_snapshot(
        &mut self,
        request: SnapshotCreateRequest,
        created_at: impl Into<String>,
    ) -> Result<Snapshot, GraphError> {
        let snapshot_key = request.id.as_str().to_owned();
        if self.snapshot_positions.contains_key(snapshot_key.as_str()) {
            return Err(GraphError::InvalidPropertyValue(format!(
                "snapshot already exists: {}",
                snapshot_key
            )));
        }

        let created_at = TemporalTimestamp::new(created_at)?;

        let snapshot = Snapshot {
            id: request.id,
            transaction_id: request.transaction_id,
            created_at,
            created_by: request.created_by,
            reason: request.reason,
            label: request.label,
            exporter_version: request.exporter_version,
            profile: request.profile,
            metadata: request.metadata,
        };

        self.snapshots.push(snapshot.clone());
        self.snapshot_positions
            .insert(snapshot_key, self.snapshots.len() - 1);

        Ok(snapshot)
    }

    /// Snapshot.
    pub fn snapshot(&self, id: &SnapshotId) -> Option<&Snapshot> {
        self.snapshot_positions
            .get(id.as_str())
            .and_then(|index| self.snapshots.get(*index))
    }

    /// List snapshots.
    pub fn list_snapshots(&self) -> &[Snapshot] {
        self.snapshots.as_slice()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_create_request_rejects_blank_reason_and_label() {
        let blank_reason = SnapshotCreateRequest::new(
            SnapshotId::new("snapshot--blank-reason").expect("snapshot ID should be valid"),
            TransactionId::new("transaction--blank-reason")
                .expect("transaction ID should be valid"),
            ActorId::new("actor--snapshot").expect("actor ID should be valid"),
            " ",
            "baseline",
        )
        .expect_err("blank reason should be rejected");
        assert!(matches!(
        blank_reason,
        GraphError::InvalidPropertyValue(message)
        if message.contains("field must not be empty: reason")
        ));

        let blank_label = SnapshotCreateRequest::new(
            SnapshotId::new("snapshot--blank-label").expect("snapshot ID should be valid"),
            TransactionId::new("transaction--blank-label").expect("transaction ID should be valid"),
            ActorId::new("actor--snapshot").expect("actor ID should be valid"),
            "checkpoint",
            " ",
        )
        .expect_err("blank label should be rejected");
        assert!(matches!(
        blank_label,
        GraphError::InvalidPropertyValue(message)
        if message.contains("field must not be empty: label")
        ));
    }

    #[test]
    fn snapshot_manager_persists_export_context_metadata_and_lookup_accessors() {
        let mut manager = SnapshotManager::new();
        let mut metadata = HashMap::new();
        metadata.insert("scope".to_owned(), "workspace".to_owned());

        let request = SnapshotCreateRequest::new(
            SnapshotId::new("snapshot--accessors").expect("snapshot ID should be valid"),
            TransactionId::new("transaction--accessors").expect("transaction ID should be valid"),
            ActorId::new("actor--snapshot").expect("actor ID should be valid"),
            "checkpoint before export",
            "pre-export",
        )
        .expect("request should be valid")
        .with_export_context("exporter-v1", "stix")
        .with_metadata(metadata.clone());

        let created = manager
            .create_snapshot(request, "2026-07-07T00:00:00Z")
            .expect("snapshot creation should succeed");

        assert_eq!(created.id().as_str(), "snapshot--accessors");
        assert_eq!(created.transaction_id().as_str(), "transaction--accessors");
        assert_eq!(created.created_at().as_str(), "2026-07-07T00:00:00Z");
        assert_eq!(created.created_by().as_str(), "actor--snapshot");
        assert_eq!(created.reason(), "checkpoint before export");
        assert_eq!(created.label(), "pre-export");
        assert_eq!(created.exporter_version(), Some("exporter-v1"));
        assert_eq!(created.profile(), Some("stix"));
        assert_eq!(created.metadata(), &metadata);

        let looked_up = manager
            .snapshot(&SnapshotId::new("snapshot--accessors").expect("snapshot ID should be valid"))
            .expect("snapshot should be retrievable by id");
        assert_eq!(looked_up.id(), created.id());
        assert_eq!(manager.list_snapshots().len(), 1);
    }

    #[test]
    fn snapshot_manager_rejects_invalid_created_at_timestamp() {
        let mut manager = SnapshotManager::new();
        let request = SnapshotCreateRequest::new(
            SnapshotId::new("snapshot--invalid-created-at").expect("snapshot ID should be valid"),
            TransactionId::new("transaction--invalid-created-at")
                .expect("transaction ID should be valid"),
            ActorId::new("actor--snapshot").expect("actor ID should be valid"),
            "checkpoint",
            "baseline",
        )
        .expect("request should be valid");

        let error = manager
            .create_snapshot(request, "not-a-timestamp")
            .expect_err("invalid created_at should be rejected");

        assert!(matches!(
        error,
        GraphError::InvalidPropertyValue(message)
        if message.contains("invalid RFC3339 timestamp")
        ));
    }

    //
    // Verify duplicate snapshot IDs are rejected by the lifecycle manager.
    #[test]
    fn snapshot_manager_rejects_duplicate_snapshot_id() {
        let mut manager = SnapshotManager::new();

        let request = SnapshotCreateRequest::new(
            SnapshotId::new("snapshot--duplicate").expect("snapshot ID should be valid"),
            TransactionId::new("transaction--dup").expect("transaction ID should be valid"),
            ActorId::new("actor--manager").expect("actor ID should be valid"),
            "checkpoint",
            "baseline",
        )
        .expect("request should be valid");

        manager
            .create_snapshot(request.clone(), "2026-07-06T15:10:00Z")
            .expect("first snapshot should be accepted");

        let error = manager
            .create_snapshot(request, "2026-07-06T15:11:00Z")
            .expect_err("duplicate snapshot ID should be rejected");

        assert!(matches!(
        error,
        GraphError::InvalidPropertyValue(message)
        if message == "snapshot already exists: snapshot--duplicate"
        ));
    }
}

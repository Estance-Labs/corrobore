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
#![warn(missing_docs)]

//! Storage abstraction boundary contracts for the intelligence graph engine.
//!
//! This crate declares stable API shapes that decouple storage callers from
//! concrete implementations such as `graph-storage`. It intentionally owns only
//! contract types and traits, not file formats or persistence behavior.

use graph_core::GraphError;
use serde::{Deserialize, Serialize};
use thiserror::Error;

mod in_memory;

pub use in_memory::InMemoryStorageBoundary;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
/// Storage record kind.
pub enum StorageRecordKind {
    /// Node version.
    NodeVersion,
    /// Relationship version.
    RelationshipVersion,
    /// Evidence.
    Evidence,
    /// Validation error.
    ValidationError,
    /// Audit event.
    AuditEvent,
    /// Snapshot.
    Snapshot,
    /// Export metadata.
    ExportMetadata,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Storage ref.
pub struct StorageRef {
    /// Segment.
    pub segment: String,
    /// Offset.
    pub offset: u64,
    /// Length.
    pub length: u32,
}

impl StorageRef {
    /// Creates a new instance.
    pub fn new(
        segment: impl Into<String>,
        offset: u64,
        length: u32,
    ) -> Result<Self, StorageApiError> {
        let segment = segment.into();
        if segment.trim().is_empty() {
            return Err(StorageApiError::InvalidStorageRefField("segment"));
        }
        if length == 0 {
            return Err(StorageApiError::InvalidStorageRefField("length"));
        }

        Ok(Self {
            segment,
            offset,
            length,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Version transition.
pub struct VersionTransition {
    /// Affected id.
    pub affected_id: String,
    /// Before version id.
    pub before_version_id: Option<String>,
    /// After version id.
    pub after_version_id: String,
}

impl VersionTransition {
    /// Creates a new instance.
    pub fn new(
        affected_id: impl Into<String>,
        before_version_id: Option<String>,
        after_version_id: impl Into<String>,
    ) -> Result<Self, StorageApiError> {
        let affected_id = affected_id.into();
        if affected_id.trim().is_empty() {
            return Err(StorageApiError::InvalidVersionTransitionField(
                "affected_id",
            ));
        }

        let after_version_id = after_version_id.into();
        if after_version_id.trim().is_empty() {
            return Err(StorageApiError::InvalidVersionTransitionField(
                "after_version_id",
            ));
        }

        if let Some(before) = &before_version_id
            && before.trim().is_empty()
        {
            return Err(StorageApiError::InvalidVersionTransitionField(
                "before_version_id",
            ));
        }

        Ok(Self {
            affected_id,
            before_version_id,
            after_version_id,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Mutation audit fingerprint.
pub struct MutationAuditFingerprint {
    /// Query text hash.
    pub query_text_hash: String,
    /// Transitions.
    pub transitions: Vec<VersionTransition>,
}

impl MutationAuditFingerprint {
    /// Creates a new instance.
    pub fn new(
        query_text_hash: impl Into<String>,
        transitions: Vec<VersionTransition>,
    ) -> Result<Self, StorageApiError> {
        let query_text_hash = query_text_hash.into();
        if query_text_hash.trim().is_empty() {
            return Err(StorageApiError::InvalidAuditFingerprintField(
                "query_text_hash",
            ));
        }

        Ok(Self {
            query_text_hash,
            transitions,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Append record request.
pub struct AppendRecordRequest {
    /// Record kind.
    pub record_kind: StorageRecordKind,
    /// Record id.
    pub record_id: String,
    /// Payload.
    pub payload: Vec<u8>,
}

impl AppendRecordRequest {
    /// Creates a new instance.
    pub fn new(
        record_kind: StorageRecordKind,
        record_id: impl Into<String>,
        payload: Vec<u8>,
    ) -> Result<Self, StorageApiError> {
        let record_id = record_id.into();
        if record_id.trim().is_empty() {
            return Err(StorageApiError::InvalidAppendRecordField("record_id"));
        }
        if payload.is_empty() {
            return Err(StorageApiError::InvalidAppendRecordField("payload"));
        }

        Ok(Self {
            record_kind,
            record_id,
            payload,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Resolve latest request.
pub struct ResolveLatestRequest {
    /// Record kind.
    pub record_kind: StorageRecordKind,
    /// Record id.
    pub record_id: String,
}

impl ResolveLatestRequest {
    /// Creates a new instance.
    pub fn new(
        record_kind: StorageRecordKind,
        record_id: impl Into<String>,
    ) -> Result<Self, StorageApiError> {
        let record_id = record_id.into();
        if record_id.trim().is_empty() {
            return Err(StorageApiError::InvalidResolveLatestField("record_id"));
        }

        Ok(Self {
            record_kind,
            record_id,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Read record request.
pub struct ReadRecordRequest {
    /// Storage ref.
    pub storage_ref: StorageRef,
}

impl ReadRecordRequest {
    /// Creates a new instance.
    pub fn new(storage_ref: StorageRef) -> Self {
        Self { storage_ref }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Read record response.
pub struct ReadRecordResponse {
    /// Record kind.
    pub record_kind: StorageRecordKind,
    /// Record id.
    pub record_id: String,
    /// Payload.
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Record persist result.
pub struct RecordPersistResult {
    /// Storage ref.
    pub storage_ref: StorageRef,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Flush result.
pub struct FlushResult {
    /// Flushed segments.
    pub flushed_segments: usize,
}

/// Storage boundary.
pub trait StorageBoundary {
    /// Appends a record to the storage backend.
    fn append_record(
        &mut self,
        request: AppendRecordRequest,
    ) -> Result<RecordPersistResult, StorageApiError>;
    /// Resolves the latest storage reference for a given record.
    fn resolve_latest(
        &self,
        request: ResolveLatestRequest,
    ) -> Result<Option<StorageRef>, StorageApiError>;
    /// Reads a record from storage.
    fn read_record(
        &self,
        request: ReadRecordRequest,
    ) -> Result<ReadRecordResponse, StorageApiError>;
    /// Flushes buffered writes to durable storage.
    fn flush(&mut self) -> Result<FlushResult, StorageApiError>;
}

#[derive(Clone, Debug, Error, PartialEq)]
/// Storage api error.
pub enum StorageApiError {
    #[error("invalid storage reference field: {0}")]
    /// Invalid storage ref field.
    InvalidStorageRefField(&'static str),

    #[error("invalid append-record field: {0}")]
    /// Invalid append record field.
    InvalidAppendRecordField(&'static str),

    #[error("invalid resolve-latest field: {0}")]
    /// Invalid resolve latest field.
    InvalidResolveLatestField(&'static str),

    #[error("invalid version transition field: {0}")]
    /// Invalid version transition field.
    InvalidVersionTransitionField(&'static str),

    #[error("invalid audit fingerprint field: {0}")]
    /// Invalid audit fingerprint field.
    InvalidAuditFingerprintField(&'static str),

    #[error("record not found: {0}")]
    /// Record not found.
    RecordNotFound(String),

    #[error("write failed: {0}")]
    /// Write failed.
    WriteFailed(String),

    #[error("read failed: {0}")]
    /// Read failed.
    ReadFailed(String),

    #[error("flush failed: {0}")]
    /// Flush failed.
    FlushFailed(String),

    #[error("graph-core validation error: {0:?}")]
    /// Graph core validation.
    GraphCoreValidation(GraphError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_ref_rejects_empty_segment() {
        let error = StorageRef::new(" ", 0, 8).expect_err("empty segment should be rejected");

        assert!(matches!(
        error,
        StorageApiError::InvalidStorageRefField(field) if field == "segment"
        ));
    }

    #[test]
    fn storage_ref_rejects_zero_length() {
        let error = StorageRef::new("nodes", 0, 0).expect_err("zero length should be rejected");

        assert!(matches!(
        error,
        StorageApiError::InvalidStorageRefField(field) if field == "length"
        ));
    }

    #[test]
    fn storage_ref_accepts_valid_fields() {
        let storage_ref = StorageRef::new("nodes", 16, 32)
            .expect("valid storage reference fields should be accepted");

        assert_eq!(
            storage_ref,
            StorageRef {
                // Segment.
                segment: "nodes".to_owned(),
                // Offset.
                offset: 16,
                // Length.
                length: 32,
            }
        );
    }

    #[test]
    fn append_record_request_rejects_empty_payload() {
        let error = AppendRecordRequest::new(StorageRecordKind::NodeVersion, "node--1", vec![])
            .expect_err("empty payload should be rejected");

        assert!(matches!(
        error,
        StorageApiError::InvalidAppendRecordField(field) if field == "payload"
        ));
    }

    #[test]
    fn append_record_request_rejects_empty_record_id() {
        let error = AppendRecordRequest::new(StorageRecordKind::NodeVersion, " ", vec![1, 2, 3])
            .expect_err("empty record ID should be rejected");

        assert!(matches!(
        error,
        StorageApiError::InvalidAppendRecordField(field) if field == "record_id"
        ));
    }

    #[test]
    fn append_record_request_accepts_valid_fields() {
        let request = AppendRecordRequest::new(
            StorageRecordKind::RelationshipVersion,
            "relationship--1",
            vec![9, 8, 7],
        )
        .expect("valid append request should be accepted");

        assert_eq!(request.record_kind, StorageRecordKind::RelationshipVersion);
        assert_eq!(request.record_id, "relationship--1");
        assert_eq!(request.payload, vec![9, 8, 7]);
    }

    #[test]
    fn version_transition_rejects_empty_after_version_id() {
        let error = VersionTransition::new("node--1", Some("node-version--1".to_owned()), " ")
            .expect_err("empty after version ID should be rejected");

        assert!(matches!(
        error,
        StorageApiError::InvalidVersionTransitionField(field)
        if field == "after_version_id"
        ));
    }

    #[test]
    fn version_transition_rejects_empty_affected_id() {
        let error =
            VersionTransition::new(" ", Some("node-version--1".to_owned()), "node-version--2")
                .expect_err("empty affected ID should be rejected");

        assert!(matches!(
        error,
        StorageApiError::InvalidVersionTransitionField(field) if field == "affected_id"
        ));
    }

    #[test]
    fn version_transition_rejects_empty_before_version_id_when_present() {
        let error = VersionTransition::new("node--1", Some(" ".to_owned()), "node-version--2")
            .expect_err("empty before-version ID should be rejected when provided");

        assert!(matches!(
        error,
        StorageApiError::InvalidVersionTransitionField(field)
        if field == "before_version_id"
        ));
    }

    #[test]
    fn mutation_audit_fingerprint_rejects_empty_hash() {
        let transition = VersionTransition::new("node--1", None, "node-version--1")
            .expect("transition fixture should be valid");
        let error = MutationAuditFingerprint::new(" ", vec![transition])
            .expect_err("empty query hash should be rejected");

        assert!(matches!(
        error,
        StorageApiError::InvalidAuditFingerprintField(field) if field == "query_text_hash"
        ));
    }

    #[test]
    fn resolve_latest_request_rejects_empty_record_id() {
        let error = ResolveLatestRequest::new(StorageRecordKind::Snapshot, " ")
            .expect_err("empty resolve-latest record ID should be rejected");

        assert!(matches!(
        error,
        StorageApiError::InvalidResolveLatestField(field) if field == "record_id"
        ));
    }

    #[test]
    fn constructors_accept_valid_domain_values() {
        let transition = VersionTransition::new("node--1", None, "node-version--1")
            .expect("valid version transition should be accepted");
        let fingerprint = MutationAuditFingerprint::new("hash-123", vec![transition.clone()])
            .expect("valid audit fingerprint should be accepted");
        let resolve_latest = ResolveLatestRequest::new(StorageRecordKind::NodeVersion, "node--1")
            .expect("valid resolve-latest request should be accepted");
        let read = ReadRecordRequest::new(
            StorageRef::new("nodes", 12, 8).expect("valid storage ref should be accepted"),
        );

        assert_eq!(transition.affected_id, "node--1");
        assert_eq!(transition.before_version_id, None);
        assert_eq!(transition.after_version_id, "node-version--1");
        assert_eq!(fingerprint.query_text_hash, "hash-123");
        assert_eq!(fingerprint.transitions.len(), 1);
        assert_eq!(resolve_latest.record_kind, StorageRecordKind::NodeVersion);
        assert_eq!(resolve_latest.record_id, "node--1");
        assert_eq!(read.storage_ref.segment, "nodes");
    }
}

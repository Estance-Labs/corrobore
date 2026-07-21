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
use std::path::PathBuf;

use graph_core::{AdjacencyDirection, NodeId, RelationshipId, RelationshipType};
use thiserror::Error;

use crate::{PersistedRecordId, PersistedRecordKind, RecordChecksum, StorageRef, StorageSegment};

/// Result alias for persistent graph storage operations.
pub type GraphStorageResult<T> = Result<T, GraphStorageError>;

/// Typed error model for persistent graph storage operations.
///
///
/// - Keep expected storage failures directly matchable by variant.
/// - Distinguish storage-root, manifest, storage-reference, persisted-envelope,
///   record-codec, append-only log, catalog lookup, metadata-index lookup,
///   adjacency lookup, and catalog-rebuild failures.
/// - Reserve explicit missing, corrupted, unsupported, invalid-record, checksum,
///   decode, unexpected-kind, missing-catalog-entry, duplicate-latest, strict
///   adjacency lookup, catalog-rebuild, and IO outcomes before the full persistence
///   implementation is introduced.
/// - Avoid exposing lower-level filesystem or decode errors as the public storage
///   contract.
///
///
/// These variants are contract stubs. Future implementations should connect them to concrete
/// root lookup, manifest decode, reference validation, envelope validation,
/// checksum validation, decode behavior, append-only record log behavior, catalog
/// indexing, catalog lookup, duplicate latest detection, adjacency lookup,
/// catalog rebuild, deterministic empty-adjacency behavior, and compatibility
/// behavior.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum GraphStorageError {
    /// The caller tried to open a graph store root that does not exist.
    #[error("storage root not found: {path:?}")]
    StorageRootNotFound {
        /// Path.
        path: PathBuf,
    },

    /// The caller tried to create a graph store root where one already exists.
    #[error("storage root already exists: {path:?}")]
    StorageRootAlreadyExists {
        /// Path.
        path: PathBuf,
    },

    /// The storage root exists, but its manifest file is missing.
    #[error("storage manifest not found: {path:?}")]
    ManifestNotFound {
        /// Path.
        path: PathBuf,
    },

    /// The manifest file exists, but the storage layer cannot safely decode or
    /// trust its content.
    #[error("storage manifest corrupted: {path:?}: {reason}")]
    ManifestCorrupted {
        /// Path.
        path: PathBuf,
        /// Reason.
        reason: String,
    },

    /// The manifest or encoded record declares a storage version that the current
    /// crate does not support.
    #[error("unsupported storage version: {version}")]
    UnsupportedStorageVersion {
        /// Version.
        version: String,
    },

    /// The manifest or encoded record declares a record format that the current
    /// crate does not support.
    #[error("unsupported record format: {format}")]
    UnsupportedRecordFormat {
        /// Format.
        format: String,
    },

    /// The manifest is structurally valid but semantically inconsistent.
    #[error("invalid storage manifest: {reason}")]
    InvalidManifest {
        /// Reason.
        reason: String,
    },

    /// A storage reference is structurally invalid or cannot safely identify one
    /// loadable unit.
    #[error("invalid storage reference: {storage_ref:?}: {reason}")]
    InvalidStorageRef {
        /// Storage ref.
        storage_ref: StorageRef,
        /// Reason.
        reason: String,
    },

    /// A persisted record envelope is internally inconsistent or unsafe to decode.
    #[error("invalid persisted record envelope: {reason}")]
    InvalidEnvelope {
        /// Reason.
        reason: String,
    },

    /// The checksum stored with an encoded record does not match the checksum
    /// calculated from its canonical bytes.
    #[error("persisted record checksum mismatch: expected {expected:?}, actual {actual:?}")]
    ChecksumMismatch {
        /// Expected.
        expected: RecordChecksum,
        /// Actual.
        actual: RecordChecksum,
    },

    /// The codec could not decode bytes into a trusted persisted record envelope.
    #[error("persisted record decode failed for {format}: {reason}")]
    DecodeFailed {
        /// Format.
        format: String,
        /// Reason.
        reason: String,
    },

    /// A decoded record is valid but belongs to a different kind than the caller
    /// requested from a typed record segment.
    #[error("unexpected persisted record kind: expected {expected:?}, actual {actual:?}")]
    UnexpectedRecordKind {
        /// Expected.
        expected: PersistedRecordKind,
        /// Actual.
        actual: PersistedRecordKind,
    },

    /// The catalog has no latest storage reference for the requested stable node
    /// identifier.
    ///
    ///
    /// - Keep an unknown node catalog entry separate from missing payload bytes or
    ///   corrupted record logs.
    /// - Give future pager callers an explicit lookup failure when a `NodeId`
    ///   cannot be resolved without scanning the append-only log.
    #[error("missing node catalog entry for {node_id:?}")]
    MissingNodeCatalogEntry {
        /// Node id.
        node_id: NodeId,
    },

    /// The catalog has no latest storage reference for the requested stable
    /// relationship identifier.
    ///
    ///
    /// - Keep an unknown relationship catalog entry separate from missing payload
    ///   bytes or corrupted record logs.
    /// - Give future pager callers an explicit lookup failure when a
    ///   `RelationshipId` cannot be resolved without scanning the append-only log.
    #[error("missing relationship catalog entry for {relationship_id:?}")]
    MissingRelationshipCatalogEntry {
        /// Relationship id.
        relationship_id: RelationshipId,
    },

    /// The label index has no entry for a label that the caller expected to be
    /// present.
    ///
    ///
    /// - Keep unknown-label lookup separate from empty graph payloads, corrupted
    ///   record logs, and missing node storage references.
    /// - Give future strict lookup callers a deterministic error when a label does
    ///   not exist in the catalog index.
    /// - Keep label-index absence independent from property, semantic, and
    ///   adjacency indexes.
    #[error("unknown label catalog entry for {label:?}")]
    UnknownLabelCatalogEntry {
        /// Label.
        label: String,
    },

    /// The relationship-type index has no entry for a relationship type that the
    /// caller expected to be present.
    ///
    ///
    /// - Keep unknown relationship-type lookup separate from empty graph payloads,
    ///   corrupted record logs, and missing relationship storage references.
    /// - Give future strict lookup callers a deterministic error when a
    ///   relationship type does not exist in the catalog index.
    /// - Keep relationship-type-index absence independent from property, semantic,
    ///   and adjacency indexes.
    #[error("unknown relationship-type catalog entry for {relationship_type:?}")]
    UnknownRelationshipTypeCatalogEntry {
        /// Relationship type.
        relationship_type: RelationshipType,
    },

    /// The adjacency catalog has no entry for an owner node and direction that a
    /// strict lookup expected to be present.
    ///
    ///
    /// - Keep unknown adjacency separate from a known node that has a deterministic
    ///   empty adjacency list.
    /// - Preserve direction in the error so incoming and outgoing failures can be
    ///   handled independently.
    /// - Avoid falling back to loading full node or relationship payloads just to
    ///   discover that adjacency metadata is unavailable.
    #[error("unknown {direction:?} adjacency catalog entry for {node_id:?}")]
    UnknownNodeAdjacencyCatalogEntry {
        /// Node id.
        node_id: NodeId,
        /// Direction.
        direction: AdjacencyDirection,
    },

    /// Two different storage references both claim to be the latest persisted
    /// location for the same stable graph record.
    ///
    ///
    /// - Reserve a deterministic consistency failure for catalog indexing and
    ///   rebuild.
    /// - Prevent later implementations from silently replacing a latest record
    ///   when the candidate is not a normal version advancement.
    /// - Keep duplicate latest detection focused on node and relationship latest
    ///   records; label, type, and adjacency indexes belong to later issues.
    /// - Box the storage references so this rare error does not inflate every
    ///   `GraphStorageResult` and trip Clippy's `result_large_err` lint.
    #[error(
        "duplicate latest persisted record for {record_id:?}: existing {existing_ref:?}, conflicting {conflicting_ref:?}"
    )]
    DuplicateLatestRecordConflict {
        /// Record id.
        record_id: PersistedRecordId,
        /// Existing ref.
        existing_ref: Box<StorageRef>,
        /// Conflicting ref.
        conflicting_ref: Box<StorageRef>,
    },

    /// A required persisted source for catalog rebuild is missing.
    ///
    ///
    /// - Keep rebuild source absence separate from an empty but valid log.
    /// - Report which persisted segment and path prevented rebuild from continuing.
    /// - Prevent reopen and recovery code from treating missing rebuild inputs as a
    ///   successful empty catalog.
    #[error("catalog rebuild source missing for {segment:?} at {path:?}")]
    CatalogRebuildSourceMissing {
        /// Segment.
        segment: StorageSegment,
        /// Path.
        path: PathBuf,
    },

    /// A persisted record could not be trusted while rebuilding the catalog.
    ///
    ///
    /// - Keep corrupted rebuild records distinct from missing catalog entries and
    ///   unknown lookup keys.
    /// - Preserve the source segment and optional storage reference for actionable
    ///   recovery diagnostics.
    /// - Ensure rebuild fails explicitly instead of accepting partial silent
    ///   catalog state.
    /// - Box the storage reference so this rare error does not inflate every
    ///   `GraphStorageResult` and trip Clippy's `result_large_err` lint.
    #[error("catalog rebuild corrupted record in {segment:?} at {storage_ref:?}: {reason}")]
    CatalogRebuildCorruptedRecord {
        /// Segment.
        segment: StorageSegment,
        /// Storage ref.
        storage_ref: Option<Box<StorageRef>>,
        /// Reason.
        reason: String,
    },

    /// Catalog rebuild failed at a named stage before a more specific error could
    /// be returned.
    ///
    ///
    /// - Provide a typed fallback for actionable rebuild errors.
    /// - Preserve the rebuild stage so callers can distinguish scan, decode,
    ///   reconstruction, and validation failures.
    /// - Avoid mapping rebuild failures to generic operation failures when the
    ///   caller needs recovery-specific handling.
    #[error("catalog rebuild failed during {stage}: {reason}")]
    CatalogRebuildFailed {
        /// Stage.
        stage: &'static str,
        /// Reason.
        reason: String,
    },

    /// A lower-level filesystem or stream operation failed while opening,
    /// appending, seeking, writing, or flushing append-only record logs.
    ///
    ///
    /// - Preserve explicit IO mapping for without exposing `std::io::Error`
    ///   as part of the public storage contract.
    /// - Keep the failing operation and optional path available for diagnostics.
    /// - Avoid hiding partial write, permission, missing parent, and flush failures
    ///   behind successful append results.
    #[error("storage IO operation failed during {operation} at {path:?}: {message}")]
    IoOperationFailed {
        /// Operation.
        operation: &'static str,
        /// Path.
        path: Option<PathBuf>,
        /// Message.
        message: String,
    },

    /// A lower-level storage operation failed before it could be mapped to a more
    /// specific storage error.
    #[error("storage operation failed during {operation}: {message}")]
    OperationFailed {
        /// Operation.
        operation: &'static str,
        /// Message.
        message: String,
    },

    /// Temporary phase marker for storage APIs that are intentionally declared
    /// before they are implemented.
    #[error("not implemented yet: {0}")]
    NotImplemented(&'static str),
}

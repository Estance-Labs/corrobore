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
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use tracing::debug;

use crate::{
    EncodedRecord, GraphStorageError, GraphStorageResult, PersistedRecordEnvelope,
    PersistedRecordKind, StorageRef, StorageRoot, StorageSegment,
    validate_persisted_record_envelope, validate_storage_ref,
};

/// Typed append-only record log segment owned by issue 54.
///
///
/// - Distinguish node and relationship append-only logs before concrete file IO is
///   introduced.
/// - Keep adjacency, catalog, snapshot, compaction, and WAL concerns out of this
///   boundary.
/// - Provide a small public type that future implementations can use to route append calls
///   without relying on raw file paths or stringly typed segment names.
///
///
/// Node records must be appended only through `NodeRecords`. Relationship records
/// must be appended only through `RelationshipRecords`. Older bytes must remain in
/// place when a later version of the same graph record is appended.
///
/// # Errors
///
///
/// Later implementation phases must reject segment/kind mismatches explicitly and
/// must not silently redirect writes to a different segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AppendOnlyRecordLogSegment {
    /// Node records.
    NodeRecords,
    /// Relationship records.
    RelationshipRecords,
}

/// Handle for one append-only graph record log.
///
///
/// - Represent a durable append target for encoded graph record envelopes.
/// - Keep the storage root and concrete file layout behind the `graph-storage`
///   boundary.
/// - Preserve a stable place for later offset, length, flush, and IO error behavior
///   without requiring a complete graph load.
///
///
/// The handle points to exactly one append-only segment. Append operations should
/// write encoded bytes at the current end of that segment and return a `StorageRef`
/// containing the segment, deterministic offset, deterministic length, and checksum
/// metadata for the appended bytes.
///
/// # Errors
///
///
/// Opening or appending must surface missing roots, incompatible log segments,
/// invalid encoded envelopes, flush failures, and lower-level IO failures as typed
/// `GraphStorageError` values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppendOnlyRecordLog {
    /// Segment.
    pub segment: AppendOnlyRecordLogSegment,
    /// Path.
    pub path: PathBuf,
}

/// Open the append-only node record log for a storage root.
///
///
/// - Declare the node-record log boundary used by later durable node appends.
/// - Keep node record persistence separate from relationship, adjacency, catalog,
///   and snapshot persistence.
/// - Avoid loading graph nodes while preparing the append target.
///
///
///   Future implementations should resolve the node-record log path under the storage root,
///   create or open the file in append mode, and return an `AppendOnlyRecordLog`
///   configured for `AppendOnlyRecordLogSegment::NodeRecords`.
///
/// # Errors
///
///
/// Missing storage roots, path conflicts, permission failures, and filesystem IO
/// failures must be mapped to explicit `GraphStorageError` variants. This function
/// must not create catalog state or adjacency state.
pub fn open_append_only_node_record_log(
    root: &StorageRoot,
) -> GraphStorageResult<AppendOnlyRecordLog> {
    open_append_only_record_log(root, AppendOnlyRecordLogSegment::NodeRecords)
}

/// Open the append-only relationship record log for a storage root.
///
///
/// - Declare the relationship-record log boundary used by later durable
///   relationship appends.
/// - Keep relationship record persistence separate from node, adjacency, catalog,
///   and snapshot persistence.
/// - Avoid loading graph relationships while preparing the append target.
///
///
///   Future implementations should resolve the relationship-record log path under the storage
///   root, create or open the file in append mode, and return an
///   `AppendOnlyRecordLog` configured for
///   `AppendOnlyRecordLogSegment::RelationshipRecords`.
///
/// # Errors
///
///
/// Missing storage roots, path conflicts, permission failures, and filesystem IO
/// failures must be mapped to explicit `GraphStorageError` variants. This function
/// must not create catalog state or adjacency state.
pub fn open_append_only_relationship_record_log(
    root: &StorageRoot,
) -> GraphStorageResult<AppendOnlyRecordLog> {
    open_append_only_record_log(root, AppendOnlyRecordLogSegment::RelationshipRecords)
}

/// Append one encoded node envelope to the node record log.
///
///
/// - Declare the durable append boundary for node record envelopes.
/// - Preserve append-only semantics by returning a new `StorageRef` instead of
///   mutating an existing record in place.
/// - Keep version retention explicit: appending a newer node version must not
///   overwrite older node bytes.
///
///
/// Future implementations should validate that `log` targets node records, validate that the
/// envelope and encoded bytes describe a node record, append exactly the encoded
/// bytes, calculate deterministic offset and length from the log position, and
/// return the resulting `StorageRef`.
///
/// # Errors
///
///
/// Invalid envelopes, unexpected record kinds, segment mismatches, offset/length
/// overflow, partial writes, flush failures, and lower-level IO failures must be
/// explicit `GraphStorageError` values.
pub fn append_encoded_node_record_envelope(
    log: &mut AppendOnlyRecordLog,
    envelope: &PersistedRecordEnvelope,
    encoded_record: &EncodedRecord,
) -> GraphStorageResult<StorageRef> {
    append_encoded_record_envelope(
        log,
        envelope,
        encoded_record,
        AppendOnlyRecordLogSegment::NodeRecords,
        PersistedRecordKind::Node,
        StorageSegment::NodeRecords,
        "append_encoded_node_record_envelope",
    )
}

/// Append one encoded relationship envelope to the relationship record log.
///
///
/// - Declare the durable append boundary for relationship record envelopes.
/// - Preserve append-only semantics by returning a new `StorageRef` instead of
///   mutating an existing record in place.
/// - Keep version retention explicit: appending a newer relationship version must
///   not overwrite older relationship bytes.
///
///
/// Future implementations should validate that `log` targets relationship records, validate
/// that the envelope and encoded bytes describe a relationship record, append
/// exactly the encoded bytes, calculate deterministic offset and length from the
/// log position, and return the resulting `StorageRef`.
///
/// # Errors
///
///
/// Invalid envelopes, unexpected record kinds, segment mismatches, offset/length
/// overflow, partial writes, flush failures, and lower-level IO failures must be
/// explicit `GraphStorageError` values.
pub fn append_encoded_relationship_record_envelope(
    log: &mut AppendOnlyRecordLog,
    envelope: &PersistedRecordEnvelope,
    encoded_record: &EncodedRecord,
) -> GraphStorageResult<StorageRef> {
    append_encoded_record_envelope(
        log,
        envelope,
        encoded_record,
        AppendOnlyRecordLogSegment::RelationshipRecords,
        PersistedRecordKind::Relationship,
        StorageSegment::RelationshipRecords,
        "append_encoded_relationship_record_envelope",
    )
}

/// Flush an append-only graph record log.
///
///
/// - Reserve a clear durability boundary for append-only record logs.
/// - Let later callers force pending bytes to the underlying storage layer without
///   introducing WAL or transaction semantics in this issue.
/// - Keep flush behavior available for both node and relationship record logs.
///
///
///   Future implementations should flush buffered bytes for the provided log and report success
///   only after the storage layer confirms the requested flush boundary.
///
/// # Errors
///
///
/// Flush failures and lower-level IO failures must be explicit
/// `GraphStorageError` values. This function must not perform compaction, catalog
/// rebuild, adjacency persistence, or transaction log behavior.
pub fn flush_append_only_record_log(log: &mut AppendOnlyRecordLog) -> GraphStorageResult<()> {
    ensure_parent_dir(&log.path, "flush_append_only_record_log")?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log.path)
        .map_err(|error| io_error("flush_append_only_record_log", Some(&log.path), error))?;

    file.flush()
        .map_err(|error| io_error("flush_append_only_record_log", Some(&log.path), error))?;
    file.sync_data()
        .map_err(|error| io_error("flush_append_only_record_log", Some(&log.path), error))
}

/// Open one typed append-only record log.
///
///
/// - Keep node and relationship log opening behind the same checked helper.
/// - Create the concrete log file and its parent directory without loading graph
///   records.
/// - Return a typed handle that later append calls can validate before writing.
///
///
///   The helper rejects missing storage roots, resolves the segment-specific path,
///   creates the parent directory, opens or creates the file in append mode, and
///   returns the corresponding handle.
///
/// # Errors
///
///
/// Missing roots return `StorageRootNotFound`; lower-level filesystem failures are
/// mapped to `IoOperationFailed`.
fn open_append_only_record_log(
    root: &StorageRoot,
    segment: AppendOnlyRecordLogSegment,
) -> GraphStorageResult<AppendOnlyRecordLog> {
    if !root.path().is_dir() {
        return Err(GraphStorageError::StorageRootNotFound {
            path: root.path().to_path_buf(),
        });
    }

    let path = append_log_path(root, segment);
    ensure_parent_dir(&path, "open_append_only_record_log")?;
    OpenOptions::new()
        .create(true)
        .append(true)
        .read(true)
        .open(&path)
        .map_err(|error| io_error("open_append_only_record_log", Some(&path), error))?;

    Ok(AppendOnlyRecordLog { segment, path })
}

/// Append one encoded record to the expected typed log.
///
///
/// - Centralize append-only behavior for node and relationship record logs.
/// - Validate the log segment, envelope kind, encoded kind, compatibility metadata,
///   and encoded byte length before writing any bytes.
/// - Return a deterministic byte-addressable `StorageRef` for the written unit.
///
///
///   The helper appends the encoded bytes at the current file end, flushes the file
///   handle, preserves existing bytes, and returns offset, length, segment, and
///   checksum metadata for the appended unit.
///
/// # Errors
///
///
/// Segment mismatches, unexpected record kinds, envelope/encoded metadata
/// mismatches, empty encoded bytes, offset overflow, write failures, and flush
/// failures are explicit storage errors.
fn append_encoded_record_envelope(
    log: &mut AppendOnlyRecordLog,
    envelope: &PersistedRecordEnvelope,
    encoded_record: &EncodedRecord,
    expected_log_segment: AppendOnlyRecordLogSegment,
    expected_record_kind: PersistedRecordKind,
    storage_segment: StorageSegment,
    operation: &'static str,
) -> GraphStorageResult<StorageRef> {
    debug!(
        operation,
        expected_log_segment = ?expected_log_segment,
        expected_record_kind = ?expected_record_kind,
        encoded_length = encoded_record.bytes.len(),
        "appending encoded record envelope"
    );
    validate_log_segment(log, expected_log_segment, operation)?;
    validate_record_kind(envelope.kind, expected_record_kind)?;
    validate_record_kind(encoded_record.kind, expected_record_kind)?;
    validate_persisted_record_envelope(envelope)?;
    validate_encoded_record_matches_envelope(envelope, encoded_record)?;

    let length = encoded_record.bytes.len() as u64;
    if length == 0 {
        return Err(GraphStorageError::InvalidEnvelope {
            reason: "encoded record bytes must not be empty".to_owned(),
        });
    }

    ensure_parent_dir(&log.path, operation)?;
    let offset = current_file_len(&log.path, operation)?;
    validate_append_offset_range(
        offset,
        length,
        storage_segment.clone(),
        Some(encoded_record.checksum.clone()),
    )?;

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log.path)
        .map_err(|error| io_error(operation, Some(&log.path), error))?;

    file.write_all(&encoded_record.bytes)
        .map_err(|error| io_error(operation, Some(&log.path), error))?;
    file.flush()
        .map_err(|error| io_error(operation, Some(&log.path), error))?;

    let storage_ref = StorageRef {
        segment: storage_segment,
        offset,
        length,
        checksum: Some(encoded_record.checksum.clone()),
    };
    validate_storage_ref(&storage_ref)?;
    debug!(
        operation,
        segment = ?storage_ref.segment,
        offset = storage_ref.offset,
        length = storage_ref.length,
        "encoded record envelope appended"
    );
    Ok(storage_ref)
}

/// Resolve the concrete file path for one append-only record log segment.
///
///
/// - Keep the first file-backed layout deterministic for tests and later catalog
///   hooks.
/// - Preserve node and relationship payload separation.
///
///
///   Node logs resolve under `nodes/node_records.log`; relationship logs resolve
///   under `relationships/relationship_records.log`.
///
/// # Errors
///
///
/// This pure path helper has no error cases.
fn append_log_path(root: &StorageRoot, segment: AppendOnlyRecordLogSegment) -> PathBuf {
    match segment {
        AppendOnlyRecordLogSegment::NodeRecords => {
            root.path().join("nodes").join("node_records.log")
        }
        AppendOnlyRecordLogSegment::RelationshipRecords => root
            .path()
            .join("relationships")
            .join("relationship_records.log"),
    }
}

/// Ensure the parent directory for a log file exists.
///
///
/// - Keep append/open call sites focused on storage semantics instead of directory
///   creation.
/// - Allow tests and later callers to construct typed log handles directly.
///
///
///   Missing parent directories are created recursively. Existing directories are
///   accepted.
///
/// # Errors
///
///
/// Filesystem failures are mapped to `IoOperationFailed` with the requested
/// operation and parent path.
fn ensure_parent_dir(path: &Path, operation: &'static str) -> GraphStorageResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| io_error(operation, Some(parent), error))?;
    }
    Ok(())
}

/// Read the current file length used as the next append offset.
///
///
/// - Make append offsets deterministic and testable.
/// - Treat missing files as empty append-only logs.
///
///
///   Existing files return their byte length. Missing files return zero.
///
/// # Errors
///
///
/// Metadata failures other than missing files are mapped to `IoOperationFailed`.
fn current_file_len(path: &Path, operation: &'static str) -> GraphStorageResult<u64> {
    match fs::metadata(path) {
        Ok(metadata) => Ok(metadata.len()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(io_error(operation, Some(path), error)),
    }
}

fn validate_append_offset_range(
    offset: u64,
    length: u64,
    segment: StorageSegment,
    checksum: Option<crate::RecordChecksum>,
) -> GraphStorageResult<()> {
    let _ = offset
        .checked_add(length)
        .ok_or_else(|| GraphStorageError::InvalidStorageRef {
            storage_ref: StorageRef {
                segment,
                offset,
                length,
                checksum,
            },
            reason: "offset plus length must not overflow".to_owned(),
        })?;
    Ok(())
}

/// Validate that the caller is appending through the expected typed log handle.
///
///
/// - Prevent node bytes from being written into relationship logs or relationship
///   bytes from being written into node logs.
/// - Fail before any filesystem mutation when the typed log is wrong.
///
///
///   Matching segments succeed. Mismatches return a typed storage error.
///
/// # Errors
///
///
/// Segment mismatches return `InvalidEnvelope` with operation context.
fn validate_log_segment(
    log: &AppendOnlyRecordLog,
    expected: AppendOnlyRecordLogSegment,
    operation: &'static str,
) -> GraphStorageResult<()> {
    if log.segment == expected {
        Ok(())
    } else {
        Err(GraphStorageError::InvalidEnvelope {
            reason: format!(
                "{operation} requires {:?} log segment, got {:?}",
                expected, log.segment
            ),
        })
    }
}

/// Validate that an envelope or encoded record belongs to the expected kind.
///
///
/// - Keep typed append APIs from accepting records from the wrong graph segment.
/// - Return the same unexpected-kind error shape used by the codec boundary.
///
///
///   Matching kinds succeed. Mismatches fail before any bytes are written.
///
/// # Errors
///
///
/// Mismatches return `UnexpectedRecordKind`.
fn validate_record_kind(
    actual: PersistedRecordKind,
    expected: PersistedRecordKind,
) -> GraphStorageResult<()> {
    if actual == expected {
        Ok(())
    } else {
        Err(GraphStorageError::UnexpectedRecordKind { expected, actual })
    }
}

/// Validate that encoded record metadata still describes the provided envelope.
///
///
/// - Catch callers passing an envelope and encoded bytes that were not produced
///   together.
/// - Preserve deterministic append behavior by refusing ambiguous metadata.
///
///
///   Storage version, record format, and kind must match between the envelope and
///   encoded record.
///
/// # Errors
///
///
/// Metadata mismatches return `InvalidEnvelope`.
fn validate_encoded_record_matches_envelope(
    envelope: &PersistedRecordEnvelope,
    encoded_record: &EncodedRecord,
) -> GraphStorageResult<()> {
    if envelope.storage_version != encoded_record.storage_version {
        return Err(GraphStorageError::InvalidEnvelope {
            reason: format!(
                "encoded storage version {:?} does not match envelope storage version {:?}",
                encoded_record.storage_version, envelope.storage_version
            ),
        });
    }

    if envelope.record_format != encoded_record.record_format {
        return Err(GraphStorageError::InvalidEnvelope {
            reason: format!(
                "encoded record format {:?} does not match envelope record format {:?}",
                encoded_record.record_format, envelope.record_format
            ),
        });
    }

    if envelope.kind != encoded_record.kind {
        return Err(GraphStorageError::UnexpectedRecordKind {
            expected: envelope.kind,
            actual: encoded_record.kind,
        });
    }

    Ok(())
}

/// Map a filesystem error into the public storage error contract.
///
///
/// - Keep `std::io::Error` out of the public error model.
/// - Preserve the operation and path needed for diagnostics.
///
///
///   The returned error is always `GraphStorageError::IoOperationFailed`.
///
/// # Errors
///
///
/// This mapper has no fallible behavior.
fn io_error(
    operation: &'static str,
    path: Option<&Path>,
    error: std::io::Error,
) -> GraphStorageError {
    GraphStorageError::IoOperationFailed {
        operation,
        path: path.map(Path::to_path_buf),
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::{
        GraphId, JsonLinesRecordCodec, RecordCodec, RecordFormat, StorageManifest, StorageSegment,
        StorageTimestamp, StorageVersion, create_node_record_envelope,
        create_relationship_record_envelope, create_storage_root,
    };
    use graph_core::{Graph, Node, NodeInput, Relationship, RelationshipInput};
    use std::fs;
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_path(test_name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "intelligence_graph_engine_issue_54_{test_name}_{}_{}",
            std::process::id(),
            unique
        ))
    }

    fn manifest() -> StorageManifest {
        StorageManifest {
            // Storage version.
            storage_version: StorageVersion::V1,
            // Graph id.
            graph_id: GraphId {
                // Value.
                value: "graph--issue-54".to_owned(),
            },
            // Created at.
            created_at: StorageTimestamp {
                // Value.
                value: "2026-07-05T00:00:00Z".to_owned(),
            },
            // Updated at.
            updated_at: StorageTimestamp {
                // Value.
                value: "2026-07-05T00:00:00Z".to_owned(),
            },
            // Record format.
            record_format: RecordFormat::JsonLinesV1,
        }
    }

    fn storage_root(test_name: &str) -> StorageRoot {
        let path = unique_temp_path(test_name);
        let _ = fs::remove_dir_all(&path);
        create_storage_root(path, manifest()).unwrap()
    }

    fn storage_ref(segment: StorageSegment) -> StorageRef {
        StorageRef {
            segment,
            // Offset.
            offset: 0,
            // Length.
            length: 1,
            // Checksum.
            checksum: None,
        }
    }

    fn node_fixture() -> Node {
        let mut graph = Graph::new();
        let node_id = graph
            .create_node(NodeInput::new(["Campaign", "FIMI"]))
            .unwrap();
        graph.get_node(&node_id).unwrap().unwrap()
    }

    fn relationship_fixture() -> Relationship {
        let mut graph = Graph::new();
        let source = graph.create_node(NodeInput::new(["Actor"])).unwrap();
        let target = graph
            .create_node(NodeInput::new(["Infrastructure"]))
            .unwrap();
        let relationship_id = graph
            .create_relationship(RelationshipInput::new(source, "USES", target).unwrap())
            .unwrap();
        graph.get_relationship(&relationship_id).unwrap().unwrap()
    }

    fn encoded_node_record() -> (PersistedRecordEnvelope, EncodedRecord) {
        let node = node_fixture();
        let envelope = create_node_record_envelope(
            &node,
            storage_ref(StorageSegment::NodeRecords),
            StorageVersion::V1,
            RecordFormat::JsonLinesV1,
            None,
        )
        .unwrap();
        let encoded = JsonLinesRecordCodec.encode_envelope(&envelope).unwrap();
        (envelope, encoded)
    }

    fn encoded_relationship_record() -> (PersistedRecordEnvelope, EncodedRecord) {
        let relationship = relationship_fixture();
        let envelope = create_relationship_record_envelope(
            &relationship,
            storage_ref(StorageSegment::RelationshipRecords),
            StorageVersion::V1,
            RecordFormat::JsonLinesV1,
            None,
        )
        .unwrap();
        let encoded = JsonLinesRecordCodec.encode_envelope(&envelope).unwrap();
        (envelope, encoded)
    }

    fn manual_log(
        segment: AppendOnlyRecordLogSegment,
        path: impl AsRef<Path>,
    ) -> AppendOnlyRecordLog {
        AppendOnlyRecordLog {
            segment,
            // Path.
            path: path.as_ref().to_path_buf(),
        }
    }

    /// Opening the node append-only log should expose a typed node log
    /// handle without loading graph records.
    ///
    /// Given: a valid storage root with only manifest metadata created.
    /// When: the node append-only record log is opened.
    /// Then: the returned handle targets node records, lives under the storage root,
    /// and does not require node payloads or adjacency payloads to be loaded.
    #[test]
    fn open_node_record_log_returns_node_segment_handle_under_storage_root() {
        let root = storage_root("open_node_record_log");

        let log = open_append_only_node_record_log(&root).unwrap();

        assert_eq!(log.segment, AppendOnlyRecordLogSegment::NodeRecords);
        assert!(log.path.starts_with(root.path()));
        assert!(log.path.to_string_lossy().contains("node"));
        let _ = fs::remove_dir_all(root.path());
    }

    /// Opening the relationship append-only log should expose a typed
    /// relationship log handle without loading graph records.
    ///
    /// Given: a valid storage root with only manifest metadata created.
    /// When: the relationship append-only record log is opened.
    /// Then: the returned handle targets relationship records, lives under the
    /// storage root, and does not require relationship payloads or adjacency
    /// payloads to be loaded.
    #[test]
    fn open_relationship_record_log_returns_relationship_segment_handle_under_storage_root() {
        let root = storage_root("open_relationship_record_log");

        let log = open_append_only_relationship_record_log(&root).unwrap();

        assert_eq!(log.segment, AppendOnlyRecordLogSegment::RelationshipRecords);
        assert!(log.path.starts_with(root.path()));
        assert!(log.path.to_string_lossy().contains("relationship"));
        let _ = fs::remove_dir_all(root.path());
    }

    /// Appending an encoded node envelope should write one durable unit
    /// and return its byte-addressable storage reference.
    ///
    /// Given: an empty node record log and one encoded node envelope.
    /// When: the encoded node envelope is appended.
    /// Then: the returned `StorageRef` points to the node-record segment at offset
    /// zero, has a length equal to the encoded bytes, carries the encoded checksum,
    /// and the log bytes equal the encoded bytes exactly.
    #[test]
    fn append_encoded_node_record_returns_deterministic_storage_ref_and_bytes() {
        let root = storage_root("append_node_record");
        let log_path = root.path().join("nodes").join("node_records.log");
        let mut log = manual_log(AppendOnlyRecordLogSegment::NodeRecords, &log_path);
        let (envelope, encoded) = encoded_node_record();

        let storage_ref =
            append_encoded_node_record_envelope(&mut log, &envelope, &encoded).unwrap();

        assert_eq!(storage_ref.segment, StorageSegment::NodeRecords);
        assert_eq!(storage_ref.offset, 0);
        assert_eq!(storage_ref.length, encoded.bytes.len() as u64);
        assert_eq!(storage_ref.checksum, Some(encoded.checksum.clone()));
        assert_eq!(fs::read(&log_path).unwrap(), encoded.bytes);
        let _ = fs::remove_dir_all(root.path());
    }

    /// Appending an encoded relationship envelope should write one
    /// durable unit and return its byte-addressable storage reference.
    ///
    /// Given: an empty relationship record log and one encoded relationship
    /// envelope.
    /// When: the encoded relationship envelope is appended.
    /// Then: the returned `StorageRef` points to the relationship-record segment at
    /// offset zero, has a length equal to the encoded bytes, carries the encoded
    /// checksum, and the log bytes equal the encoded bytes exactly.
    #[test]
    fn append_encoded_relationship_record_returns_deterministic_storage_ref_and_bytes() {
        let root = storage_root("append_relationship_record");
        let log_path = root
            .path()
            .join("relationships")
            .join("relationship_records.log");
        let mut log = manual_log(AppendOnlyRecordLogSegment::RelationshipRecords, &log_path);
        let (envelope, encoded) = encoded_relationship_record();

        let storage_ref =
            append_encoded_relationship_record_envelope(&mut log, &envelope, &encoded).unwrap();

        assert_eq!(storage_ref.segment, StorageSegment::RelationshipRecords);
        assert_eq!(storage_ref.offset, 0);
        assert_eq!(storage_ref.length, encoded.bytes.len() as u64);
        assert_eq!(storage_ref.checksum, Some(encoded.checksum.clone()));
        assert_eq!(fs::read(&log_path).unwrap(), encoded.bytes);
        let _ = fs::remove_dir_all(root.path());
    }

    /// Appending the same graph record more than once should preserve
    /// older bytes instead of mutating them in place.
    ///
    /// Given: an empty node record log and one encoded node envelope representing a
    /// graph record version.
    /// When: the encoded envelope is appended twice.
    /// Then: the first append remains at offset zero, the second append starts
    /// after the first encoded length, and the durable file contains both encoded
    /// units in append order.
    #[test]
    fn append_encoded_node_record_twice_preserves_previous_versions() {
        let root = storage_root("append_node_record_twice");
        let log_path = root.path().join("nodes").join("node_records.log");
        let mut log = manual_log(AppendOnlyRecordLogSegment::NodeRecords, &log_path);
        let (envelope, encoded) = encoded_node_record();

        let first_ref = append_encoded_node_record_envelope(&mut log, &envelope, &encoded).unwrap();
        let second_ref =
            append_encoded_node_record_envelope(&mut log, &envelope, &encoded).unwrap();

        assert_eq!(first_ref.offset, 0);
        assert_eq!(first_ref.length, encoded.bytes.len() as u64);
        assert_eq!(second_ref.offset, first_ref.length);
        assert_eq!(second_ref.length, encoded.bytes.len() as u64);

        let mut expected_bytes = encoded.bytes.clone();
        expected_bytes.extend_from_slice(&encoded.bytes);
        assert_eq!(fs::read(&log_path).unwrap(), expected_bytes);
        let _ = fs::remove_dir_all(root.path());
    }

    /// Flushing an append-only record log should make durability errors
    /// explicit without introducing WAL or compaction behavior.
    ///
    /// Given: a node record log with one appended encoded node envelope.
    /// When: the append-only log is flushed.
    /// Then: flushing succeeds, leaves the appended bytes intact, and does not
    /// rewrite or compact the log.
    #[test]
    fn flush_append_only_record_log_keeps_appended_bytes_intact() {
        let root = storage_root("flush_append_only_record_log");
        let log_path = root.path().join("nodes").join("node_records.log");
        let mut log = manual_log(AppendOnlyRecordLogSegment::NodeRecords, &log_path);
        let (envelope, encoded) = encoded_node_record();
        append_encoded_node_record_envelope(&mut log, &envelope, &encoded).unwrap();

        flush_append_only_record_log(&mut log).unwrap();

        assert_eq!(fs::read(&log_path).unwrap(), encoded.bytes);
        let _ = fs::remove_dir_all(root.path());
    }

    /// Appending an encoded record through the wrong typed log should
    /// fail explicitly rather than silently writing to the wrong segment.
    ///
    /// Given: a node record log and an encoded relationship envelope.
    /// When: the relationship envelope is appended through the node append API.
    /// Then: the append fails with a typed storage error and does not report the
    /// phase marker `NotImplemented` once phase 3 is complete.
    #[test]
    fn append_node_api_rejects_relationship_envelope_without_writing_bytes() {
        let root = storage_root("reject_relationship_in_node_log");
        let log_path = root.path().join("nodes").join("node_records.log");
        let mut log = manual_log(AppendOnlyRecordLogSegment::NodeRecords, &log_path);
        let (relationship_envelope, encoded_relationship) = encoded_relationship_record();

        let error = append_encoded_node_record_envelope(
            &mut log,
            &relationship_envelope,
            &encoded_relationship,
        )
        .unwrap_err();

        assert!(!matches!(error, GraphStorageError::NotImplemented(_)));
        assert!(!log_path.exists() || fs::read(&log_path).unwrap().is_empty());
        let _ = fs::remove_dir_all(root.path());
    }

    #[test]
    fn open_node_record_log_reports_missing_storage_root() {
        let root = StorageRoot {
            path: unique_temp_path("missing_storage_root_for_open"),
        };

        let error = open_append_only_node_record_log(&root)
            .expect_err("opening append-only log should fail for a missing root path");

        assert!(matches!(
            error,
            GraphStorageError::StorageRootNotFound { .. }
        ));
    }

    #[test]
    fn open_relationship_record_log_reports_missing_storage_root() {
        let root = StorageRoot {
            path: unique_temp_path("missing_storage_root_for_relationship_open"),
        };

        let error = open_append_only_relationship_record_log(&root)
            .expect_err("opening relationship append-only log should fail for a missing root");

        assert!(matches!(
            error,
            GraphStorageError::StorageRootNotFound { .. }
        ));
    }

    #[test]
    fn append_node_api_rejects_mismatched_log_segment_before_write() {
        let root = storage_root("reject_mismatched_log_segment");
        let log_path = root
            .path()
            .join("relationships")
            .join("relationship_records.log");
        let mut log = manual_log(AppendOnlyRecordLogSegment::RelationshipRecords, &log_path);
        let (node_envelope, encoded_node) = encoded_node_record();

        let error = append_encoded_node_record_envelope(&mut log, &node_envelope, &encoded_node)
            .expect_err("node append API should reject relationship log segment");

        assert!(matches!(error, GraphStorageError::InvalidEnvelope { .. }));
        assert!(!log_path.exists());
        let _ = fs::remove_dir_all(root.path());
    }

    #[test]
    fn append_node_api_rejects_encoded_storage_version_mismatch() {
        let root = storage_root("encoded_storage_version_mismatch");
        let log_path = root.path().join("nodes").join("node_records.log");
        let mut log = manual_log(AppendOnlyRecordLogSegment::NodeRecords, &log_path);
        let (envelope, mut encoded) = encoded_node_record();
        encoded.storage_version = StorageVersion::Unsupported("V999".to_owned());

        let error = append_encoded_node_record_envelope(&mut log, &envelope, &encoded)
            .expect_err("mismatched encoded storage version should fail validation");

        assert!(matches!(error, GraphStorageError::InvalidEnvelope { .. }));
        assert!(!log_path.exists() || fs::read(&log_path).unwrap().is_empty());
        let _ = fs::remove_dir_all(root.path());
    }

    #[test]
    fn append_node_api_rejects_encoded_record_format_mismatch() {
        let root = storage_root("encoded_record_format_mismatch");
        let log_path = root.path().join("nodes").join("node_records.log");
        let mut log = manual_log(AppendOnlyRecordLogSegment::NodeRecords, &log_path);
        let (envelope, mut encoded) = encoded_node_record();
        encoded.record_format = RecordFormat::Unsupported("BinaryV2".to_owned());

        let error = append_encoded_node_record_envelope(&mut log, &envelope, &encoded)
            .expect_err("mismatched encoded record format should fail validation");

        assert!(matches!(error, GraphStorageError::InvalidEnvelope { .. }));
        assert!(!log_path.exists() || fs::read(&log_path).unwrap().is_empty());
        let _ = fs::remove_dir_all(root.path());
    }

    #[test]
    fn append_relationship_api_rejects_node_envelope_without_writing_bytes() {
        let root = storage_root("reject_node_in_relationship_log");
        let log_path = root
            .path()
            .join("relationships")
            .join("relationship_records.log");
        let mut log = manual_log(AppendOnlyRecordLogSegment::RelationshipRecords, &log_path);
        let (node_envelope, encoded_node) = encoded_node_record();

        let error =
            append_encoded_relationship_record_envelope(&mut log, &node_envelope, &encoded_node)
                .expect_err("relationship append API should reject node envelopes");

        assert!(matches!(
            error,
            GraphStorageError::UnexpectedRecordKind {
                expected: PersistedRecordKind::Relationship,
                actual: PersistedRecordKind::Node,
            }
        ));
        assert!(!log_path.exists() || fs::read(&log_path).unwrap().is_empty());
        let _ = fs::remove_dir_all(root.path());
    }

    #[test]
    fn append_relationship_api_rejects_node_log_segment_before_write() {
        let root = storage_root("relationship_api_wrong_segment");
        let log_path = root.path().join("nodes").join("node_records.log");
        let mut log = manual_log(AppendOnlyRecordLogSegment::NodeRecords, &log_path);
        let (relationship_envelope, encoded_relationship) = encoded_relationship_record();

        let error = append_encoded_relationship_record_envelope(
            &mut log,
            &relationship_envelope,
            &encoded_relationship,
        )
        .expect_err("relationship append API should reject node log segment");

        assert!(matches!(error, GraphStorageError::InvalidEnvelope { .. }));
        assert!(!log_path.exists());
        let _ = fs::remove_dir_all(root.path());
    }

    #[test]
    fn append_node_api_rejects_empty_encoded_record_bytes() {
        let root = storage_root("empty_encoded_record_bytes");
        let log_path = root.path().join("nodes").join("node_records.log");
        let mut log = manual_log(AppendOnlyRecordLogSegment::NodeRecords, &log_path);
        let (envelope, mut encoded) = encoded_node_record();
        encoded.bytes.clear();

        let error = append_encoded_node_record_envelope(&mut log, &envelope, &encoded)
            .expect_err("empty encoded record bytes should be rejected");

        assert!(matches!(
        error,
        GraphStorageError::InvalidEnvelope { reason }
        if reason == "encoded record bytes must not be empty"
        ));
        assert!(!log_path.exists() || fs::read(&log_path).unwrap().is_empty());
        let _ = fs::remove_dir_all(root.path());
    }

    #[test]
    fn validate_record_kind_accepts_match_and_rejects_mismatch() {
        validate_record_kind(PersistedRecordKind::Node, PersistedRecordKind::Node)
            .expect("matching kinds should be accepted");

        let error =
            validate_record_kind(PersistedRecordKind::Relationship, PersistedRecordKind::Node)
                .expect_err("kind mismatch should be rejected");

        assert!(matches!(
            error,
            GraphStorageError::UnexpectedRecordKind {
                expected: PersistedRecordKind::Node,
                actual: PersistedRecordKind::Relationship,
            }
        ));
    }

    #[test]
    fn validate_encoded_record_matches_envelope_rejects_kind_mismatch() {
        let (envelope, mut encoded) = encoded_node_record();
        encoded.kind = PersistedRecordKind::Relationship;

        let error = validate_encoded_record_matches_envelope(&envelope, &encoded)
            .expect_err("encoded kind mismatch should be rejected");

        assert!(matches!(
            error,
            GraphStorageError::UnexpectedRecordKind {
                expected: PersistedRecordKind::Node,
                actual: PersistedRecordKind::Relationship,
            }
        ));
    }

    #[test]
    fn current_file_len_returns_zero_for_missing_file() {
        let path = unique_temp_path("missing_len_file");
        let _ = fs::remove_file(&path);

        let len = current_file_len(&path, "current_file_len")
            .expect("missing files should be treated as empty logs");

        assert_eq!(len, 0);
    }

    #[test]
    fn flush_append_only_record_log_creates_missing_parent_directories() {
        let root = storage_root("flush_creates_parent_dirs");
        let nested_path = root.path().join("nested").join("logs").join("records.log");
        let mut log = manual_log(AppendOnlyRecordLogSegment::NodeRecords, &nested_path);

        flush_append_only_record_log(&mut log)
            .expect("flush should create parent directories when missing");

        assert!(nested_path.is_file());
        let _ = fs::remove_dir_all(root.path());
    }

    #[test]
    fn open_node_record_log_reports_io_error_when_segment_parent_is_a_file() {
        let root = storage_root("open_node_log_parent_is_file");
        let nodes_path = root.path().join("nodes");
        fs::write(&nodes_path, b"not a directory")
            .expect("fixture should create file at nodes path");

        let error = open_append_only_node_record_log(&root)
            .expect_err("opening node log should fail when segment parent is a file");

        assert!(matches!(
        error,
        GraphStorageError::IoOperationFailed {
        operation: "open_append_only_record_log",
        path: Some(path),
        ..
        } if path == nodes_path
        ));
        let _ = fs::remove_dir_all(root.path());
    }

    #[test]
    fn append_node_api_reports_io_error_when_log_path_is_directory() {
        let root = storage_root("append_node_log_path_is_directory");
        let directory_path = root.path().join("nodes").join("node_records.log");
        fs::create_dir_all(&directory_path)
            .expect("fixture should create directory where file path is expected");
        let mut log = manual_log(AppendOnlyRecordLogSegment::NodeRecords, &directory_path);
        let (envelope, encoded) = encoded_node_record();

        let error = append_encoded_node_record_envelope(&mut log, &envelope, &encoded)
            .expect_err("append should fail when log path points to directory");

        assert!(matches!(
        error,
        GraphStorageError::IoOperationFailed {
        operation: "append_encoded_node_record_envelope",
        path: Some(path),
        ..
        } if path == directory_path
        ));
        let _ = fs::remove_dir_all(root.path());
    }

    #[test]
    fn flush_append_only_record_log_reports_io_error_when_path_is_directory() {
        let root = storage_root("flush_log_path_is_directory");
        let directory_path = root
            .path()
            .join("relationships")
            .join("relationship_records.log");
        fs::create_dir_all(&directory_path)
            .expect("fixture should create directory where file path is expected");
        let mut log = manual_log(
            AppendOnlyRecordLogSegment::RelationshipRecords,
            &directory_path,
        );

        let error = flush_append_only_record_log(&mut log)
            .expect_err("flush should fail when log path points to directory");

        assert!(matches!(
        error,
        GraphStorageError::IoOperationFailed {
        operation: "flush_append_only_record_log",
        path: Some(path),
        ..
        } if path == directory_path
        ));
        let _ = fs::remove_dir_all(root.path());
    }

    #[test]
    fn append_log_path_resolves_expected_node_and_relationship_locations() {
        let root = storage_root("append_log_path_layout");

        let node_path = append_log_path(&root, AppendOnlyRecordLogSegment::NodeRecords);
        let relationship_path =
            append_log_path(&root, AppendOnlyRecordLogSegment::RelationshipRecords);

        assert!(node_path.ends_with("nodes/node_records.log"));
        assert!(relationship_path.ends_with("relationships/relationship_records.log"));
        let _ = fs::remove_dir_all(root.path());
    }

    #[test]
    fn validate_log_segment_accepts_matching_segment() {
        let log = manual_log(AppendOnlyRecordLogSegment::NodeRecords, "node_records.log");

        validate_log_segment(
            &log,
            AppendOnlyRecordLogSegment::NodeRecords,
            "validate_log_segment_accepts_matching_segment",
        )
        .expect("matching segment should pass validation");
    }

    #[test]
    fn validate_encoded_record_matches_envelope_accepts_consistent_metadata() {
        let (envelope, encoded) = encoded_node_record();

        validate_encoded_record_matches_envelope(&envelope, &encoded)
            .expect("matching envelope and encoded metadata should validate");
    }

    #[test]
    fn ensure_parent_dir_is_noop_when_path_has_no_parent() {
        ensure_parent_dir(Path::new("log_without_parent"), "ensure_parent_dir")
            .expect("paths without parent component should be accepted");
    }

    #[test]
    fn current_file_len_returns_existing_file_length() {
        let root = storage_root("current_file_len_existing_file");
        let log_path = root.path().join("nodes").join("node_records.log");
        fs::create_dir_all(log_path.parent().unwrap()).expect("parent should be creatable");
        fs::write(&log_path, b"abcd").expect("fixture should write sample file bytes");

        let len = current_file_len(&log_path, "current_file_len")
            .expect("existing files should return deterministic byte length");

        assert_eq!(len, 4);
        let _ = fs::remove_dir_all(root.path());
    }

    #[test]
    fn io_error_maps_operation_with_and_without_path_context() {
        let explicit = io_error(
            "append",
            Some(Path::new("/tmp/example.log")),
            std::io::Error::other("disk failed"),
        );
        assert!(matches!(
        explicit,
        GraphStorageError::IoOperationFailed {
        operation: "append",
        path: Some(path),
        message,
        } if path == Path::new("/tmp/example.log") && message.contains("disk failed")
        ));

        let without_path = io_error("flush", None, std::io::Error::other("sync failed"));
        assert!(matches!(
        without_path,
        GraphStorageError::IoOperationFailed {
        operation: "flush",
        path: None,
        message,
        } if message.contains("sync failed")
        ));
    }

    #[test]
    fn validate_append_offset_range_rejects_u64_overflow() {
        let error = validate_append_offset_range(u64::MAX, 1, StorageSegment::NodeRecords, None)
            .expect_err("offset overflow should be rejected before writing bytes");

        assert!(matches!(
        error,
        GraphStorageError::InvalidStorageRef {
        storage_ref,
        reason,
        } if storage_ref.segment == StorageSegment::NodeRecords
        && storage_ref.offset == u64::MAX
        && storage_ref.length == 1
        && reason == "offset plus length must not overflow"
        ));
    }

    #[test]
    fn ensure_parent_dir_reports_io_error_when_parent_component_is_file() {
        let root = storage_root("ensure_parent_dir_parent_is_file");
        let file_parent = root.path().join("file_parent");
        fs::write(&file_parent, b"not a directory")
            .expect("fixture should create file where directory is expected");

        let child_path = file_parent.join("records.log");
        let error = ensure_parent_dir(&child_path, "ensure_parent_dir")
            .expect_err("file parent should fail directory creation");

        assert!(matches!(
        error,
        GraphStorageError::IoOperationFailed {
        operation: "ensure_parent_dir",
        path: Some(path),
        ..
        } if path == file_parent
        ));
        let _ = fs::remove_dir_all(root.path());
    }

    #[test]
    fn current_file_len_reports_io_error_for_non_directory_component() {
        let root = storage_root("current_file_len_not_a_directory");
        let file_parent = root.path().join("not_a_directory");
        fs::write(&file_parent, b"fixture").expect("fixture should create non-directory component");
        let nested_path = file_parent.join("record.log");

        let error = current_file_len(&nested_path, "current_file_len")
            .expect_err("non-directory component should produce metadata io error");

        assert!(matches!(
        error,
        GraphStorageError::IoOperationFailed {
        operation: "current_file_len",
        path: Some(path),
        ..
        } if path == nested_path
        ));
        let _ = fs::remove_dir_all(root.path());
    }

    #[test]
    fn append_relationship_api_rejects_empty_encoded_record_bytes() {
        let root = storage_root("empty_encoded_relationship_record_bytes");
        let log_path = root
            .path()
            .join("relationships")
            .join("relationship_records.log");
        let mut log = manual_log(AppendOnlyRecordLogSegment::RelationshipRecords, &log_path);
        let (envelope, mut encoded) = encoded_relationship_record();
        encoded.bytes.clear();

        let error = append_encoded_relationship_record_envelope(&mut log, &envelope, &encoded)
            .expect_err("empty encoded relationship bytes should be rejected");

        assert!(matches!(
        error,
        GraphStorageError::InvalidEnvelope { reason }
        if reason == "encoded record bytes must not be empty"
        ));
        assert!(!log_path.exists() || fs::read(&log_path).unwrap().is_empty());
        let _ = fs::remove_dir_all(root.path());
    }

    #[test]
    fn append_relationship_api_rejects_encoded_storage_version_mismatch() {
        let root = storage_root("relationship_encoded_storage_version_mismatch");
        let log_path = root
            .path()
            .join("relationships")
            .join("relationship_records.log");
        let mut log = manual_log(AppendOnlyRecordLogSegment::RelationshipRecords, &log_path);
        let (envelope, mut encoded) = encoded_relationship_record();
        encoded.storage_version = StorageVersion::Unsupported("V999".to_owned());

        let error = append_encoded_relationship_record_envelope(&mut log, &envelope, &encoded)
            .expect_err("mismatched encoded relationship storage version should fail validation");

        assert!(matches!(error, GraphStorageError::InvalidEnvelope { .. }));
        assert!(!log_path.exists() || fs::read(&log_path).unwrap().is_empty());
        let _ = fs::remove_dir_all(root.path());
    }

    #[test]
    fn append_relationship_api_rejects_encoded_record_format_mismatch() {
        let root = storage_root("relationship_encoded_record_format_mismatch");
        let log_path = root
            .path()
            .join("relationships")
            .join("relationship_records.log");
        let mut log = manual_log(AppendOnlyRecordLogSegment::RelationshipRecords, &log_path);
        let (envelope, mut encoded) = encoded_relationship_record();
        encoded.record_format = RecordFormat::Unsupported("BinaryV2".to_owned());

        let error = append_encoded_relationship_record_envelope(&mut log, &envelope, &encoded)
            .expect_err("mismatched encoded relationship record format should fail validation");

        assert!(matches!(error, GraphStorageError::InvalidEnvelope { .. }));
        assert!(!log_path.exists() || fs::read(&log_path).unwrap().is_empty());
        let _ = fs::remove_dir_all(root.path());
    }
}

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
//! Crash-consistency tests for the append-only record log (audit §6).
//!
//! A durable graph store must survive process/host crashes that leave the last
//! append only partially written. The append-only logs are checksummed JSON
//! Lines, so recovery must uphold two guarantees:
//!
//! 1. **Complete records survive a clean tail truncation.** If a crash drops an
//!    entire trailing record (truncation lands on a record boundary), every
//!    fully-flushed record before it must still be recoverable.
//! 2. **A torn tail is rejected explicitly, never silently accepted.** If a crash
//!    leaves a partially-written final record (truncation lands mid-record),
//!    recovery must surface a typed `GraphStorageError::CatalogRebuildCorruptedRecord`
//!    rather than panicking, hanging, or returning a half-decoded record.
//!
//! These tests build a real on-disk node log, then corrupt/truncate the tail
//! bytes to simulate the crash and assert the recovery contract.

#![allow(clippy::unwrap_used)]

use std::fs::{self, OpenOptions};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use graph_core::{Graph, Node, NodeInput, PropertyValue};
use graph_storage::{
    GraphStorageError, JsonLinesRecordCodec, RecordFormat, StorageManifest, StorageRef,
    StorageRoot, StorageSegment, StorageTimestamp, StorageVersion,
    append_encoded_node_record_envelope, create_node_record_envelope, create_storage_root,
    encode_persisted_record_envelope, flush_append_only_record_log,
    open_append_only_node_record_log, read_node_record_log_for_catalog_rebuild,
};

fn unique_temp_path(test_name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "corrobore_issue_238_torn_write_{test_name}_{}_{}",
        std::process::id(),
        unique
    ))
}

fn manifest() -> StorageManifest {
    StorageManifest {
        storage_version: StorageVersion::V1,
        graph_id: graph_storage::GraphId {
            value: "graph--issue-238-torn-write".to_owned(),
        },
        created_at: StorageTimestamp {
            value: "2026-07-15T00:00:00Z".to_owned(),
        },
        updated_at: StorageTimestamp {
            value: "2026-07-15T00:00:00Z".to_owned(),
        },
        record_format: RecordFormat::JsonLinesV1,
    }
}

fn storage_root(test_name: &str) -> StorageRoot {
    let path = unique_temp_path(test_name);
    let _ = fs::remove_dir_all(&path);
    create_storage_root(path, manifest()).unwrap()
}

fn node_fixture(name: &str) -> Node {
    let mut graph = Graph::new();
    let node_id = graph
        .create_node(
            NodeInput::new(["Campaign", "FIMI"])
                .with_property("name", PropertyValue::String(name.to_owned())),
        )
        .unwrap();
    graph.get_node(&node_id).unwrap().unwrap()
}

fn node_storage_ref(offset: u64, length: u64) -> StorageRef {
    StorageRef {
        segment: StorageSegment::NodeRecords,
        offset,
        length,
        checksum: None,
    }
}

/// Write `count` valid node records into the node log and return the log path and
/// the on-disk byte length after a durable flush.
fn write_node_records(root: &StorageRoot, count: usize) -> (PathBuf, u64) {
    let codec = JsonLinesRecordCodec::default();
    let mut log = open_append_only_node_record_log(root).unwrap();

    for index in 0..count {
        let node = node_fixture(&format!("node-{index}"));
        let envelope = create_node_record_envelope(
            &node,
            node_storage_ref(index as u64 * 512, 512),
            StorageVersion::V1,
            RecordFormat::JsonLinesV1,
            None,
        )
        .unwrap();
        let encoded = encode_persisted_record_envelope(&codec, &envelope).unwrap();
        append_encoded_node_record_envelope(&mut log, &envelope, &encoded).unwrap();
    }
    flush_append_only_record_log(&mut log).unwrap();

    let path = log.path.clone();
    let length = fs::metadata(&path).unwrap().len();
    (path, length)
}

/// Truncate a file to `new_length` bytes, simulating a crash that stopped the
/// write partway through.
fn truncate_file(path: &PathBuf, new_length: u64) {
    let file = OpenOptions::new().write(true).open(path).unwrap();
    file.set_len(new_length).unwrap();
}

/// Byte offset where the final record starts (one past the second-to-last
/// newline). Truncating between this offset and the end of the file lands inside
/// the final record, simulating a torn write.
fn last_record_start(path: &PathBuf) -> u64 {
    let bytes = fs::read(path).unwrap();
    let newline_positions: Vec<usize> = bytes
        .iter()
        .enumerate()
        .filter_map(|(index, byte)| (*byte == b'\n').then_some(index))
        .collect();
    assert!(
        newline_positions.len() >= 2,
        "need at least two records to isolate a torn final record"
    );
    (newline_positions[newline_positions.len() - 2] + 1) as u64
}

// Baseline: an intact, flushed log reads back all records without error.
#[test]
fn intact_node_log_recovers_all_records() {
    let root = storage_root("intact");
    let (_path, _length) = write_node_records(&root, 3);

    let records = read_node_record_log_for_catalog_rebuild(&root).unwrap();
    assert_eq!(records.len(), 3);

    let _ = fs::remove_dir_all(root.path());
}

// Clean tail truncation: dropping the whole trailing record (truncating exactly
// at the previous record boundary) leaves the earlier complete records intact.
#[test]
fn clean_tail_truncation_preserves_complete_records() {
    let root = storage_root("clean_tail");
    let (path, _length) = write_node_records(&root, 3);

    // Find the boundary before the last record, then find the boundary before
    // *that* record to drop exactly the final record.
    let full = fs::read(&path).unwrap();
    let newline_positions: Vec<usize> = full
        .iter()
        .enumerate()
        .filter_map(|(index, byte)| (*byte == b'\n').then_some(index))
        .collect();
    assert_eq!(
        newline_positions.len(),
        3,
        "three records => three newlines"
    );
    let boundary_before_last = (newline_positions[1] + 1) as u64;

    truncate_file(&path, boundary_before_last);

    let records = read_node_record_log_for_catalog_rebuild(&root).unwrap();
    assert_eq!(
        records.len(),
        2,
        "two fully-flushed records must survive a clean tail truncation"
    );

    let _ = fs::remove_dir_all(root.path());
}

// Torn tail: truncating in the middle of the final record leaves a partial JSON
// line. Recovery must reject it with a typed corruption error, never a panic or
// a silently-decoded partial record.
#[test]
fn torn_tail_write_is_rejected_with_typed_corruption_error() {
    let root = storage_root("torn_tail");
    let (path, length) = write_node_records(&root, 3);

    // Cut a handful of bytes off the end so the final record's JSON is truncated
    // mid-line (no terminating newline, invalid JSON).
    let record_start = last_record_start(&path);
    assert!(
        length > record_start,
        "final record must contain payload bytes"
    );
    let torn_length = length - 8;
    assert!(
        torn_length > record_start,
        "truncation must land inside the final record, not on a boundary"
    );
    truncate_file(&path, torn_length);

    let error = read_node_record_log_for_catalog_rebuild(&root)
        .expect_err("a torn final record must not be silently recovered");
    assert!(
        matches!(
            error,
            GraphStorageError::CatalogRebuildCorruptedRecord { .. }
        ),
        "expected CatalogRebuildCorruptedRecord, got {error:?}"
    );

    let _ = fs::remove_dir_all(root.path());
}

// Byte-level corruption of the tail (flipping bytes without changing length) must
// also be rejected as a typed corruption error rather than accepted.
#[test]
fn corrupted_tail_bytes_are_rejected_with_typed_corruption_error() {
    let root = storage_root("corrupted_tail");
    let (path, _length) = write_node_records(&root, 2);

    let mut bytes = fs::read(&path).unwrap();
    // Corrupt a byte inside the last record (just before the trailing newline).
    let last = bytes.len() - 2;
    bytes[last] = b'~';
    fs::write(&path, &bytes).unwrap();

    let error = read_node_record_log_for_catalog_rebuild(&root)
        .expect_err("corrupted trailing bytes must not be silently recovered");
    assert!(
        matches!(
            error,
            GraphStorageError::CatalogRebuildCorruptedRecord { .. }
        ),
        "expected CatalogRebuildCorruptedRecord, got {error:?}"
    );

    let _ = fs::remove_dir_all(root.path());
}

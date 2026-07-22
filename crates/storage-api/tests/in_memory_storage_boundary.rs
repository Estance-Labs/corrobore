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
//! Behavioral contract tests for the in-memory `StorageBoundary` implementation.
//!
//! Intent: prove the storage abstraction boundary is a live, enforceable
//! contract rather than an inert set of shapes. The in-memory backend is the
//! reference implementor and test double that alternative backends must match,
//! so every assertion here drives it *through the trait object*
//! (`&mut dyn StorageBoundary`) — never through concrete methods. This guarantees
//! the trait surface alone is sufficient to append, resolve, read, and flush.

use storage_api::{
    AppendRecordRequest, InMemoryStorageBoundary, ReadRecordRequest, ResolveLatestRequest,
    StorageApiError, StorageBoundary, StorageRecordKind,
};

//
// Verify an appended record can be resolved and read back through the trait,
// yielding the exact kind, id, and payload that were written (round-trip
// fidelity is the minimum guarantee any backend must uphold).
#[test]
fn append_then_resolve_and_read_round_trips_through_trait() {
    let mut backend = InMemoryStorageBoundary::default();
    let boundary: &mut dyn StorageBoundary = &mut backend;

    let append = AppendRecordRequest::new(
        StorageRecordKind::NodeVersion,
        "node--1",
        vec![0xDE, 0xAD, 0xBE, 0xEF],
    )
    .expect("append request should be valid");

    let persisted = boundary
        .append_record(append)
        .expect("append should succeed on a fresh backend");

    let resolved = boundary
        .resolve_latest(
            ResolveLatestRequest::new(StorageRecordKind::NodeVersion, "node--1")
                .expect("resolve request should be valid"),
        )
        .expect("resolve should succeed")
        .expect("the just-appended record should resolve to a storage ref");

    assert_eq!(
        resolved, persisted.storage_ref,
        "resolve_latest must return the ref produced by append_record"
    );

    let read = boundary
        .read_record(ReadRecordRequest::new(resolved))
        .expect("reading the resolved ref should succeed");

    assert_eq!(read.record_kind, StorageRecordKind::NodeVersion);
    assert_eq!(read.record_id, "node--1");
    assert_eq!(read.payload, vec![0xDE, 0xAD, 0xBE, 0xEF]);
}

//
// Verify that appending a newer version of the same record supersedes the
// previous one for resolution, while the older payload remains readable by its
// original ref (append-only history is preserved; latest wins for resolution).
#[test]
fn latest_resolution_follows_most_recent_append() {
    let mut backend = InMemoryStorageBoundary::default();
    let boundary: &mut dyn StorageBoundary = &mut backend;

    let first = boundary
        .append_record(
            AppendRecordRequest::new(StorageRecordKind::NodeVersion, "node--7", vec![1])
                .expect("first append request should be valid"),
        )
        .expect("first append should succeed");

    let second = boundary
        .append_record(
            AppendRecordRequest::new(StorageRecordKind::NodeVersion, "node--7", vec![2, 2])
                .expect("second append request should be valid"),
        )
        .expect("second append should succeed");

    assert_ne!(
        first.storage_ref, second.storage_ref,
        "each append must occupy a distinct storage ref"
    );

    let resolved = boundary
        .resolve_latest(
            ResolveLatestRequest::new(StorageRecordKind::NodeVersion, "node--7")
                .expect("resolve request should be valid"),
        )
        .expect("resolve should succeed")
        .expect("record should resolve");

    assert_eq!(
        resolved, second.storage_ref,
        "resolve_latest must return the most recent append"
    );

    let historical = boundary
        .read_record(ReadRecordRequest::new(first.storage_ref))
        .expect("the superseded record should still be readable by its ref");

    assert_eq!(historical.payload, vec![1]);
}

//
// Verify that records are namespaced by (kind, id): the same id under a
// different kind resolves independently and never collides.
#[test]
fn resolution_is_namespaced_by_record_kind() {
    let mut backend = InMemoryStorageBoundary::default();
    let boundary: &mut dyn StorageBoundary = &mut backend;

    boundary
        .append_record(
            AppendRecordRequest::new(StorageRecordKind::NodeVersion, "shared--id", vec![10])
                .expect("node append request should be valid"),
        )
        .expect("node append should succeed");
    boundary
        .append_record(
            AppendRecordRequest::new(
                StorageRecordKind::RelationshipVersion,
                "shared--id",
                vec![20],
            )
            .expect("relationship append request should be valid"),
        )
        .expect("relationship append should succeed");

    let node_ref = boundary
        .resolve_latest(
            ResolveLatestRequest::new(StorageRecordKind::NodeVersion, "shared--id")
                .expect("node resolve request should be valid"),
        )
        .expect("resolve should succeed")
        .expect("node record should resolve");
    let relationship_ref = boundary
        .resolve_latest(
            ResolveLatestRequest::new(StorageRecordKind::RelationshipVersion, "shared--id")
                .expect("relationship resolve request should be valid"),
        )
        .expect("resolve should succeed")
        .expect("relationship record should resolve");

    assert_ne!(
        node_ref, relationship_ref,
        "records sharing an id but differing in kind must not collide"
    );
    assert_eq!(
        boundary
            .read_record(ReadRecordRequest::new(node_ref))
            .expect("node read should succeed")
            .payload,
        vec![10]
    );
    assert_eq!(
        boundary
            .read_record(ReadRecordRequest::new(relationship_ref))
            .expect("relationship read should succeed")
            .payload,
        vec![20]
    );
}

//
// Verify that resolving a record that was never written returns `None` (a
// missing latest pointer is not an error condition).
#[test]
fn resolve_latest_of_unknown_record_returns_none() {
    let backend = InMemoryStorageBoundary::default();
    let boundary: &dyn StorageBoundary = &backend;

    let resolved = boundary
        .resolve_latest(
            ResolveLatestRequest::new(StorageRecordKind::Evidence, "missing--1")
                .expect("resolve request should be valid"),
        )
        .expect("resolving an unknown record should not error");

    assert!(
        resolved.is_none(),
        "an unwritten record must resolve to None"
    );
}

//
// Verify that reading a storage ref that does not exist surfaces a typed
// `RecordNotFound` error rather than a panic or silent empty payload.
#[test]
fn read_of_unknown_storage_ref_reports_record_not_found() {
    let backend = InMemoryStorageBoundary::default();
    let boundary: &dyn StorageBoundary = &backend;

    let dangling = storage_api::StorageRef::new("memory", 999, 4)
        .expect("a syntactically valid but dangling ref should construct");

    let error = boundary
        .read_record(ReadRecordRequest::new(dangling))
        .expect_err("reading a dangling ref must fail");

    assert!(matches!(error, StorageApiError::RecordNotFound(_)));
}

//
// Verify that flush reports the number of records durably committed since the
// previous flush, and that a subsequent flush with no new writes reports zero
// (idempotent flush boundary).
#[test]
fn flush_reports_pending_writes_then_resets() {
    let mut backend = InMemoryStorageBoundary::default();
    let boundary: &mut dyn StorageBoundary = &mut backend;

    boundary
        .append_record(
            AppendRecordRequest::new(StorageRecordKind::AuditEvent, "audit--1", vec![1])
                .expect("first audit append should be valid"),
        )
        .expect("append should succeed");
    boundary
        .append_record(
            AppendRecordRequest::new(StorageRecordKind::AuditEvent, "audit--2", vec![2])
                .expect("second audit append should be valid"),
        )
        .expect("append should succeed");

    let first_flush = boundary.flush().expect("flush should succeed");
    assert_eq!(
        first_flush.flushed_segments, 2,
        "flush must account for both pending appends"
    );

    let second_flush = boundary
        .flush()
        .expect("a flush with no pending writes should still succeed");
    assert_eq!(
        second_flush.flushed_segments, 0,
        "a flush with nothing pending must report zero"
    );
}

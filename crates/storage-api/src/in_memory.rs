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
//! In-memory reference implementation of the storage boundary.
//!
//! This module provides [`InMemoryStorageBoundary`], a fully in-memory backend
//! that satisfies the [`StorageBoundary`](crate::StorageBoundary) contract. It
//! serves two purposes:
//!
//! - **Test double** — lets callers exercise storage-dependent logic without a
//!   filesystem, so the boundary is validated behaviorally rather than left as
//!   an inert set of type shapes.
//! - **Reference backend** — documents, in executable form, the observable
//!   semantics any alternative backend (such as the file-backed `graph-storage`
//!   store) must reproduce: append assigns a distinct storage ref, resolution
//!   returns the most recent ref for a `(kind, id)` pair, reads return the exact
//!   persisted payload, and flush accounts for writes committed since the prior
//!   flush.
//!
//! The backend is append-only: superseded records remain readable by their
//! original ref, while `resolve_latest` always tracks the newest write.

use std::collections::HashMap;

use crate::{
    AppendRecordRequest, FlushResult, ReadRecordRequest, ReadRecordResponse, RecordPersistResult,
    ResolveLatestRequest, StorageApiError, StorageBoundary, StorageRecordKind, StorageRef,
};

/// Segment name used for every record persisted by the in-memory backend.
///
/// The in-memory backend keeps all payloads in a single logical segment and
/// distinguishes records by a monotonically increasing offset.
const IN_MEMORY_SEGMENT: &str = "memory";

/// A single record retained by the in-memory backend, kept addressable by the
/// [`StorageRef`] produced when it was appended.
#[derive(Clone, Debug)]
struct StoredRecord {
    record_kind: StorageRecordKind,
    record_id: String,
    payload: Vec<u8>,
}

/// In-memory implementation of [`StorageBoundary`].
///
/// See the [module documentation](self) for the semantics this backend
/// guarantees. Construct one with [`InMemoryStorageBoundary::default`].
#[derive(Debug, Default)]
pub struct InMemoryStorageBoundary {
    /// All appended records, keyed by the offset of the ref they were assigned.
    records: HashMap<u64, StoredRecord>,
    /// Latest storage ref per `(kind, id)`; superseded on each new append.
    latest: HashMap<(StorageRecordKind, String), StorageRef>,
    /// Monotonic offset cursor; the next append starts here.
    next_offset: u64,
    /// Records appended since the last successful flush.
    pending_writes: usize,
}

impl StorageBoundary for InMemoryStorageBoundary {
    fn append_record(
        &mut self,
        request: AppendRecordRequest,
    ) -> Result<RecordPersistResult, StorageApiError> {
        // `AppendRecordRequest` construction already guarantees a non-empty
        // payload; guard only the boundary condition that the length cannot be
        // represented in a `StorageRef` (its `length` field is a `u32`).
        let length = u32::try_from(request.payload.len()).map_err(|_| {
            StorageApiError::WriteFailed(format!(
                "payload of {} bytes exceeds the addressable segment length",
                request.payload.len()
            ))
        })?;

        let offset = self.next_offset;
        let storage_ref = StorageRef::new(IN_MEMORY_SEGMENT, offset, length)?;

        self.records.insert(
            offset,
            StoredRecord {
                record_kind: request.record_kind,
                record_id: request.record_id.clone(),
                payload: request.payload,
            },
        );
        self.latest.insert(
            (request.record_kind, request.record_id),
            storage_ref.clone(),
        );

        // Advance the cursor past this record so the next append is distinct.
        self.next_offset = offset + u64::from(length);
        self.pending_writes += 1;

        Ok(RecordPersistResult { storage_ref })
    }

    fn resolve_latest(
        &self,
        request: ResolveLatestRequest,
    ) -> Result<Option<StorageRef>, StorageApiError> {
        Ok(self
            .latest
            .get(&(request.record_kind, request.record_id))
            .cloned())
    }

    fn read_record(
        &self,
        request: ReadRecordRequest,
    ) -> Result<ReadRecordResponse, StorageApiError> {
        let storage_ref = request.storage_ref;
        let record = self.records.get(&storage_ref.offset).filter(|record| {
            // The offset uniquely identifies the record; validate the rest of
            // the ref matches so a mismatched segment or length is reported as
            // missing rather than silently returning an unrelated payload.
            storage_ref.segment == IN_MEMORY_SEGMENT
                && storage_ref.length as usize == record.payload.len()
        });

        match record {
            Some(record) => Ok(ReadRecordResponse {
                record_kind: record.record_kind,
                record_id: record.record_id.clone(),
                payload: record.payload.clone(),
            }),
            None => Err(StorageApiError::RecordNotFound(format!(
                "no record at {}:{}",
                storage_ref.segment, storage_ref.offset
            ))),
        }
    }

    fn flush(&mut self) -> Result<FlushResult, StorageApiError> {
        let flushed_segments = self.pending_writes;
        self.pending_writes = 0;
        Ok(FlushResult { flushed_segments })
    }
}

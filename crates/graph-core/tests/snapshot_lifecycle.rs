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

use graph_core::{
    ActorId, GraphError, Snapshot, SnapshotCreateRequest, SnapshotId, SnapshotManager,
    TransactionId,
};

fn assert_clone_debug<T: Clone + std::fmt::Debug>() {
    let _ = std::any::type_name::<T>();
}

fn assert_type<T>() {
    let _ = std::any::type_name::<T>();
}

//
// Verify snapshot primitives are exposed through the public graph-core facade.
#[test]
fn public_facade_exports_snapshot_contract_types() {
    assert_clone_debug::<SnapshotId>();
    assert_clone_debug::<Snapshot>();
    assert_clone_debug::<SnapshotCreateRequest>();
    assert_type::<SnapshotManager>();
}

//
// Verify snapshot create-request validation rejects empty required fields.
#[test]
fn snapshot_create_request_rejects_empty_required_fields() {
    let request_error = SnapshotCreateRequest::new(
        SnapshotId::new("snapshot--1").expect("snapshot ID should be valid"),
        TransactionId::new("transaction--1").expect("transaction ID should be valid"),
        ActorId::new("actor--1").expect("actor ID should be valid"),
        "",
        "checkpoint",
    )
    .expect_err("empty required field should be rejected");

    assert!(matches!(
    request_error,
    GraphError::InvalidPropertyValue(message)
    if message == "snapshot create request field must not be empty: reason"
    ));
}

//
// Verify snapshot manager stores lifecycle fields and metadata deterministically.
#[test]
fn snapshot_manager_creates_and_reads_snapshot_records() {
    let mut manager = SnapshotManager::new();
    let mut metadata = HashMap::new();
    metadata.insert("workspace".to_owned(), "workspace--incident-1".to_owned());

    let request = SnapshotCreateRequest::new(
        SnapshotId::new("snapshot--incident-1").expect("snapshot ID should be valid"),
        TransactionId::new("transaction--500").expect("transaction ID should be valid"),
        ActorId::new("actor--orchestrator").expect("actor ID should be valid"),
        "checkpoint before export",
        "incident-1-baseline",
    )
    .expect("request should be valid")
    .with_export_context("stix-mvp-v1", "stix-mvp")
    .with_metadata(metadata.clone());

    let created = manager
        .create_snapshot(request, "2026-07-06T15:10:00Z")
        .expect("snapshot should be created");

    assert_eq!(created.id().as_str(), "snapshot--incident-1");
    assert_eq!(created.transaction_id().as_str(), "transaction--500");
    assert_eq!(created.created_by().as_str(), "actor--orchestrator");
    assert_eq!(created.reason(), "checkpoint before export");
    assert_eq!(created.label(), "incident-1-baseline");
    assert_eq!(created.exporter_version(), Some("stix-mvp-v1"));
    assert_eq!(created.profile(), Some("stix-mvp"));
    assert_eq!(created.metadata(), &metadata);

    let fetched = manager
        .snapshot(created.id())
        .expect("snapshot should be retrievable by ID");
    assert_eq!(fetched, &created);
}

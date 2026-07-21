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
use storage_api::{
    AppendRecordRequest, MutationAuditFingerprint, ResolveLatestRequest, StorageApiError,
    StorageRecordKind, StorageRef, VersionTransition,
};

//
// Verify the storage abstraction API enforces basic typed construction rules.
#[test]
fn contract_validates_minimum_shapes_for_core_requests() {
    let append =
        AppendRecordRequest::new(StorageRecordKind::NodeVersion, "node--1", vec![0x01, 0x02])
            .expect("append request should be valid");

    let resolve = ResolveLatestRequest::new(StorageRecordKind::NodeVersion, "node--1")
        .expect("resolve request should be valid");

    assert_eq!(append.record_id, "node--1");
    assert_eq!(resolve.record_id, "node--1");
}

//
// Verify audit fingerprint contract captures explicit transition metadata.
#[test]
fn audit_fingerprint_captures_query_hash_and_transitions() {
    let transition = VersionTransition::new(
        "node--1",
        Some("node-version--1".to_owned()),
        "node-version--2",
    )
    .expect("transition should be valid");

    let fingerprint = MutationAuditFingerprint::new("f00dbabe", vec![transition])
        .expect("fingerprint should be valid");

    assert_eq!(fingerprint.query_text_hash, "f00dbabe");
    assert_eq!(fingerprint.transitions.len(), 1);
}

//
// Verify storage reference validation rejects zero-length records.
#[test]
fn storage_ref_rejects_zero_length() {
    let error = StorageRef::new("segment-node-log", 10, 0)
        .expect_err("zero-length refs should be rejected");

    assert!(matches!(
    error,
    StorageApiError::InvalidStorageRefField(field) if field == "length"
    ));
}

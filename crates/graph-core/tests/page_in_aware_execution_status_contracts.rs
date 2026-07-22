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
use graph_core::{
    ExecutionContinuation, ExecutionStatusCode, PageIdentity, PageIdentityKind,
    PageInAwareExecutionStatus, PageInStatus, StorageRef,
};

//
// Verify execution status can represent complete, partial, blocked, and rejected
// outcomes with explicit machine-matchable status codes.
#[test]
fn page_in_aware_execution_status_codes_are_explicit_and_matchable() {
    assert_eq!(ExecutionStatusCode::Complete, ExecutionStatusCode::Complete);
    assert_eq!(ExecutionStatusCode::Partial, ExecutionStatusCode::Partial);
    assert_eq!(ExecutionStatusCode::Blocked, ExecutionStatusCode::Blocked);
    assert_eq!(ExecutionStatusCode::Rejected, ExecutionStatusCode::Rejected);
}

//
// Verify the contract can carry page-in result context and continuation hints
// for bounded traversal resumption.
#[test]
fn page_in_aware_execution_status_carries_page_context_and_continuation() {
    let status = PageInAwareExecutionStatus::new(
        ExecutionStatusCode::Partial,
        "Expansion paused after budget guard.",
    )
    .with_page_in_status(PageInStatus::Loaded)
    .with_page(PageIdentity {
        kind: PageIdentityKind::Adjacency,
        segment: "adjacency/outgoing".to_owned(),
        page_id: "node--42/outgoing/page-2".to_owned(),
        storage_ref: Some(StorageRef::Page {
            segment: "adjacency/outgoing".to_owned(),
            page_id: 2,
        }),
    })
    .with_continuation(ExecutionContinuation {
        token: "continue://ws-1/frontier-2".to_owned(),
        resume_from_hop: 2,
    });

    assert_eq!(status.code(), ExecutionStatusCode::Partial);
    assert_eq!(status.message(), "Expansion paused after budget guard.");
    assert_eq!(status.page_in_status(), Some(PageInStatus::Loaded));
    assert!(matches!(
    status.page(),
    Some(PageIdentity {
    kind: PageIdentityKind::Adjacency,
    segment,
    page_id,
    ..
    }) if segment == "adjacency/outgoing" && page_id == "node--42/outgoing/page-2"
    ));
    assert!(matches!(
    status.continuation(),
    Some(ExecutionContinuation { token, resume_from_hop })
    if token == "continue://ws-1/frontier-2" && *resume_from_hop == 2
    ));
}

//
// Verify rejected execution statuses can include deterministic fix hints for
// callers and agent-safe remediation.
#[test]
fn page_in_aware_execution_status_supports_fix_hints() {
    let status = PageInAwareExecutionStatus::new(
        ExecutionStatusCode::Rejected,
        "Traversal rejected due to overbroad objective.",
    )
    .with_fix_hint("Add relationship filters and explicit limit before retry.");

    assert_eq!(status.code(), ExecutionStatusCode::Rejected);
    assert_eq!(
        status.fix_hint(),
        Some("Add relationship filters and explicit limit before retry.")
    );
}

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
//! Page-in aware execution status contracts.
//!
//! This module defines status payloads used by advanced traversal execution
//! when page-in, budget guards, or policy constraints influence completion.

use serde::{Deserialize, Serialize};

use crate::graph_pager::{PageIdentity, PageInStatus};

/// Stable status code for page-in aware traversal execution.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ExecutionStatusCode {
    /// Execution completed within configured bounds.
    Complete,

    /// Execution returned a bounded subset and can continue later.
    Partial,

    /// Execution paused because safety policy blocked further expansion.
    Blocked,

    /// Execution was rejected before traversal continued.
    Rejected,
}

/// Continuation payload for resuming bounded traversal execution.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExecutionContinuation {
    /// Opaque continuation token for the next request.
    pub token: String,

    /// Hop index where traversal can resume.
    pub resume_from_hop: u64,
}

/// Structured page-in aware execution status contract.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PageInAwareExecutionStatus {
    code: ExecutionStatusCode,
    message: String,
    page_in_status: Option<PageInStatus>,
    page: Option<PageIdentity>,
    continuation: Option<ExecutionContinuation>,
    fix_hint: Option<String>,
}

impl PageInAwareExecutionStatus {
    /// Build a status payload with mandatory code and message.
    pub fn new(code: ExecutionStatusCode, message: impl Into<String>) -> Self {
        Self {
            code,
            // Message.
            message: message.into(),
            // Page in status.
            page_in_status: None,
            // Page.
            page: None,
            // Continuation.
            continuation: None,
            // Fix hint.
            fix_hint: None,
        }
    }

    /// Attach page-in outcome context.
    pub fn with_page_in_status(mut self, page_in_status: PageInStatus) -> Self {
        self.page_in_status = Some(page_in_status);
        self
    }

    /// Attach the page identity involved in the status outcome.
    pub fn with_page(mut self, page: PageIdentity) -> Self {
        self.page = Some(page);
        self
    }

    /// Attach continuation payload for resumable execution.
    pub fn with_continuation(mut self, continuation: ExecutionContinuation) -> Self {
        self.continuation = Some(continuation);
        self
    }

    /// Attach an optional remediation hint.
    pub fn with_fix_hint(mut self, fix_hint: impl Into<String>) -> Self {
        self.fix_hint = Some(fix_hint.into());
        self
    }

    /// Return the stable execution status code.
    pub fn code(&self) -> ExecutionStatusCode {
        self.code
    }

    /// Return human-readable status message.
    pub fn message(&self) -> &str {
        self.message.as_str()
    }

    /// Return optional page-in status.
    pub fn page_in_status(&self) -> Option<PageInStatus> {
        self.page_in_status
    }

    /// Return optional page identity associated with the status.
    pub fn page(&self) -> Option<&PageIdentity> {
        self.page.as_ref()
    }

    /// Return optional continuation payload.
    pub fn continuation(&self) -> Option<&ExecutionContinuation> {
        self.continuation.as_ref()
    }

    /// Return optional fix hint.
    pub fn fix_hint(&self) -> Option<&str> {
        self.fix_hint.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PageIdentityKind, StorageRef};

    #[test]
    fn status_can_represent_blocked_execution_with_page_context() {
        let status = PageInAwareExecutionStatus::new(
            ExecutionStatusCode::Blocked,
            "Traversal blocked by policy.",
        )
        .with_page_in_status(PageInStatus::Miss)
        .with_page(PageIdentity {
            kind: PageIdentityKind::Adjacency,
            segment: "adjacency/outgoing".to_owned(),
            page_id: "node--1/outgoing/page-1".to_owned(),
            storage_ref: Some(StorageRef::Page {
                segment: "adjacency/outgoing".to_owned(),
                page_id: 1,
            }),
        })
        .with_fix_hint("Add narrower filters before retry.");

        assert_eq!(status.code(), ExecutionStatusCode::Blocked);
        assert_eq!(status.page_in_status(), Some(PageInStatus::Miss));
        assert_eq!(
            status.fix_hint(),
            Some("Add narrower filters before retry.")
        );
        assert!(status.page().is_some());
    }
}

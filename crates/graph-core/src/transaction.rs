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
//! Transaction and provenance metadata shape for graph-core records.
//!
//! Module boundary:
//! this module owns optional metadata links that identify workspaces, actors,
//! sessions, requests, transactions, and extraction runs. It does not own IAM,
//! audit storage, authorization policy, or workflow orchestration.

use serde::{Deserialize, Serialize};

use crate::{ActorId, ExtractionRunId, RequestId, SessionId, TransactionId, WorkspaceId};

/// Optional transaction and provenance references attached to graph records.
///
/// These fields keep graph-core records traceable without coupling the in-memory
/// graph core to external audit systems or request orchestration layers.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransactionMetadata {
    /// Stable identifier of the logical transaction that produced the record.
    pub transaction_id: Option<TransactionId>,
    /// Workspace context that owns or produced the record.
    pub workspace_id: Option<WorkspaceId>,
    /// Actor context that produced or changed the record.
    pub actor_id: Option<ActorId>,
    /// Session context associated with the record mutation.
    pub session_id: Option<SessionId>,
    /// Request context associated with the record mutation.
    pub request_id: Option<RequestId>,
    /// Extraction run context associated with the record mutation.
    pub extraction_run_id: Option<ExtractionRunId>,
}

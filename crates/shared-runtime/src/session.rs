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
use crate::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Runtime session.
pub struct RuntimeSession {
    /// Id.
    pub id: SessionId,
    /// Actor.
    pub actor: ActorRef,
    /// Workspace id.
    pub workspace_id: WorkspaceId,
    /// Started at.
    pub started_at: RuntimeTimestamp,
    /// Metadata.
    pub metadata: HashMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Runtime session metadata.
pub struct RuntimeSessionMetadata {
    /// Id.
    pub id: SessionId,
    /// Actor.
    pub actor: ActorRef,
    /// Workspace id.
    pub workspace_id: WorkspaceId,
    /// Started at.
    pub started_at: RuntimeTimestamp,
    /// Metadata.
    pub metadata: HashMap<String, String>,
}

impl From<&RuntimeSession> for RuntimeSessionMetadata {
    fn from(value: &RuntimeSession) -> Self {
        Self {
            // Id.
            id: value.id.clone(),
            // Actor.
            actor: value.actor.clone(),
            // Workspace id.
            workspace_id: value.workspace_id.clone(),
            // Started at.
            started_at: value.started_at.clone(),
            // Metadata.
            metadata: value.metadata.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Start session request.
pub struct StartSessionRequest {
    /// Id.
    pub id: SessionId,
    /// Actor.
    pub actor: Option<ActorRef>,
    /// Workspace id.
    pub workspace_id: WorkspaceId,
    /// Started at.
    pub started_at: RuntimeTimestamp,
    /// Metadata.
    pub metadata: HashMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Runtime transaction metadata.
pub struct RuntimeTransactionMetadata {
    /// Transaction id.
    pub transaction_id: TransactionId,
    /// Workspace id.
    pub workspace_id: WorkspaceId,
    /// Session id.
    pub session_id: SessionId,
    /// Actor.
    pub actor: ActorRef,
    /// Started at.
    pub started_at: RuntimeTimestamp,
    /// Policy name.
    pub policy_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Create transaction metadata request.
pub struct CreateTransactionMetadataRequest {
    /// Transaction id.
    pub transaction_id: TransactionId,
    /// Session id.
    pub session_id: SessionId,
    /// Started at.
    pub started_at: RuntimeTimestamp,
    /// Policy name.
    pub policy_name: Option<String>,
}

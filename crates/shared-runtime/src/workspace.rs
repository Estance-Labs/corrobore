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
/// Workspace name.
pub struct WorkspaceName {
    value: String,
}

impl WorkspaceName {
    /// Creates a new instance.
    pub fn new(value: impl Into<String>) -> Result<Self, RuntimeError> {
        let value = value.into();

        if value.trim().is_empty() {
            return Err(RuntimeError::InvalidWorkspaceName);
        }

        Ok(Self { value })
    }

    /// Returns the value as str.
    pub fn as_str(&self) -> &str {
        self.value.as_str()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Workspace status.
pub enum WorkspaceStatus {
    /// Open.
    Open,
    /// Closed.
    Closed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Workspace.
pub struct Workspace {
    /// Id.
    pub id: WorkspaceId,
    /// Name.
    pub name: WorkspaceName,
    /// Status.
    pub status: WorkspaceStatus,
    /// Created by.
    pub created_by: ActorRef,
    /// Created at.
    pub created_at: RuntimeTimestamp,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Create workspace request.
pub struct CreateWorkspaceRequest {
    /// Id.
    pub id: WorkspaceId,
    /// Name.
    pub name: WorkspaceName,
    /// Created by.
    pub created_by: ActorRef,
    /// Created at.
    pub created_at: RuntimeTimestamp,
}

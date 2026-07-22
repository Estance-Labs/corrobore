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

#[derive(Clone, Debug, Default)]
/// Workspace registry.
pub struct WorkspaceRegistry {
    workspaces_by_id: HashMap<String, Workspace>,
}

impl WorkspaceRegistry {
    //
    // Keep workspace creation in a single operation that validates uniqueness,
    // sets the initial open status, and stores the workspace in the registry.
    /// Creates the workspace.
    pub fn create_workspace(
        &mut self,
        request: CreateWorkspaceRequest,
    ) -> Result<WorkspaceId, RuntimeError> {
        let key = request.id.as_str().to_owned();

        if self.workspaces_by_id.contains_key(&key) {
            return Err(RuntimeError::WorkspaceAlreadyExists(request.id));
        }

        let workspace = Workspace {
            id: request.id.clone(),
            name: request.name,
            status: WorkspaceStatus::Open,
            created_by: request.created_by,
            created_at: request.created_at,
        };

        self.workspaces_by_id.insert(key, workspace);

        Ok(request.id)
    }

    //
    // Expose read-only access to a workspace by ID so runtime components can
    // validate scope before operating.
    /// Workspace.
    pub fn workspace(&self, workspace_id: &WorkspaceId) -> Result<&Workspace, RuntimeError> {
        self.workspaces_by_id
            .get(workspace_id.as_str())
            .ok_or_else(|| RuntimeError::WorkspaceNotFound(workspace_id.clone()))
    }

    //
    // Support lifecycle transition from open to closed without deleting
    // workspace metadata.
    /// Close workspace.
    pub fn close_workspace(&mut self, workspace_id: &WorkspaceId) -> Result<(), RuntimeError> {
        let workspace = self
            .workspaces_by_id
            .get_mut(workspace_id.as_str())
            .ok_or_else(|| RuntimeError::WorkspaceNotFound(workspace_id.clone()))?;

        workspace.status = WorkspaceStatus::Closed;

        Ok(())
    }

    //
    // Provide a complete registry listing for runtime orchestration and
    // observability.
    /// List workspaces.
    pub fn list_workspaces(&self) -> Vec<&Workspace> {
        self.workspaces_by_id.values().collect()
    }
}

#[derive(Clone, Debug, Default)]
/// Session registry.
pub struct SessionRegistry {
    sessions_by_id: HashMap<String, RuntimeSession>,
}

impl SessionRegistry {
    // Runtime sessions are mandatory for mutation and query traceability.
    // Anonymous graph operations are rejected so every operation is bound to
    // explicit actor/session metadata for auditability.
    /// Start session.
    pub fn start_session(
        &mut self,
        request: StartSessionRequest,
    ) -> Result<SessionId, RuntimeError> {
        let key = request.id.as_str().to_owned();

        if self.sessions_by_id.contains_key(&key) {
            return Err(RuntimeError::SessionAlreadyExists(request.id));
        }

        let actor = request.actor.ok_or(RuntimeError::MissingActor)?;

        let session = RuntimeSession {
            id: request.id.clone(),
            actor,
            workspace_id: request.workspace_id,
            started_at: request.started_at,
            metadata: request.metadata,
        };

        self.sessions_by_id.insert(key, session);

        Ok(request.id)
    }

    /// Session.
    pub fn session(&self, session_id: &SessionId) -> Result<&RuntimeSession, RuntimeError> {
        self.sessions_by_id
            .get(session_id.as_str())
            .ok_or_else(|| RuntimeError::SessionNotFound(session_id.clone()))
    }

    /// Returns the session metadata.
    pub fn read_session_metadata(
        &self,
        session_id: &SessionId,
    ) -> Result<RuntimeSessionMetadata, RuntimeError> {
        let session = self.session(session_id)?;
        Ok(RuntimeSessionMetadata::from(session))
    }

    /// Validates the workspace session consistency.
    pub fn validate_workspace_session_consistency(
        &self,
        workspace_id: &WorkspaceId,
        session_id: &SessionId,
    ) -> Result<(), RuntimeError> {
        let session = self.session(session_id)?;

        if session.workspace_id != *workspace_id {
            return Err(RuntimeError::WorkspaceSessionMismatch {
                workspace_id: workspace_id.clone(),
                session_workspace_id: session.workspace_id.clone(),
                session_id: session_id.clone(),
            });
        }

        Ok(())
    }

    // Runtime transaction metadata captures identity and policy context.
    // It is not a full ACID transaction manager and does not provide
    // isolation/locking guarantees; those belong to future storage layers.
    /// Creates the transaction metadata from session.
    pub fn create_transaction_metadata_from_session(
        &self,
        request: CreateTransactionMetadataRequest,
    ) -> Result<RuntimeTransactionMetadata, RuntimeError> {
        let session = self.session(&request.session_id)?;
        let policy_name = request
            .policy_name
            .ok_or(RuntimeError::MissingTransactionMetadata("policy_name"))?;

        if policy_name.trim().is_empty() {
            return Err(RuntimeError::MissingTransactionMetadata("policy_name"));
        }

        Ok(RuntimeTransactionMetadata {
            transaction_id: request.transaction_id,
            workspace_id: session.workspace_id.clone(),
            session_id: session.id.clone(),
            actor: session.actor.clone(),
            started_at: request.started_at,
            policy_name,
        })
    }

    // Validation remains metadata-focused and intentionally lightweight so this
    // model stays compatible with future WAL and audit-log integration.
    /// Validates the transaction metadata for mutation.
    pub fn validate_transaction_metadata_for_mutation(
        &self,
        metadata: Option<&RuntimeTransactionMetadata>,
    ) -> Result<(), RuntimeError> {
        let metadata = metadata.ok_or(RuntimeError::MissingTransactionMetadata(
            "transaction_metadata",
        ))?;

        let session = self.session(&metadata.session_id)?;

        if metadata.workspace_id != session.workspace_id {
            return Err(RuntimeError::TransactionWorkspaceMismatch {
                transaction_id: metadata.transaction_id.clone(),
                workspace_id: metadata.workspace_id.clone(),
                session_workspace_id: session.workspace_id.clone(),
                session_id: metadata.session_id.clone(),
            });
        }

        if metadata.actor != session.actor {
            return Err(RuntimeError::TransactionActorMismatch {
                transaction_id: metadata.transaction_id.clone(),
                actor_id: metadata.actor.actor_id.clone(),
                session_actor_id: session.actor.actor_id.clone(),
                session_id: metadata.session_id.clone(),
            });
        }

        Ok(())
    }
}

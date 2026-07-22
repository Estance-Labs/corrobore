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
use graph_core::{ActorId, WorkspaceId};
use shared_runtime::{
    ActorKind, ActorRef, CreateWorkspaceRequest, RuntimeTimestamp, WorkspaceName,
    WorkspaceRegistry, WorkspaceStatus,
};

fn actor_ref(value: &str) -> ActorRef {
    ActorRef::new(
        ActorId::new(value).expect("actor id should be valid"),
        ActorKind::Agent,
    )
}

#[test]
fn create_workspace_registers_workspace_with_expected_metadata() {
    let mut registry = WorkspaceRegistry::default();
    let created_by = actor_ref("actor--runtime");
    let workspace_name = WorkspaceName::new("Case Alpha").expect("workspace name should be valid");

    let workspace_id = registry
        .create_workspace(CreateWorkspaceRequest {
            id: WorkspaceId::new("workspace--alpha").expect("workspace id should be valid"),
            name: workspace_name.clone(),
            created_by: created_by.clone(),
            created_at: RuntimeTimestamp::from_millis(1_727_000_000_000),
        })
        .expect("workspace creation should succeed");

    let workspace = registry
        .workspace(&workspace_id)
        .expect("workspace should be retrievable after creation");

    assert_eq!(workspace.id, workspace_id);
    assert_eq!(workspace.name, workspace_name);
    assert_eq!(workspace.status, WorkspaceStatus::Open);
    assert_eq!(workspace.created_by, created_by);
}

#[test]
fn create_workspace_rejects_duplicate_workspace_id() {
    let mut registry = WorkspaceRegistry::default();
    let id = WorkspaceId::new("workspace--duplicate").expect("workspace id should be valid");

    let first_result = registry.create_workspace(CreateWorkspaceRequest {
        id: id.clone(),
        name: WorkspaceName::new("Case One").expect("workspace name should be valid"),
        created_by: actor_ref("actor--one"),
        created_at: RuntimeTimestamp::from_millis(1),
    });

    let second_result = registry.create_workspace(CreateWorkspaceRequest {
        id,
        name: WorkspaceName::new("Case Two").expect("workspace name should be valid"),
        created_by: actor_ref("actor--two"),
        created_at: RuntimeTimestamp::from_millis(2),
    });

    assert!(first_result.is_ok());
    assert!(second_result.is_err());
}

#[test]
fn close_workspace_marks_workspace_as_closed() {
    let mut registry = WorkspaceRegistry::default();
    let workspace_id = registry
        .create_workspace(CreateWorkspaceRequest {
            id: WorkspaceId::new("workspace--close").expect("workspace id should be valid"),
            name: WorkspaceName::new("Case Closure").expect("workspace name should be valid"),
            created_by: actor_ref("actor--closer"),
            created_at: RuntimeTimestamp::from_millis(10),
        })
        .expect("workspace creation should succeed");

    registry
        .close_workspace(&workspace_id)
        .expect("close workspace should succeed");

    let workspace = registry
        .workspace(&workspace_id)
        .expect("workspace should still be retrievable after close");

    assert_eq!(workspace.status, WorkspaceStatus::Closed);
}

#[test]
fn list_workspaces_returns_registered_workspaces() {
    let mut registry = WorkspaceRegistry::default();

    registry
        .create_workspace(CreateWorkspaceRequest {
            id: WorkspaceId::new("workspace--a").expect("workspace id should be valid"),
            name: WorkspaceName::new("Case A").expect("workspace name should be valid"),
            created_by: actor_ref("actor--a"),
            created_at: RuntimeTimestamp::from_millis(1),
        })
        .expect("workspace creation should succeed");

    registry
        .create_workspace(CreateWorkspaceRequest {
            id: WorkspaceId::new("workspace--b").expect("workspace id should be valid"),
            name: WorkspaceName::new("Case B").expect("workspace name should be valid"),
            created_by: actor_ref("actor--b"),
            created_at: RuntimeTimestamp::from_millis(2),
        })
        .expect("workspace creation should succeed");

    let all = registry.list_workspaces();

    assert_eq!(all.len(), 2);
}

#[test]
fn workspace_lookup_returns_not_found_for_unknown_id() {
    let registry = WorkspaceRegistry::default();
    let missing = WorkspaceId::new("workspace--missing").expect("workspace id should be valid");

    let error = registry
        .workspace(&missing)
        .expect_err("lookup for missing workspace should fail");

    assert_eq!(
        error,
        shared_runtime::RuntimeError::WorkspaceNotFound(missing)
    );
}

#[test]
fn close_workspace_returns_not_found_for_unknown_id() {
    let mut registry = WorkspaceRegistry::default();
    let missing =
        WorkspaceId::new("workspace--missing-close").expect("workspace id should be valid");

    let error = registry
        .close_workspace(&missing)
        .expect_err("close on missing workspace should fail");

    assert_eq!(
        error,
        shared_runtime::RuntimeError::WorkspaceNotFound(missing)
    );
}

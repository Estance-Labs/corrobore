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
#![allow(clippy::unwrap_used)]
use graph_core::{
    GraphError, GraphRuntime, GraphStoreRef, RuntimeId, RuntimeOpenRequest, RuntimePolicyRef,
    SessionRegistryRef, WorkspaceRegistryRef,
};

fn runtime_id(value: &str) -> RuntimeId {
    RuntimeId::new(value).expect("test runtime ID should be valid")
}

fn baseline_open_request() -> RuntimeOpenRequest {
    RuntimeOpenRequest {
        runtime_id: runtime_id("runtime--epic-0004"),
        graph_store: GraphStoreRef::new("durable-store").expect("graph store ref should be valid"),
        workspace_registry: WorkspaceRegistryRef::new("workspace-registry")
            .expect("workspace registry ref should be valid"),
        session_registry: SessionRegistryRef::new("session-registry")
            .expect("session registry ref should be valid"),
        policy: RuntimePolicyRef::new("default-runtime-policy")
            .expect("runtime policy ref should be valid"),
    }
}

//
// Verify that the runtime boundary is available as a stable public API surface
// from graph-core's crate facade. This protects the initial runtime layer from
// leaking private module details to callers.
//
// Given runtime boundary types exposed through `graph_core`,
// when an integration test imports those types only from the public facade,
// then the runtime API should be available without private-module imports.
#[test]
fn runtime_boundary_types_are_available_from_the_public_facade() {
    let request = baseline_open_request();
    let runtime = GraphRuntime::open(request).expect("runtime should open");

    assert!(runtime.is_open());
}

//
// Verify that the runtime open lifecycle preserves boundary references while
// remaining independent from storage layout internals.
//
// Given a valid runtime open request with boundary references,
// when the runtime is opened,
// then the runtime should remain open and expose the same runtime identity.
#[test]
fn runtime_open_lifecycle_keeps_runtime_open_with_boundary_references() {
    let expected_runtime_id = runtime_id("runtime--shared-1");
    let request = RuntimeOpenRequest {
        runtime_id: expected_runtime_id.clone(),
        graph_store: GraphStoreRef::new("graph-store-ref")
            .expect("graph store ref should be valid"),
        workspace_registry: WorkspaceRegistryRef::new("workspace-registry-ref")
            .expect("workspace registry ref should be valid"),
        session_registry: SessionRegistryRef::new("session-registry-ref")
            .expect("session registry ref should be valid"),
        policy: RuntimePolicyRef::new("policy-ref").expect("policy ref should be valid"),
    };

    let runtime = GraphRuntime::open(request).expect("runtime open should succeed");

    assert_eq!(runtime.runtime_id(), &expected_runtime_id);
    assert!(runtime.is_open());
}

//
// Verify that runtime configuration validation reports a deterministic typed
// error when required boundary references are invalid.
//
// Given an open request with an invalid graph store reference,
// when the runtime is opened,
// then opening should fail with `GraphError::InvalidRuntimeConfiguration`.
#[test]
fn runtime_open_rejects_invalid_runtime_configuration() {
    let request = RuntimeOpenRequest {
        runtime_id: runtime_id("runtime--invalid-config"),
        graph_store: GraphStoreRef::new(" ").expect("constructor should reserve invalid value"),
        workspace_registry: WorkspaceRegistryRef::new("workspace-registry")
            .expect("workspace registry ref should be valid"),
        session_registry: SessionRegistryRef::new("session-registry")
            .expect("session registry ref should be valid"),
        policy: RuntimePolicyRef::new("default-runtime-policy")
            .expect("runtime policy ref should be valid"),
    };

    let error = GraphRuntime::open(request).expect_err("invalid configuration should fail");

    assert!(matches!(error, GraphError::InvalidRuntimeConfiguration(_)));
}

//
// Verify that runtime state-checking APIs produce an explicit typed error when
// called on a runtime that is not open.
//
// Given a runtime test fixture in closed state,
// when open-state validation is requested,
// then the API should return `GraphError::RuntimeNotOpen`.
#[test]
fn runtime_state_check_returns_runtime_not_open_error_for_closed_runtime() {
    let runtime = GraphRuntime::closed_for_tests(runtime_id("runtime--closed"));

    let error = runtime
        .ensure_open()
        .expect_err("closed runtime should fail open-state validation");

    assert!(matches!(error, GraphError::RuntimeNotOpen));
}

#[test]
fn runtime_state_check_succeeds_for_open_runtime() {
    let runtime = GraphRuntime::open(baseline_open_request())
        .expect("baseline open request should create an open runtime");

    runtime
        .ensure_open()
        .expect("open runtime should pass open-state validation");
}

#[test]
fn runtime_open_rejects_blank_workspace_registry_reference() {
    let request = RuntimeOpenRequest {
        runtime_id: runtime_id("runtime--invalid-workspace-registry"),
        graph_store: GraphStoreRef::new("durable-store").expect("graph store ref should be valid"),
        workspace_registry: WorkspaceRegistryRef::new(" ")
            .expect("constructor should reserve invalid value"),
        session_registry: SessionRegistryRef::new("session-registry")
            .expect("session registry ref should be valid"),
        policy: RuntimePolicyRef::new("default-runtime-policy")
            .expect("runtime policy ref should be valid"),
    };

    let error = GraphRuntime::open(request).expect_err("invalid workspace registry should fail");

    assert!(matches!(
    error,
    GraphError::InvalidRuntimeConfiguration(message)
    if message == "workspace registry reference is required"
    ));
}

#[test]
fn runtime_open_rejects_blank_session_registry_reference() {
    let request = RuntimeOpenRequest {
        runtime_id: runtime_id("runtime--invalid-session-registry"),
        graph_store: GraphStoreRef::new("durable-store").expect("graph store ref should be valid"),
        workspace_registry: WorkspaceRegistryRef::new("workspace-registry")
            .expect("workspace registry ref should be valid"),
        session_registry: SessionRegistryRef::new(" ")
            .expect("constructor should reserve invalid value"),
        policy: RuntimePolicyRef::new("default-runtime-policy")
            .expect("runtime policy ref should be valid"),
    };

    let error = GraphRuntime::open(request).expect_err("invalid session registry should fail");

    assert!(matches!(
    error,
    GraphError::InvalidRuntimeConfiguration(message)
    if message == "session registry reference is required"
    ));
}

#[test]
fn runtime_open_rejects_blank_runtime_policy_reference() {
    let request = RuntimeOpenRequest {
        runtime_id: runtime_id("runtime--invalid-runtime-policy"),
        graph_store: GraphStoreRef::new("durable-store").expect("graph store ref should be valid"),
        workspace_registry: WorkspaceRegistryRef::new("workspace-registry")
            .expect("workspace registry ref should be valid"),
        session_registry: SessionRegistryRef::new("session-registry")
            .expect("session registry ref should be valid"),
        policy: RuntimePolicyRef::new(" ").expect("constructor should reserve invalid value"),
    };

    let error = GraphRuntime::open(request).expect_err("invalid runtime policy should fail");

    assert!(matches!(
    error,
    GraphError::InvalidRuntimeConfiguration(message)
    if message == "runtime policy reference is required"
    ));
}

#[test]
fn runtime_accessors_return_runtime_scoped_boundary_references() {
    let runtime = GraphRuntime::open(RuntimeOpenRequest {
        runtime_id: runtime_id("runtime--accessor-check"),
        graph_store: GraphStoreRef::new("graph-store-ref")
            .expect("graph store ref should be valid"),
        workspace_registry: WorkspaceRegistryRef::new("workspace-registry-ref")
            .expect("workspace registry ref should be valid"),
        session_registry: SessionRegistryRef::new("session-registry-ref")
            .expect("session registry ref should be valid"),
        policy: RuntimePolicyRef::new("policy-ref").expect("policy ref should be valid"),
    })
    .expect("runtime should open");

    assert_eq!(
        runtime.graph_store(),
        &GraphStoreRef::new("graph-store-ref").unwrap()
    );
    assert_eq!(
        runtime.workspace_registry(),
        &WorkspaceRegistryRef::new("workspace-registry-ref").unwrap()
    );
    assert_eq!(
        runtime.session_registry(),
        &SessionRegistryRef::new("session-registry-ref").unwrap()
    );
    assert_eq!(
        runtime.policy(),
        &RuntimePolicyRef::new("policy-ref").unwrap()
    );
}

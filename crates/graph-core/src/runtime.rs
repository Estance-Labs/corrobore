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
use serde::{Deserialize, Serialize};

use crate::{
    ExpansionBudget, ExpansionDirection, ExpansionFilters, ExpansionGuards, ExpansionResult,
    GraphError, GraphPager, GraphWorkingSetManager, GraphWorkingSetStats, LoadingProfile, NodeId,
    RuntimeId, SupernodePolicy, WorkingSetEvictionOutcome, WorkingSetHotBudget, WorkingSetId,
    expand_working_set_from_graph_adjacency, working_set_expansion::ExpansionRequest,
};

/// Opaque boundary reference to the graph store used by a runtime.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphStoreRef {
    name: String,
}

impl GraphStoreRef {
    /// Reserve a graph store boundary reference for runtime open requests.
    ///
    /// Validation intent:
    ///
    /// - This constructor keeps value capture lightweight so issue-level tests
    ///   can construct both valid and invalid runtime-open requests.
    /// - Runtime-level validation happens in `RuntimeOpenRequest::validate`.
    pub fn new(name: impl Into<String>) -> Result<Self, GraphError> {
        Ok(Self { name: name.into() })
    }

    fn validate(&self) -> Result<(), GraphError> {
        if self.name.trim().is_empty() {
            return Err(GraphError::InvalidRuntimeConfiguration(
                "graph store reference is required".to_owned(),
            ));
        }

        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Workspace registry ref.
pub struct WorkspaceRegistryRef {
    name: String,
}

impl WorkspaceRegistryRef {
    /// Reserve a workspace-registry boundary reference for runtime open
    /// requests.
    ///
    /// Validation intent:
    ///
    /// - Keep construction simple in this issue.
    /// - Enforce required values at runtime-open validation boundaries.
    pub fn new(name: impl Into<String>) -> Result<Self, GraphError> {
        Ok(Self { name: name.into() })
    }

    fn validate(&self) -> Result<(), GraphError> {
        if self.name.trim().is_empty() {
            return Err(GraphError::InvalidRuntimeConfiguration(
                "workspace registry reference is required".to_owned(),
            ));
        }

        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Session registry ref.
pub struct SessionRegistryRef {
    name: String,
}

impl SessionRegistryRef {
    /// Reserve a session-registry boundary reference for runtime open requests.
    ///
    /// Validation intent:
    ///
    /// - Keep construction simple in this issue.
    /// - Enforce required values at runtime-open validation boundaries.
    pub fn new(name: impl Into<String>) -> Result<Self, GraphError> {
        Ok(Self { name: name.into() })
    }

    fn validate(&self) -> Result<(), GraphError> {
        if self.name.trim().is_empty() {
            return Err(GraphError::InvalidRuntimeConfiguration(
                "session registry reference is required".to_owned(),
            ));
        }

        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Runtime policy ref.
pub struct RuntimePolicyRef {
    name: String,
}

impl RuntimePolicyRef {
    /// Reserve a runtime-policy boundary reference for runtime open requests.
    ///
    /// Validation intent:
    ///
    /// - Keep construction simple in this issue.
    /// - Enforce required values at runtime-open validation boundaries.
    pub fn new(name: impl Into<String>) -> Result<Self, GraphError> {
        Ok(Self { name: name.into() })
    }

    fn validate(&self) -> Result<(), GraphError> {
        if self.name.trim().is_empty() {
            return Err(GraphError::InvalidRuntimeConfiguration(
                "runtime policy reference is required".to_owned(),
            ));
        }

        Ok(())
    }
}

/// Runtime boundary open request.
///
/// Responsibility scope:
///
/// - Captures typed boundary references needed to open a runtime.
/// - Keeps workspace/session/policy wiring explicit for future runtime features.
///
/// Non-goals in this issue:
///
/// - Does not model storage layout internals.
/// - Does not implement workspace/session registries.
/// - Does not implement Cypher gateway or audit behavior.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeOpenRequest {
    /// Runtime id.
    pub runtime_id: RuntimeId,
    /// Graph store.
    pub graph_store: GraphStoreRef,
    /// Workspace registry.
    pub workspace_registry: WorkspaceRegistryRef,
    /// Session registry.
    pub session_registry: SessionRegistryRef,
    /// Policy.
    pub policy: RuntimePolicyRef,
}

impl RuntimeOpenRequest {
    /// Validate that a runtime-open request contains all required boundary
    /// references before runtime construction.
    ///
    /// Validation behavior:
    ///
    /// - Ensure runtime ID is present by type construction.
    /// - Ensure graph store reference is not blank.
    /// - Ensure workspace registry reference is not blank.
    /// - Ensure session registry reference is not blank.
    /// - Ensure runtime policy reference is not blank.
    pub fn validate(&self) -> Result<(), GraphError> {
        self.graph_store.validate()?;
        self.workspace_registry.validate()?;
        self.session_registry.validate()?;
        self.policy.validate()?;
        Ok(())
    }
}

/// Shared runtime boundary above graph-core, working-set, and durable storage.
///
/// Responsibility scope:
///
/// - Owns runtime lifecycle state (`open` vs `not open`).
/// - Carries typed references to graph store, workspace registry, session
///   registry, and runtime policy boundaries.
///
/// Non-goals in this issue:
///
/// - Does not own or model storage layout.
/// - Does not implement workspace/session lifecycle behavior.
/// - Does not execute Cypher or emit audit events.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphRuntime {
    runtime_id: RuntimeId,
    graph_store: GraphStoreRef,
    workspace_registry: WorkspaceRegistryRef,
    session_registry: SessionRegistryRef,
    policy: RuntimePolicyRef,
    is_open: bool,
}

/// Pager-backed runtime query request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PagerBackedRuntimeQuery {
    /// Target working set identifier.
    pub working_set_id: WorkingSetId,
    /// Seed node identifiers.
    pub seed_node_ids: Vec<NodeId>,
    /// Expansion direction.
    pub direction: ExpansionDirection,
    /// Expansion filters.
    pub filters: ExpansionFilters,
    /// Expansion guards.
    pub guards: ExpansionGuards,
    /// Hop limit.
    pub hop_limit: u64,
    /// Loading profile.
    pub loading_profile: LoadingProfile,
    /// Expansion budget.
    pub expansion_budget: ExpansionBudget,
    /// Optional supernode policy.
    pub supernode_policy: Option<SupernodePolicy>,
}

impl PagerBackedRuntimeQuery {
    /// Build a pager-backed runtime query request.
    pub fn new(
        working_set_id: WorkingSetId,
        seed_node_ids: Vec<NodeId>,
        direction: ExpansionDirection,
        filters: ExpansionFilters,
        hop_limit: u64,
        loading_profile: LoadingProfile,
        expansion_budget: ExpansionBudget,
    ) -> Self {
        Self {
            working_set_id,
            seed_node_ids,
            direction,
            filters,
            guards: ExpansionGuards::empty(),
            hop_limit,
            loading_profile,
            expansion_budget,
            supernode_policy: None,
        }
    }

    /// Return a copy with non-filter guards applied.
    pub fn with_guards(mut self, guards: ExpansionGuards) -> Self {
        self.guards = guards;
        self
    }

    /// Return a copy with a supernode policy applied.
    pub fn with_supernode_policy(mut self, policy: SupernodePolicy) -> Self {
        self.supernode_policy = Some(policy);
        self
    }
}

/// Result payload for one pager-backed runtime query execution.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PagerBackedRuntimeResult {
    /// Working set targeted by this execution.
    pub working_set_id: WorkingSetId,
    /// Expansion result produced by graph-core expansion.
    pub expansion: ExpansionResult,
    /// Deterministic post-expansion eviction outcome.
    pub eviction: WorkingSetEvictionOutcome,
    /// Final working-set stats after eviction.
    pub stats: GraphWorkingSetStats,
}

/// Runtime wrapper integrating `GraphPager` expansion and deterministic hot-budget eviction.
#[derive(Clone, Debug)]
pub struct PagerBackedRuntime {
    manager: GraphWorkingSetManager,
    hot_budget: WorkingSetHotBudget,
}

impl PagerBackedRuntime {
    /// Construct an empty pager-backed runtime with a deterministic hot budget.
    pub fn new(hot_budget: WorkingSetHotBudget) -> Self {
        Self {
            manager: GraphWorkingSetManager::new(),
            hot_budget,
        }
    }

    /// Return the internal working-set manager.
    pub fn manager(&self) -> &GraphWorkingSetManager {
        &self.manager
    }

    /// Execute one pager-backed query request with expansion then deterministic eviction.
    pub fn execute_query<P: GraphPager + ?Sized>(
        &mut self,
        pager: &P,
        query: PagerBackedRuntimeQuery,
    ) -> Result<PagerBackedRuntimeResult, GraphError> {
        if self.manager.get_working_set(&query.working_set_id).is_err() {
            self.manager
                .create_working_set(crate::GraphWorkingSetCreateRequest::new(
                    query.working_set_id.clone(),
                ))?;
        }

        let mut expansion_request = ExpansionRequest::new(
            query.working_set_id.clone(),
            query.seed_node_ids.clone(),
            query.direction,
            query.filters.clone(),
            query.hop_limit,
            query.loading_profile.clone(),
            query.expansion_budget.clone(),
        )
        .with_guards(query.guards.clone());
        if let Some(policy) = query.supernode_policy {
            expansion_request = expansion_request.with_supernode_policy(policy);
        }

        let expansion =
            expand_working_set_from_graph_adjacency(&mut self.manager, pager, expansion_request)?;
        let eviction = self
            .manager
            .enforce_hot_budget_deterministic(&query.working_set_id, &self.hot_budget)?;
        let stats = self.manager.stats(&query.working_set_id)?.clone();

        Ok(PagerBackedRuntimeResult {
            working_set_id: query.working_set_id,
            expansion,
            eviction,
            stats,
        })
    }
}

impl GraphRuntime {
    /// Open a graph runtime from a validated runtime-open request.
    ///
    /// Validation and implementation behavior:
    ///
    /// - Validate request boundary references first.
    /// - Construct runtime boundary references without coupling to storage
    ///   layout internals.
    /// - Mark runtime as open once construction succeeds.
    pub fn open(request: RuntimeOpenRequest) -> Result<Self, GraphError> {
        request.validate()?;

        Ok(Self {
            runtime_id: request.runtime_id,
            graph_store: request.graph_store,
            workspace_registry: request.workspace_registry,
            session_registry: request.session_registry,
            policy: request.policy,
            is_open: true,
        })
    }

    /// Build a closed runtime fixture for integration tests that need to assert
    /// explicit runtime-not-open behavior.
    pub fn closed_for_tests(runtime_id: RuntimeId) -> Self {
        Self {
            runtime_id,
            // Graph store.
            graph_store: GraphStoreRef {
                // Name.
                name: "test-graph-store".to_owned(),
            },
            // Workspace registry.
            workspace_registry: WorkspaceRegistryRef {
                // Name.
                name: "test-workspace-registry".to_owned(),
            },
            // Session registry.
            session_registry: SessionRegistryRef {
                // Name.
                name: "test-session-registry".to_owned(),
            },
            // Policy.
            policy: RuntimePolicyRef {
                // Name.
                name: "test-policy".to_owned(),
            },
            // Is open.
            is_open: false,
        }
    }

    /// Return the runtime's stable typed identifier.
    pub fn runtime_id(&self) -> &RuntimeId {
        &self.runtime_id
    }

    /// Return whether the runtime is currently open.
    pub fn is_open(&self) -> bool {
        self.is_open
    }

    /// Validate that runtime state is open for operations that require an open
    /// lifecycle state.
    pub fn ensure_open(&self) -> Result<(), GraphError> {
        if self.is_open {
            Ok(())
        } else {
            Err(GraphError::RuntimeNotOpen)
        }
    }

    /// Return a runtime-scoped graph-store boundary reference.
    pub fn graph_store(&self) -> &GraphStoreRef {
        &self.graph_store
    }

    /// Return a runtime-scoped workspace-registry boundary reference.
    pub fn workspace_registry(&self) -> &WorkspaceRegistryRef {
        &self.workspace_registry
    }

    /// Return a runtime-scoped session-registry boundary reference.
    pub fn session_registry(&self) -> &SessionRegistryRef {
        &self.session_registry
    }

    /// Return a runtime-scoped policy boundary reference.
    pub fn policy(&self) -> &RuntimePolicyRef {
        &self.policy
    }
}

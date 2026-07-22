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
use super::*;

/// Direction policy for adjacency-driven working-set expansion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ExpansionDirection {
    /// Expand relationships where the frontier node is the source endpoint.
    Outgoing,
    /// Expand relationships where the frontier node is the target endpoint.
    Incoming,
    /// Expand both outgoing then incoming adjacency for each frontier node.
    Both,
}

/// Relationship-type and label filters applied before records become hot.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpansionFilters {
    pub(crate) relationship_type_filters: Vec<RelationshipType>,
    pub(crate) label_filters: LabelSet,
}

impl ExpansionFilters {
    /// Build explicit expansion filters while preserving caller-provided order.
    pub fn new(relationship_type_filters: Vec<RelationshipType>, label_filters: LabelSet) -> Self {
        Self {
            relationship_type_filters,
            label_filters,
        }
    }

    /// Build filters that accept every relationship type and every node label.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Return relationship type filters supplied by the caller.
    pub fn relationship_type_filters(&self) -> &[RelationshipType] {
        &self.relationship_type_filters
    }

    /// Return label filters supplied by the caller.
    pub fn label_filters(&self) -> &LabelSet {
        &self.label_filters
    }

    /// Return whether the request includes at least one relationship-type guard.
    pub fn has_relationship_type_filters(&self) -> bool {
        !self.relationship_type_filters.is_empty()
    }

    /// Return whether the request includes at least one label guard.
    pub fn has_label_filters(&self) -> bool {
        !self.label_filters.is_empty()
    }
}

/// Non-graph-filter guards that can make supernode expansion bounded.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpansionGuards {
    pub(crate) has_time_window: bool,
    pub(crate) explicit_limit: Option<u64>,
}

impl ExpansionGuards {
    /// Build explicit non-filter guards for a future supernode policy check.
    ///
    ///
    /// keep time-window and explicit result-limit presence separate from budget
    /// limits so can tell "bounded by caller intent" apart from "bounded
    /// by engine safety budget".
    ///
    ///
    /// later implementation should pass these booleans into `SupernodePolicy`
    /// before expanding a high-degree node.
    ///
    /// # Errors
    ///
    /// none expected at construction time; semantic validation belongs to the
    /// expansion planner that knows the original query or graph plan.
    pub fn new(has_time_window: bool, explicit_limit: Option<u64>) -> Self {
        Self {
            has_time_window,
            explicit_limit,
        }
    }

    /// Build guard metadata for an unconstrained request.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Return a copy with the time-window guard marked present.
    pub fn with_time_window(mut self) -> Self {
        self.has_time_window = true;
        self
    }

    /// Return a copy with an explicit caller result limit attached.
    pub fn with_explicit_limit(mut self, limit: u64) -> Self {
        self.explicit_limit = Some(limit);
        self
    }

    /// Return whether the caller supplied a time-window constraint.
    pub fn has_time_window(&self) -> bool {
        self.has_time_window
    }

    /// Return the explicit result limit supplied by the caller, when available.
    pub fn explicit_limit(&self) -> Option<u64> {
        self.explicit_limit
    }

    /// Return whether the caller supplied an explicit result limit.
    pub fn has_explicit_limit(&self) -> bool {
        self.explicit_limit.is_some()
    }
}

/// Request object for budgeted expansion from seed nodes into a working set.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExpansionRequest {
    pub(crate) working_set_id: WorkingSetId,
    pub(crate) seed_node_ids: Vec<NodeId>,
    pub(crate) direction: ExpansionDirection,
    pub(crate) filters: ExpansionFilters,
    #[serde(default)]
    pub(crate) guards: ExpansionGuards,
    pub(crate) hop_limit: u64,
    pub(crate) loading_profile: LoadingProfile,
    pub(crate) budget: ExpansionBudget,
    #[serde(default)]
    pub(crate) supernode_policy: Option<SupernodePolicy>,
}

impl ExpansionRequest {
    /// Build a deterministic expansion request.
    ///
    /// `filters` groups relationship-type and label filters so this public
    /// constructor stays within the repository's Clippy argument limit.
    pub fn new(
        working_set_id: WorkingSetId,
        seed_node_ids: Vec<NodeId>,
        direction: ExpansionDirection,
        filters: ExpansionFilters,
        hop_limit: u64,
        loading_profile: LoadingProfile,
        budget: ExpansionBudget,
    ) -> Self {
        Self {
            working_set_id,
            seed_node_ids,
            direction,
            filters,
            // Guards.
            guards: ExpansionGuards::empty(),
            hop_limit,
            loading_profile,
            budget,
            // Supernode policy.
            supernode_policy: None,
        }
    }

    /// Return a copy with non-filter supernode guards attached.
    pub fn with_guards(mut self, guards: ExpansionGuards) -> Self {
        self.guards = guards;
        self
    }

    /// Return a copy with an explicit supernode policy attached.
    ///
    ///
    /// let phase 2 tests and phase 3 implementation configure high-degree safety
    /// without changing the existing expansion constructor signature.
    pub fn with_supernode_policy(mut self, policy: SupernodePolicy) -> Self {
        self.supernode_policy = Some(policy);
        self
    }

    /// Return the target working-set ID.
    pub fn working_set_id(&self) -> &WorkingSetId {
        &self.working_set_id
    }

    /// Return the seed node IDs that anchor expansion.
    pub fn seed_node_ids(&self) -> &[NodeId] {
        &self.seed_node_ids
    }

    /// Return the requested adjacency direction policy.
    pub fn direction(&self) -> ExpansionDirection {
        self.direction
    }

    /// Return relationship type filters supplied by the caller.
    pub fn relationship_type_filters(&self) -> &[RelationshipType] {
        self.filters.relationship_type_filters()
    }

    /// Return label filters supplied by the caller.
    pub fn label_filters(&self) -> &LabelSet {
        self.filters.label_filters()
    }

    /// Return non-filter guards supplied by the caller.
    pub fn guards(&self) -> &ExpansionGuards {
        &self.guards
    }

    /// Return whether at least one relationship-type filter guard is present.
    pub fn has_relationship_filter(&self) -> bool {
        self.filters.has_relationship_type_filters()
    }

    /// Return whether at least one label filter guard is present.
    pub fn has_label_filter(&self) -> bool {
        self.filters.has_label_filters()
    }

    /// Return whether a time-window guard is present.
    pub fn has_time_window(&self) -> bool {
        self.guards.has_time_window()
    }

    /// Return whether an explicit result limit guard is present.
    pub fn has_explicit_limit(&self) -> bool {
        self.guards.has_explicit_limit()
    }

    /// Return the maximum traversal depth requested by the caller.
    pub fn hop_limit(&self) -> u64 {
        self.hop_limit
    }

    /// Return the loading profile selected for expansion.
    pub fn loading_profile(&self) -> &LoadingProfile {
        &self.loading_profile
    }

    /// Return the explicit expansion budget.
    pub fn budget(&self) -> &ExpansionBudget {
        &self.budget
    }

    /// Return the optional high-degree-node safety policy.
    pub fn supernode_policy(&self) -> Option<&SupernodePolicy> {
        self.supernode_policy.as_ref()
    }
}

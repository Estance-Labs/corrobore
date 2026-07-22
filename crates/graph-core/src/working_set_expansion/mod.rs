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
//! Budgeted working-set expansion over graph-core adjacency.
//!
//! This module owns request/result contracts and the first deterministic
//! one-hop expansion implementation over the `GraphPager` seam.
//!
//!
//!
//! - Declare the request/result seams needed to attach `SupernodePolicy` to
//!   runtime expansion.
//! - Keep the current traversal behavior unchanged.
//! - Do not detect high-degree nodes, enforce supernode guards, block traversal
//!   branches, or record skipped supernodes until the implementation phase.
//!
//!
//!
//! - Detect high-degree frontier nodes from lightweight adjacency counts.
//! - Enforce `SupernodePolicy` guard requirements before expanding adjacency.
//! - Return typed partial expansion results and explanation metadata for blocked
//!   supernodes.
//! - Keep acceptance/integration validation for phase 4.

pub(crate) use serde::{Deserialize, Serialize};

pub(crate) use crate::{
    GraphError, NodeId, RelationshipId,
    expansion_budget::{
        ExpansionBudget, ExpansionBudgetExceeded, ExpansionBudgetUsage, SupernodeExpansionBlocked,
        SupernodePolicy,
    },
    graph_pager::{
        AdjacencyDirection, GraphPager, GraphPagerError, GraphRecordRef, PagedAdjacency,
        PagedAdjacencyEntry,
    },
    loading_profile::LoadingProfile,
    properties::LabelSet,
    relationship::RelationshipType,
    working_set::{LoadingState, WarmAdjacencyEntry, WarmAdjacencyEntryInput, WorkingSetId},
    working_set_explanation::{
        BudgetCounterExplanation, ExpansionFixHint, FixHintScope, HotNodeExplanation,
        HotNodeLoadReason, HotRelationshipExplanation, HotRelationshipLoadReason,
        SeedNodeExplanation, SeedSourceKind, SeedSourceMetadata, SkippedExpansionExplanation,
        SkippedExpansionReason, SupernodeBlockExplanation, SupernodeBlockReason, SupernodeGuard,
        WarmAdjacencyExplanation, WarmAdjacencyReason, WorkingSetExplanation,
    },
    working_set_manager::GraphWorkingSetManager,
    working_set_telemetry::{TelemetryQueryDescriptor, WorkingSetDecisionEvent},
};

pub(crate) use crate::bandit_controller::{BanditContext, WorkingSetAction, WorkingSetController};

mod contracts;
mod engine;
mod result;
mod supernode;

pub use contracts::*;
pub use engine::*;
pub use result::*;
pub use supernode::*;

pub(crate) fn supernode_missing_guards(error: &SupernodeExpansionBlocked) -> Vec<SupernodeGuard> {
    let mut missing_guards = Vec::new();

    if error.relationship_filter_required {
        missing_guards.push(SupernodeGuard::RelationshipFilter);
    }
    if error.label_filter_required {
        missing_guards.push(SupernodeGuard::LabelFilter);
    }
    if error.time_window_required {
        missing_guards.push(SupernodeGuard::TimeWindow);
    }
    if error.limit_required {
        missing_guards.push(SupernodeGuard::Limit);
    }

    missing_guards
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ExpansionLimit, ExpansionSafetyErrorCode, GraphRecordMetadata,
        GraphWorkingSetCreateRequest, PagedNode, PagedRelationship, PropertyMap, StorageRef,
        default_fimi_investigation_profile, default_generic_loading_profile,
    };
    use std::collections::HashMap;

    fn node_id(value: &str) -> NodeId {
        NodeId::new(value).expect("test node ID should be valid")
    }

    fn relationship_id(value: &str) -> RelationshipId {
        RelationshipId::new(value).expect("test relationship ID should be valid")
    }

    fn relationship_type(value: &str) -> RelationshipType {
        RelationshipType::new(value).expect("test relationship type should be valid")
    }

    fn working_set_id(value: &str) -> WorkingSetId {
        WorkingSetId::new(value).expect("test working set ID should be valid")
    }

    fn generous_budget() -> ExpansionBudget {
        ExpansionBudget {
            // Max loaded node count.
            max_loaded_node_count: 100,
            // Max loaded relationship count.
            max_loaded_relationship_count: 100,
            // Max hot node count.
            max_hot_node_count: 100,
            // Max hot relationship count.
            max_hot_relationship_count: 100,
            // Max warm adjacency entry count.
            max_warm_adjacency_entry_count: 100,
            // Max hop count.
            max_hop_count: 5,
            // Max supernode expansion count.
            max_supernode_expansion_count: 5,
            // Max payload byte count.
            max_payload_byte_count: 1_048_576,
            // Max execution time ms.
            max_execution_time_ms: 1_000,
        }
    }

    fn expansion_request_for(working_set_id: WorkingSetId) -> ExpansionRequest {
        ExpansionRequest::new(
            working_set_id,
            vec![node_id("node--seed")],
            ExpansionDirection::Outgoing,
            ExpansionFilters::empty(),
            1,
            default_generic_loading_profile(),
            generous_budget(),
        )
    }

    #[test]
    fn filters_and_guards_expose_presence_flags_and_values() {
        let supports = relationship_type("SUPPORTS");
        let label_filters = vec!["Campaign".to_owned(), "Narrative".to_owned()];
        let filters = ExpansionFilters::new(vec![supports.clone()], label_filters.clone());

        assert!(filters.has_relationship_type_filters());
        assert!(filters.has_label_filters());
        assert_eq!(filters.relationship_type_filters(), &[supports]);
        assert_eq!(filters.label_filters(), &label_filters);

        let guards = ExpansionGuards::new(false, None)
            .with_time_window()
            .with_explicit_limit(25);

        assert!(guards.has_time_window());
        assert_eq!(guards.explicit_limit(), Some(25));
        assert!(guards.has_explicit_limit());
    }

    #[test]
    fn expansion_request_with_guards_and_policy_exposes_all_accessors() {
        let ws_id = working_set_id("working-set--request-accessors");
        let seed = node_id("node--seed-a");
        let relationship_filter = relationship_type("USES");
        let label_filters = vec!["Campaign".to_owned()];
        let filters =
            ExpansionFilters::new(vec![relationship_filter.clone()], label_filters.clone());
        let guards = ExpansionGuards::empty()
            .with_time_window()
            .with_explicit_limit(10);
        let policy = SupernodePolicy {
            degree_threshold: 5,
            require_relationship_filter: true,
            require_label_filter: true,
            require_time_window: true,
            require_limit: true,
        };
        let budget = generous_budget();
        let profile = default_generic_loading_profile();

        let request = ExpansionRequest::new(
            ws_id.clone(),
            vec![seed.clone()],
            ExpansionDirection::Incoming,
            filters,
            2,
            profile.clone(),
            budget.clone(),
        )
        .with_guards(guards.clone())
        .with_supernode_policy(policy.clone());

        assert_eq!(request.working_set_id(), &ws_id);
        assert_eq!(request.seed_node_ids(), &[seed]);
        assert_eq!(request.direction(), ExpansionDirection::Incoming);
        assert_eq!(request.relationship_type_filters(), &[relationship_filter]);
        assert_eq!(request.label_filters(), &label_filters);
        assert_eq!(request.guards(), &guards);
        assert!(request.has_relationship_filter());
        assert!(request.has_label_filter());
        assert!(request.has_time_window());
        assert!(request.has_explicit_limit());
        assert_eq!(request.hop_limit(), 2);
        assert_eq!(request.loading_profile(), &profile);
        assert_eq!(request.budget(), &budget);
        assert_eq!(request.supernode_policy(), Some(&policy));
    }

    #[test]
    fn expansion_result_with_supernode_error_marks_partial_and_preserves_usage() {
        let ws_id = working_set_id("working-set--supernode-result");
        let usage = ExpansionBudgetUsage {
            loaded_node_count: 1,
            loaded_relationship_count: 0,
            hot_node_count: 1,
            hot_relationship_count: 0,
            warm_adjacency_entry_count: 0,
            hop_count: 0,
            supernode_expansion_count: 0,
            payload_byte_count: 0,
            execution_time_ms: 0,
        };
        let explanation = WorkingSetExplanation::new();
        let supernode_error = SupernodeExpansionBlocked {
            code: ExpansionSafetyErrorCode::SupernodeExpansionBlocked,
            observed_degree: 11,
            degree_threshold: 5,
            relationship_filter_required: true,
            label_filter_required: false,
            time_window_required: true,
            limit_required: false,
            fix_hint: "Add relationship and time-window guards".to_owned(),
        };

        let result = ExpansionResult::new(
            ws_id.clone(),
            ExpansionResultStatus::Complete,
            usage.clone(),
            explanation,
            None,
        )
        .with_supernode_error(supernode_error.clone());

        assert_eq!(result.working_set_id(), &ws_id);
        assert_eq!(result.status(), ExpansionResultStatus::Partial);
        assert_eq!(result.usage(), &usage);
        assert!(result.budget_error().is_none());
        assert_eq!(result.supernode_error(), Some(&supernode_error));
    }

    #[derive(Clone)]
    struct MockPager {
        outgoing: HashMap<NodeId, PagedAdjacency>,
        incoming: HashMap<NodeId, PagedAdjacency>,
        metadata: HashMap<GraphRecordRef, GraphRecordMetadata>,
        nodes: HashMap<NodeId, PagedNode>,
        relationships: HashMap<RelationshipId, PagedRelationship>,
        adjacency_error: Option<GraphPagerError>,
    }

    impl MockPager {
        fn empty() -> Self {
            Self {
                // Outgoing.
                outgoing: HashMap::new(),
                // Incoming.
                incoming: HashMap::new(),
                // Metadata.
                metadata: HashMap::new(),
                // Nodes.
                nodes: HashMap::new(),
                // Relationships.
                relationships: HashMap::new(),
                // Adjacency error.
                adjacency_error: None,
            }
        }
    }

    impl GraphPager for MockPager {
        fn load_node_payload(&self, node_id: &NodeId) -> Result<PagedNode, GraphPagerError> {
            self.nodes
                .get(node_id)
                .cloned()
                .ok_or_else(|| GraphPagerError::UnavailableRecord {
                    record_ref: GraphRecordRef::Node(node_id.clone()),
                })
        }

        fn load_relationship_payload(
            &self,
            relationship_id: &RelationshipId,
        ) -> Result<PagedRelationship, GraphPagerError> {
            self.relationships
                .get(relationship_id)
                .cloned()
                .ok_or_else(|| GraphPagerError::UnavailableRecord {
                    record_ref: GraphRecordRef::Relationship(relationship_id.clone()),
                })
        }

        fn load_outgoing_adjacency(
            &self,
            node_id: &NodeId,
        ) -> Result<PagedAdjacency, GraphPagerError> {
            if let Some(error) = &self.adjacency_error {
                return Err(error.clone());
            }
            self.outgoing
                .get(node_id)
                .cloned()
                .ok_or_else(|| GraphPagerError::UnavailableRecord {
                    record_ref: GraphRecordRef::Node(node_id.clone()),
                })
        }

        fn load_incoming_adjacency(
            &self,
            node_id: &NodeId,
        ) -> Result<PagedAdjacency, GraphPagerError> {
            if let Some(error) = &self.adjacency_error {
                return Err(error.clone());
            }
            self.incoming
                .get(node_id)
                .cloned()
                .ok_or_else(|| GraphPagerError::UnavailableRecord {
                    record_ref: GraphRecordRef::Node(node_id.clone()),
                })
        }

        fn load_indexed_metadata(
            &self,
            record_ref: &GraphRecordRef,
        ) -> Result<GraphRecordMetadata, GraphPagerError> {
            self.metadata.get(record_ref).cloned().ok_or_else(|| {
                GraphPagerError::UnavailableRecord {
                    record_ref: record_ref.clone(),
                }
            })
        }
    }

    #[test]
    fn adjacency_directions_respect_requested_policy() {
        assert_eq!(
            adjacency_directions(ExpansionDirection::Outgoing),
            vec![AdjacencyDirection::Outgoing]
        );
        assert_eq!(
            adjacency_directions(ExpansionDirection::Incoming),
            vec![AdjacencyDirection::Incoming]
        );
        assert_eq!(
            adjacency_directions(ExpansionDirection::Both),
            vec![AdjacencyDirection::Outgoing, AdjacencyDirection::Incoming]
        );
    }

    #[test]
    fn relationship_and_label_filters_accept_expected_values() {
        let supports = relationship_type("SUPPORTS");
        let promotes = relationship_type("PROMOTES");

        assert!(relationship_type_allowed(&[], &supports));
        assert!(relationship_type_allowed(
            std::slice::from_ref(&supports),
            &supports
        ));
        assert!(!relationship_type_allowed(
            std::slice::from_ref(&supports),
            &promotes
        ));

        let labels = vec!["Campaign".to_owned(), "Narrative".to_owned()];
        assert!(labels_allowed(&Vec::new(), &labels));
        assert!(labels_allowed(&vec!["Campaign".to_owned()], &labels));
        assert!(!labels_allowed(&vec!["Claim".to_owned()], &labels));
    }

    #[test]
    fn hot_load_reasons_follow_profile_configuration() {
        let fimi = default_fimi_investigation_profile();
        let generic = default_generic_loading_profile();

        assert_eq!(
            hot_node_reason(&fimi, &vec!["Campaign".to_owned()]),
            HotNodeLoadReason::ProfileHotLabel
        );
        assert_eq!(
            hot_node_reason(&generic, &vec!["Campaign".to_owned()]),
            HotNodeLoadReason::TraversalExpansion
        );

        assert_eq!(
            hot_relationship_reason(&generic, &relationship_type("SUPPORTS")),
            HotRelationshipLoadReason::PrioritizedRelationshipType
        );
        assert_eq!(
            hot_relationship_reason(&generic, &relationship_type("MENTIONS")),
            HotRelationshipLoadReason::TraversalExpansion
        );
    }

    #[test]
    fn relationship_endpoints_follow_adjacency_direction() {
        let frontier = node_id("node--frontier");
        let neighbor = node_id("node--neighbor");

        let outgoing = relationship_endpoints(&frontier, &neighbor, AdjacencyDirection::Outgoing);
        assert_eq!(outgoing.0, frontier);
        assert_eq!(outgoing.1, neighbor);

        let incoming = relationship_endpoints(
            &node_id("node--frontier"),
            &node_id("node--neighbor"),
            AdjacencyDirection::Incoming,
        );
        assert_eq!(incoming.0, node_id("node--neighbor"));
        assert_eq!(incoming.1, node_id("node--frontier"));
    }

    #[test]
    fn relationship_type_for_entry_uses_hint_when_present() {
        let entry = PagedAdjacencyEntry {
            relationship_id: relationship_id("relationship--hint"),
            neighbor_node_id: node_id("node--neighbor"),
            relationship_type: Some(relationship_type("MENTIONS")),
            relationship_storage_ref: None,
            neighbor_storage_ref: None,
        };

        let rel_type = relationship_type_for_entry(&MockPager::empty(), &entry)
            .expect("relationship type hint should be returned without metadata lookup");

        assert_eq!(rel_type.as_str(), "MENTIONS");
    }

    #[test]
    fn relationship_type_for_entry_falls_back_to_indexed_metadata() {
        let rel_id = relationship_id("relationship--metadata");
        let mut pager = MockPager::empty();
        pager.metadata.insert(
            GraphRecordRef::Relationship(rel_id.clone()),
            GraphRecordMetadata {
                // Record ref.
                record_ref: GraphRecordRef::Relationship(rel_id.clone()),
                // Storage ref.
                storage_ref: Some(StorageRef::Record {
                    // Collection.
                    collection: "relationships".to_owned(),
                    // Key.
                    key: rel_id.as_str().to_owned(),
                }),
                // Loading state.
                loading_state: LoadingState::Indexed,
                // Labels.
                labels: Vec::new(),
                // Relationship type.
                relationship_type: Some(relationship_type("SUPPORTS")),
                // Indexed properties.
                indexed_properties: PropertyMap::new(),
            },
        );

        let entry = PagedAdjacencyEntry {
            // Relationship id.
            relationship_id: rel_id,
            // Neighbor node id.
            neighbor_node_id: node_id("node--neighbor"),
            // Relationship type.
            relationship_type: None,
            // Relationship storage ref.
            relationship_storage_ref: None,
            // Neighbor storage ref.
            neighbor_storage_ref: None,
        };

        let rel_type = relationship_type_for_entry(&pager, &entry)
            .expect("indexed metadata should provide relationship type");
        assert_eq!(rel_type.as_str(), "SUPPORTS");
    }

    #[test]
    fn relationship_type_for_entry_reports_missing_metadata_type() {
        let rel_id = relationship_id("relationship--missing-type");
        let mut pager = MockPager::empty();
        pager.metadata.insert(
            GraphRecordRef::Relationship(rel_id.clone()),
            GraphRecordMetadata {
                // Record ref.
                record_ref: GraphRecordRef::Relationship(rel_id.clone()),
                // Storage ref.
                storage_ref: None,
                // Loading state.
                loading_state: LoadingState::Indexed,
                // Labels.
                labels: Vec::new(),
                // Relationship type.
                relationship_type: None,
                // Indexed properties.
                indexed_properties: PropertyMap::new(),
            },
        );

        let entry = PagedAdjacencyEntry {
            // Relationship id.
            relationship_id: rel_id.clone(),
            // Neighbor node id.
            neighbor_node_id: node_id("node--neighbor"),
            // Relationship type.
            relationship_type: None,
            // Relationship storage ref.
            relationship_storage_ref: None,
            // Neighbor storage ref.
            neighbor_storage_ref: None,
        };

        let error = relationship_type_for_entry(&pager, &entry)
            .expect_err("missing relationship type metadata should return typed invariant error");
        assert!(matches!(
        error,
        GraphError::InternalInvariantViolation(message)
        if message.contains("missing relationship type metadata")
        && message.contains(rel_id.as_str())
        ));
    }

    #[test]
    fn load_adjacency_maps_pager_errors_to_internal_invariant_error() {
        let mut pager = MockPager::empty();
        pager.adjacency_error = Some(GraphPagerError::MissingPage {
            storage_ref: StorageRef::Page {
                segment: "adjacency/outgoing".to_owned(),
                page_id: 9,
            },
        });

        let error = load_adjacency(&pager, &node_id("node--seed"), AdjacencyDirection::Outgoing)
            .expect_err("pager failures should map into typed graph errors");

        assert!(matches!(
        error,
        GraphError::InternalInvariantViolation(message)
        if message.contains("graph pager error") && message.contains("missing storage page")
        ));
    }

    #[test]
    fn attach_warm_frontier_records_warm_entries_and_usage() {
        let source = node_id("node--source");
        let target = node_id("node--target");
        let rel_id = relationship_id("relationship--warm");
        let rel_type = relationship_type("SUPPORTS");

        let adjacency = PagedAdjacency {
            owner_node_id: source.clone(),
            direction: AdjacencyDirection::Outgoing,
            entries: vec![PagedAdjacencyEntry {
                relationship_id: rel_id.clone(),
                neighbor_node_id: target.clone(),
                relationship_type: Some(rel_type.clone()),
                relationship_storage_ref: Some(StorageRef::Offset {
                    segment: "relationships".to_owned(),
                    byte_offset: 10,
                }),
                neighbor_storage_ref: Some(StorageRef::Offset {
                    segment: "nodes".to_owned(),
                    byte_offset: 20,
                }),
            }],
            storage_ref: None,
        };

        let mut pager = MockPager::empty();
        pager.outgoing.insert(source.clone(), adjacency);
        pager.metadata.insert(
            GraphRecordRef::Node(target.clone()),
            GraphRecordMetadata {
                // Record ref.
                record_ref: GraphRecordRef::Node(target.clone()),
                // Storage ref.
                storage_ref: None,
                // Loading state.
                loading_state: LoadingState::Indexed,
                // Labels.
                labels: vec!["Campaign".to_owned()],
                // Relationship type.
                relationship_type: None,
                // Indexed properties.
                indexed_properties: PropertyMap::new(),
            },
        );

        let ws_id = working_set_id("working-set--warm-frontier");
        let mut manager = GraphWorkingSetManager::new();
        manager
            .create_working_set(GraphWorkingSetCreateRequest::new(ws_id.clone()))
            .expect("working set should be created");
        let request = expansion_request_for(ws_id.clone());
        let mut usage = empty_usage();
        let mut explanation = WorkingSetExplanation::new();

        let budget_error = attach_warm_frontier(
            &mut manager,
            &pager,
            &request,
            &mut usage,
            &mut explanation,
            &source,
        )
        .expect("warm frontier should attach successfully");

        assert!(budget_error.is_none());
        assert_eq!(usage.warm_adjacency_entry_count, 1);
        assert_eq!(usage.loaded_relationship_count, 1);
        assert_eq!(explanation.warm_adjacency_entries().len(), 1);
        assert_eq!(
            explanation.warm_adjacency_entries()[0].relationship_id,
            rel_id
        );

        let working_set = manager
            .get_working_set(&ws_id)
            .expect("working set should remain readable");
        let warm_entries = working_set
            .warm_adjacency_for_source(&source)
            .expect("warm adjacency should be stored on the source node");
        assert_eq!(warm_entries.len(), 1);
        assert_eq!(warm_entries[0].target_node_id(), &target);
    }

    #[test]
    fn attach_warm_frontier_returns_budget_error_when_warm_limit_exceeded() {
        let source = node_id("node--source-budget");
        let target = node_id("node--target-budget");
        let rel_id = relationship_id("relationship--warm-budget");

        let adjacency = PagedAdjacency {
            owner_node_id: source.clone(),
            direction: AdjacencyDirection::Outgoing,
            entries: vec![PagedAdjacencyEntry {
                relationship_id: rel_id.clone(),
                neighbor_node_id: target.clone(),
                relationship_type: Some(relationship_type("SUPPORTS")),
                relationship_storage_ref: None,
                neighbor_storage_ref: None,
            }],
            storage_ref: None,
        };

        let mut pager = MockPager::empty();
        pager.outgoing.insert(source.clone(), adjacency);
        pager.metadata.insert(
            GraphRecordRef::Node(target),
            GraphRecordMetadata {
                // Record ref.
                record_ref: GraphRecordRef::Node(node_id("node--target-budget")),
                // Storage ref.
                storage_ref: None,
                // Loading state.
                loading_state: LoadingState::Indexed,
                // Labels.
                labels: vec!["Campaign".to_owned()],
                // Relationship type.
                relationship_type: None,
                // Indexed properties.
                indexed_properties: PropertyMap::new(),
            },
        );

        let ws_id = working_set_id("working-set--warm-budget");
        let mut manager = GraphWorkingSetManager::new();
        manager
            .create_working_set(GraphWorkingSetCreateRequest::new(ws_id.clone()))
            .expect("working set should be created");
        let request = ExpansionRequest::new(
            ws_id,
            vec![node_id("node--seed")],
            ExpansionDirection::Outgoing,
            ExpansionFilters::empty(),
            1,
            default_generic_loading_profile(),
            ExpansionBudget {
                // Max warm adjacency entry count.
                max_warm_adjacency_entry_count: 0,
                ..generous_budget()
            },
        );
        let mut usage = empty_usage();
        let mut explanation = WorkingSetExplanation::new();

        let budget_error = attach_warm_frontier(
            &mut manager,
            &pager,
            &request,
            &mut usage,
            &mut explanation,
            &source,
        )
        .expect("budget overflow on warm frontier should return a typed partial error");

        let error = budget_error.expect("warm frontier should stop on budget limit");
        assert_eq!(error.limit, ExpansionLimit::WarmAdjacencyEntryCount);
        assert_eq!(
            error.code,
            ExpansionSafetyErrorCode::ExpansionBudgetExceeded
        );
        assert_eq!(usage.warm_adjacency_entry_count, 0);
        assert_eq!(explanation.warm_adjacency_entries().len(), 0);
        assert_eq!(explanation.skipped_expansions().len(), 1);
    }

    #[test]
    fn supernode_block_explanation_carries_missing_guard_list() {
        let error = SupernodeExpansionBlocked {
            code: ExpansionSafetyErrorCode::SupernodeExpansionBlocked,
            observed_degree: 8,
            degree_threshold: 3,
            relationship_filter_required: true,
            label_filter_required: false,
            time_window_required: true,
            limit_required: true,
            fix_hint: "Add relationship, time-window, and limit guards".to_owned(),
        };

        let explanation = build_supernode_block_explanation(node_id("node--supernode"), &error)
            .expect("supernode block explanation should be built");

        assert_eq!(explanation.node_id, node_id("node--supernode"));
        assert_eq!(explanation.observed_degree, 8);
        assert_eq!(explanation.degree_threshold, 3);
        assert!(
            explanation
                .missing_guards
                .contains(&SupernodeGuard::RelationshipFilter)
        );
        assert!(
            explanation
                .missing_guards
                .contains(&SupernodeGuard::TimeWindow)
        );
        assert!(explanation.missing_guards.contains(&SupernodeGuard::Limit));
        assert!(
            !explanation
                .missing_guards
                .contains(&SupernodeGuard::LabelFilter)
        );
    }

    #[test]
    fn seed_explanation_uses_explicit_node_source_metadata() {
        let seed = node_id("node--seed-explicit");

        let explanation = seed_explanation(&seed);

        assert_eq!(explanation.node_id, seed);
        assert_eq!(explanation.source.kind, SeedSourceKind::ExplicitNodeId);
        assert_eq!(
            explanation.source.source_id,
            Some("node--seed-explicit".to_owned())
        );
        assert_eq!(explanation.source.source_label, None);
        assert_eq!(explanation.source.score, None);
    }

    #[test]
    fn record_profile_skip_adds_blocked_by_profile_with_fix_hint() {
        let mut explanation = WorkingSetExplanation::new();
        let ws_id = working_set_id("working-set--skip-profile");
        let mut manager = GraphWorkingSetManager::new();
        manager
            .create_working_set(GraphWorkingSetCreateRequest::new(ws_id.clone()))
            .expect("working set should be created");

        record_profile_skip(
            &mut manager,
            &mut explanation,
            &ws_id,
            node_id("node--source"),
            Some(node_id("node--candidate")),
            Some(relationship_id("relationship--candidate")),
            Some(relationship_type("SUPPORTS")),
        )
        .expect("profile skip should be recorded");

        let telemetry = manager
            .telemetry(&ws_id)
            .expect("telemetry recorder should be available");
        assert_eq!(telemetry.events().len(), 1);
        assert_eq!(explanation.skipped_expansions().len(), 1);
        let skipped = &explanation.skipped_expansions()[0];
        assert_eq!(skipped.reason, SkippedExpansionReason::BlockedByProfile);
        assert!(skipped.budget_counter.is_none());
        assert!(matches!(
            skipped.fix_hint,
            Some(ExpansionFixHint {
                scope: FixHintScope::LoadingProfile,
                ..
            })
        ));
    }

    #[test]
    fn record_budget_skip_adds_budget_counter_details() {
        let mut explanation = WorkingSetExplanation::new();
        let error = crate::ExpansionBudgetExceeded {
            code: ExpansionSafetyErrorCode::ExpansionBudgetExceeded,
            limit: ExpansionLimit::HotNodeCount,
            allowed: 2,
            consumed: 3,
            fix_hint: "reduce hot-set breadth".to_owned(),
        };

        let ws_id = working_set_id("working-set--skip-budget");
        let mut manager = GraphWorkingSetManager::new();
        manager
            .create_working_set(GraphWorkingSetCreateRequest::new(ws_id.clone()))
            .expect("working set should be created");

        record_budget_skip(
            &mut manager,
            &mut explanation,
            &ws_id,
            node_id("node--source"),
            Some(node_id("node--candidate")),
            Some(relationship_id("relationship--candidate")),
            Some(relationship_type("USES")),
            &error,
        )
        .expect("budget skip should be recorded");

        let telemetry = manager
            .telemetry(&ws_id)
            .expect("telemetry recorder should be available");
        assert_eq!(telemetry.events().len(), 1);
        assert_eq!(explanation.skipped_expansions().len(), 1);
        let skipped = &explanation.skipped_expansions()[0];
        assert_eq!(skipped.reason, SkippedExpansionReason::BudgetLimit);
        assert!(matches!(
            skipped.budget_counter,
            Some(BudgetCounterExplanation {
                limit: ExpansionLimit::HotNodeCount,
                allowed: 2,
                consumed: 3,
                remaining: None,
            })
        ));
        assert!(matches!(
            skipped.fix_hint,
            Some(ExpansionFixHint {
                scope: FixHintScope::Budget,
                ..
            })
        ));
    }

    #[test]
    fn supernode_missing_guards_is_empty_when_policy_requires_none() {
        let error = SupernodeExpansionBlocked {
            code: ExpansionSafetyErrorCode::SupernodeExpansionBlocked,
            observed_degree: 9,
            degree_threshold: 4,
            relationship_filter_required: false,
            label_filter_required: false,
            time_window_required: false,
            limit_required: false,
            fix_hint: "no additional guard required".to_owned(),
        };

        let missing = supernode_missing_guards(&error);

        assert!(missing.is_empty());
    }

    #[test]
    fn expand_working_set_loads_seed_only_when_hop_limit_is_zero() {
        let ws_id = working_set_id("working-set--seed-only");
        let mut manager = GraphWorkingSetManager::new();
        manager
            .create_working_set(GraphWorkingSetCreateRequest::new(ws_id.clone()))
            .expect("working set should be created");

        let seed = node_id("node--seed-only");
        let mut pager = MockPager::empty();
        pager.nodes.insert(
            seed.clone(),
            PagedNode {
                // Node.
                node: crate::Node {
                    // Id.
                    id: seed.clone(),
                    // Version id.
                    version_id: crate::NodeVersionId::new("node-version--seed-only")
                        .expect("node version ID should be valid"),
                    // Version.
                    version: 1,
                    // Current.
                    current: true,
                    // Previous version id.
                    previous_version_id: None,
                    // Labels.
                    labels: vec!["Campaign".to_owned()],
                    // Properties.
                    properties: PropertyMap::new(),
                    // Status.
                    status: crate::RecordStatus::Candidate,
                    // Confidence.
                    confidence: None,
                    // Source reliability.
                    source_reliability: None,
                    // Information credibility.
                    information_credibility: None,
                    // Extraction run id.
                    extraction_run_id: None,
                    // Evidence refs.
                    evidence_refs: Vec::new(),
                    // Temporal.
                    temporal: crate::TemporalMetadata::default(),
                    // Transaction.
                    transaction: crate::TransactionMetadata::default(),
                },
                storage_ref: None,
            },
        );

        let request = ExpansionRequest::new(
            ws_id.clone(),
            vec![seed.clone()],
            ExpansionDirection::Outgoing,
            ExpansionFilters::empty(),
            0,
            default_generic_loading_profile(),
            generous_budget(),
        );

        let result = expand_working_set_from_graph_adjacency(&mut manager, &pager, request)
            .expect("seed-only expansion should succeed");

        assert_eq!(result.status(), ExpansionResultStatus::Complete);
        assert_eq!(result.usage().loaded_node_count, 1);
        assert_eq!(result.usage().hot_node_count, 1);
        assert_eq!(result.explanation().seed_nodes().len(), 1);
        assert_eq!(result.explanation().hot_nodes().len(), 1);
        assert!(result.explanation().skipped_expansions().is_empty());
    }

    #[test]
    fn expand_working_set_records_profile_skip_when_relationship_filter_blocks_candidate() {
        let ws_id = working_set_id("working-set--profile-skip");
        let mut manager = GraphWorkingSetManager::new();
        manager
            .create_working_set(GraphWorkingSetCreateRequest::new(ws_id.clone()))
            .expect("working set should be created");

        let seed = node_id("node--seed-profile");
        let target = node_id("node--target-profile");
        let rel_id = relationship_id("relationship--profile-skip");

        let mut pager = MockPager::empty();
        pager.nodes.insert(
            seed.clone(),
            PagedNode {
                // Node.
                node: crate::Node {
                    // Id.
                    id: seed.clone(),
                    // Version id.
                    version_id: crate::NodeVersionId::new("node-version--seed-profile")
                        .expect("node version ID should be valid"),
                    // Version.
                    version: 1,
                    // Current.
                    current: true,
                    // Previous version id.
                    previous_version_id: None,
                    // Labels.
                    labels: vec!["Campaign".to_owned()],
                    // Properties.
                    properties: PropertyMap::new(),
                    // Status.
                    status: crate::RecordStatus::Candidate,
                    // Confidence.
                    confidence: None,
                    // Source reliability.
                    source_reliability: None,
                    // Information credibility.
                    information_credibility: None,
                    // Extraction run id.
                    extraction_run_id: None,
                    // Evidence refs.
                    evidence_refs: Vec::new(),
                    // Temporal.
                    temporal: crate::TemporalMetadata::default(),
                    // Transaction.
                    transaction: crate::TransactionMetadata::default(),
                },
                storage_ref: None,
            },
        );
        pager.outgoing.insert(
            seed.clone(),
            PagedAdjacency {
                // Owner node id.
                owner_node_id: seed.clone(),
                // Direction.
                direction: AdjacencyDirection::Outgoing,
                // Entries.
                entries: vec![PagedAdjacencyEntry {
                    // Relationship id.
                    relationship_id: rel_id,
                    // Neighbor node id.
                    neighbor_node_id: target,
                    // Relationship type.
                    relationship_type: Some(relationship_type("MENTIONS")),
                    // Relationship storage ref.
                    relationship_storage_ref: None,
                    // Neighbor storage ref.
                    neighbor_storage_ref: None,
                }],
                // Storage ref.
                storage_ref: None,
            },
        );

        let request = ExpansionRequest::new(
            ws_id,
            vec![seed.clone()],
            ExpansionDirection::Outgoing,
            ExpansionFilters::new(vec![relationship_type("SUPPORTS")], Vec::new()),
            1,
            default_generic_loading_profile(),
            generous_budget(),
        );

        let result = expand_working_set_from_graph_adjacency(&mut manager, &pager, request)
            .expect("expansion with blocked relationship type should succeed");

        assert_eq!(result.status(), ExpansionResultStatus::Complete);
        assert_eq!(result.explanation().skipped_expansions().len(), 1);
        assert_eq!(
            result.explanation().skipped_expansions()[0].reason,
            SkippedExpansionReason::BlockedByProfile
        );
    }

    #[test]
    fn expand_working_set_records_profile_skip_when_label_filter_blocks_candidate() {
        let ws_id = working_set_id("working-set--label-skip");
        let mut manager = GraphWorkingSetManager::new();
        manager
            .create_working_set(GraphWorkingSetCreateRequest::new(ws_id.clone()))
            .expect("working set should be created");

        let seed = node_id("node--seed-label");
        let target = node_id("node--target-label");
        let rel_id = relationship_id("relationship--label-skip");

        let mut pager = MockPager::empty();
        pager.nodes.insert(
            seed.clone(),
            PagedNode {
                // Node.
                node: crate::Node {
                    // Id.
                    id: seed.clone(),
                    // Version id.
                    version_id: crate::NodeVersionId::new("node-version--seed-label")
                        .expect("node version ID should be valid"),
                    // Version.
                    version: 1,
                    // Current.
                    current: true,
                    // Previous version id.
                    previous_version_id: None,
                    // Labels.
                    labels: vec!["Campaign".to_owned()],
                    // Properties.
                    properties: PropertyMap::new(),
                    // Status.
                    status: crate::RecordStatus::Candidate,
                    // Confidence.
                    confidence: None,
                    // Source reliability.
                    source_reliability: None,
                    // Information credibility.
                    information_credibility: None,
                    // Extraction run id.
                    extraction_run_id: None,
                    // Evidence refs.
                    evidence_refs: Vec::new(),
                    // Temporal.
                    temporal: crate::TemporalMetadata::default(),
                    // Transaction.
                    transaction: crate::TransactionMetadata::default(),
                },
                storage_ref: None,
            },
        );
        pager.outgoing.insert(
            seed.clone(),
            PagedAdjacency {
                // Owner node id.
                owner_node_id: seed.clone(),
                // Direction.
                direction: AdjacencyDirection::Outgoing,
                // Entries.
                entries: vec![PagedAdjacencyEntry {
                    // Relationship id.
                    relationship_id: rel_id,
                    // Neighbor node id.
                    neighbor_node_id: target.clone(),
                    // Relationship type.
                    relationship_type: Some(relationship_type("SUPPORTS")),
                    // Relationship storage ref.
                    relationship_storage_ref: None,
                    // Neighbor storage ref.
                    neighbor_storage_ref: None,
                }],
                // Storage ref.
                storage_ref: None,
            },
        );
        pager.metadata.insert(
            GraphRecordRef::Node(target.clone()),
            GraphRecordMetadata {
                // Record ref.
                record_ref: GraphRecordRef::Node(target),
                // Storage ref.
                storage_ref: None,
                // Loading state.
                loading_state: LoadingState::Indexed,
                // Labels.
                labels: vec!["Campaign".to_owned()],
                // Relationship type.
                relationship_type: None,
                // Indexed properties.
                indexed_properties: PropertyMap::new(),
            },
        );

        let request = ExpansionRequest::new(
            ws_id,
            vec![seed],
            ExpansionDirection::Outgoing,
            ExpansionFilters::new(Vec::new(), vec!["ThreatActor".to_owned()]),
            1,
            default_generic_loading_profile(),
            generous_budget(),
        );

        let result = expand_working_set_from_graph_adjacency(&mut manager, &pager, request).expect(
            "label-filter blocked candidate should return complete result with skip explanation",
        );

        assert_eq!(result.status(), ExpansionResultStatus::Complete);
        assert_eq!(result.explanation().skipped_expansions().len(), 1);
        assert_eq!(
            result.explanation().skipped_expansions()[0].reason,
            SkippedExpansionReason::BlockedByProfile
        );
    }

    #[test]
    fn expand_working_set_returns_partial_when_seed_budget_is_exceeded() {
        let ws_id = working_set_id("working-set--seed-budget");
        let mut manager = GraphWorkingSetManager::new();
        manager
            .create_working_set(GraphWorkingSetCreateRequest::new(ws_id.clone()))
            .expect("working set should be created");

        let request = ExpansionRequest::new(
            ws_id,
            vec![node_id("node--seed-budget")],
            ExpansionDirection::Outgoing,
            ExpansionFilters::empty(),
            1,
            default_generic_loading_profile(),
            ExpansionBudget {
                // Max loaded node count.
                max_loaded_node_count: 0,
                ..generous_budget()
            },
        );

        let result =
            expand_working_set_from_graph_adjacency(&mut manager, &MockPager::empty(), request)
                .expect("budget-limited expansion should return partial result");

        assert_eq!(result.status(), ExpansionResultStatus::Partial);
        assert!(result.budget_error().is_some());
        assert_eq!(result.explanation().skipped_expansions().len(), 1);
        assert_eq!(
            result.explanation().skipped_expansions()[0].reason,
            SkippedExpansionReason::BudgetLimit
        );
    }

    #[test]
    fn check_supernode_expansion_guards_returns_typed_block_error() {
        let policy = SupernodePolicy {
            degree_threshold: 3,
            require_relationship_filter: true,
            require_label_filter: true,
            require_time_window: true,
            require_limit: true,
        };
        let request = ExpansionRequest::new(
            working_set_id("working-set--supernode-guards"),
            vec![node_id("node--seed-supernode-guards")],
            ExpansionDirection::Outgoing,
            ExpansionFilters::empty(),
            1,
            default_generic_loading_profile(),
            generous_budget(),
        );

        let error = check_supernode_expansion_guards(
            &policy,
            &node_id("node--source-supernode-guards"),
            9,
            &request,
        )
        .expect_err("missing required supernode guards should return a typed block error");

        assert!(matches!(
            error,
            GraphError::SupernodeExpansionBlocked(SupernodeExpansionBlocked {
                observed_degree: 9,
                degree_threshold: 3,
                relationship_filter_required: true,
                label_filter_required: true,
                time_window_required: true,
                limit_required: true,
                ..
            })
        ));
    }

    #[test]
    fn record_supernode_blocked_expansion_records_skip_and_block_metadata() {
        let source = node_id("node--source-supernode-record");
        let error = SupernodeExpansionBlocked {
            code: ExpansionSafetyErrorCode::SupernodeExpansionBlocked,
            observed_degree: 12,
            degree_threshold: 5,
            relationship_filter_required: true,
            label_filter_required: false,
            time_window_required: true,
            limit_required: false,
            fix_hint: "Add relationship filter and time window".to_owned(),
        };
        let mut explanation = WorkingSetExplanation::new();

        record_supernode_blocked_expansion(&mut explanation, source.clone(), &error)
            .expect("supernode blocked expansion should record explanation metadata");

        assert_eq!(explanation.skipped_expansions().len(), 1);
        assert_eq!(explanation.supernode_blocks().len(), 1);
        assert_eq!(
            explanation.skipped_expansions()[0].reason,
            SkippedExpansionReason::SupernodePolicy
        );
        assert_eq!(explanation.supernode_blocks()[0].node_id, source);
        assert!(
            explanation.supernode_blocks()[0]
                .missing_guards
                .contains(&SupernodeGuard::RelationshipFilter)
        );
        assert!(
            explanation.supernode_blocks()[0]
                .missing_guards
                .contains(&SupernodeGuard::TimeWindow)
        );
    }

    #[test]
    fn expand_working_set_returns_partial_when_supernode_policy_blocks_frontier() {
        let ws_id = working_set_id("working-set--supernode-block");
        let mut manager = GraphWorkingSetManager::new();
        manager
            .create_working_set(GraphWorkingSetCreateRequest::new(ws_id.clone()))
            .expect("working set should be created");

        let seed = node_id("node--seed-supernode-block");
        let mut pager = MockPager::empty();
        pager.nodes.insert(
            seed.clone(),
            PagedNode {
                // Node.
                node: crate::Node {
                    // Id.
                    id: seed.clone(),
                    // Version id.
                    version_id: crate::NodeVersionId::new("node-version--supernode-block")
                        .expect("node version ID should be valid"),
                    // Version.
                    version: 1,
                    // Current.
                    current: true,
                    // Previous version id.
                    previous_version_id: None,
                    // Labels.
                    labels: vec!["Campaign".to_owned()],
                    // Properties.
                    properties: PropertyMap::new(),
                    // Status.
                    status: crate::RecordStatus::Candidate,
                    // Confidence.
                    confidence: None,
                    // Source reliability.
                    source_reliability: None,
                    // Information credibility.
                    information_credibility: None,
                    // Extraction run id.
                    extraction_run_id: None,
                    // Evidence refs.
                    evidence_refs: Vec::new(),
                    // Temporal.
                    temporal: crate::TemporalMetadata::default(),
                    // Transaction.
                    transaction: crate::TransactionMetadata::default(),
                },
                storage_ref: None,
            },
        );
        pager.outgoing.insert(
            seed.clone(),
            PagedAdjacency {
                // Owner node id.
                owner_node_id: seed.clone(),
                // Direction.
                direction: AdjacencyDirection::Outgoing,
                // Entries.
                entries: vec![PagedAdjacencyEntry {
                    // Relationship id.
                    relationship_id: relationship_id("relationship--supernode-block"),
                    // Neighbor node id.
                    neighbor_node_id: node_id("node--neighbor-supernode-block"),
                    // Relationship type.
                    relationship_type: Some(relationship_type("SUPPORTS")),
                    // Relationship storage ref.
                    relationship_storage_ref: None,
                    // Neighbor storage ref.
                    neighbor_storage_ref: None,
                }],
                // Storage ref.
                storage_ref: None,
            },
        );

        let policy = SupernodePolicy {
            // Degree threshold.
            degree_threshold: 0,
            // Require relationship filter.
            require_relationship_filter: true,
            // Require label filter.
            require_label_filter: false,
            // Require time window.
            require_time_window: false,
            // Require limit.
            require_limit: false,
        };
        let request = ExpansionRequest::new(
            ws_id,
            vec![seed.clone()],
            ExpansionDirection::Outgoing,
            ExpansionFilters::empty(),
            1,
            default_generic_loading_profile(),
            generous_budget(),
        )
        .with_supernode_policy(policy);

        let result = expand_working_set_from_graph_adjacency(&mut manager, &pager, request)
            .expect("supernode guard block should return a typed partial result");

        assert_eq!(result.status(), ExpansionResultStatus::Partial);
        assert!(result.budget_error().is_none());
        assert!(result.supernode_error().is_some());
        assert_eq!(result.explanation().skipped_expansions().len(), 1);
        assert_eq!(result.explanation().supernode_blocks().len(), 1);
        assert_eq!(result.usage().loaded_node_count, 1);
        assert_eq!(result.usage().loaded_relationship_count, 0);
    }

    #[test]
    fn expand_working_set_returns_partial_when_supernode_budget_counter_is_exceeded() {
        let ws_id = working_set_id("working-set--supernode-budget");
        let mut manager = GraphWorkingSetManager::new();
        manager
            .create_working_set(GraphWorkingSetCreateRequest::new(ws_id.clone()))
            .expect("working set should be created");

        let seed = node_id("node--seed-supernode-budget");
        let target = node_id("node--target-supernode-budget");
        let rel_id = relationship_id("relationship--supernode-budget");
        let rel_type = relationship_type("SUPPORTS");

        let mut pager = MockPager::empty();
        pager.nodes.insert(
            seed.clone(),
            PagedNode {
                // Node.
                node: crate::Node {
                    // Id.
                    id: seed.clone(),
                    // Version id.
                    version_id: crate::NodeVersionId::new("node-version--supernode-budget")
                        .expect("node version ID should be valid"),
                    // Version.
                    version: 1,
                    // Current.
                    current: true,
                    // Previous version id.
                    previous_version_id: None,
                    // Labels.
                    labels: vec!["Campaign".to_owned()],
                    // Properties.
                    properties: PropertyMap::new(),
                    // Status.
                    status: crate::RecordStatus::Candidate,
                    // Confidence.
                    confidence: None,
                    // Source reliability.
                    source_reliability: None,
                    // Information credibility.
                    information_credibility: None,
                    // Extraction run id.
                    extraction_run_id: None,
                    // Evidence refs.
                    evidence_refs: Vec::new(),
                    // Temporal.
                    temporal: crate::TemporalMetadata::default(),
                    // Transaction.
                    transaction: crate::TransactionMetadata::default(),
                },
                storage_ref: None,
            },
        );
        pager.nodes.insert(
            target.clone(),
            PagedNode {
                // Node.
                node: crate::Node {
                    // Id.
                    id: target.clone(),
                    // Version id.
                    version_id: crate::NodeVersionId::new("node-version--target-supernode-budget")
                        .expect("node version ID should be valid"),
                    // Version.
                    version: 1,
                    // Current.
                    current: true,
                    // Previous version id.
                    previous_version_id: None,
                    // Labels.
                    labels: vec!["Campaign".to_owned()],
                    // Properties.
                    properties: PropertyMap::new(),
                    // Status.
                    status: crate::RecordStatus::Candidate,
                    // Confidence.
                    confidence: None,
                    // Source reliability.
                    source_reliability: None,
                    // Information credibility.
                    information_credibility: None,
                    // Extraction run id.
                    extraction_run_id: None,
                    // Evidence refs.
                    evidence_refs: Vec::new(),
                    // Temporal.
                    temporal: crate::TemporalMetadata::default(),
                    // Transaction.
                    transaction: crate::TransactionMetadata::default(),
                },
                storage_ref: None,
            },
        );
        pager.relationships.insert(
            rel_id.clone(),
            PagedRelationship {
                // Relationship.
                relationship: crate::Relationship {
                    // Id.
                    id: rel_id.clone(),
                    // Version id.
                    version_id: crate::RelationshipVersionId::new(
                        "relationship-version--supernode-budget",
                    )
                    .expect("relationship version ID should be valid"),
                    // Version.
                    version: 1,
                    // Current.
                    current: true,
                    // Previous version id.
                    previous_version_id: None,
                    // Rel type.
                    rel_type: rel_type.clone(),
                    // Source.
                    source: seed.clone(),
                    // Target.
                    target: target.clone(),
                    // Properties.
                    properties: PropertyMap::new(),
                    // Status.
                    status: crate::RecordStatus::Candidate,
                    // Confidence.
                    confidence: None,
                    // Source reliability.
                    source_reliability: None,
                    // Information credibility.
                    information_credibility: None,
                    // Extraction run id.
                    extraction_run_id: None,
                    evidence_refs: Vec::new(),
                    temporal: crate::TemporalMetadata::default(),
                    transaction: crate::TransactionMetadata::default(),
                },
                storage_ref: None,
            },
        );
        pager.outgoing.insert(
            seed.clone(),
            PagedAdjacency {
                // Owner node id.
                owner_node_id: seed.clone(),
                // Direction.
                direction: AdjacencyDirection::Outgoing,
                // Entries.
                entries: vec![PagedAdjacencyEntry {
                    // Relationship id.
                    relationship_id: rel_id.clone(),
                    // Neighbor node id.
                    neighbor_node_id: target,
                    // Relationship type.
                    relationship_type: Some(rel_type),
                    // Relationship storage ref.
                    relationship_storage_ref: None,
                    // Neighbor storage ref.
                    neighbor_storage_ref: None,
                }],
                // Storage ref.
                storage_ref: None,
            },
        );

        let request = ExpansionRequest::new(
            ws_id,
            vec![seed],
            ExpansionDirection::Outgoing,
            ExpansionFilters::empty(),
            1,
            default_generic_loading_profile(),
            ExpansionBudget {
                // Max supernode expansion count.
                max_supernode_expansion_count: 0,
                ..generous_budget()
            },
        )
        .with_supernode_policy(SupernodePolicy {
            // Degree threshold.
            degree_threshold: 0,
            // Require relationship filter.
            require_relationship_filter: false,
            // Require label filter.
            require_label_filter: false,
            // Require time window.
            require_time_window: false,
            // Require limit.
            require_limit: false,
        });

        let result = expand_working_set_from_graph_adjacency(&mut manager, &pager, request)
            .expect("supernode counter overflow should return partial result");

        assert_eq!(result.status(), ExpansionResultStatus::Partial);
        assert!(result.supernode_error().is_none());
        assert!(result.budget_error().is_some());
        assert_eq!(result.usage().supernode_expansion_count, 0);
        assert_eq!(
            result.explanation().skipped_expansions()[0].reason,
            SkippedExpansionReason::BudgetLimit
        );
    }

    #[test]
    fn expand_working_set_maps_relationship_metadata_errors_from_pager() {
        let ws_id = working_set_id("working-set--metadata-error");
        let mut manager = GraphWorkingSetManager::new();
        manager
            .create_working_set(GraphWorkingSetCreateRequest::new(ws_id.clone()))
            .expect("working set should be created");

        let seed = node_id("node--seed-metadata-error");
        let target = node_id("node--target-metadata-error");
        let rel_id = relationship_id("relationship--metadata-error");

        let mut pager = MockPager::empty();
        pager.nodes.insert(
            seed.clone(),
            PagedNode {
                // Node.
                node: crate::Node {
                    // Id.
                    id: seed.clone(),
                    // Version id.
                    version_id: crate::NodeVersionId::new("node-version--metadata-error")
                        .expect("node version ID should be valid"),
                    // Version.
                    version: 1,
                    // Current.
                    current: true,
                    // Previous version id.
                    previous_version_id: None,
                    // Labels.
                    labels: vec!["Campaign".to_owned()],
                    // Properties.
                    properties: PropertyMap::new(),
                    // Status.
                    status: crate::RecordStatus::Candidate,
                    // Confidence.
                    confidence: None,
                    // Source reliability.
                    source_reliability: None,
                    // Information credibility.
                    information_credibility: None,
                    // Extraction run id.
                    extraction_run_id: None,
                    // Evidence refs.
                    evidence_refs: Vec::new(),
                    // Temporal.
                    temporal: crate::TemporalMetadata::default(),
                    // Transaction.
                    transaction: crate::TransactionMetadata::default(),
                },
                storage_ref: None,
            },
        );
        pager.outgoing.insert(
            seed,
            PagedAdjacency {
                // Owner node id.
                owner_node_id: node_id("node--seed-metadata-error"),
                // Direction.
                direction: AdjacencyDirection::Outgoing,
                // Entries.
                entries: vec![PagedAdjacencyEntry {
                    // Relationship id.
                    relationship_id: rel_id.clone(),
                    // Neighbor node id.
                    neighbor_node_id: target,
                    // Relationship type.
                    relationship_type: None,
                    // Relationship storage ref.
                    relationship_storage_ref: None,
                    // Neighbor storage ref.
                    neighbor_storage_ref: None,
                }],
                // Storage ref.
                storage_ref: None,
            },
        );

        let request = ExpansionRequest::new(
            ws_id,
            vec![node_id("node--seed-metadata-error")],
            ExpansionDirection::Outgoing,
            ExpansionFilters::empty(),
            1,
            default_generic_loading_profile(),
            generous_budget(),
        );

        let error = expand_working_set_from_graph_adjacency(&mut manager, &pager, request)
            .expect_err("missing relationship metadata should map pager error to graph invariant");

        assert!(matches!(
        error,
        GraphError::InternalInvariantViolation(message)
        if message.contains("graph pager error") && message.contains(rel_id.as_str())
        ));
    }

    #[test]
    fn helper_usage_builders_increment_expected_counters() {
        let empty = empty_usage();
        assert_eq!(empty.loaded_node_count, 0);
        assert_eq!(empty.loaded_relationship_count, 0);
        assert_eq!(empty.warm_adjacency_entry_count, 0);
        assert_eq!(empty.supernode_expansion_count, 0);

        let with_node = with_hot_node_usage(&empty);
        assert_eq!(with_node.loaded_node_count, 1);
        assert_eq!(with_node.hot_node_count, 1);

        let with_relationship = with_hot_relationship_usage(&with_node);
        assert_eq!(with_relationship.loaded_relationship_count, 1);
        assert_eq!(with_relationship.hot_relationship_count, 1);

        let with_warm = with_warm_adjacency_usage(&with_relationship);
        assert_eq!(with_warm.loaded_relationship_count, 2);
        assert_eq!(with_warm.warm_adjacency_entry_count, 1);

        let with_supernode = with_supernode_expansion_usage(&with_warm);
        assert_eq!(with_supernode.supernode_expansion_count, 1);
    }

    #[test]
    fn observed_degree_and_map_pager_error_helpers_are_deterministic() {
        let owner = node_id("node--degree-owner");
        let adjacency = PagedAdjacency {
            owner_node_id: owner,
            direction: AdjacencyDirection::Outgoing,
            entries: vec![
                PagedAdjacencyEntry {
                    // Relationship id.
                    relationship_id: relationship_id("relationship--degree-1"),
                    // Neighbor node id.
                    neighbor_node_id: node_id("node--degree-neighbor-1"),
                    // Relationship type.
                    relationship_type: Some(relationship_type("USES")),
                    // Relationship storage ref.
                    relationship_storage_ref: None,
                    // Neighbor storage ref.
                    neighbor_storage_ref: None,
                },
                PagedAdjacencyEntry {
                    // Relationship id.
                    relationship_id: relationship_id("relationship--degree-2"),
                    // Neighbor node id.
                    neighbor_node_id: node_id("node--degree-neighbor-2"),
                    // Relationship type.
                    relationship_type: Some(relationship_type("USES")),
                    // Relationship storage ref.
                    relationship_storage_ref: None,
                    // Neighbor storage ref.
                    neighbor_storage_ref: None,
                },
            ],
            // Storage ref.
            storage_ref: None,
        };

        assert_eq!(
            observed_degree_from_adjacency(&adjacency)
                .expect("adjacency entry count should map to observed degree"),
            2
        );

        let mapped = map_pager_error(GraphPagerError::UnavailableRecord {
            // Record ref.
            record_ref: GraphRecordRef::Node(node_id("node--missing")),
        });
        assert!(matches!(
        mapped,
        GraphError::InternalInvariantViolation(message)
        if message.contains("graph pager error") && message.contains("unavailable graph record")
        ));
    }

    #[test]
    fn mock_pager_returns_unavailable_record_errors_for_missing_entries() {
        let pager = MockPager::empty();
        let missing_node = node_id("node--missing");
        let missing_relationship = relationship_id("relationship--missing");

        let node_error = pager
            .load_node_payload(&missing_node)
            .expect_err("missing node payload should return unavailable record");
        assert!(matches!(
        node_error,
        GraphPagerError::UnavailableRecord {
        record_ref: GraphRecordRef::Node(node)
        } if node == missing_node
        ));

        let relationship_error = pager
            .load_relationship_payload(&missing_relationship)
            .expect_err("missing relationship payload should return unavailable record");
        assert!(matches!(
        relationship_error,
        GraphPagerError::UnavailableRecord {
        record_ref: GraphRecordRef::Relationship(relationship)
        } if relationship == missing_relationship
        ));

        let outgoing_error = pager
            .load_outgoing_adjacency(&missing_node)
            .expect_err("missing outgoing adjacency should return unavailable record");
        assert!(matches!(
        outgoing_error,
        GraphPagerError::UnavailableRecord {
        record_ref: GraphRecordRef::Node(node)
        } if node == missing_node
        ));

        let incoming_error = pager
            .load_incoming_adjacency(&missing_node)
            .expect_err("missing incoming adjacency should return unavailable record");
        assert!(matches!(
        incoming_error,
        GraphPagerError::UnavailableRecord {
        record_ref: GraphRecordRef::Node(node)
        } if node == missing_node
        ));
    }

    #[test]
    fn check_supernode_expansion_guards_accepts_request_with_all_required_guards() {
        let policy = SupernodePolicy {
            degree_threshold: 3,
            require_relationship_filter: true,
            require_label_filter: true,
            require_time_window: true,
            require_limit: true,
        };
        let request = ExpansionRequest::new(
            working_set_id("working-set--supernode-guards-ok"),
            vec![node_id("node--seed-supernode-guards-ok")],
            ExpansionDirection::Outgoing,
            ExpansionFilters::new(vec![relationship_type("USES")], vec!["Campaign".to_owned()]),
            1,
            default_generic_loading_profile(),
            generous_budget(),
        )
        .with_guards(
            ExpansionGuards::empty()
                .with_time_window()
                .with_explicit_limit(50),
        );

        check_supernode_expansion_guards(
            &policy,
            &node_id("node--source-supernode-guards-ok"),
            9,
            &request,
        )
        .expect("request with all required guards should pass supernode validation");
    }
}

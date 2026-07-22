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
//! working-set streaming acceptance suite.
//!
//!
//!
//! - Add executable acceptance tests for the public contract.
//! - Keep tests on public `graph_core` APIs only.
//! - Use in-memory graph fixtures and mock pagers only.
//! - Do not require semantic search, Cypher execution, production persistent
//!   storage, network services, or background workers.

use graph_core::{
    AdjacencyDirection, ExpansionBudget, ExpansionBudgetUsage, ExpansionDirection,
    ExpansionFilters, ExpansionLimit, ExpansionResultStatus, ExpansionSafetyErrorCode,
    FixHintScope, Graph, GraphPager, GraphPagerResult, GraphRecordMetadata, GraphRecordRef,
    GraphWorkingSetCreateRequest, GraphWorkingSetManager, LoadingProfileKind, LoadingState, NodeId,
    NodeInput, PagedAdjacency, PagedNode, PagedRelationship, PropertyValue, RelationshipId,
    RelationshipInput, RelationshipType, SkippedExpansionReason, StorageRef, SupernodeGuard,
    SupernodePolicy, WarmAdjacencyEntry, WarmAdjacencyEntryInput, WarmAdjacencyReason,
    WarmAdjacencyRelevanceScore, WorkingSetId, default_fimi_investigation_profile,
    expand_working_set_from_graph_adjacency, lookup_loading_profile,
};

fn working_set_id(value: &str) -> WorkingSetId {
    WorkingSetId::new(value).expect("acceptance working-set ID should be valid")
}

fn node_id(value: &str) -> NodeId {
    NodeId::new(value).expect("acceptance node ID should be valid")
}

fn relationship_id(value: &str) -> RelationshipId {
    RelationshipId::new(value).expect("acceptance relationship ID should be valid")
}

fn relationship_type(value: &str) -> RelationshipType {
    RelationshipType::new(value).expect("acceptance relationship type should be valid")
}

fn create_request(id: &WorkingSetId) -> GraphWorkingSetCreateRequest {
    GraphWorkingSetCreateRequest::new(id.clone())
}

fn create_manager_with_working_set(id: &WorkingSetId) -> GraphWorkingSetManager {
    let mut manager = GraphWorkingSetManager::new();
    manager
        .create_working_set(create_request(id))
        .expect("acceptance working set should be created");
    manager
}

fn permissive_budget() -> ExpansionBudget {
    ExpansionBudget {
        max_loaded_node_count: 100,
        max_loaded_relationship_count: 100,
        max_hot_node_count: 50,
        max_hot_relationship_count: 50,
        max_warm_adjacency_entry_count: 50,
        max_hop_count: 3,
        max_supernode_expansion_count: 10,
        max_payload_byte_count: 1_048_576,
        max_execution_time_ms: 1_000,
    }
}

fn one_hop_request(
    working_set_id: WorkingSetId,
    seed_node_id: NodeId,
    budget: ExpansionBudget,
) -> graph_core::ExpansionRequest {
    graph_core::ExpansionRequest::new(
        working_set_id,
        vec![seed_node_id],
        ExpansionDirection::Outgoing,
        ExpansionFilters::empty(),
        1,
        default_fimi_investigation_profile(),
        budget,
    )
}

fn graph_with_one_hop_and_warm_frontier() -> (
    Graph,
    NodeId,
    NodeId,
    NodeId,
    RelationshipId,
    RelationshipId,
) {
    let mut graph = Graph::new();
    let campaign = graph
        .create_node(NodeInput::new(["Campaign"]))
        .expect("campaign node should be created");
    let narrative = graph
        .create_node(NodeInput::new(["Narrative"]))
        .expect("narrative node should be created");
    let source = graph
        .create_node(NodeInput::new(["Source"]))
        .expect("source node should be created");

    let campaign_to_narrative = graph
        .create_relationship(
            RelationshipInput::new(campaign.clone(), "PROMOTES", narrative.clone())
                .expect("campaign to narrative relationship input should be valid"),
        )
        .expect("campaign to narrative relationship should be created");
    let narrative_to_source = graph
        .create_relationship(
            RelationshipInput::new(narrative.clone(), "SUPPORTS", source.clone())
                .expect("narrative to source relationship input should be valid"),
        )
        .expect("narrative to source relationship should be created");

    (
        graph,
        campaign,
        narrative,
        source,
        campaign_to_narrative,
        narrative_to_source,
    )
}

fn graph_with_high_degree_campaign(degree: usize) -> (Graph, NodeId) {
    let mut graph = Graph::new();
    let campaign = graph
        .create_node(NodeInput::new(["Campaign"]))
        .expect("campaign node should be created");

    for index in 0..degree {
        let narrative = graph
            .create_node(NodeInput::new(["Narrative"]))
            .expect("narrative node should be created");
        graph
            .create_relationship(
                RelationshipInput::new(campaign.clone(), "PROMOTES", narrative)
                    .expect("campaign to narrative relationship input should be valid"),
            )
            .unwrap_or_else(|error| panic!("relationship {index} should be created: {error}"));
    }

    (graph, campaign)
}

fn profile_contains_relationship_type(types: &[RelationshipType], expected: &str) -> bool {
    types
        .iter()
        .any(|relationship_type| relationship_type.as_str() == expected)
}

struct InMemoryMockPager {
    graph: Graph,
}

impl InMemoryMockPager {
    fn new(graph: Graph) -> Self {
        Self { graph }
    }
}

impl GraphPager for InMemoryMockPager {
    fn load_node_payload(&self, node_id: &NodeId) -> GraphPagerResult<PagedNode> {
        GraphPager::load_node_payload(&self.graph, node_id)
    }

    fn load_relationship_payload(
        &self,
        relationship_id: &RelationshipId,
    ) -> GraphPagerResult<PagedRelationship> {
        GraphPager::load_relationship_payload(&self.graph, relationship_id)
    }

    fn load_outgoing_adjacency(&self, node_id: &NodeId) -> GraphPagerResult<PagedAdjacency> {
        GraphPager::load_outgoing_adjacency(&self.graph, node_id)
    }

    fn load_incoming_adjacency(&self, node_id: &NodeId) -> GraphPagerResult<PagedAdjacency> {
        GraphPager::load_incoming_adjacency(&self.graph, node_id)
    }

    fn load_indexed_metadata(
        &self,
        record_ref: &GraphRecordRef,
    ) -> GraphPagerResult<GraphRecordMetadata> {
        GraphPager::load_indexed_metadata(&self.graph, record_ref)
    }
}

//
// Validate that the public graph-core facade exposes enough working-set API to
// create a bounded working set without touching a storage backend.
//
// Given a graph-core caller using only public crate exports,
// when it creates a `GraphWorkingSetManager` and a `GraphWorkingSetCreateRequest`,
// then a working set should be created and retrievable by its `WorkingSetId`.
#[test]
fn validates_working_set_creation_contract() {
    let mut manager = GraphWorkingSetManager::new();
    let id = working_set_id("acceptance--working-set-creation");

    let created = manager
        .create_working_set(create_request(&id))
        .expect("working set should be created");
    assert_eq!(created.id(), &id);

    let retrieved = manager
        .get_working_set(&id)
        .expect("working set should be retrievable");
    assert_eq!(retrieved.id(), &id);
}

//
// Validate that semantic or explicit entry points can be represented as seed
// nodes before graph expansion starts.
//
// Given a created working set,
// when seed node IDs are loaded into it,
// then those IDs should be tracked as seed nodes without forcing unrelated graph records hot.
#[test]
fn validates_seed_node_tracking_contract() {
    let id = working_set_id("acceptance--seed-tracking");
    let mut manager = create_manager_with_working_set(&id);
    let campaign = node_id("campaign--seed");
    let narrative = node_id("narrative--seed");

    let working_set = manager
        .load_seed_node_ids(&id, [campaign.clone(), narrative.clone()], false)
        .expect("seed nodes should be loaded");

    assert!(working_set.seed_node_ids().contains(&campaign));
    assert!(working_set.seed_node_ids().contains(&narrative));
    assert!(!working_set.hot_node_ids().contains(&campaign));
    assert_eq!(
        working_set.node_loading_state(&campaign),
        Some(LoadingState::Indexed)
    );
}

//
// Validate that active node records can be promoted into the hot working-set
// ring independently from the full persistent graph.
//
// Given a created working set with one or more seed nodes,
// when selected node IDs are marked hot,
// then the working set should expose those IDs as hot nodes and count them in stats.
#[test]
fn validates_hot_node_tracking_contract() {
    let id = working_set_id("acceptance--hot-node-tracking");
    let mut manager = create_manager_with_working_set(&id);
    let campaign = node_id("campaign--hot");

    let working_set = manager
        .load_seed_node_ids(&id, [campaign.clone()], true)
        .expect("seed should be loaded as hot");

    assert!(working_set.seed_node_ids().contains(&campaign));
    assert!(working_set.hot_node_ids().contains(&campaign));
    assert_eq!(working_set.stats().hot_node_count(), 1);
    assert_eq!(
        working_set.node_loading_state(&campaign),
        Some(LoadingState::Hot)
    );
}

//
// Validate that active relationship records are first-class hot working-set
// records, not implied only by hot nodes.
//
// Given a created working set,
// when a relationship ID is tracked as hot,
// then the working set should expose that relationship and update hot relationship stats.
#[test]
fn validates_hot_relationship_tracking_contract() {
    let id = working_set_id("acceptance--hot-relationship-tracking");
    let mut manager = create_manager_with_working_set(&id);
    let relationship = relationship_id("relationship--hot");

    let working_set = manager
        .add_hot_relationship(&id, relationship.clone())
        .expect("relationship should be tracked as hot");

    assert!(working_set.hot_relationship_ids().contains(&relationship));
    assert_eq!(working_set.stats().hot_relationship_count(), 1);
    assert_eq!(
        working_set.relationship_loading_state(&relationship),
        Some(LoadingState::Hot)
    );
}

//
// Validate warm adjacency as lightweight frontier metadata rather than full
// payload loading.
//
// Given a hot or indexed source node and a neighboring target represented only by metadata,
// when a warm adjacency entry is attached,
// then relationship ID, relationship type, direction, target labels, loading state,
// relevance score, and storage references should be inspectable without loading node or relationship payloads.
#[test]
fn validates_warm_adjacency_without_full_payload_loading_contract() {
    let id = working_set_id("acceptance--warm-adjacency");
    let mut manager = create_manager_with_working_set(&id);
    let source = node_id("campaign--warm-source");
    let target = node_id("narrative--warm-target");
    let relationship = relationship_id("relationship--warm");
    let relationship_type = relationship_type("PROMOTES");
    let relationship_storage_ref = StorageRef::Record {
        collection: "relationships".to_owned(),
        key: relationship.as_str().to_owned(),
    };
    let target_storage_ref = StorageRef::Record {
        collection: "nodes".to_owned(),
        key: target.as_str().to_owned(),
    };

    let entry = WarmAdjacencyEntry::new(
        WarmAdjacencyEntryInput::new(
            relationship.clone(),
            relationship_type.clone(),
            source.clone(),
            target.clone(),
            vec!["Narrative".to_owned()],
            AdjacencyDirection::Outgoing,
        )
        .with_relevance_score(
            WarmAdjacencyRelevanceScore::new(0.82).expect("warm relevance score should be valid"),
        )
        .with_target_loading_state(LoadingState::Indexed)
        .with_storage_refs(
            Some(relationship_storage_ref.clone()),
            Some(target_storage_ref.clone()),
        ),
    )
    .expect("warm adjacency entry should be valid");

    let working_set = manager
        .add_warm_adjacency(&id, source.clone(), entry)
        .expect("warm adjacency should be attached");

    let entries = working_set
        .warm_adjacency_for_source(&source)
        .expect("source should expose warm adjacency entries");
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    assert_eq!(entry.relationship_id(), &relationship);
    assert_eq!(entry.relationship_type(), &relationship_type);
    assert_eq!(entry.source_node_id(), &source);
    assert_eq!(entry.target_node_id(), &target);
    assert_eq!(entry.target_labels(), &vec!["Narrative".to_owned()]);
    assert_eq!(entry.direction(), AdjacencyDirection::Outgoing);
    assert_eq!(entry.target_loading_state(), LoadingState::Indexed);
    assert_eq!(
        entry.relevance_score().map(|score| score.value()),
        Some(0.82)
    );
    assert_eq!(
        entry.relationship_storage_ref(),
        Some(&relationship_storage_ref)
    );
    assert_eq!(entry.target_storage_ref(), Some(&target_storage_ref));
    assert!(entry.is_target_unloaded());
    assert!(!working_set.hot_node_ids().contains(&target));
    assert!(!working_set.hot_relationship_ids().contains(&relationship));
}

//
// Validate that built-in loading profile lookup remains stable for every Epic
// 0002 domain profile.
//
// Given the stable names `cti_investigation`, `fimi_investigation`, `crisis_investigation`, and `generic`,
// when callers resolve profiles through the public lookup function,
// then each name should return the matching built-in profile kind with deterministic profile content.
#[test]
fn validates_loading_profile_lookup_contract() {
    let cases = [
        ("cti_investigation", LoadingProfileKind::CtiInvestigation),
        ("fimi_investigation", LoadingProfileKind::FimiInvestigation),
        (
            "crisis_investigation",
            LoadingProfileKind::CrisisInvestigation,
        ),
        ("generic", LoadingProfileKind::Generic),
    ];

    for (name, expected_kind) in cases {
        let first = lookup_loading_profile(name).expect("profile name should resolve");
        let second =
            lookup_loading_profile(name).expect("profile name should resolve deterministically");

        assert_eq!(first.kind, expected_kind);
        assert_eq!(first, second);
    }

    let fimi = lookup_loading_profile("fimi_investigation").expect("FIMI profile should resolve");
    assert!(fimi.hot_labels.iter().any(|label| label == "Campaign"));
    assert!(profile_contains_relationship_type(
        &fimi.prioritized_relationship_types,
        "PROMOTES"
    ));
    assert!(profile_contains_relationship_type(
        &fimi.blocked_by_default_relationship_types,
        "MENTIONS"
    ));
}

//
// Validate that expansion budgets represent every dimension needed to keep
// working-set expansion bounded and explainable.
//
// Given an `ExpansionBudget` and an `ExpansionBudgetUsage`,
// when the acceptance suite inspects budget dimensions,
// then loaded records, hot records, warm adjacency, hop count, supernode count, payload bytes, and execution time should all be represented.
#[test]
fn validates_expansion_budget_representation_contract() {
    let budget = permissive_budget();
    let usage = ExpansionBudgetUsage {
        loaded_node_count: 10,
        loaded_relationship_count: 11,
        hot_node_count: 5,
        hot_relationship_count: 6,
        warm_adjacency_entry_count: 7,
        hop_count: 2,
        supernode_expansion_count: 1,
        payload_byte_count: 512,
        execution_time_ms: 25,
    };

    assert_eq!(budget.check_usage(&usage), Ok(()));

    let exhausted_usage = ExpansionBudgetUsage {
        loaded_node_count: 101,
        ..usage
    };
    let error = budget
        .check_usage(&exhausted_usage)
        .expect_err("exhausted usage should return a budget error");

    assert_eq!(
        error.code,
        ExpansionSafetyErrorCode::ExpansionBudgetExceeded
    );
    assert_eq!(error.limit, ExpansionLimit::LoadedNodeCount);
    assert_eq!(error.allowed, 100);
    assert_eq!(error.consumed, 101);
    assert!(error.fix_hint.contains("filter") || error.fix_hint.contains("LIMIT"));
}

//
// Validate that budgeted expansion can return a partial result instead of
// pretending the whole traversal succeeded or failing without context.
//
// Given a working-set expansion request whose graph adjacency would exceed one or more configured limits,
// when expansion is executed against an in-memory graph fixture,
// then the result should expose partial status, loaded records before the stop, skipped expansion explanations, and budget usage.
#[test]
fn validates_budgeted_expansion_partial_failure_contract() {
    let (graph, campaign, _narrative, _source, relationship, _warm_relationship) =
        graph_with_one_hop_and_warm_frontier();
    let id = working_set_id("acceptance--budget-partial");
    let mut manager = create_manager_with_working_set(&id);
    let mut budget = permissive_budget();
    budget.max_hot_relationship_count = 0;

    let result = expand_working_set_from_graph_adjacency(
        &mut manager,
        &graph,
        one_hop_request(id.clone(), campaign.clone(), budget),
    )
    .expect("budgeted expansion should return a typed partial result");

    assert_eq!(result.status(), ExpansionResultStatus::Partial);
    let budget_error = result
        .budget_error()
        .expect("partial result should expose budget error");
    assert_eq!(budget_error.limit, ExpansionLimit::HotRelationshipCount);
    assert_eq!(budget_error.allowed, 0);
    assert_eq!(budget_error.consumed, 1);
    assert_eq!(result.usage().hot_node_count, 1);
    assert_eq!(result.usage().hot_relationship_count, 0);

    let skipped = result.explanation().skipped_expansions();
    assert_eq!(skipped.len(), 1);
    assert_eq!(skipped[0].reason, SkippedExpansionReason::BudgetLimit);
    assert_eq!(skipped[0].relationship_id.as_ref(), Some(&relationship));
    assert_eq!(
        skipped[0].fix_hint.as_ref().map(|hint| hint.scope),
        Some(FixHintScope::Budget)
    );

    let working_set = manager
        .get_working_set(&id)
        .expect("working set should remain inspectable after partial expansion");
    assert!(working_set.hot_node_ids().contains(&campaign));
    assert!(!working_set.hot_relationship_ids().contains(&relationship));
}

//
// Validate that supernode protection blocks unsafe high-degree expansion before
// a broad node can explode the working set.
//
// Given a high-degree source node and a supernode policy requiring guards,
// when expansion lacks required relationship, label, time-window, or limit guards,
// then expansion should be blocked with a stable `SUPERNODE_EXPANSION_BLOCKED` explanation and fix hint.
#[test]
fn validates_supernode_blocking_contract() {
    let (graph, campaign) = graph_with_high_degree_campaign(3);
    let id = working_set_id("acceptance--supernode-block");
    let mut manager = create_manager_with_working_set(&id);
    let policy = SupernodePolicy {
        degree_threshold: 2,
        require_relationship_filter: true,
        require_label_filter: true,
        require_time_window: true,
        require_limit: true,
    };

    let request = one_hop_request(id.clone(), campaign.clone(), permissive_budget())
        .with_supernode_policy(policy);
    let result = expand_working_set_from_graph_adjacency(&mut manager, &graph, request)
        .expect("supernode blocking should return a typed partial result");

    assert_eq!(result.status(), ExpansionResultStatus::Partial);
    let supernode_error = result
        .supernode_error()
        .expect("partial result should expose supernode error");
    assert_eq!(
        supernode_error.code,
        ExpansionSafetyErrorCode::SupernodeExpansionBlocked
    );
    assert_eq!(supernode_error.observed_degree, 3);
    assert_eq!(supernode_error.degree_threshold, 2);
    assert!(supernode_error.fix_hint.contains("relationship filter"));
    assert!(supernode_error.fix_hint.contains("LIMIT"));

    let explanation = result.explanation();
    assert_eq!(explanation.skipped_expansions().len(), 1);
    assert_eq!(
        explanation.skipped_expansions()[0].reason,
        SkippedExpansionReason::SupernodePolicy
    );
    assert_eq!(explanation.supernode_blocks().len(), 1);
    let block = &explanation.supernode_blocks()[0];
    assert_eq!(block.node_id, campaign);
    assert_eq!(block.observed_degree, 3);
    assert!(
        block
            .missing_guards
            .contains(&SupernodeGuard::RelationshipFilter)
    );
    assert!(block.missing_guards.contains(&SupernodeGuard::LabelFilter));
    assert!(block.missing_guards.contains(&SupernodeGuard::TimeWindow));
    assert!(block.missing_guards.contains(&SupernodeGuard::Limit));
    assert_eq!(block.fix_hint.scope, FixHintScope::SupernodeGuard);
}

//
// Validate that working-set explanations remain inspectable as structured data
// for agents and diagnostics.
//
// Given a working set populated with seed, hot, warm, skipped, budget, and supernode explanation entries,
// when the explanation is requested through the public manager or expansion API,
// then the output should preserve stable record IDs, reasons, counters, and fix hints without generating analyst prose.
#[test]
fn validates_working_set_explanation_output_contract() {
    let (graph, campaign, narrative, source, hot_relationship, warm_relationship) =
        graph_with_one_hop_and_warm_frontier();
    let id = working_set_id("acceptance--explanation-output");
    let mut manager = create_manager_with_working_set(&id);

    let result = expand_working_set_from_graph_adjacency(
        &mut manager,
        &graph,
        one_hop_request(id.clone(), campaign.clone(), permissive_budget()),
    )
    .expect("expansion should complete and expose explanation output");

    assert_eq!(result.status(), ExpansionResultStatus::Complete);
    let explanation = result.explanation();
    assert_eq!(explanation.seed_nodes().len(), 1);
    assert_eq!(explanation.seed_nodes()[0].node_id, campaign);
    assert_eq!(
        explanation.seed_nodes()[0].source.kind,
        graph_core::SeedSourceKind::ExplicitNodeId
    );
    assert!(
        explanation
            .hot_nodes()
            .iter()
            .any(|node| node.node_id == narrative
                && node.profile_kind == Some(LoadingProfileKind::FimiInvestigation))
    );
    assert!(
        explanation
            .hot_relationships()
            .iter()
            .any(|relationship| relationship.relationship_id == hot_relationship)
    );
    assert!(
        explanation
            .warm_adjacency_entries()
            .iter()
            .any(|entry| entry.relationship_id == warm_relationship
                && entry.source_node_id == narrative
                && entry.target_node_id == source
                && entry.reason == WarmAdjacencyReason::RingBoundary)
    );
    assert!(explanation.consumed_budget().is_some());

    let manager_explanation = manager
        .explanation(&id)
        .expect("manager-owned explanation container should remain available");
    assert!(manager_explanation.seed_nodes().is_empty());
}

//
// Validate pager compatibility without depending on a production persistent
// storage backend.
//
// Given a mock or in-memory type implementing `GraphPager`,
// when acceptance tests request node, relationship, and adjacency records through the pager contract,
// then working-set code should consume those records through the trait boundary only.
#[test]
fn validates_mock_pager_contract_compatibility() {
    let (graph, campaign, narrative, _source, relationship, _warm_relationship) =
        graph_with_one_hop_and_warm_frontier();
    let pager = InMemoryMockPager::new(graph);

    let paged_node = pager
        .load_node_payload(&campaign)
        .expect("mock pager should load node payload");
    assert_eq!(paged_node.node.id(), &campaign);

    let paged_relationship = pager
        .load_relationship_payload(&relationship)
        .expect("mock pager should load relationship payload");
    assert_eq!(paged_relationship.relationship.id(), &relationship);

    let adjacency = pager
        .load_outgoing_adjacency(&campaign)
        .expect("mock pager should load lightweight outgoing adjacency");
    assert_eq!(adjacency.owner_node_id, campaign);
    assert_eq!(adjacency.direction, AdjacencyDirection::Outgoing);
    assert_eq!(adjacency.entries.len(), 1);
    assert_eq!(adjacency.entries[0].neighbor_node_id, narrative);

    let metadata = pager
        .load_indexed_metadata(&GraphRecordRef::Node(
            adjacency.entries[0].neighbor_node_id.clone(),
        ))
        .expect("mock pager should load indexed metadata");
    assert_eq!(metadata.loading_state, LoadingState::Indexed);
    assert!(metadata.labels.iter().any(|label| label == "Narrative"));
}

//
// Validate that the acceptance suite remains backend-neutral.
//
// Given the complete acceptance suite,
// when it runs in CI or locally,
// then it should not require production persistent storage, semantic search indexes, Cypher execution, network services, or background workers.
#[test]
fn validates_acceptance_suite_has_no_production_backend_dependency() {
    let (graph, campaign, _narrative, _source, _relationship, _warm_relationship) =
        graph_with_one_hop_and_warm_frontier();
    let pager = InMemoryMockPager::new(graph);
    let id = working_set_id("acceptance--backend-neutral");
    let mut manager = create_manager_with_working_set(&id);

    let result = expand_working_set_from_graph_adjacency(
        &mut manager,
        &pager,
        one_hop_request(id, campaign, permissive_budget()),
    )
    .expect("acceptance expansion should run against an in-memory mock pager");

    assert_eq!(result.status(), ExpansionResultStatus::Complete);
    assert!(result.budget_error().is_none());
    assert!(result.supernode_error().is_none());
}

//
// Validate that does not regress the public graph-core behavior
// delivered by .
//
// Given the existing graph-core public node, relationship, property, identifier, confidence, temporal, and transaction contracts,
// when the acceptance suite is run with the normal graph-core tests,
// then those contracts should continue to pass unchanged.
#[test]
fn validates_epic_0001_public_behavior_still_passes() {
    let mut graph = Graph::new();
    let campaign = graph
        .create_node(NodeInput::new(["Campaign"]).with_property(
            "name",
            PropertyValue::String("Operation Example".to_owned()),
        ))
        .expect("campaign node should be created");
    let narrative = graph
        .create_node(NodeInput::new(["Narrative"]))
        .expect("narrative node should be created");
    let relationship = graph
        .create_relationship(
            RelationshipInput::new(campaign.clone(), "PROMOTES", narrative.clone())
                .expect("relationship input should be valid"),
        )
        .expect("relationship should be created");

    let stored_campaign = graph
        .get_node(&campaign)
        .expect("graph read should succeed")
        .expect("campaign should be visible");
    assert!(stored_campaign.has_label("Campaign"));
    assert_eq!(
        stored_campaign.property("name"),
        Some(&PropertyValue::String("Operation Example".to_owned()))
    );

    let relationships = graph
        .relationships_between(&campaign, &narrative)
        .expect("relationships between nodes should be readable");
    assert_eq!(relationships.len(), 1);
    assert_eq!(relationships[0].id(), &relationship);
    assert_eq!(relationships[0].rel_type().as_str(), "PROMOTES");
}

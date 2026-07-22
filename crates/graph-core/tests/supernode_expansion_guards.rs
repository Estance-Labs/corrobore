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
use graph_core::{
    AdjacencyDirection, ExpansionBudget, ExpansionBudgetUsage, ExpansionDirection,
    ExpansionFilters, ExpansionGuards, ExpansionRequest, ExpansionResult, ExpansionResultStatus,
    ExpansionSafetyErrorCode, Graph, GraphError, GraphWorkingSetCreateRequest,
    GraphWorkingSetManager, NodeId, NodeInput, PagedAdjacency, PagedAdjacencyEntry, RelationshipId,
    RelationshipInput, RelationshipType, SkippedExpansionReason, SupernodeExpansionBlocked,
    SupernodeGuard, SupernodePolicy, WorkingSetExplanation, WorkingSetId,
    check_supernode_expansion_guards, default_generic_loading_profile,
    expand_working_set_from_graph_adjacency, observed_degree_from_adjacency,
    record_supernode_blocked_expansion,
};

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

fn budget() -> ExpansionBudget {
    ExpansionBudget {
        max_loaded_node_count: 100,
        max_loaded_relationship_count: 100,
        max_hot_node_count: 50,
        max_hot_relationship_count: 50,
        max_warm_adjacency_entry_count: 100,
        max_hop_count: 2,
        max_supernode_expansion_count: 5,
        max_payload_byte_count: 1_048_576,
        max_execution_time_ms: 1_500,
    }
}

fn empty_usage() -> ExpansionBudgetUsage {
    ExpansionBudgetUsage {
        loaded_node_count: 0,
        loaded_relationship_count: 0,
        hot_node_count: 0,
        hot_relationship_count: 0,
        warm_adjacency_entry_count: 0,
        hop_count: 0,
        supernode_expansion_count: 0,
        payload_byte_count: 0,
        execution_time_ms: 0,
    }
}

fn strict_supernode_policy() -> SupernodePolicy {
    SupernodePolicy {
        degree_threshold: 3,
        require_relationship_filter: true,
        require_label_filter: true,
        require_time_window: true,
        require_limit: true,
    }
}

fn expansion_request(filters: ExpansionFilters, guards: ExpansionGuards) -> ExpansionRequest {
    expansion_request_for(
        working_set_id("working-set--issue-44"),
        node_id("node--supernode"),
        filters,
        guards,
    )
}

fn expansion_request_for(
    working_set_id: WorkingSetId,
    seed_node_id: NodeId,
    filters: ExpansionFilters,
    guards: ExpansionGuards,
) -> ExpansionRequest {
    ExpansionRequest::new(
        working_set_id,
        vec![seed_node_id],
        ExpansionDirection::Outgoing,
        filters,
        1,
        default_generic_loading_profile(),
        budget(),
    )
    .with_guards(guards)
    .with_supernode_policy(strict_supernode_policy())
}

fn high_degree_adjacency(degree: u64) -> PagedAdjacency {
    let owner_node_id = node_id("node--supernode");
    let relationship_type = relationship_type("MENTIONS");

    PagedAdjacency {
        owner_node_id,
        direction: AdjacencyDirection::Outgoing,
        entries: (0..degree)
            .map(|index| PagedAdjacencyEntry {
                relationship_id: relationship_id(&format!("relationship--supernode-{index}")),
                neighbor_node_id: node_id(&format!("node--neighbor-{index}")),
                relationship_type: Some(relationship_type.clone()),
                relationship_storage_ref: None,
                neighbor_storage_ref: None,
            })
            .collect(),
        storage_ref: None,
    }
}

fn high_degree_graph(degree: u64) -> (Graph, NodeId) {
    let mut graph = Graph::new();
    let supernode_id = graph
        .create_node(NodeInput::new(["Campaign"]))
        .expect("supernode should be created");

    for _ in 0..degree {
        let neighbor_id = graph
            .create_node(NodeInput::new(["Campaign"]))
            .expect("neighbor should be created");
        graph
            .create_relationship(
                RelationshipInput::new(supernode_id.clone(), "MENTIONS", neighbor_id)
                    .expect("relationship input should be valid"),
            )
            .expect("relationship should be created");
    }

    (graph, supernode_id)
}

fn create_manager(working_set_id: &WorkingSetId) -> GraphWorkingSetManager {
    let mut manager = GraphWorkingSetManager::new();
    manager
        .create_working_set(GraphWorkingSetCreateRequest::new(working_set_id.clone()))
        .expect("working set should be created");
    manager
}

//
// Specify that supernode degree detection uses the already loaded lightweight
// adjacency page and does not require payload loading.
//
// Given a deliberately high-degree adjacency fixture,
// when `observed_degree_from_adjacency` is called,
// then it should return the deterministic adjacency entry count.
#[test]
fn observed_degree_counts_loaded_adjacency_entries() {
    let adjacency = high_degree_adjacency(4);

    let observed_degree = observed_degree_from_adjacency(&adjacency)
        .expect("adjacency entry count should produce an observed degree");

    assert_eq!(observed_degree, 4);
}

//
// Specify that high-degree expansion is blocked when the policy requires every
// narrowing guard and the request does not provide them.
//
// Given a supernode policy requiring relationship, label, time-window, and limit guards,
// when a high-degree node is evaluated with an unguarded request,
// then a typed `SupernodeExpansionBlocked` error should be returned.
#[test]
fn supernode_guard_check_blocks_unguarded_high_degree_expansion() {
    let policy = strict_supernode_policy();
    let request = expansion_request(ExpansionFilters::empty(), ExpansionGuards::empty());

    let error = check_supernode_expansion_guards(&policy, &node_id("node--supernode"), 4, &request)
        .expect_err("unguarded high-degree expansion should be blocked");

    let GraphError::SupernodeExpansionBlocked(blocked) = error else {
        panic!("expected GraphError::SupernodeExpansionBlocked");
    };

    assert_eq!(
        blocked.code,
        ExpansionSafetyErrorCode::SupernodeExpansionBlocked
    );
    assert_eq!(blocked.observed_degree, 4);
    assert_eq!(blocked.degree_threshold, 3);
    assert!(blocked.relationship_filter_required);
    assert!(blocked.label_filter_required);
    assert!(blocked.time_window_required);
    assert!(blocked.limit_required);
    assert!(blocked.fix_hint.contains("relationship"));
    assert!(blocked.fix_hint.contains("label"));
    assert!(blocked.fix_hint.contains("time"));
    assert!(blocked.fix_hint.contains("LIMIT") || blocked.fix_hint.contains("limit"));
}

//
// Specify that non-supernode expansion is not blocked by supernode guard policy.
//
// Given the observed degree is below the policy threshold,
// when the request is otherwise unguarded,
// then the supernode policy should not block expansion.
#[test]
fn supernode_guard_check_allows_non_supernode_without_extra_guards() {
    let policy = strict_supernode_policy();
    let request = expansion_request(ExpansionFilters::empty(), ExpansionGuards::empty());

    let result = check_supernode_expansion_guards(&policy, &node_id("node--small"), 2, &request);

    assert_eq!(result, Ok(()));
}

//
// Specify that bounded expansion remains possible for high-degree nodes when the
// caller supplies every guard required by `SupernodePolicy`.
//
// Given a high-degree node and explicit relationship, label, time-window, and limit guards,
// when the supernode guard check runs,
// then expansion should be accepted so deterministic bounded traversal can continue.
#[test]
fn supernode_guard_check_allows_high_degree_expansion_with_all_required_guards() {
    let policy = strict_supernode_policy();
    let filters = ExpansionFilters::new(
        vec![relationship_type("MENTIONS")],
        vec!["Campaign".to_owned()],
    );
    let guards = ExpansionGuards::new(true, Some(10));
    let request = expansion_request(filters, guards);

    let result =
        check_supernode_expansion_guards(&policy, &node_id("node--supernode"), 4, &request);

    assert_eq!(result, Ok(()));
}

//
// Specify that a blocked supernode is visible in both skipped-expansion metadata
// and the dedicated supernode block explanation output.
//
// Given a `SupernodeExpansionBlocked` payload,
// when the blocked expansion is recorded,
// then explanation output should include skipped-supernode and block records with
// missing guard metadata and a fix hint.
#[test]
fn record_supernode_blocked_expansion_adds_skipped_and_block_metadata() {
    let mut explanation = WorkingSetExplanation::new();
    let error = SupernodeExpansionBlocked {
        code: ExpansionSafetyErrorCode::SupernodeExpansionBlocked,
        observed_degree: 4,
        degree_threshold: 3,
        relationship_filter_required: true,
        label_filter_required: true,
        time_window_required: true,
        limit_required: true,
        fix_hint: "Add relationship, label, time-window, and LIMIT guards.".to_owned(),
    };

    record_supernode_blocked_expansion(&mut explanation, node_id("node--supernode"), &error)
        .expect("blocked supernode metadata should be recordable");

    let skipped = explanation.skipped_expansions();
    assert_eq!(skipped.len(), 1);
    assert_eq!(skipped[0].source_node_id, node_id("node--supernode"));
    assert_eq!(skipped[0].reason, SkippedExpansionReason::SupernodePolicy);
    assert_eq!(
        &skipped[0].fix_hint.as_ref().expect("fix hint").message,
        &error.fix_hint
    );

    let blocks = explanation.supernode_blocks();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].node_id, node_id("node--supernode"));
    assert_eq!(blocks[0].observed_degree, 4);
    assert_eq!(blocks[0].degree_threshold, 3);
    assert!(
        blocks[0]
            .missing_guards
            .contains(&SupernodeGuard::RelationshipFilter)
    );
    assert!(
        blocks[0]
            .missing_guards
            .contains(&SupernodeGuard::LabelFilter)
    );
    assert!(
        blocks[0]
            .missing_guards
            .contains(&SupernodeGuard::TimeWindow)
    );
    assert!(blocks[0].missing_guards.contains(&SupernodeGuard::Limit));
    assert!(blocks[0].fix_hint.message.contains("LIMIT"));
}

//
// Specify the result payload contract for partial expansion caused by supernode
// policy rather than generic budget exhaustion.
//
// Given an otherwise complete expansion result,
// when a supernode error is attached,
// then the result should become partial and expose the typed supernode error.
#[test]
fn expansion_result_can_carry_partial_supernode_error() {
    let error = SupernodeExpansionBlocked {
        code: ExpansionSafetyErrorCode::SupernodeExpansionBlocked,
        observed_degree: 4,
        degree_threshold: 3,
        relationship_filter_required: true,
        label_filter_required: false,
        time_window_required: false,
        limit_required: true,
        fix_hint: "Add relationship and LIMIT guards.".to_owned(),
    };

    let result = ExpansionResult::new(
        working_set_id("working-set--issue-44"),
        ExpansionResultStatus::Complete,
        empty_usage(),
        WorkingSetExplanation::new(),
        None,
    )
    .with_supernode_error(error.clone());

    assert_eq!(result.status(), ExpansionResultStatus::Partial);
    assert_eq!(result.budget_error(), None);
    assert_eq!(result.supernode_error(), Some(&error));
}

//
// Validate the full acceptance path for blocking an unbounded high-degree node
// through the public graph pager, working-set manager, and expansion API.
//
// Given a real in-memory graph with a node above the supernode threshold,
// when expansion runs without required guards,
// then expansion should stop as a typed partial result before loading neighbor
// relationships or nodes, and explanation output should identify the skipped supernode.
#[test]
fn acceptance_blocks_unbounded_supernode_during_graph_expansion() {
    let (graph, supernode_id) = high_degree_graph(4);
    let working_set_id = working_set_id("working-set--issue-44-blocked-acceptance");
    let mut manager = create_manager(&working_set_id);
    let request = expansion_request_for(
        working_set_id.clone(),
        supernode_id.clone(),
        ExpansionFilters::empty(),
        ExpansionGuards::empty(),
    );

    let result = expand_working_set_from_graph_adjacency(&mut manager, &graph, request)
        .expect("blocked supernode should return a partial result, not a hard failure");

    assert_eq!(result.status(), ExpansionResultStatus::Partial);
    assert_eq!(result.budget_error(), None);

    let supernode_error = result
        .supernode_error()
        .expect("partial result should carry a typed supernode error");
    assert_eq!(supernode_error.observed_degree, 4);
    assert_eq!(supernode_error.degree_threshold, 3);
    assert!(supernode_error.fix_hint.contains("relationship"));
    assert!(supernode_error.fix_hint.contains("label"));
    assert!(supernode_error.fix_hint.contains("time"));
    assert!(
        supernode_error.fix_hint.contains("LIMIT") || supernode_error.fix_hint.contains("limit")
    );

    assert_eq!(result.usage().loaded_node_count, 1);
    assert_eq!(result.usage().loaded_relationship_count, 0);
    assert_eq!(result.usage().hot_node_count, 1);
    assert_eq!(result.usage().hot_relationship_count, 0);

    let skipped = result.explanation().skipped_expansions();
    assert_eq!(skipped.len(), 1);
    assert_eq!(skipped[0].source_node_id, supernode_id);
    assert_eq!(skipped[0].reason, SkippedExpansionReason::SupernodePolicy);

    let blocks = result.explanation().supernode_blocks();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].observed_degree, 4);
    assert_eq!(blocks[0].degree_threshold, 3);
    assert!(
        blocks[0]
            .missing_guards
            .contains(&SupernodeGuard::RelationshipFilter)
    );
    assert!(
        blocks[0]
            .missing_guards
            .contains(&SupernodeGuard::LabelFilter)
    );
    assert!(
        blocks[0]
            .missing_guards
            .contains(&SupernodeGuard::TimeWindow)
    );
    assert!(blocks[0].missing_guards.contains(&SupernodeGuard::Limit));

    let stats = manager
        .stats(&working_set_id)
        .expect("working-set stats should remain readable");
    assert_eq!(stats.hot_node_count(), 1);
    assert_eq!(stats.hot_relationship_count(), 0);
}

//
// Validate the full acceptance path for allowing a high-degree node when the
// caller supplies explicit relationship, label, time-window, and limit guards.
//
// Given a real in-memory graph with a node above the supernode threshold,
// when expansion runs with all required guards,
// then traversal should complete deterministically and load the bounded subgraph.
#[test]
fn acceptance_allows_bounded_supernode_expansion_with_explicit_constraints() {
    let (graph, supernode_id) = high_degree_graph(4);
    let working_set_id = working_set_id("working-set--issue-44-allowed-acceptance");
    let mut manager = create_manager(&working_set_id);
    let filters = ExpansionFilters::new(
        vec![relationship_type("MENTIONS")],
        vec!["Campaign".to_owned()],
    );
    let guards = ExpansionGuards::new(true, Some(4));
    let request = expansion_request_for(working_set_id.clone(), supernode_id, filters, guards);

    let result = expand_working_set_from_graph_adjacency(&mut manager, &graph, request)
        .expect("guarded supernode expansion should complete");

    assert_eq!(result.status(), ExpansionResultStatus::Complete);
    assert_eq!(result.budget_error(), None);
    assert_eq!(result.supernode_error(), None);
    assert_eq!(result.usage().loaded_node_count, 5);
    assert_eq!(result.usage().loaded_relationship_count, 4);
    assert_eq!(result.usage().hot_node_count, 5);
    assert_eq!(result.usage().hot_relationship_count, 4);
    assert_eq!(result.usage().supernode_expansion_count, 1);
    assert!(result.explanation().skipped_expansions().is_empty());
    assert!(result.explanation().supernode_blocks().is_empty());

    let stats = manager
        .stats(&working_set_id)
        .expect("working-set stats should remain readable");
    assert_eq!(stats.hot_node_count(), 5);
    assert_eq!(stats.hot_relationship_count(), 4);
}

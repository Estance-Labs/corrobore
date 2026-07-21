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
    AdjacencyDirection, GraphError, GraphWorkingSetCreateRequest, GraphWorkingSetManager,
    LoadingState, NodeId, RelationshipId, RelationshipType, WarmAdjacencyEntry,
    WarmAdjacencyEntryInput, WorkingSetId,
};

fn working_set_id(value: &str) -> WorkingSetId {
    WorkingSetId::new(value).expect("working set ID should be valid")
}

fn node_id(value: &str) -> NodeId {
    NodeId::new(value).expect("node ID should be valid")
}

fn relationship_id(value: &str) -> RelationshipId {
    RelationshipId::new(value).expect("relationship ID should be valid")
}

fn relationship_type(value: &str) -> RelationshipType {
    RelationshipType::new(value).expect("relationship type should be valid")
}

fn create_request(id: &WorkingSetId) -> GraphWorkingSetCreateRequest {
    GraphWorkingSetCreateRequest::new(id.clone())
}

fn warm_entry(
    relationship_id: RelationshipId,
    source_node_id: NodeId,
    target_node_id: NodeId,
) -> WarmAdjacencyEntry {
    WarmAdjacencyEntry::new(WarmAdjacencyEntryInput::new(
        relationship_id,
        relationship_type("RELATED_TO"),
        source_node_id,
        target_node_id,
        vec!["Neighbor".to_owned()],
        AdjacencyDirection::Outgoing,
    ))
    .expect("warm adjacency entry should be valid")
}

//
// Validate through the public `graph_core` crate facade. This is the
// acceptance-level path for lifecycle, seed, hot, warm, pinned, dirty, stats, and
// explanation behavior.
//
// Given a public `GraphWorkingSetManager` and one working-set create request,
// when the caller performs the complete initial manager lifecycle,
// then the working set should satisfy the acceptance criteria.
#[test]
fn acceptance_manager_supports_initial_in_memory_working_set_lifecycle() {
    let mut manager = GraphWorkingSetManager::new();
    let working_set_id = working_set_id("working-set--acceptance");
    let hot_seed = node_id("node--seed-hot");
    let indexed_seed = node_id("node--seed-indexed");
    let warm_target = node_id("node--warm-target");
    let hot_relationship = relationship_id("relationship--hot-acceptance");
    let warm_relationship = relationship_id("relationship--warm-acceptance");
    let dirty_relationship = relationship_id("relationship--dirty-acceptance");

    assert_eq!(
        manager
            .create_working_set(create_request(&working_set_id))
            .expect("working set should be created")
            .id(),
        &working_set_id
    );

    manager
        .load_seed_node_ids(&working_set_id, [hot_seed.clone()], true)
        .expect("hot seed should be loaded");
    manager
        .load_seed_node_ids(&working_set_id, [indexed_seed.clone()], false)
        .expect("indexed seed should be loaded");
    manager
        .add_hot_relationship(&working_set_id, hot_relationship.clone())
        .expect("hot relationship should be tracked");
    manager
        .add_warm_adjacency(
            &working_set_id,
            indexed_seed.clone(),
            warm_entry(
                warm_relationship.clone(),
                indexed_seed.clone(),
                warm_target.clone(),
            ),
        )
        .expect("warm adjacency should be tracked");
    manager
        .pin_node(&working_set_id, hot_seed.clone())
        .expect("node should be pinned");
    manager
        .pin_relationship(&working_set_id, hot_relationship.clone())
        .expect("relationship should be pinned");
    manager
        .mark_dirty_node(&working_set_id, indexed_seed.clone())
        .expect("dirty node should be tracked");
    manager
        .mark_dirty_relationship(&working_set_id, dirty_relationship.clone())
        .expect("dirty relationship should be tracked");

    let working_set = manager
        .get_working_set(&working_set_id)
        .expect("working set should be retrievable");

    assert!(working_set.seed_node_ids().contains(&hot_seed));
    assert!(working_set.seed_node_ids().contains(&indexed_seed));
    assert!(working_set.hot_node_ids().contains(&hot_seed));
    assert_eq!(
        working_set.node_loading_state(&hot_seed),
        Some(LoadingState::Hot)
    );
    assert_eq!(
        working_set.node_loading_state(&indexed_seed),
        Some(LoadingState::Indexed)
    );
    assert_eq!(
        working_set.node_loading_state(&warm_target),
        Some(LoadingState::Warm)
    );
    assert!(
        working_set
            .hot_relationship_ids()
            .contains(&hot_relationship)
    );
    assert_eq!(
        working_set.relationship_loading_state(&warm_relationship),
        Some(LoadingState::Warm)
    );
    assert!(working_set.pinned_node_ids().contains(&hot_seed));
    assert!(
        working_set
            .pinned_relationship_ids()
            .contains(&hot_relationship)
    );
    assert!(working_set.dirty_node_ids().contains(&indexed_seed));
    assert!(
        working_set
            .dirty_relationship_ids()
            .contains(&dirty_relationship)
    );
    assert_eq!(
        working_set
            .warm_adjacency_for_source(&indexed_seed)
            .expect("warm adjacency should be grouped by source")[0]
            .target_node_id(),
        &warm_target
    );

    let stats = manager.stats(&working_set_id).expect("stats should exist");
    assert_eq!(stats.hot_node_count(), 1);
    assert_eq!(stats.hot_relationship_count(), 1);
    assert_eq!(stats.warm_node_count(), 1);
    assert_eq!(stats.warm_relationship_count(), 1);

    let explanation = manager
        .explanation(&working_set_id)
        .expect("explanation should exist");
    assert!(explanation.seed_nodes().is_empty());
    assert!(explanation.hot_nodes().is_empty());
    assert!(explanation.hot_relationships().is_empty());
    assert!(explanation.warm_adjacency_entries().is_empty());
}

//
// Validate the typed error boundary for missing working sets through the public
// manager facade.
//
// Given an empty manager and an unknown working-set ID,
// when callers request the missing working set and its stats,
// then both paths should return `GraphError::WorkingSetNotFound` with that ID.
#[test]
fn acceptance_missing_working_set_read_paths_return_typed_errors() {
    let manager = GraphWorkingSetManager::new();
    let missing_id = working_set_id("working-set--missing-acceptance");

    let lookup_error = manager
        .get_working_set(&missing_id)
        .expect_err("missing working set should fail");
    assert!(matches!(
    lookup_error,
    GraphError::WorkingSetNotFound(id) if id == missing_id
    ));

    let stats_error = manager
        .stats(&missing_id)
        .expect_err("missing stats should fail");
    assert!(matches!(
    stats_error,
    GraphError::WorkingSetNotFound(id) if id == missing_id
    ));
}

//
// Validate manager isolation and unpin preservation as an integration scenario.
//
// Given two managed working sets and one pinned working set with extra state,
// when records are mutated and pins are released,
// then state should remain isolated and unpin should not erase unrelated records.
#[test]
fn integration_manager_isolates_working_sets_and_unpin_preserves_state() {
    let mut manager = GraphWorkingSetManager::new();
    let first_id = working_set_id("working-set--first");
    let second_id = working_set_id("working-set--second");
    let first_seed = node_id("node--first-seed");
    let second_seed = node_id("node--second-seed");
    let first_relationship = relationship_id("relationship--first-hot");
    let warm_relationship = relationship_id("relationship--first-warm");
    let pinned_relationship = relationship_id("relationship--first-pinned");
    let target = node_id("node--first-target");

    manager
        .create_working_set(create_request(&first_id))
        .unwrap();
    manager
        .create_working_set(create_request(&second_id))
        .unwrap();
    manager
        .load_seed_node_ids(&first_id, [first_seed.clone()], true)
        .unwrap();
    manager
        .add_hot_relationship(&first_id, first_relationship.clone())
        .unwrap();
    manager
        .add_warm_adjacency(
            &first_id,
            first_seed.clone(),
            warm_entry(
                warm_relationship.clone(),
                first_seed.clone(),
                target.clone(),
            ),
        )
        .unwrap();
    manager.pin_node(&first_id, first_seed.clone()).unwrap();
    manager
        .pin_relationship(&first_id, pinned_relationship.clone())
        .unwrap();
    manager.mark_dirty_node(&first_id, target.clone()).unwrap();
    manager
        .load_seed_node_ids(&second_id, [second_seed.clone()], false)
        .unwrap();

    manager.unpin_node(&first_id, &first_seed).unwrap();
    let first = manager
        .unpin_relationship(&first_id, &pinned_relationship)
        .unwrap();

    assert!(!first.pinned_node_ids().contains(&first_seed));
    assert!(
        !first
            .pinned_relationship_ids()
            .contains(&pinned_relationship)
    );
    assert!(first.hot_node_ids().contains(&first_seed));
    assert!(first.hot_relationship_ids().contains(&first_relationship));
    assert_eq!(
        first.relationship_loading_state(&warm_relationship),
        Some(LoadingState::Warm)
    );
    assert!(first.dirty_node_ids().contains(&target));
    assert!(!first.seed_node_ids().contains(&second_seed));

    let second = manager.get_working_set(&second_id).unwrap();
    assert!(second.seed_node_ids().contains(&second_seed));
    assert!(!second.seed_node_ids().contains(&first_seed));
    assert_eq!(manager.stats(&second_id).unwrap().hot_node_count(), 0);
}

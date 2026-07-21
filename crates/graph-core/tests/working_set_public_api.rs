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
use graph_core::{GraphWorkingSet, LoadingState, NodeId, RelationshipId, WorkingSetId};

fn working_set_id(value: &str) -> WorkingSetId {
    WorkingSetId::new(value).expect("valid working set ID should be accepted")
}

fn node_id(value: &str) -> NodeId {
    NodeId::new(value).expect("valid node ID should be accepted")
}

fn relationship_id(value: &str) -> RelationshipId {
    RelationshipId::new(value).expect("valid relationship ID should be accepted")
}

//
// Verify that the working set identifier is exposed through the crate facade and
// follows the same public construction contract as the other graph-core IDs.
//
// Given a non-empty working set ID string,
// when `WorkingSetId::new` is called,
// then construction should succeed and `as_str` should return the original value.
#[test]
fn working_set_id_accepts_valid_value() {
    let id = WorkingSetId::new("working-set--1").expect("valid working set ID should be accepted");

    assert_eq!(id.as_str(), "working-set--1");
}

//
// Verify that invalid working set IDs are rejected at the public API boundary.
//
// Given empty or whitespace-only strings,
// when `WorkingSetId::new` is called,
// then construction should fail instead of creating meaningless identifiers.
#[test]
fn working_set_id_rejects_empty_and_whitespace_values() {
    assert!(WorkingSetId::new("").is_err());
    assert!(WorkingSetId::new(" ").is_err());
    assert!(WorkingSetId::new("\t\n").is_err());
}

//
// Verify that loading state is modeled as explicit variants instead of boolean
// loaded/unloaded flags.
//
// Given all supported loading states,
// when the variants are used through the public facade,
// then each state should remain distinct and comparable.
#[test]
fn loading_state_variants_are_explicit_and_distinct() {
    assert_ne!(LoadingState::Cold, LoadingState::Indexed);
    assert_ne!(LoadingState::Indexed, LoadingState::Warm);
    assert_ne!(LoadingState::Warm, LoadingState::Hot);

    let states = [
        LoadingState::Cold,
        LoadingState::Indexed,
        LoadingState::Warm,
        LoadingState::Hot,
    ];

    assert_eq!(states.len(), 4);
}

//
// Verify that a working set can be created as a bounded object separate from the
// full graph and without requiring any persistent storage backend.
//
// Given a valid working set ID,
// when a `GraphWorkingSet` is created,
// then it should start empty, keep its ID, and expose zero loaded-record stats.
#[test]
fn working_set_starts_empty_and_separate_from_the_full_graph() {
    let id = working_set_id("working-set--empty");
    let working_set = GraphWorkingSet::new(id.clone());

    assert_eq!(working_set.id(), &id);
    assert!(working_set.seed_node_ids().is_empty());
    assert!(working_set.hot_node_ids().is_empty());
    assert!(working_set.hot_relationship_ids().is_empty());
    assert!(working_set.pinned_node_ids().is_empty());
    assert!(working_set.pinned_relationship_ids().is_empty());
    assert!(working_set.dirty_node_ids().is_empty());
    assert!(working_set.dirty_relationship_ids().is_empty());
    assert_eq!(working_set.stats().hot_node_count(), 0);
    assert_eq!(working_set.stats().hot_relationship_count(), 0);
    assert_eq!(working_set.stats().warm_node_count(), 0);
    assert_eq!(working_set.stats().warm_relationship_count(), 0);
}

//
// Verify that seed nodes can be recorded without loading unrelated records or
// promoting the seed to hot by default.
//
// Given a new working set and a seed node ID,
// when the seed node is recorded,
// then only the seed collection should include it and no unrelated node should appear.
#[test]
fn seed_nodes_can_be_recorded_without_loading_unrelated_records() {
    let mut working_set = GraphWorkingSet::new(working_set_id("working-set--seeds"));
    let seed = node_id("node--seed");
    let unrelated = node_id("node--unrelated");

    working_set.record_seed_node(seed.clone());

    assert!(working_set.seed_node_ids().contains(&seed));
    assert!(!working_set.seed_node_ids().contains(&unrelated));
    assert!(!working_set.hot_node_ids().contains(&seed));
    assert_eq!(working_set.node_loading_state(&unrelated), None);
}

//
// Verify that hot node IDs and hot relationship IDs are tracked independently.
//
// Given a new working set,
// when a hot node and a hot relationship are recorded,
// then each ID should appear only in the appropriate hot collection.
#[test]
fn hot_nodes_and_relationships_are_tracked_independently() {
    let mut working_set = GraphWorkingSet::new(working_set_id("working-set--hot"));
    let node = node_id("node--hot");
    let relationship = relationship_id("relationship--hot");

    working_set.track_hot_node(node.clone());
    working_set.track_hot_relationship(relationship.clone());

    assert!(working_set.hot_node_ids().contains(&node));
    assert!(working_set.hot_relationship_ids().contains(&relationship));
    assert!(working_set.seed_node_ids().is_empty());
}

//
// Verify that pinned records can be represented separately for future eviction
// policies.
//
// Given a node and relationship in a working set,
// when both are pinned,
// then they should be exposed through the pinned collections.
#[test]
fn pinned_nodes_and_relationships_are_represented_explicitly() {
    let mut working_set = GraphWorkingSet::new(working_set_id("working-set--pinned"));
    let node = node_id("node--pinned");
    let relationship = relationship_id("relationship--pinned");

    working_set.pin_node(node.clone());
    working_set.pin_relationship(relationship.clone());

    assert!(working_set.pinned_node_ids().contains(&node));
    assert!(
        working_set
            .pinned_relationship_ids()
            .contains(&relationship)
    );
}

//
// Verify that dirty records are represented separately from clean loaded records.
//
// Given a new working set,
// when node and relationship records are marked dirty,
// then dirty collections should include them without requiring them to be hot first.
#[test]
fn dirty_records_are_tracked_separately_from_hot_records() {
    let mut working_set = GraphWorkingSet::new(working_set_id("working-set--dirty"));
    let node = node_id("node--dirty");
    let relationship = relationship_id("relationship--dirty");

    working_set.mark_dirty_node(node.clone());
    working_set.mark_dirty_relationship(relationship.clone());

    assert!(working_set.dirty_node_ids().contains(&node));
    assert!(working_set.dirty_relationship_ids().contains(&relationship));
    assert!(!working_set.hot_node_ids().contains(&node));
    assert!(!working_set.hot_relationship_ids().contains(&relationship));
}

//
// Verify that node and relationship loading states can be managed explicitly.
//
// Given a node and relationship in a working set,
// when explicit loading states are assigned,
// then the same states should be returned from the public API.
#[test]
fn loading_states_can_be_set_for_nodes_and_relationships() {
    let mut working_set = GraphWorkingSet::new(working_set_id("working-set--states"));
    let node = node_id("node--state");
    let relationship = relationship_id("relationship--state");

    working_set.set_node_loading_state(node.clone(), LoadingState::Warm);
    working_set.set_relationship_loading_state(relationship.clone(), LoadingState::Hot);

    assert_eq!(
        working_set.node_loading_state(&node),
        Some(LoadingState::Warm)
    );
    assert_eq!(
        working_set.relationship_loading_state(&relationship),
        Some(LoadingState::Hot)
    );
}

//
// Verify that tracking hot records updates the public placeholder stats used by
// later bounded-loading phases.
//
// Given a new working set,
// when one node and one relationship are tracked as hot,
// then hot counters should reflect those records while warm counters remain zero.
#[test]
fn hot_record_stats_reflect_tracked_hot_records() {
    let mut working_set = GraphWorkingSet::new(working_set_id("working-set--stats"));

    working_set.track_hot_node(node_id("node--stats-hot"));
    working_set.track_hot_relationship(relationship_id("relationship--stats-hot"));

    assert_eq!(working_set.stats().hot_node_count(), 1);
    assert_eq!(working_set.stats().hot_relationship_count(), 1);
    assert_eq!(working_set.stats().warm_node_count(), 0);
    assert_eq!(working_set.stats().warm_relationship_count(), 0);
}

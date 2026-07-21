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
    AdjacencyDirection, GraphError, GraphWorkingSet, LoadingState, NodeId, RelationshipId,
    RelationshipType, StorageRef, WarmAdjacencyEntry, WarmAdjacencyEntryInput,
    WarmAdjacencyRelevanceScore, WorkingSetId,
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

fn relevance_score(value: f64) -> WarmAdjacencyRelevanceScore {
    WarmAdjacencyRelevanceScore::new(value).expect("test relevance score should be valid")
}

fn node_storage_ref(value: &str) -> StorageRef {
    StorageRef::External {
        uri: format!("memory://graph/nodes/{value}"),
    }
}

fn relationship_storage_ref(value: &str) -> StorageRef {
    StorageRef::External {
        uri: format!("memory://graph/relationships/{value}"),
    }
}

fn warm_entry_input(
    source_node_id: NodeId,
    target_node_id: NodeId,
    target_loading_state: LoadingState,
) -> WarmAdjacencyEntryInput {
    WarmAdjacencyEntryInput::new(
        relationship_id("relationship--promotes-1"),
        relationship_type("PROMOTES"),
        source_node_id,
        target_node_id,
        vec!["Narrative".to_owned(), "Claim".to_owned()],
        AdjacencyDirection::Outgoing,
    )
    .with_relevance_score(relevance_score(0.82))
    .with_target_loading_state(target_loading_state)
    .with_storage_refs(
        Some(relationship_storage_ref("relationship--promotes-1")),
        Some(node_storage_ref("node--narrative-1")),
    )
}

fn warm_entry(
    source_node_id: NodeId,
    target_node_id: NodeId,
    target_loading_state: LoadingState,
) -> WarmAdjacencyEntry {
    WarmAdjacencyEntry::new(warm_entry_input(
        source_node_id,
        target_node_id,
        target_loading_state,
    ))
    .expect("warm adjacency entry should be valid")
}

//
// Verify that a warm adjacency entry can represent a neighboring node using only
// lightweight frontier metadata.
//
// Given relationship identity, relationship type, endpoint IDs, target labels,
// direction, relevance score, loading state, and storage references,
// when a warm adjacency entry is constructed,
// then all public accessors should expose that metadata without requiring a full
// node payload or full relationship payload.
#[test]
fn warm_adjacency_entry_exposes_lightweight_frontier_metadata() {
    let source_node_id = node_id("node--campaign-1");
    let target_node_id = node_id("node--narrative-1");
    let relationship_id = relationship_id("relationship--promotes-1");
    let relationship_type = relationship_type("PROMOTES");
    let target_labels = vec!["Narrative".to_owned(), "Claim".to_owned()];
    let relevance_score = relevance_score(0.82);
    let relationship_ref = relationship_storage_ref("relationship--promotes-1");
    let target_ref = node_storage_ref("node--narrative-1");

    let entry = WarmAdjacencyEntry::new(
        WarmAdjacencyEntryInput::new(
            relationship_id.clone(),
            relationship_type.clone(),
            source_node_id.clone(),
            target_node_id.clone(),
            target_labels.clone(),
            AdjacencyDirection::Outgoing,
        )
        .with_relevance_score(relevance_score)
        .with_target_loading_state(LoadingState::Indexed)
        .with_storage_refs(Some(relationship_ref.clone()), Some(target_ref.clone())),
    )
    .expect("warm adjacency entry should be constructible from lightweight metadata");

    assert_eq!(entry.relationship_id(), &relationship_id);
    assert_eq!(entry.relationship_type(), &relationship_type);
    assert_eq!(entry.source_node_id(), &source_node_id);
    assert_eq!(entry.target_node_id(), &target_node_id);
    assert_eq!(entry.target_labels(), &target_labels);
    assert_eq!(entry.direction(), AdjacencyDirection::Outgoing);
    assert_eq!(entry.relevance_score(), Some(relevance_score));
    assert_eq!(entry.target_loading_state(), LoadingState::Indexed);
    assert_eq!(entry.relationship_storage_ref(), Some(&relationship_ref));
    assert_eq!(entry.target_storage_ref(), Some(&target_ref));
}

//
// Verify that warm adjacency helpers distinguish loaded from unloaded target
// states without callers comparing raw enum values everywhere.
//
// Given warm adjacency entries for indexed and hot target nodes,
// when callers ask whether the target is loaded or unloaded,
// then indexed targets should require page-in and hot targets should be reported
// as already loaded.
#[test]
fn warm_adjacency_entry_distinguishes_loaded_and_unloaded_targets() {
    let unloaded_entry = warm_entry(
        node_id("node--campaign-1"),
        node_id("node--narrative-1"),
        LoadingState::Indexed,
    );
    let loaded_entry = warm_entry(
        node_id("node--campaign-1"),
        node_id("node--narrative-1"),
        LoadingState::Hot,
    );

    assert!(!unloaded_entry.is_target_loaded());
    assert!(unloaded_entry.is_target_unloaded());
    assert!(loaded_entry.is_target_loaded());
    assert!(!loaded_entry.is_target_unloaded());
}

//
// Verify that relevance scores have a bounded public contract before they are
// used for expansion ordering.
//
// Given representative valid and invalid score values,
// when scores are constructed,
// then finite values inside the accepted range should succeed and invalid values
// should be rejected through the typed graph error boundary.
#[test]
fn warm_adjacency_relevance_score_rejects_invalid_values() {
    assert_eq!(
        WarmAdjacencyRelevanceScore::new(0.0)
            .expect("zero should be a valid relevance score")
            .value(),
        0.0
    );
    assert_eq!(
        WarmAdjacencyRelevanceScore::new(1.0)
            .expect("one should be a valid relevance score")
            .value(),
        1.0
    );

    for invalid_value in [f64::NAN, f64::INFINITY, -0.01, 1.01] {
        assert!(matches!(
        WarmAdjacencyRelevanceScore::new(invalid_value),
        Err(GraphError::InvalidConfidence(value)) if value == invalid_value || invalid_value.is_nan() && value.is_nan()
        ));
    }
}

//
// Verify that a working set can hold a warm adjacency frontier without making
// the neighbor hot.
//
// Given a working set and a warm adjacency entry,
// when the entry is attached to the source/frontier node,
// then the working set should expose the warm frontier, mark the target and
// relationship as warm, and keep hot record counters unchanged.
#[test]
fn graph_working_set_attaches_warm_adjacency_without_hot_payloads() {
    let source_node_id = node_id("node--campaign-1");
    let target_node_id = node_id("node--narrative-1");
    let relationship_id = relationship_id("relationship--promotes-1");
    let entry = WarmAdjacencyEntry::new(
        WarmAdjacencyEntryInput::new(
            relationship_id.clone(),
            relationship_type("PROMOTES"),
            source_node_id.clone(),
            target_node_id.clone(),
            vec!["Narrative".to_owned()],
            AdjacencyDirection::Outgoing,
        )
        .with_relevance_score(relevance_score(0.7))
        .with_target_loading_state(LoadingState::Warm)
        .with_storage_refs(
            Some(relationship_storage_ref("relationship--promotes-1")),
            Some(node_storage_ref("node--narrative-1")),
        ),
    )
    .expect("warm adjacency entry should be valid");

    let mut working_set = GraphWorkingSet::new(working_set_id("working-set--issue-40"));
    working_set.record_seed_node(source_node_id.clone());

    working_set
        .attach_warm_adjacency(source_node_id.clone(), entry)
        .expect("warm adjacency should attach to its source node");

    let entries = working_set
        .warm_adjacency_for_source(&source_node_id)
        .expect("warm adjacency should be attached to the source node");

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].target_node_id(), &target_node_id);
    assert_eq!(entries[0].target_loading_state(), LoadingState::Warm);
    assert_eq!(
        working_set.node_loading_state(&target_node_id),
        Some(LoadingState::Warm)
    );
    assert_eq!(
        working_set.relationship_loading_state(&relationship_id),
        Some(LoadingState::Warm)
    );
    assert_eq!(working_set.stats().hot_node_count(), 0);
    assert_eq!(working_set.stats().hot_relationship_count(), 0);
    assert_eq!(working_set.stats().warm_node_count(), 1);
    assert_eq!(working_set.stats().warm_relationship_count(), 1);
}

//
// Verify that a warm adjacency entry cannot be attached under the wrong source
// node, because that would make the working-set frontier inconsistent.
//
// Given an entry whose source node is `node--campaign-1`,
// when callers try to attach it under `node--campaign-2`,
// then the working set should reject the inconsistent attachment.
#[test]
fn graph_working_set_rejects_warm_adjacency_source_mismatch() {
    let entry = warm_entry(
        node_id("node--campaign-1"),
        node_id("node--narrative-1"),
        LoadingState::Warm,
    );
    let mismatched_source_node_id = node_id("node--campaign-2");
    let mut working_set = GraphWorkingSet::new(working_set_id("working-set--issue-40"));

    assert!(
        working_set
            .attach_warm_adjacency(mismatched_source_node_id, entry)
            .is_err()
    );
}

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
    AdjacencyDirection, Graph, GraphError, GraphPager, GraphRecordRef, GraphWorkingSet,
    LoadingState, NodeId, NodeInput, RelationshipInput, RelationshipType, WarmAdjacencyEntry,
    WarmAdjacencyEntryInput, WarmAdjacencyRelevanceScore, WorkingSetId,
};

fn working_set_id(value: &str) -> WorkingSetId {
    WorkingSetId::new(value).expect("test working set ID should be valid")
}

fn relevance_score(value: f64) -> WarmAdjacencyRelevanceScore {
    WarmAdjacencyRelevanceScore::new(value).expect("test relevance score should be valid")
}

fn graph_with_campaign_to_narrative() -> (Graph, NodeId, NodeId) {
    let mut graph = Graph::new();
    let campaign_id = graph
        .create_node(NodeInput::new(["Campaign"]))
        .expect("campaign node should be created");
    let narrative_id = graph
        .create_node(NodeInput::new(["Narrative", "Claim"]))
        .expect("narrative node should be created");

    graph
        .create_relationship(
            RelationshipInput::new(campaign_id.clone(), "PROMOTES", narrative_id.clone())
                .expect("relationship input should be valid"),
        )
        .expect("relationship should be created");

    (graph, campaign_id, narrative_id)
}

#[test]
fn acceptance_warm_adjacency_can_be_attached_from_pager_metadata_without_hot_payloads() {
    let (graph, campaign_id, narrative_id) = graph_with_campaign_to_narrative();
    let adjacency = graph
        .load_outgoing_adjacency(&campaign_id)
        .expect("outgoing adjacency should load");
    let paged_entry = adjacency
        .entries
        .first()
        .expect("campaign should have one outgoing relationship");
    let target_metadata = graph
        .load_indexed_metadata(&GraphRecordRef::Node(narrative_id.clone()))
        .expect("target metadata should load without requiring the target payload");

    let warm_entry = WarmAdjacencyEntry::new(
        WarmAdjacencyEntryInput::new(
            paged_entry.relationship_id.clone(),
            paged_entry
                .relationship_type
                .clone()
                .expect("relationship type should be indexed in adjacency"),
            campaign_id.clone(),
            paged_entry.neighbor_node_id.clone(),
            target_metadata.labels.clone(),
            adjacency.direction,
        )
        .with_relevance_score(relevance_score(0.91))
        .with_target_loading_state(LoadingState::Warm)
        .with_storage_refs(
            paged_entry.relationship_storage_ref.clone(),
            paged_entry.neighbor_storage_ref.clone(),
        ),
    )
    .expect("warm adjacency should be constructible from pager metadata");

    let mut working_set = GraphWorkingSet::new(working_set_id("working-set--acceptance-40"));
    working_set.record_seed_node(campaign_id.clone());
    working_set
        .attach_warm_adjacency(campaign_id.clone(), warm_entry)
        .expect("warm adjacency should attach to the frontier source");

    let entries = working_set
        .warm_adjacency_for_source(&campaign_id)
        .expect("warm frontier should be available for the campaign");
    let entry = entries
        .first()
        .expect("warm frontier should contain the pager-derived edge");
    let expected_labels = vec!["Narrative".to_owned(), "Claim".to_owned()];

    assert_eq!(entry.source_node_id(), &campaign_id);
    assert_eq!(entry.target_node_id(), &narrative_id);
    assert_eq!(entry.relationship_type().as_str(), "PROMOTES");
    assert_eq!(entry.direction(), AdjacencyDirection::Outgoing);
    assert_eq!(entry.target_labels(), &expected_labels);
    assert_eq!(entry.target_loading_state(), LoadingState::Warm);
    assert!(entry.relationship_storage_ref().is_some());
    assert!(entry.target_storage_ref().is_some());

    assert!(working_set.hot_node_ids().is_empty());
    assert!(working_set.hot_relationship_ids().is_empty());
    assert_eq!(
        working_set.node_loading_state(&narrative_id),
        Some(LoadingState::Warm)
    );
    assert_eq!(working_set.stats().warm_node_count(), 1);
    assert_eq!(working_set.stats().warm_relationship_count(), 1);
}

#[test]
fn acceptance_invalid_warm_adjacency_state_is_rejected_or_impossible_to_construct() {
    assert!(matches!(
    NodeId::new(" "),
    Err(GraphError::InvalidIdentifier(kind)) if kind == "NodeId"
    ));
    assert!(matches!(
    RelationshipType::new(" "),
    Err(GraphError::InvalidRelationshipType(value)) if value.trim().is_empty()
    ));
    assert!(matches!(
    WarmAdjacencyRelevanceScore::new(f64::NAN),
    Err(GraphError::InvalidConfidence(value)) if value.is_nan()
    ));

    let source_node_id = NodeId::new("node--campaign-1").expect("source ID should be valid");
    let mismatched_source_node_id =
        NodeId::new("node--campaign-2").expect("mismatched source ID should be valid");
    let target_node_id = NodeId::new("node--narrative-1").expect("target ID should be valid");
    let relationship_id = graph_core::RelationshipId::new("relationship--promotes-1")
        .expect("relationship ID should be valid");
    let entry = WarmAdjacencyEntry::new(
        WarmAdjacencyEntryInput::new(
            relationship_id,
            RelationshipType::new("PROMOTES").expect("relationship type should be valid"),
            source_node_id,
            target_node_id,
            vec!["Narrative".to_owned()],
            AdjacencyDirection::Outgoing,
        )
        .with_relevance_score(relevance_score(0.5))
        .with_target_loading_state(LoadingState::Warm),
    )
    .expect("typed warm adjacency should be valid before mismatched attachment");

    let mut working_set = GraphWorkingSet::new(working_set_id("working-set--acceptance-40"));
    assert!(matches!(
    working_set.attach_warm_adjacency(mismatched_source_node_id, entry),
    Err(GraphError::InternalInvariantViolation(message))
    if message.contains("warm adjacency source mismatch")
    ));
}

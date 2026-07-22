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

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use graph_core::{
    AdjacencyDirection, ExpansionBudget, ExpansionDirection, ExpansionFilters, Graph, GraphPager,
    GraphPagerError, GraphPagerResult, GraphRecordMetadata, GraphRecordRef, LoadingState, NodeId,
    PagedAdjacency, PagedAdjacencyEntry, PagedNode, PagedRelationship, PagerBackedRuntime,
    PagerBackedRuntimeQuery, PropertyMap, RelationshipId, RelationshipType, StorageRef,
    WorkingSetHotBudget, WorkingSetId, default_generic_loading_profile,
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

#[derive(Clone, Default)]
struct PagerCallCounters {
    loaded_nodes: Arc<Mutex<Vec<NodeId>>>,
}

impl PagerCallCounters {
    fn record_node_load(&self, node_id: &NodeId) {
        self.loaded_nodes.lock().unwrap().push(node_id.clone());
    }

    fn loaded_nodes(&self) -> Vec<NodeId> {
        self.loaded_nodes.lock().unwrap().clone()
    }
}

#[derive(Clone)]
struct MockPager {
    outgoing: HashMap<NodeId, PagedAdjacency>,
    incoming: HashMap<NodeId, PagedAdjacency>,
    metadata: HashMap<GraphRecordRef, GraphRecordMetadata>,
    nodes: HashMap<NodeId, PagedNode>,
    relationships: HashMap<RelationshipId, PagedRelationship>,
    counters: PagerCallCounters,
}

impl GraphPager for MockPager {
    fn load_node_payload(&self, node_id: &NodeId) -> GraphPagerResult<PagedNode> {
        self.counters.record_node_load(node_id);
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
    ) -> GraphPagerResult<PagedRelationship> {
        self.relationships
            .get(relationship_id)
            .cloned()
            .ok_or_else(|| GraphPagerError::UnavailableRecord {
                record_ref: GraphRecordRef::Relationship(relationship_id.clone()),
            })
    }

    fn load_outgoing_adjacency(&self, node_id: &NodeId) -> GraphPagerResult<PagedAdjacency> {
        self.outgoing
            .get(node_id)
            .cloned()
            .ok_or_else(|| GraphPagerError::UnavailableRecord {
                record_ref: GraphRecordRef::Node(node_id.clone()),
            })
    }

    fn load_incoming_adjacency(&self, node_id: &NodeId) -> GraphPagerResult<PagedAdjacency> {
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
    ) -> GraphPagerResult<GraphRecordMetadata> {
        self.metadata
            .get(record_ref)
            .cloned()
            .ok_or_else(|| GraphPagerError::UnavailableRecord {
                record_ref: record_ref.clone(),
            })
    }
}

fn make_pager(
    storage_refs_enabled: bool,
) -> (MockPager, PagerCallCounters, NodeId, NodeId, NodeId, NodeId) {
    let seed = node_id("node--seed");
    let target_a = node_id("node--target-a");
    let target_b = node_id("node--target-b");
    let unrelated = node_id("node--unrelated");
    let rel_a = relationship_id("relationship--a");
    let rel_b = relationship_id("relationship--b");

    let mut graph = Graph::new();
    let seed_node = graph
        .create_node(graph_core::NodeInput::new(["Seed"]))
        .unwrap();
    let target_a_node = graph
        .create_node(graph_core::NodeInput::new(["Target"]))
        .unwrap();
    let target_b_node = graph
        .create_node(graph_core::NodeInput::new(["Target"]))
        .unwrap();
    let unrelated_node = graph
        .create_node(graph_core::NodeInput::new(["Unrelated"]))
        .unwrap();
    let _ = graph.create_relationship(
        graph_core::RelationshipInput::new(seed_node.clone(), "REL", target_a_node.clone())
            .unwrap(),
    );
    let _ = graph.create_relationship(
        graph_core::RelationshipInput::new(seed_node.clone(), "REL", target_b_node.clone())
            .unwrap(),
    );

    let counters = PagerCallCounters::default();
    let storage_ref = || {
        if storage_refs_enabled {
            Some(StorageRef::Record {
                collection: "nodes".to_owned(),
                key: "record".to_owned(),
            })
        } else {
            None
        }
    };

    let mut metadata = HashMap::new();
    for node in [&seed, &target_a, &target_b, &unrelated] {
        metadata.insert(
            GraphRecordRef::Node(node.clone()),
            GraphRecordMetadata {
                record_ref: GraphRecordRef::Node(node.clone()),
                storage_ref: storage_ref(),
                loading_state: LoadingState::Indexed,
                labels: vec!["Node".to_owned()],
                relationship_type: None,
                indexed_properties: PropertyMap::new(),
            },
        );
    }
    for (relationship, storage_key) in [(&rel_a, "relationship--a"), (&rel_b, "relationship--b")] {
        metadata.insert(
            GraphRecordRef::Relationship(relationship.clone()),
            GraphRecordMetadata {
                record_ref: GraphRecordRef::Relationship(relationship.clone()),
                storage_ref: if storage_refs_enabled {
                    Some(StorageRef::Record {
                        collection: "relationships".to_owned(),
                        key: storage_key.to_owned(),
                    })
                } else {
                    None
                },
                loading_state: LoadingState::Indexed,
                labels: Vec::new(),
                relationship_type: Some(relationship_type("REL")),
                indexed_properties: PropertyMap::new(),
            },
        );
    }

    let mut outgoing = HashMap::new();
    outgoing.insert(
        seed.clone(),
        PagedAdjacency {
            owner_node_id: seed.clone(),
            direction: AdjacencyDirection::Outgoing,
            entries: vec![
                PagedAdjacencyEntry {
                    relationship_id: rel_a.clone(),
                    neighbor_node_id: target_a.clone(),
                    relationship_type: Some(relationship_type("REL")),
                    relationship_storage_ref: storage_ref(),
                    neighbor_storage_ref: storage_ref(),
                },
                PagedAdjacencyEntry {
                    relationship_id: rel_b.clone(),
                    neighbor_node_id: target_b.clone(),
                    relationship_type: Some(relationship_type("REL")),
                    relationship_storage_ref: storage_ref(),
                    neighbor_storage_ref: storage_ref(),
                },
            ],
            storage_ref: storage_ref(),
        },
    );
    outgoing.insert(
        target_a.clone(),
        PagedAdjacency {
            owner_node_id: target_a.clone(),
            direction: AdjacencyDirection::Outgoing,
            entries: Vec::new(),
            storage_ref: storage_ref(),
        },
    );
    outgoing.insert(
        target_b.clone(),
        PagedAdjacency {
            owner_node_id: target_b.clone(),
            direction: AdjacencyDirection::Outgoing,
            entries: Vec::new(),
            storage_ref: storage_ref(),
        },
    );

    let incoming = HashMap::new();
    let nodes = HashMap::from([
        (
            seed.clone(),
            PagedNode {
                node: graph.get_node(&seed_node).unwrap().unwrap(),
                storage_ref: storage_ref(),
            },
        ),
        (
            target_a.clone(),
            PagedNode {
                node: graph.get_node(&target_a_node).unwrap().unwrap(),
                storage_ref: storage_ref(),
            },
        ),
        (
            target_b.clone(),
            PagedNode {
                node: graph.get_node(&target_b_node).unwrap().unwrap(),
                storage_ref: storage_ref(),
            },
        ),
        (
            unrelated.clone(),
            PagedNode {
                node: graph.get_node(&unrelated_node).unwrap().unwrap(),
                storage_ref: storage_ref(),
            },
        ),
    ]);
    let relationships = HashMap::from([
        (
            rel_a.clone(),
            PagedRelationship {
                relationship: graph
                    .create_relationship(
                        graph_core::RelationshipInput::new(
                            seed_node.clone(),
                            "REL",
                            target_a_node.clone(),
                        )
                        .unwrap(),
                    )
                    .and_then(|id| graph.get_relationship(&id))
                    .unwrap()
                    .unwrap(),
                storage_ref: storage_ref(),
            },
        ),
        (
            rel_b.clone(),
            PagedRelationship {
                relationship: graph
                    .create_relationship(
                        graph_core::RelationshipInput::new(seed_node, "REL", target_b_node.clone())
                            .unwrap(),
                    )
                    .and_then(|id| graph.get_relationship(&id))
                    .unwrap()
                    .unwrap(),
                storage_ref: storage_ref(),
            },
        ),
    ]);

    (
        MockPager {
            outgoing,
            incoming,
            metadata,
            nodes,
            relationships,
            counters: counters.clone(),
        },
        counters,
        seed,
        target_a,
        target_b,
        unrelated,
    )
}

fn generous_budget() -> ExpansionBudget {
    ExpansionBudget {
        max_loaded_node_count: 100,
        max_loaded_relationship_count: 100,
        max_hot_node_count: 100,
        max_hot_relationship_count: 100,
        max_warm_adjacency_entry_count: 100,
        max_hop_count: 3,
        max_supernode_expansion_count: 100,
        max_payload_byte_count: 1_048_576,
        max_execution_time_ms: 10_000,
    }
}

#[test]
fn pager_backed_runtime_applies_deterministic_budgeted_promotions_and_evictions() {
    let (pager, _, seed, _, _, _) = make_pager(false);
    let mut runtime = PagerBackedRuntime::new(WorkingSetHotBudget {
        max_hot_node_count: 2,
        max_hot_relationship_count: 1,
    });
    let query = PagerBackedRuntimeQuery::new(
        WorkingSetId::new("working-set--runtime-budget").unwrap(),
        vec![seed.clone()],
        ExpansionDirection::Outgoing,
        ExpansionFilters::empty(),
        1,
        default_generic_loading_profile(),
        generous_budget(),
    );

    let result = runtime.execute_query(&pager, query).unwrap();
    assert!(result.stats.hot_node_count() <= 2);
    assert!(result.stats.hot_relationship_count() <= 1);
    assert_eq!(
        result.eviction.evicted_hot_node_ids,
        vec![node_id("node--target-b")]
    );
    assert_eq!(
        result.eviction.evicted_hot_relationship_ids,
        vec![relationship_id("relationship--b")]
    );
}

#[test]
fn pager_backed_runtime_keeps_targeted_traversal_bounded_without_full_hydration() {
    let (pager, counters, seed, _, _, unrelated) = make_pager(false);
    let mut runtime = PagerBackedRuntime::new(WorkingSetHotBudget {
        max_hot_node_count: 4,
        max_hot_relationship_count: 4,
    });
    let query = PagerBackedRuntimeQuery::new(
        WorkingSetId::new("working-set--runtime-targeted").unwrap(),
        vec![seed],
        ExpansionDirection::Outgoing,
        ExpansionFilters::empty(),
        1,
        default_generic_loading_profile(),
        generous_budget(),
    );

    let _ = runtime.execute_query(&pager, query).unwrap();
    let loaded_nodes = counters.loaded_nodes();
    assert!(
        !loaded_nodes.contains(&unrelated),
        "targeted traversal should not hydrate unrelated nodes"
    );
}

#[test]
fn pager_backed_runtime_preserves_equivalent_results_for_ephemeral_and_persistent_refs() {
    let (ephemeral_pager, _, seed, _, _, _) = make_pager(false);
    let (persistent_pager, _, _, _, _, _) = make_pager(true);
    let query = PagerBackedRuntimeQuery::new(
        WorkingSetId::new("working-set--runtime-equivalence").unwrap(),
        vec![seed],
        ExpansionDirection::Outgoing,
        ExpansionFilters::empty(),
        1,
        default_generic_loading_profile(),
        generous_budget(),
    );

    let mut ephemeral_runtime = PagerBackedRuntime::new(WorkingSetHotBudget {
        max_hot_node_count: 2,
        max_hot_relationship_count: 1,
    });
    let mut persistent_runtime = PagerBackedRuntime::new(WorkingSetHotBudget {
        max_hot_node_count: 2,
        max_hot_relationship_count: 1,
    });

    let ephemeral = ephemeral_runtime
        .execute_query(&ephemeral_pager, query.clone())
        .unwrap();
    let persistent = persistent_runtime
        .execute_query(&persistent_pager, query)
        .unwrap();

    assert_eq!(ephemeral.expansion.status(), persistent.expansion.status());
    assert_eq!(ephemeral.stats, persistent.stats);
    assert_eq!(ephemeral.eviction, persistent.eviction);
}

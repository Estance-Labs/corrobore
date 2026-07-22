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
    ExpansionBudget, ExpansionDirection, ExpansionFilters, ExpansionLimit, ExpansionRequest,
    ExpansionResultStatus, Graph, GraphWorkingSetCreateRequest, GraphWorkingSetManager,
    LoadingState, NodeId, NodeInput, RelationshipId, RelationshipInput, RelationshipType,
    SkippedExpansionReason, WorkingSetId, default_fimi_investigation_profile,
    expand_working_set_from_graph_adjacency,
};

fn working_set_id(value: &str) -> WorkingSetId {
    WorkingSetId::new(value).expect("acceptance working set ID should be valid")
}

fn rel_type(value: &str) -> RelationshipType {
    RelationshipType::new(value).expect("acceptance relationship type should be valid")
}

fn generous_budget() -> ExpansionBudget {
    ExpansionBudget {
        max_loaded_node_count: 32,
        max_loaded_relationship_count: 32,
        max_hot_node_count: 32,
        max_hot_relationship_count: 32,
        max_warm_adjacency_entry_count: 32,
        max_hop_count: 3,
        max_supernode_expansion_count: 8,
        max_payload_byte_count: 1_048_576,
        max_execution_time_ms: 1_000,
    }
}

fn manager_with_working_set(id: &WorkingSetId) -> GraphWorkingSetManager {
    let mut manager = GraphWorkingSetManager::new();
    manager
        .create_working_set(GraphWorkingSetCreateRequest::new(id.clone()))
        .expect("acceptance working set should be created");
    manager
}

fn request(
    working_set_id: &WorkingSetId,
    seed_node_ids: Vec<NodeId>,
    direction: ExpansionDirection,
    filters: ExpansionFilters,
    hop_limit: u64,
    budget: ExpansionBudget,
) -> ExpansionRequest {
    ExpansionRequest::new(
        working_set_id.clone(),
        seed_node_ids,
        direction,
        filters,
        hop_limit,
        default_fimi_investigation_profile(),
        budget,
    )
}

fn create_node(graph: &mut Graph, labels: &[&str]) -> NodeId {
    graph
        .create_node(NodeInput::new(labels.iter().copied()))
        .expect("acceptance node should be created")
}

fn create_relationship(
    graph: &mut Graph,
    source: &NodeId,
    relationship_type: &str,
    target: &NodeId,
) -> RelationshipId {
    graph
        .create_relationship(
            RelationshipInput::new(source.clone(), relationship_type, target.clone())
                .expect("acceptance relationship input should be valid"),
        )
        .expect("acceptance relationship should be created")
}

//
// Validate the issue-43 happy path at the public crate boundary, not only inside
// the unit-test module that owns private helpers.
//
// Given a graph containing a FIMI campaign -> narrative -> claim path and one
// unrelated subgraph,
// when a one-hop outgoing expansion starts from the campaign with relationship and
// label filters,
// then only the bounded campaign/narrative subgraph should become hot, the next
// ring should remain warm, and unrelated records should remain outside the working set.
#[test]
fn acceptance_expands_filtered_one_hop_without_loading_unrelated_graph_area() {
    let mut graph = Graph::new();
    let campaign = create_node(&mut graph, &["Campaign"]);
    let narrative = create_node(&mut graph, &["Narrative"]);
    let claim = create_node(&mut graph, &["Claim"]);
    let unrelated_campaign = create_node(&mut graph, &["Campaign"]);
    let unrelated_narrative = create_node(&mut graph, &["Narrative"]);
    let promotes = create_relationship(&mut graph, &campaign, "PROMOTES", &narrative);
    let makes_claim = create_relationship(&mut graph, &narrative, "MAKES_CLAIM", &claim);
    let unrelated_promotes = create_relationship(
        &mut graph,
        &unrelated_campaign,
        "PROMOTES",
        &unrelated_narrative,
    );
    let working_set_id = working_set_id("working-set--acceptance-one-hop");
    let mut manager = manager_with_working_set(&working_set_id);

    let result = expand_working_set_from_graph_adjacency(
        &mut manager,
        &graph,
        request(
            &working_set_id,
            vec![campaign.clone()],
            ExpansionDirection::Outgoing,
            ExpansionFilters::new(vec![rel_type("PROMOTES")], vec!["Narrative".to_owned()]),
            1,
            generous_budget(),
        ),
    )
    .expect("filtered one-hop expansion should complete");

    assert_eq!(result.status(), ExpansionResultStatus::Complete);
    assert!(result.budget_error().is_none());
    assert_eq!(result.usage().hot_node_count, 2);
    assert_eq!(result.usage().hot_relationship_count, 1);

    let working_set = manager
        .get_working_set(&working_set_id)
        .expect("working set should remain available after expansion");
    assert!(working_set.hot_node_ids().contains(&campaign));
    assert!(working_set.hot_node_ids().contains(&narrative));
    assert!(!working_set.hot_node_ids().contains(&claim));
    assert!(!working_set.hot_node_ids().contains(&unrelated_campaign));
    assert!(!working_set.hot_node_ids().contains(&unrelated_narrative));
    assert!(working_set.hot_relationship_ids().contains(&promotes));
    assert!(!working_set.hot_relationship_ids().contains(&makes_claim));
    assert!(
        !working_set
            .hot_relationship_ids()
            .contains(&unrelated_promotes)
    );

    let warm_entries = working_set
        .warm_adjacency_for_source(&narrative)
        .expect("the next ring should be retained as warm adjacency");
    assert_eq!(warm_entries.len(), 1);
    assert_eq!(warm_entries[0].relationship_id(), &makes_claim);
    assert_eq!(warm_entries[0].target_node_id(), &claim);
    assert_eq!(warm_entries[0].target_loading_state(), LoadingState::Warm);
}

//
// Validate that budget exhaustion remains useful to callers instead of collapsing
// into an opaque error or over-loading records.
//
// Given a seed with one outgoing relationship and a relationship budget of zero,
// when expansion evaluates the first adjacency candidate,
// then the returned result should be partial, carry a typed relationship budget
// error with a fix hint, and preserve the hot seed as usable partial output.
#[test]
fn acceptance_returns_partial_output_with_typed_budget_error() {
    let mut graph = Graph::new();
    let campaign = create_node(&mut graph, &["Campaign"]);
    let narrative = create_node(&mut graph, &["Narrative"]);
    let promotes = create_relationship(&mut graph, &campaign, "PROMOTES", &narrative);
    let working_set_id = working_set_id("working-set--acceptance-budget");
    let mut manager = manager_with_working_set(&working_set_id);
    let budget = ExpansionBudget {
        max_loaded_relationship_count: 0,
        max_hot_relationship_count: 0,
        ..generous_budget()
    };

    let result = expand_working_set_from_graph_adjacency(
        &mut manager,
        &graph,
        request(
            &working_set_id,
            vec![campaign.clone()],
            ExpansionDirection::Outgoing,
            ExpansionFilters::empty(),
            1,
            budget,
        ),
    )
    .expect("budget stop should return a partial result");

    assert_eq!(result.status(), ExpansionResultStatus::Partial);
    let budget_error = result
        .budget_error()
        .expect("partial result should carry the exhausted budget");
    assert_eq!(budget_error.limit, ExpansionLimit::LoadedRelationshipCount);
    assert_eq!(budget_error.allowed, 0);
    assert!(budget_error.consumed > budget_error.allowed);
    assert!(budget_error.fix_hint.contains("filter") || budget_error.fix_hint.contains("LIMIT"));

    let working_set = manager
        .get_working_set(&working_set_id)
        .expect("working set should remain available after partial expansion");
    assert!(working_set.hot_node_ids().contains(&campaign));
    assert!(!working_set.hot_node_ids().contains(&narrative));
    assert!(!working_set.hot_relationship_ids().contains(&promotes));
    assert!(
        result
            .explanation()
            .skipped_expansions()
            .iter()
            .any(
                |skipped| skipped.relationship_id.as_ref() == Some(&promotes)
                    && skipped.reason == SkippedExpansionReason::BudgetLimit
            )
    );
}

//
// Validate incoming traversal through the same public expansion operation used for
// outgoing traversal.
//
// Given a narrative that makes a claim,
// when expansion starts from the claim and requests incoming adjacency,
// then the narrative and the incoming relationship should become hot while the
// result explains the loaded relationship endpoints in source -> target order.
#[test]
fn acceptance_expands_incoming_adjacency_from_seed() {
    let mut graph = Graph::new();
    let narrative = create_node(&mut graph, &["Narrative"]);
    let claim = create_node(&mut graph, &["Claim"]);
    let makes_claim = create_relationship(&mut graph, &narrative, "MAKES_CLAIM", &claim);
    let working_set_id = working_set_id("working-set--acceptance-incoming");
    let mut manager = manager_with_working_set(&working_set_id);

    let result = expand_working_set_from_graph_adjacency(
        &mut manager,
        &graph,
        request(
            &working_set_id,
            vec![claim.clone()],
            ExpansionDirection::Incoming,
            ExpansionFilters::new(vec![rel_type("MAKES_CLAIM")], vec!["Narrative".to_owned()]),
            1,
            generous_budget(),
        ),
    )
    .expect("incoming one-hop expansion should complete");

    assert_eq!(result.status(), ExpansionResultStatus::Complete);
    let working_set = manager
        .get_working_set(&working_set_id)
        .expect("working set should remain available after incoming expansion");
    assert!(working_set.hot_node_ids().contains(&claim));
    assert!(working_set.hot_node_ids().contains(&narrative));
    assert!(working_set.hot_relationship_ids().contains(&makes_claim));
    assert!(
        result
            .explanation()
            .hot_relationships()
            .iter()
            .any(|entry| {
                entry.relationship_id == makes_claim
                    && entry.source_node_id == narrative
                    && entry.target_node_id == claim
            })
    );
}

//
// Validate the explicit zero-hop boundary so seed loading cannot accidentally scan
// or promote adjacent graph records.
//
// Given a seed with outgoing adjacency,
// when the request uses hop limit zero,
// then only the seed should be hot, usage should report zero hops, and no hot
// relationship should be loaded.
#[test]
fn acceptance_zero_hop_loads_only_seed_records() {
    let mut graph = Graph::new();
    let campaign = create_node(&mut graph, &["Campaign"]);
    let narrative = create_node(&mut graph, &["Narrative"]);
    let promotes = create_relationship(&mut graph, &campaign, "PROMOTES", &narrative);
    let working_set_id = working_set_id("working-set--acceptance-zero-hop");
    let mut manager = manager_with_working_set(&working_set_id);
    let budget = ExpansionBudget {
        max_hop_count: 0,
        ..generous_budget()
    };

    let result = expand_working_set_from_graph_adjacency(
        &mut manager,
        &graph,
        request(
            &working_set_id,
            vec![campaign.clone()],
            ExpansionDirection::Outgoing,
            ExpansionFilters::empty(),
            0,
            budget,
        ),
    )
    .expect("zero-hop expansion should complete");

    assert_eq!(result.status(), ExpansionResultStatus::Complete);
    assert_eq!(result.usage().hop_count, 0);
    assert_eq!(result.usage().hot_node_count, 1);
    assert_eq!(result.usage().hot_relationship_count, 0);

    let working_set = manager
        .get_working_set(&working_set_id)
        .expect("working set should remain available after zero-hop expansion");
    assert!(working_set.hot_node_ids().contains(&campaign));
    assert!(!working_set.hot_node_ids().contains(&narrative));
    assert!(!working_set.hot_relationship_ids().contains(&promotes));
}

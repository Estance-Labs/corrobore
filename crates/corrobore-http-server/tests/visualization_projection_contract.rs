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
use std::collections::BTreeMap;

use corrobore_http_server::visualization::{
    MAX_VISUALIZATION_COMPUTATION_UNITS, MAX_VISUALIZATION_NODES, MAX_VISUALIZATION_PAYLOAD_BYTES,
    MAX_VISUALIZATION_PROPERTIES_PER_RECORD, MAX_VISUALIZATION_RELATIONSHIPS,
    VisualizationAntiPheromoneVector, VisualizationNavigationField, VisualizationPheromoneVector,
    VisualizationProjectionBudget, VisualizationProjectionError, VisualizationProjectionRequest,
    VisualizationTemporalBoundary, project_resolved_graph,
};
use graph_core::{Graph, NodeInput, PropertyValue, RelationshipInput};

fn projection_budget(
    max_nodes: usize,
    max_relationships: usize,
    max_properties_per_record: usize,
    max_payload_bytes: usize,
    max_computation_units: usize,
) -> VisualizationProjectionBudget {
    VisualizationProjectionBudget::new(
        max_nodes,
        max_relationships,
        max_properties_per_record,
        max_payload_bytes,
        max_computation_units,
    )
    .expect("test projection budget should be valid")
}

fn current_request(budget: VisualizationProjectionBudget) -> VisualizationProjectionRequest {
    VisualizationProjectionRequest::new(VisualizationTemporalBoundary::current(), budget)
}

fn connected_graph(reverse_property_order: bool) -> Graph {
    let mut graph = Graph::new();
    let mut first = NodeInput::new(["Entity"]);
    if reverse_property_order {
        first = first
            .with_property("zeta", PropertyValue::Integer(7))
            .with_property("alpha", PropertyValue::String("first".to_owned()));
    } else {
        first = first
            .with_property("alpha", PropertyValue::String("first".to_owned()))
            .with_property("zeta", PropertyValue::Integer(7));
    }
    let first = graph
        .create_node(first)
        .expect("first node should be created");
    let second = graph
        .create_node(
            NodeInput::new(["Observation"])
                .with_property("name", PropertyValue::String("second".to_owned())),
        )
        .expect("second node should be created");

    graph
        .create_relationship(
            RelationshipInput::new(first, "OBSERVED", second)
                .expect("relationship input should be valid")
                .with_property("weight", PropertyValue::Float(0.8)),
        )
        .expect("relationship should be created");

    graph
}

#[test]
fn projection_budget_rejects_zero_and_hard_limit_overages_with_fix_hints() {
    let zero = VisualizationProjectionBudget::new(0, 1, 1, 1024, 1)
        .expect_err("a zero node budget should be rejected");
    assert!(matches!(
        zero,
        VisualizationProjectionError::InvalidBudget {
            ref field,
            requested: 0,
            minimum: 1,
            ref fix_hint,
            ..
        } if field == "max_nodes" && fix_hint.contains("between")
    ));

    let excessive = VisualizationProjectionBudget::new(MAX_VISUALIZATION_NODES + 1, 1, 1, 1024, 1)
        .expect_err("a budget above the server hard limit should be rejected");
    assert!(matches!(
        excessive,
        VisualizationProjectionError::InvalidBudget {
            ref field,
            requested,
            maximum,
            ..
        } if field == "max_nodes"
            && requested == MAX_VISUALIZATION_NODES + 1
            && maximum == MAX_VISUALIZATION_NODES
    ));

    for (field, result) in [
        (
            "max_relationships",
            VisualizationProjectionBudget::new(1, MAX_VISUALIZATION_RELATIONSHIPS + 1, 1, 1024, 1),
        ),
        (
            "max_properties_per_record",
            VisualizationProjectionBudget::new(
                1,
                1,
                MAX_VISUALIZATION_PROPERTIES_PER_RECORD + 1,
                1024,
                1,
            ),
        ),
        (
            "max_payload_bytes",
            VisualizationProjectionBudget::new(1, 1, 1, MAX_VISUALIZATION_PAYLOAD_BYTES + 1, 1),
        ),
        (
            "max_computation_units",
            VisualizationProjectionBudget::new(
                1,
                1,
                1,
                1024,
                MAX_VISUALIZATION_COMPUTATION_UNITS + 1,
            ),
        ),
    ] {
        assert!(matches!(
            result,
            Err(VisualizationProjectionError::InvalidBudget {
                field: invalid_field,
                ..
            }) if invalid_field == field
        ));
    }

    let deserialized_invalid: VisualizationProjectionBudget =
        serde_json::from_value(serde_json::json!({
            "max_nodes": 0,
            "max_relationships": 1,
            "max_properties_per_record": 1,
            "max_payload_bytes": 1024,
            "max_computation_units": 1
        }))
        .expect("the wire shape should deserialize before execution validation");
    let result = project_resolved_graph(
        &Graph::new(),
        &current_request(deserialized_invalid),
        &BTreeMap::new(),
    );
    assert!(matches!(
        result,
        Err(VisualizationProjectionError::InvalidBudget {
            ref field,
            requested: 0,
            ..
        }) if field == "max_nodes"
    ));
}

#[test]
fn temporal_boundaries_preserve_resolved_current_snapshot_and_timeshot_identity() {
    let current = VisualizationTemporalBoundary::current();
    let snapshot = VisualizationTemporalBoundary::snapshot(
        "snapshot--baseline",
        "transaction--42",
        "2026-07-17T00:00:00Z",
    )
    .expect("snapshot boundary should be valid");
    let timeshot = VisualizationTemporalBoundary::timeshot(
        "timeshot--analysis-43",
        Some("transaction--43"),
        "2026-07-17T00:01:00Z",
    )
    .expect("timeshot boundary should be valid");

    assert_eq!(current.kind(), "current");
    assert_eq!(snapshot.kind(), "snapshot");
    assert_eq!(snapshot.boundary_id(), Some("snapshot--baseline"));
    assert_eq!(snapshot.transaction_id(), Some("transaction--42"));
    assert_eq!(timeshot.kind(), "timeshot");
    assert_eq!(timeshot.boundary_id(), Some("timeshot--analysis-43"));
    assert_eq!(timeshot.transaction_id(), Some("transaction--43"));
    assert_eq!(timeshot.at(), Some("2026-07-17T00:01:00Z"));
}

#[test]
fn temporal_boundaries_reject_empty_identity_and_malformed_timestamps() {
    let empty =
        VisualizationTemporalBoundary::snapshot("", "transaction--1", "2026-07-17T00:00:00Z")
            .expect_err("an empty boundary id should be rejected");
    assert!(matches!(
        empty,
        VisualizationProjectionError::InvalidTemporalBoundary { ref field, .. }
            if field == "boundary_id"
    ));

    let malformed =
        VisualizationTemporalBoundary::timeshot("timeshot--1", None::<String>, "not-a-timestamp")
            .expect_err("a malformed timestamp should be rejected");
    assert!(matches!(
        malformed,
        VisualizationProjectionError::InvalidTemporalBoundary {
            ref field,
            ref fix_hint,
        } if field == "at" && fix_hint.contains("RFC 3339")
    ));
}

#[test]
fn equivalent_graphs_with_different_property_insertion_order_serialize_identically() {
    let budget = projection_budget(10, 10, 10, 16_384, 100);
    let request = current_request(budget);

    let first = project_resolved_graph(&connected_graph(false), &request, &BTreeMap::new())
        .expect("first projection should succeed");
    let second = project_resolved_graph(&connected_graph(true), &request, &BTreeMap::new())
        .expect("second projection should succeed");

    assert_eq!(
        serde_json::to_vec(&first).expect("first response should serialize"),
        serde_json::to_vec(&second).expect("second response should serialize")
    );
}

#[test]
fn truncation_reports_omissions_and_never_returns_dangling_relationships() {
    let mut graph = connected_graph(false);
    graph
        .create_node(NodeInput::new(["Source"]))
        .expect("third node should be created");

    let response = project_resolved_graph(
        &graph,
        &current_request(projection_budget(1, 10, 10, 16_384, 100)),
        &BTreeMap::new(),
    )
    .expect("bounded projection should succeed");

    assert_eq!(response.nodes.len(), 1);
    assert!(response.relationships.is_empty());
    assert!(response.metadata.partial);
    assert_eq!(response.metadata.returned_nodes, 1);
    assert_eq!(response.metadata.omitted_nodes, 2);
    assert_eq!(response.metadata.returned_relationships, 0);
    assert_eq!(response.metadata.omitted_relationships, 1);
    assert_eq!(response.metadata.requested_budget.max_nodes, 1);
    assert_eq!(
        response.metadata.applied_budget,
        response.metadata.requested_budget
    );

    let returned_ids: Vec<&str> = response.nodes.iter().map(|node| node.id.as_str()).collect();
    for relationship in &response.relationships {
        assert!(returned_ids.contains(&relationship.source.as_str()));
        assert!(returned_ids.contains(&relationship.target.as_str()));
    }
}

#[test]
fn computation_budget_is_applied_deterministically_and_reported() {
    let mut graph = Graph::new();
    graph
        .create_node(NodeInput::new(["Entity"]))
        .expect("first node should be created");
    graph
        .create_node(NodeInput::new(["Event"]))
        .expect("second node should be created");

    let response = project_resolved_graph(
        &graph,
        &current_request(projection_budget(10, 10, 10, 16_384, 1)),
        &BTreeMap::new(),
    )
    .expect("computation-bounded projection should succeed");

    assert_eq!(response.nodes.len(), 1);
    assert_eq!(response.metadata.computation_units, 1);
    assert_eq!(response.metadata.omitted_nodes, 1);
    assert!(response.metadata.partial);
}

#[test]
fn property_and_payload_limits_produce_a_serializable_partial_response_within_budget() {
    let mut graph = Graph::new();
    graph
        .create_node(
            NodeInput::new(["Entity"])
                .with_property("a", PropertyValue::String("a".repeat(2_048)))
                .with_property("b", PropertyValue::String("b".repeat(2_048)))
                .with_property("c", PropertyValue::String("c".repeat(2_048))),
        )
        .expect("large node should be created");
    let budget = projection_budget(10, 10, 2, 2_048, 100);

    let response =
        project_resolved_graph(&graph, &current_request(budget.clone()), &BTreeMap::new())
            .expect("payload-bounded projection should succeed");
    let serialized = serde_json::to_vec(&response).expect("response should serialize");

    assert!(serialized.len() <= budget.max_payload_bytes);
    assert!(response.metadata.partial);
    assert!(response.metadata.omitted_properties >= 1);
}

#[test]
fn navigation_fields_round_trip_without_exposing_graph_core_field_types() {
    let graph = connected_graph(false);
    let mut navigation = BTreeMap::new();
    navigation.insert(
        "relationship--1".to_owned(),
        VisualizationNavigationField {
            scope: "fimi_investigation".to_owned(),
            tick: 7,
            positive: Some(VisualizationPheromoneVector {
                access_frequency: 4.0,
                downstream_success: 0.75,
                ..VisualizationPheromoneVector::default()
            }),
            negative: Some(VisualizationAntiPheromoneVector {
                dead_end: 1.0,
                contradictory_path: 0.5,
                ..VisualizationAntiPheromoneVector::default()
            }),
        },
    );

    let response = project_resolved_graph(
        &graph,
        &current_request(projection_budget(10, 10, 10, 16_384, 100)),
        &navigation,
    )
    .expect("projection with navigation data should succeed");
    let encoded = serde_json::to_vec(&response).expect("response should serialize");
    let decoded = serde_json::from_slice(&encoded).expect("response should deserialize");

    assert_eq!(response, decoded);
    let field = response.relationships[0]
        .navigation
        .as_ref()
        .expect("relationship should carry navigation data");
    assert_eq!(field.scope, "fimi_investigation");
    assert_eq!(field.tick, 7);
    assert_eq!(
        field.positive.as_ref().map(|value| value.access_frequency),
        Some(4.0)
    );
    assert_eq!(
        field
            .negative
            .as_ref()
            .map(|value| value.contradictory_path),
        Some(0.5)
    );
}

#[test]
fn empty_and_disconnected_graphs_project_deterministically() {
    let request = current_request(projection_budget(10, 10, 10, 16_384, 100));
    let empty = project_resolved_graph(&Graph::new(), &request, &BTreeMap::new())
        .expect("empty graph should project");
    assert!(empty.nodes.is_empty());
    assert!(empty.relationships.is_empty());
    assert!(!empty.metadata.partial);

    let mut disconnected = Graph::new();
    disconnected
        .create_node(NodeInput::new(["Entity"]))
        .expect("first disconnected node should be created");
    disconnected
        .create_node(NodeInput::new(["Event"]))
        .expect("second disconnected node should be created");
    let first = project_resolved_graph(&disconnected, &request, &BTreeMap::new())
        .expect("first disconnected projection should succeed");
    let second = project_resolved_graph(&disconnected, &request, &BTreeMap::new())
        .expect("second disconnected projection should succeed");

    assert_eq!(first, second);
    assert_eq!(first.nodes.len(), 2);
    assert!(first.relationships.is_empty());
}

// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

#![allow(clippy::unwrap_used)]

use cypher_executor::{CypherPipelineExecutor, ExecutionPolicy, ExecutionResultData};
use graph_core::{Graph, NodeInput, PropertyValue};

fn graph() -> Graph {
    let mut graph = Graph::new();
    for (name, score, tags) in [
        ("alpha", 10, vec!["c2"]),
        ("beta", 10, vec!["c2", "malware"]),
        ("gamma", 5, vec!["archive"]),
    ] {
        graph
            .create_node(
                NodeInput::new(["Indicator"])
                    .with_property("name", PropertyValue::String(name.to_owned()))
                    .with_property("score", PropertyValue::Integer(score))
                    .with_property(
                        "tags",
                        PropertyValue::StringList(tags.into_iter().map(str::to_owned).collect()),
                    ),
            )
            .expect("fixture node should be created");
    }
    graph
}

#[test]
fn nested_boolean_membership_and_multikey_sort_execute_from_the_structured_ast() {
    let mut executor =
        CypherPipelineExecutor::with_graph(ExecutionPolicy::strict_default(), graph());
    let result = executor
        .execute(
            "MATCH (n:Indicator) \
             WHERE (n.score >= 10 AND n.tags IN ['c2']) OR n.name = 'gamma' \
             RETURN n.name, n.score ORDER BY n.score DESC, n.name ASC",
        )
        .expect("advanced read should execute");
    let ExecutionResultData::Records(records) = result.data else {
        panic!("expected records");
    };
    assert_eq!(
        records
            .iter()
            .map(|record| record.fields["n.name"].as_str())
            .collect::<Vec<_>>(),
        vec!["alpha", "beta", "gamma"]
    );
}

#[test]
fn numeric_aggregations_execute_with_documented_precision_and_null_rules() {
    let mut executor =
        CypherPipelineExecutor::with_graph(ExecutionPolicy::strict_default(), graph());
    let result = executor
        .execute(
            "MATCH (n:Indicator) \
             RETURN COUNT(n), SUM(n.score), AVG(n.score), MIN(n.score), MAX(n.score)",
        )
        .expect("numeric aggregation should execute");
    let ExecutionResultData::Records(records) = result.data else {
        panic!("expected one aggregate record");
    };
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].fields["count"], "3");
    assert_eq!(records[0].fields["sum(n.score)"], "25");
    assert_eq!(records[0].fields["min(n.score)"], "5");
    assert_eq!(records[0].fields["max(n.score)"], "10");
    let average = records[0].fields["avg(n.score)"].parse::<f64>().unwrap();
    assert!((average - (25.0 / 3.0)).abs() < 1e-12);
}

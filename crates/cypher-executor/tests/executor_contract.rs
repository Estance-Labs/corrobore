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
use cypher_executor::{
    CypherPipelineExecutor, ExecutionPolicy, ExecutionResultData, ExecutionStatus,
};
use graph_core::{Graph, NodeInput, PropertyValue, RecordStatus, RelationshipInput};

fn build_indicator_graph() -> Graph {
    let mut graph = Graph::new();

    let alpha = graph
        .create_node(
            NodeInput::new(["Indicator"])
                .with_status(RecordStatus::Exportable)
                .with_property("name", PropertyValue::String("alpha".to_owned()))
                .with_property("score", PropertyValue::Integer(10))
                .with_property("active", PropertyValue::Bool(true)),
        )
        .expect("alpha node should be created");
    let beta = graph
        .create_node(
            NodeInput::new(["Indicator"])
                .with_status(RecordStatus::Exportable)
                .with_property("name", PropertyValue::String("beta".to_owned()))
                .with_property("score", PropertyValue::Integer(20))
                .with_property("active", PropertyValue::Bool(false)),
        )
        .expect("beta node should be created");
    graph
        .create_node(
            NodeInput::new(["Indicator"])
                .with_status(RecordStatus::Exportable)
                .with_property("name", PropertyValue::String("alpha".to_owned()))
                .with_property("score", PropertyValue::Integer(15))
                .with_property("active", PropertyValue::Bool(true)),
        )
        .expect("duplicate-name node should be created");

    graph
        .create_relationship(
            RelationshipInput::new(alpha, "RELATED_TO", beta)
                .expect("relationship input should be valid")
                .with_status(RecordStatus::Exportable)
                .with_property("enabled", PropertyValue::Bool(true)),
        )
        .expect("relationship should be created");

    graph
}

#[test]
fn execute_returns_success_records_for_supported_read_query() {
    let mut executor = CypherPipelineExecutor::new(ExecutionPolicy::strict_default());

    let result = executor
        .execute("MATCH (n) RETURN n LIMIT 1")
        .expect("supported read query should execute");

    assert_eq!(result.status, ExecutionStatus::Success);
    assert!(matches!(result.data, ExecutionResultData::Records(_)));
}

#[test]
fn execute_returns_rejected_result_for_mutation_under_read_only_policy() {
    let mut executor = CypherPipelineExecutor::new(ExecutionPolicy::strict_default());

    let result = executor
        .execute("CREATE (n:Indicator {name: 'x'})")
        .expect("mutation should return structured rejection under strict default policy");

    assert_eq!(result.status, ExecutionStatus::Rejected);
    assert_eq!(result.validation_errors.len(), 1);
    assert_eq!(
        result.validation_errors[0].code,
        "WRITE_PERMISSION_REQUIRED"
    );
}

#[test]
fn execute_traverses_graph_for_relationship_match_query() {
    let mut graph = Graph::new();
    let actor = graph
        .create_node(
            NodeInput::new(["Actor"])
                .with_status(RecordStatus::Exportable)
                .with_property("name", PropertyValue::String("alpha".to_owned())),
        )
        .expect("actor node creation should succeed");
    let narrative = graph
        .create_node(
            NodeInput::new(["Narrative"])
                .with_status(RecordStatus::Exportable)
                .with_property("name", PropertyValue::String("n1".to_owned())),
        )
        .expect("narrative node creation should succeed");
    graph
        .create_relationship(
            RelationshipInput::new(actor.clone(), "AMPLIFIES", narrative)
                .expect("relationship input should be valid")
                .with_status(RecordStatus::Exportable),
        )
        .expect("relationship creation should succeed");

    let mut executor = CypherPipelineExecutor::with_graph(ExecutionPolicy::strict_default(), graph);

    let result = executor
        .execute("MATCH (a:Actor)-[:AMPLIFIES]->(n:Narrative) WHERE a.name = 'alpha' RETURN a, n")
        .expect("relationship read query should execute");

    assert_eq!(result.status, ExecutionStatus::Success);
    match result.data {
        ExecutionResultData::Records(records) => {
            assert_eq!(records.len(), 1);
            let fields = &records[0].fields;
            assert_eq!(fields.get("a"), Some(&actor.as_str().to_owned()));
            assert!(fields.contains_key("n"));
        }
        _ => panic!("expected records for relationship traversal"),
    }
}

#[test]
fn execute_returns_invalid_query_for_empty_text() {
    let mut executor = CypherPipelineExecutor::new(ExecutionPolicy::strict_default());

    let error = executor
        .execute(" ")
        .expect_err("empty query should return typed invalid query error");

    assert_eq!(
        error.to_string(),
        "invalid query: query text must not be empty"
    );
}

#[test]
fn execute_returns_invalid_query_for_unsupported_feature() {
    let mut executor = CypherPipelineExecutor::new(ExecutionPolicy::strict_default());

    let error = executor
        .execute("UNWIND [1, 2, 3] AS x RETURN x")
        .expect_err("unsupported feature should map to invalid query");

    assert!(error.to_string().starts_with("invalid query:"));
}

#[test]
fn execute_applies_integer_filter_order_skip_and_limit() {
    let graph = build_indicator_graph();
    let mut executor = CypherPipelineExecutor::with_graph(ExecutionPolicy::strict_default(), graph);

    let result = executor
 .execute(
 "MATCH (n:Indicator) WHERE n.score >= 10 RETURN n.score ORDER BY n.score DESC SKIP 1 LIMIT 1",
 )
 .expect("filtered ordered query should execute");

    assert_eq!(result.status, ExecutionStatus::Success);
    match result.data {
        ExecutionResultData::Records(records) => {
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].fields.get("n.score"), Some(&"15".to_owned()));
        }
        _ => panic!("expected one record after ordering and pagination"),
    }
}

#[test]
fn execute_applies_boolean_where_filter() {
    let graph = build_indicator_graph();
    let mut executor = CypherPipelineExecutor::with_graph(ExecutionPolicy::strict_default(), graph);

    let result = executor
        .execute("MATCH (n:Indicator) WHERE n.active = true RETURN n")
        .expect("bool filter query should execute");

    match result.data {
        ExecutionResultData::Records(records) => {
            assert_eq!(records.len(), 2);
        }
        _ => panic!("expected matching records for boolean filter"),
    }
}

#[test]
fn execute_applies_distinct_projection() {
    let graph = build_indicator_graph();
    let mut executor = CypherPipelineExecutor::with_graph(ExecutionPolicy::strict_default(), graph);

    let result = executor
        .execute("MATCH (n:Indicator) RETURN DISTINCT n.name ORDER BY n.name")
        .expect("distinct query should execute");

    match result.data {
        ExecutionResultData::Records(records) => {
            assert_eq!(records.len(), 2);
            assert_eq!(records[0].fields.get("n.name"), Some(&"alpha".to_owned()));
            assert_eq!(records[1].fields.get("n.name"), Some(&"beta".to_owned()));
        }
        _ => panic!("expected distinct records"),
    }
}

#[test]
fn execute_returns_count_projection_for_aggregation_only_return() {
    let graph = build_indicator_graph();
    let mut executor = CypherPipelineExecutor::with_graph(ExecutionPolicy::strict_default(), graph);

    let result = executor
        .execute("MATCH (n:Indicator) RETURN COUNT(n)")
        .expect("count query should execute");

    match result.data {
        ExecutionResultData::Records(records) => {
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].fields.get("count"), Some(&"3".to_owned()));
        }
        _ => panic!("expected count record"),
    }
}

#[test]
fn execute_relationship_query_can_project_relationship_binding() {
    let graph = build_indicator_graph();
    let mut executor = CypherPipelineExecutor::with_graph(ExecutionPolicy::strict_default(), graph);

    let result = executor
        .execute("MATCH (a:Indicator)-[r:RELATED_TO]->(b:Indicator) RETURN r")
        .expect("relationship projection query should execute");

    match result.data {
        ExecutionResultData::Records(records) => {
            assert_eq!(records.len(), 1);
            assert!(records[0].fields.contains_key("r"));
        }
        _ => panic!("expected one relationship record"),
    }
}

#[test]
fn execute_where_with_mismatched_literal_type_filters_out_all_rows() {
    let graph = build_indicator_graph();
    let mut executor = CypherPipelineExecutor::with_graph(ExecutionPolicy::strict_default(), graph);

    let result = executor
        .execute("MATCH (n:Indicator) WHERE n.score = '10' RETURN n")
        .expect("query should execute even when where type does not match");

    match result.data {
        ExecutionResultData::Records(records) => assert!(records.is_empty()),
        _ => {
            panic!("records container should be returned for read queries")
        }
    }
}

#[test]
fn execute_allows_mutation_when_read_only_policy_is_disabled() {
    let mut executor = CypherPipelineExecutor::new(ExecutionPolicy {
        read_only_by_default: false,
    });

    let result = executor
        .execute("CREATE (n:Indicator {name: 'x'})")
        .expect("mutation should succeed when read-only policy is disabled");

    assert_eq!(result.status, ExecutionStatus::Success);
    assert!(matches!(
        result.data,
        ExecutionResultData::MutationSummary {
            nodes_created: 1,
            ..
        }
    ));
}

#[test]
fn execute_applies_string_filter_and_ascending_order() {
    let graph = build_indicator_graph();
    let mut executor = CypherPipelineExecutor::with_graph(ExecutionPolicy::strict_default(), graph);

    let result = executor
        .execute("MATCH (n:Indicator) WHERE n.name = 'beta' RETURN n.name ORDER BY n.name ASC")
        .expect("string equality query should execute");

    match result.data {
        ExecutionResultData::Records(records) => {
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].fields.get("n.name"), Some(&"beta".to_owned()));
        }
        _ => panic!("expected projected records"),
    }
}

#[test]
fn execute_relationship_query_with_non_matching_type_returns_no_records() {
    let graph = build_indicator_graph();
    let mut executor = CypherPipelineExecutor::with_graph(ExecutionPolicy::strict_default(), graph);

    let result = executor
        .execute("MATCH (a:Indicator)-[:NON_EXISTING]->(b:Indicator) RETURN a, b")
        .expect("relationship query should execute even if type does not match any edge");

    match result.data {
        ExecutionResultData::Records(records) => assert!(records.is_empty()),
        _ => panic!("expected records container for read queries"),
    }
}

#[test]
fn execute_projects_float_and_list_property_values_as_strings() {
    let mut graph = Graph::new();
    graph
        .create_node(
            NodeInput::new(["Indicator"])
                .with_status(RecordStatus::Exportable)
                .with_property(
                    "tags",
                    PropertyValue::StringList(vec!["alpha".to_owned(), "beta".to_owned()]),
                )
                .with_property("scores", PropertyValue::IntegerList(vec![1, 2, 3]))
                .with_property("weights", PropertyValue::FloatList(vec![1.5, 2.5]))
                .with_property("flags", PropertyValue::BoolList(vec![true, false]))
                .with_property("ratio", PropertyValue::Float(0.75))
                .with_property("note", PropertyValue::Null),
        )
        .expect("node creation should succeed");

    let mut executor = CypherPipelineExecutor::with_graph(ExecutionPolicy::strict_default(), graph);

    let result = executor
        .execute("MATCH (n:Indicator) RETURN n.tags, n.scores, n.weights, n.flags, n.ratio, n.note")
        .expect("projection query should execute");

    match result.data {
        ExecutionResultData::Records(records) => {
            assert_eq!(records.len(), 1);
            let fields = &records[0].fields;
            assert_eq!(fields.get("n.tags"), Some(&"alpha,beta".to_owned()));
            assert_eq!(fields.get("n.scores"), Some(&"1,2,3".to_owned()));
            assert_eq!(fields.get("n.weights"), Some(&"1.5,2.5".to_owned()));
            assert_eq!(fields.get("n.flags"), Some(&"true,false".to_owned()));
            assert_eq!(fields.get("n.ratio"), Some(&"0.75".to_owned()));
            assert_eq!(fields.get("n.note"), Some(&"null".to_owned()));
        }
        _ => panic!("expected one projected record"),
    }
}

#[test]
fn execute_applies_integer_lt_and_lte_filters() {
    let graph = build_indicator_graph();
    let mut executor = CypherPipelineExecutor::with_graph(ExecutionPolicy::strict_default(), graph);

    let lt_result = executor
        .execute("MATCH (n:Indicator) WHERE n.score < 15 RETURN n")
        .expect("lt filter query should execute");
    match lt_result.data {
        ExecutionResultData::Records(records) => assert_eq!(records.len(), 1),
        _ => panic!("expected records container"),
    }

    let lte_result = executor
        .execute("MATCH (n:Indicator) WHERE n.score <= 15 RETURN n")
        .expect("lte filter query should execute");
    match lte_result.data {
        ExecutionResultData::Records(records) => assert_eq!(records.len(), 2),
        _ => panic!("expected records container"),
    }
}

#[test]
fn execute_applies_boolean_not_equal_filter() {
    let graph = build_indicator_graph();
    let mut executor = CypherPipelineExecutor::with_graph(ExecutionPolicy::strict_default(), graph);

    let result = executor
        .execute("MATCH (n:Indicator) WHERE n.active <> true RETURN n")
        .expect("boolean not-equal filter query should execute");

    match result.data {
        ExecutionResultData::Records(records) => assert_eq!(records.len(), 1),
        _ => panic!("expected records container"),
    }
}

#[test]
fn execute_optional_match_query_is_supported_for_node_scan() {
    let graph = build_indicator_graph();
    let mut executor = CypherPipelineExecutor::with_graph(ExecutionPolicy::strict_default(), graph);

    let result = executor
        .execute("OPTIONAL MATCH (n:Indicator) RETURN n LIMIT 2")
        .expect("optional match query should execute");

    assert_eq!(result.status, ExecutionStatus::Success);
    match result.data {
        ExecutionResultData::Records(records) => assert_eq!(records.len(), 2),
        _ => panic!("expected projected records"),
    }
}

// ---------------------------------------------------------------------------
// Mutation execution: CREATE
// ---------------------------------------------------------------------------

#[test]
fn execute_create_node_creates_and_returns_node() {
    let mut executor = CypherPipelineExecutor::new(ExecutionPolicy {
        read_only_by_default: false,
    });

    let result = executor
        .execute("CREATE (n:Indicator {name: 'created'}) RETURN n")
        .expect("CREATE with RETURN should execute");

    assert_eq!(result.status, ExecutionStatus::Success);
    match &result.data {
        ExecutionResultData::Records(records) => {
            assert_eq!(records.len(), 1);
            assert!(records[0].fields.contains_key("n"));
        }
        _ => panic!("expected records for CREATE with RETURN"),
    }
}

#[test]
fn execute_create_node_without_return_produces_mutation_summary() {
    let mut executor = CypherPipelineExecutor::new(ExecutionPolicy {
        read_only_by_default: false,
    });

    let result = executor
        .execute("CREATE (n:Indicator {name: 'test'})")
        .expect("CREATE without RETURN should execute");

    assert_eq!(result.status, ExecutionStatus::Success);
    match &result.data {
        ExecutionResultData::MutationSummary {
            nodes_created,
            relationships_created,
            ..
        } => {
            assert_eq!(*nodes_created, 1);
            assert_eq!(*relationships_created, 0);
        }
        _ => panic!("expected mutation summary for CREATE without RETURN"),
    }
}

#[test]
fn execute_create_node_persists_properties_in_graph() {
    let mut executor = CypherPipelineExecutor::new(ExecutionPolicy {
        read_only_by_default: false,
    });

    executor
        .execute("CREATE (n:Indicator {name: 'persisted', score: 42})")
        .expect("CREATE should execute");

    // Verify the node exists by reading it back
    let result = executor
        .execute("MATCH (n:Indicator) WHERE n.name = 'persisted' RETURN n.name, n.score")
        .expect("read-back query should execute");

    match &result.data {
        ExecutionResultData::Records(records) => {
            assert_eq!(records.len(), 1);
            assert_eq!(
                records[0].fields.get("n.name"),
                Some(&"persisted".to_owned())
            );
            assert_eq!(records[0].fields.get("n.score"), Some(&"42".to_owned()));
        }
        _ => panic!("expected records for read-back query"),
    }
}

// ---------------------------------------------------------------------------
// Mutation execution: MERGE
// ---------------------------------------------------------------------------

#[test]
fn execute_merge_creates_node_when_not_found() {
    let mut executor = CypherPipelineExecutor::new(ExecutionPolicy {
        read_only_by_default: false,
    });

    let result = executor
        .execute("MERGE (n:Indicator {name: 'merged'}) RETURN n")
        .expect("MERGE creating new node should execute");

    assert_eq!(result.status, ExecutionStatus::Success);
    match &result.data {
        ExecutionResultData::Records(records) => {
            assert_eq!(records.len(), 1);
            assert!(records[0].fields.contains_key("n"));
        }
        _ => panic!("expected records for MERGE with RETURN"),
    }
}

#[test]
fn execute_merge_finds_existing_node_without_creating_duplicate() {
    let mut executor = CypherPipelineExecutor::new(ExecutionPolicy {
        read_only_by_default: false,
    });

    // Create the node first
    executor
        .execute("CREATE (n:Indicator {name: 'existing'})")
        .expect("initial CREATE should execute");

    // MERGE should find it, not create a duplicate
    executor
        .execute("MERGE (n:Indicator {name: 'existing'})")
        .expect("MERGE should execute");

    // Count should still be 1
    let result = executor
        .execute("MATCH (n:Indicator) WHERE n.name = 'existing' RETURN COUNT(n)")
        .expect("count query should execute");

    match &result.data {
        ExecutionResultData::Records(records) => {
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].fields.get("count"), Some(&"1".to_owned()));
        }
        _ => panic!("expected count record"),
    }
}

#[test]
fn execute_merge_relationship_creates_and_counts_the_edge() {
    let mut executor = CypherPipelineExecutor::new(ExecutionPolicy {
        read_only_by_default: false,
    });
    executor
        .execute("CREATE (s:Entity {id: 'source'})")
        .expect("source node should execute");
    executor
        .execute("CREATE (o:Entity {id: 'target'})")
        .expect("target node should execute");

    let result = executor
        .execute("MATCH (s:Entity {id: 'source'}) MERGE (s)-[r:TARGETS]->(o:Entity {id: 'target'})")
        .expect("relationship MERGE should execute");

    match result.data {
        ExecutionResultData::MutationSummary {
            relationships_created,
            ..
        } => assert_eq!(relationships_created, 1),
        _ => panic!("expected mutation summary for relationship MERGE"),
    }
    let read = executor
        .execute("MATCH (s:Entity)-[r:TARGETS]->(o:Entity) RETURN COUNT(r)")
        .expect("created relationship should be readable");
    match read.data {
        ExecutionResultData::Records(records) => {
            assert_eq!(records[0].fields.get("count"), Some(&"1".to_owned()));
        }
        _ => panic!("expected relationship count record"),
    }
}

#[test]
fn execute_create_relationship_creates_and_counts_the_edge() {
    let mut executor = CypherPipelineExecutor::new(ExecutionPolicy {
        read_only_by_default: false,
    });
    executor
        .execute("CREATE (s:Entity {id: 'source'})")
        .expect("source node should execute");

    let result = executor
        .execute(
            "MATCH (s:Entity {id: 'source'}) CREATE (s)-[r:TARGETS]->(o:Entity {id: 'target'})",
        )
        .expect("relationship CREATE should execute");

    match result.data {
        ExecutionResultData::MutationSummary {
            relationships_created,
            ..
        } => assert_eq!(relationships_created, 1),
        _ => panic!("expected mutation summary for relationship CREATE"),
    }
    let read = executor
        .execute("MATCH (s:Entity)-[r:TARGETS]->(o:Entity) RETURN COUNT(r)")
        .expect("created relationship should be readable");
    match read.data {
        ExecutionResultData::Records(records) => {
            assert_eq!(records[0].fields.get("count"), Some(&"1".to_owned()));
        }
        _ => panic!("expected relationship count record"),
    }
}

// ---------------------------------------------------------------------------
// Mutation execution: SET
// ---------------------------------------------------------------------------

#[test]
fn execute_match_set_updates_property() {
    let mut executor = CypherPipelineExecutor::new(ExecutionPolicy {
        read_only_by_default: false,
    });

    executor
        .execute("CREATE (n:Indicator {name: 'target', score: 10})")
        .expect("setup CREATE should execute");

    let result = executor
        .execute("MATCH (n:Indicator) WHERE n.name = 'target' SET n.score = 99 RETURN n.score")
        .expect("MATCH+SET+RETURN should execute");

    assert_eq!(result.status, ExecutionStatus::Success);
    match &result.data {
        ExecutionResultData::Records(records) => {
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].fields.get("n.score"), Some(&"99".to_owned()));
        }
        _ => panic!("expected records for SET with RETURN"),
    }
}

#[test]
fn execute_match_set_with_multiple_assignments_updates_all() {
    let mut executor = CypherPipelineExecutor::new(ExecutionPolicy {
        read_only_by_default: false,
    });

    executor
        .execute("CREATE (n:Indicator {name: 'multi', score: 1, active: false})")
        .expect("setup CREATE should execute");

    let result = executor
        .execute(
            "MATCH (n:Indicator) WHERE n.name = 'multi' SET n.score = 50, n.active = true RETURN n.score, n.active",
        )
        .expect("SET with multiple assignments should execute");

    match &result.data {
        ExecutionResultData::Records(records) => {
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].fields.get("n.score"), Some(&"50".to_owned()));
            assert_eq!(records[0].fields.get("n.active"), Some(&"true".to_owned()));
        }
        _ => panic!("expected records for multi-SET"),
    }
}

// ---------------------------------------------------------------------------
// Mutation execution: DELETE
// ---------------------------------------------------------------------------

#[test]
fn execute_match_delete_tombstones_node() {
    let mut executor = CypherPipelineExecutor::new(ExecutionPolicy {
        read_only_by_default: false,
    });

    executor
        .execute("CREATE (n:Indicator {name: 'doomed'})")
        .expect("setup CREATE should execute");

    let result = executor
        .execute("MATCH (n:Indicator) WHERE n.name = 'doomed' DELETE n")
        .expect("MATCH+DELETE should execute");

    assert_eq!(result.status, ExecutionStatus::Success);

    // Node should no longer be found
    let check = executor
        .execute("MATCH (n:Indicator) WHERE n.name = 'doomed' RETURN n")
        .expect("read-back query should execute");

    match &check.data {
        ExecutionResultData::Records(records) => assert!(records.is_empty()),
        _ => panic!("expected empty records after DELETE"),
    }
}

// ---------------------------------------------------------------------------
// Mutation execution: REMOVE
// ---------------------------------------------------------------------------

#[test]
fn execute_match_remove_nullifies_property() {
    let mut executor = CypherPipelineExecutor::new(ExecutionPolicy {
        read_only_by_default: false,
    });

    executor
        .execute("CREATE (n:Indicator {name: 'removable', score: 10})")
        .expect("setup CREATE should execute");

    let result = executor
        .execute("MATCH (n:Indicator) WHERE n.name = 'removable' REMOVE n.score RETURN n.name")
        .expect("MATCH+REMOVE+RETURN should execute");

    assert_eq!(result.status, ExecutionStatus::Success);
    match &result.data {
        ExecutionResultData::Records(records) => {
            assert_eq!(records.len(), 1);
        }
        _ => panic!("expected records for REMOVE with RETURN"),
    }

    // Verify the property is gone
    let check = executor
        .execute("MATCH (n:Indicator) WHERE n.name = 'removable' RETURN n.score")
        .expect("read-back should execute");

    match &check.data {
        ExecutionResultData::Records(records) => {
            assert_eq!(records.len(), 1);
            // score should be null or absent
            let score = records[0].fields.get("n.score");
            assert!(
                score.is_none() || score == Some(&"null".to_owned()),
                "removed property should be null or absent"
            );
        }
        _ => panic!("expected records"),
    }
}

// ---------------------------------------------------------------------------
// Mutation policy enforcement
// ---------------------------------------------------------------------------

#[test]
fn execute_rejects_mixed_query_under_read_only_policy() {
    let mut executor = CypherPipelineExecutor::new(ExecutionPolicy::strict_default());

    let result = executor
        .execute("MATCH (n:Indicator) SET n.score = 1 RETURN n")
        .expect("mixed query should return structured rejection");

    assert_eq!(result.status, ExecutionStatus::Rejected);
    assert_eq!(result.validation_errors.len(), 1);
    assert_eq!(
        result.validation_errors[0].code,
        "WRITE_PERMISSION_REQUIRED"
    );
}

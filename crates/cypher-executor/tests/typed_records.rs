// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//! Typed record values beside the string fields (epic #12, item #258).
//!
//! A protocol adapter that has to encode a record for a wire format cannot
//! guess whether `"3"` was an integer, a string, or a list of one element. The
//! executor therefore keeps a typed twin of every projected field, and the
//! ordered column list the RETURN clause declared. The string fields and the
//! HTTP JSON they feed are unchanged.
#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;

use cypher_executor::{CypherPipelineExecutor, ExecutionPolicy, ExecutionResultData, RecordValue};

fn executor() -> CypherPipelineExecutor {
    CypherPipelineExecutor::new(ExecutionPolicy {
        read_only_by_default: false,
    })
}

fn records(result: cypher_executor::ExecutionResult) -> Vec<cypher_executor::ExecutionRecord> {
    match result.data {
        ExecutionResultData::Records(records) => records,
        other => panic!("expected records, got {other:?}"),
    }
}

#[test]
fn projected_properties_keep_their_native_type() {
    let mut executor = executor();
    executor
        .execute(
            "CREATE (n:Sensor {name: 'north', reading: 42, ratio: 0.5, live: true, tags: ['a', 'b']}) RETURN n",
        )
        .unwrap();

    let result = executor
        .execute(
            "MATCH (n:Sensor) RETURN n.name, n.reading, n.ratio, n.live, n.tags, n.missing LIMIT 1",
        )
        .unwrap();

    // The column order is the RETURN order, not the hash-map order.
    assert_eq!(
        result.columns,
        vec![
            "n.name",
            "n.reading",
            "n.ratio",
            "n.live",
            "n.tags",
            "n.missing"
        ]
    );
    let record = records(result).remove(0);
    assert_eq!(record.values["n.name"], RecordValue::String("north".into()));
    assert_eq!(record.values["n.reading"], RecordValue::Integer(42));
    assert_eq!(record.values["n.ratio"], RecordValue::Float("0.5".into()));
    assert_eq!(record.values["n.live"], RecordValue::Boolean(true));
    assert_eq!(
        record.values["n.tags"],
        RecordValue::List(vec![
            RecordValue::String("a".into()),
            RecordValue::String("b".into())
        ])
    );
    // A property the node does not have is absent from both twins, exactly as
    // before: the typed map adds information, it never invents a null.
    assert!(!record.values.contains_key("n.missing"));
    assert!(!record.fields.contains_key("n.missing"));
    // The string twin is byte-identical to the historical rendering.
    assert_eq!(record.fields["n.reading"], "42");
    assert_eq!(record.fields["n.tags"], "a,b");
}

#[test]
fn whole_node_and_relationship_projections_carry_structure() {
    let mut executor = executor();
    executor
        .execute("CREATE (a:Actor {name: 'APT28'})")
        .unwrap();
    executor
        .execute("MATCH (a:Actor) CREATE (a)-[r:USES]->(m:Malware {name: 'X-Agent'})")
        .unwrap();
    executor
        .execute("MATCH (a:Actor)-[r:USES]->(m:Malware) SET r.since = 2019")
        .unwrap();

    let result = executor
        .execute("MATCH (a:Actor)-[r:USES]->(m:Malware) RETURN a, r, m LIMIT 1")
        .unwrap();
    assert_eq!(result.columns, vec!["a", "r", "m"]);
    let record = records(result).remove(0);

    let RecordValue::Node(actor) = &record.values["a"] else {
        panic!("a must be a node, got {:?}", record.values["a"]);
    };
    assert_eq!(actor.labels, vec!["Actor".to_owned()]);
    assert_eq!(
        actor.properties.get("name"),
        Some(&RecordValue::String("APT28".into()))
    );
    // Native metadata travels as reserved property keys, the same names Cypher
    // property access exposes.
    assert_eq!(
        actor.properties.get("status"),
        Some(&RecordValue::String("candidate".into()))
    );
    // The string twin still holds the identifier alone.
    assert_eq!(record.fields["a"], actor.id);

    let RecordValue::Relationship(uses) = &record.values["r"] else {
        panic!("r must be a relationship");
    };
    assert_eq!(uses.rel_type, "USES");
    assert_eq!(uses.source_id, actor.id);
    assert_eq!(
        uses.properties.get("since"),
        Some(&RecordValue::Integer(2019))
    );
    let RecordValue::Node(malware) = &record.values["m"] else {
        panic!("m must be a node");
    };
    assert_eq!(uses.target_id, malware.id);
}

#[test]
fn aggregations_are_typed_and_ordered() {
    let mut executor = executor();
    for (name, value) in [("a", 1), ("b", 2), ("c", 4)] {
        executor
            .execute(&format!(
                "CREATE (n:Sample {{name: '{name}', value: {value}}}) RETURN n"
            ))
            .unwrap();
    }

    let result = executor
        .execute("MATCH (n:Sample) RETURN count(n), sum(n.value), avg(n.value), max(n.value)")
        .unwrap();
    assert_eq!(
        result.columns,
        vec!["count", "sum(n.value)", "avg(n.value)", "max(n.value)"]
    );
    let record = records(result).remove(0);
    assert_eq!(record.values["count"], RecordValue::Integer(3));
    // Integral sums, minima and maxima stay integers; an average is a float
    // even when it happens to be whole, as in every graph database drivers know.
    assert_eq!(record.values["sum(n.value)"], RecordValue::Integer(7));
    assert_eq!(record.values["max(n.value)"], RecordValue::Integer(4));
    assert!(matches!(
        record.values["avg(n.value)"],
        RecordValue::Float(_)
    ));
}

#[test]
fn mutations_and_empty_results_have_no_columns() {
    let mut executor = executor();
    let created = executor.execute("CREATE (n:Plain {name: 'x'})").unwrap();
    assert!(created.columns.is_empty());
    assert!(matches!(
        created.data,
        ExecutionResultData::MutationSummary { .. }
    ));

    let validated = executor.validate("MATCH (n) RETURN n LIMIT 1").unwrap();
    assert!(validated.columns.is_empty());
}

#[test]
fn json_properties_become_nested_typed_values() {
    // A JSON property is the one PropertyValue that nests; its typed twin is
    // a map or list of typed values, never an opaque string.
    let nested = RecordValue::from(&graph_core::PropertyValue::Json(serde_json::json!({
        "k": [1, 2.5, "s", null, true],
        "m": {"inner": 1}
    })));
    let RecordValue::Map(map) = nested else {
        panic!("json object must become a map");
    };
    assert_eq!(
        map["k"],
        RecordValue::List(vec![
            RecordValue::Integer(1),
            RecordValue::Float("2.5".into()),
            RecordValue::String("s".into()),
            RecordValue::Null,
            RecordValue::Boolean(true),
        ])
    );
    let mut inner = BTreeMap::new();
    inner.insert("inner".to_owned(), RecordValue::Integer(1));
    assert_eq!(map["m"], RecordValue::Map(inner));
}

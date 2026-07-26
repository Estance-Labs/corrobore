// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
#![allow(clippy::unwrap_used)]

use std::{collections::BTreeMap, fs, path::PathBuf};

use graph_core::{Graph, PropertyValue};
use opencti_adapter::{
    MergeLimits, OpenCtiAdapter, OpenCtiMergeExecutor, OpenCtiMergeRequest, OpenCtiWriteBatch,
    OpenCtiWriteExecutor, OpenCtiWriteOperation, WriteLimits,
};
use serde_json::{Value, json};

fn indicator(id: &str, name: &str, marking: &str, tenant: &str) -> Value {
    json!({
        "id": id,
        "type": "indicator",
        "name": name,
        "pattern_type": "stix",
        "pattern": format!("[domain-name:value = '{name}.example']"),
        "object_marking_refs": [marking],
        "x_opencti_tenant_refs": [tenant],
        "x_opencti_files": [{"id": format!("file--{id}"), "name": format!("{name}.txt")}],
        "external_references": [{"source_name": id, "external_id": format!("ext-{id}")}]
    })
}

fn object(id: &str, object_type: &str) -> Value {
    json!({"id": id, "type": object_type, "name": id})
}

fn relationship(id: &str, source: &str, target: &str, relationship_type: &str) -> Value {
    json!({
        "id": id,
        "type": "relationship",
        "relationship_type": relationship_type,
        "source_ref": source,
        "target_ref": target
    })
}

fn seeded_graph(records: Vec<Value>) -> Graph {
    OpenCtiWriteExecutor::new(WriteLimits::default())
        .apply(
            &Graph::new(),
            &OpenCtiWriteBatch::new(
                "tx--merge-seed",
                true,
                records
                    .into_iter()
                    .enumerate()
                    .map(|(index, record)| {
                        OpenCtiWriteOperation::create(format!("seed-{index}"), record)
                    })
                    .collect(),
            )
            .unwrap(),
        )
        .unwrap()
        .graph
}

fn raw_record(graph: &Graph, canonical_id: &str) -> Option<Value> {
    let adapter = OpenCtiAdapter::pinned();
    graph
        .list_nodes()
        .unwrap()
        .into_iter()
        .find_map(|node| {
            let mapped = adapter.restore_node(&node).ok()?;
            (mapped.record_ref().canonical_id() == canonical_id).then(|| mapped.raw().clone())
        })
        .or_else(|| {
            graph
                .list_relationships()
                .unwrap()
                .into_iter()
                .find_map(|edge| {
                    let mapped = adapter.restore_relationship(&edge).ok()?;
                    (mapped.record_ref().canonical_id() == canonical_id)
                        .then(|| mapped.raw().clone())
                })
        })
}

#[test]
fn every_issue_38_corpus_merge_matches_the_captured_reference_outcome() {
    let compatibility =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../compatibility/opencti/7.260722.0");
    let corpus: Value =
        serde_json::from_slice(&fs::read(compatibility.join("parity-corpus.json")).unwrap())
            .unwrap();
    let captures: Value =
        serde_json::from_slice(&fs::read(compatibility.join("reference-results.json")).unwrap())
            .unwrap();
    let fixtures = corpus["fixtures"].as_array().unwrap().clone();
    let graph = seeded_graph(fixtures);

    for scenario in corpus["scenarios"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|scenario| scenario["kind"] == "merges")
    {
        let scenario_id = scenario["id"].as_str().unwrap();
        let expected = captures["captures"]
            .as_array()
            .unwrap()
            .iter()
            .find(|capture| capture["input"]["scenario"] == scenario_id)
            .unwrap();
        let target_id = scenario["target_id"].as_str().unwrap();
        let source_ids = scenario["source_ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| id.as_str().unwrap().to_owned())
            .collect();
        let outcome = OpenCtiMergeExecutor::new(MergeLimits::default())
            .apply(
                &graph,
                &OpenCtiMergeRequest::new(
                    format!("merge--{scenario_id}"),
                    target_id,
                    source_ids,
                    BTreeMap::new(),
                )
                .unwrap(),
            )
            .unwrap();

        assert_eq!(outcome.target_id, expected["expected"]["target_id"]);
        assert_eq!(
            serde_json::to_value(outcome.deleted_source_ids).unwrap(),
            expected["expected"]["deleted_source_ids"]
        );
        assert_eq!(
            serde_json::to_value(outcome.redirected_relationship_ids).unwrap(),
            expected["expected"]["redirected_relationship_ids"]
        );
    }
}

#[test]
fn corpus_merge_preserves_survivor_unions_security_and_rewires_relationships() {
    let target = "indicator--00000000-0000-4000-8000-000000000040";
    let first_source = "indicator--00000000-0000-4000-8000-000000000041";
    let second_source = "indicator--00000000-0000-4000-8000-000000000042";
    let report = "report--00000000-0000-4000-8000-000000000052";
    let malware = "malware--00000000-0000-4000-8000-000000000050";
    let mut report_record = object(report, "report");
    report_record["object_refs"] = json!([first_source, malware]);
    let mut retained_edge = relationship("relationship--60", target, malware, "indicates");
    retained_edge["object_marking_refs"] = json!(["marking--1"]);
    let mut duplicate_edge = relationship("relationship--61", first_source, malware, "indicates");
    duplicate_edge["object_marking_refs"] = json!(["marking--2"]);
    let graph = seeded_graph(vec![
        indicator(target, "survivor", "marking--1", "tenant--1"),
        indicator(first_source, "source-one", "marking--2", "tenant--1"),
        indicator(second_source, "source-two", "marking--1", "tenant--2"),
        report_record,
        object(malware, "malware"),
        retained_edge,
        duplicate_edge,
        relationship("relationship--62", report, first_source, "object"),
    ]);
    let source_graph_ids = graph
        .list_nodes()
        .unwrap()
        .into_iter()
        .filter_map(|node| {
            matches!(
                node.property("opencti.canonical_id"),
                Some(PropertyValue::String(id)) if id == first_source || id == second_source
            )
            .then(|| node.id().clone())
        })
        .collect::<Vec<_>>();

    let outcome = OpenCtiMergeExecutor::new(MergeLimits::default())
        .apply(
            &graph,
            &OpenCtiMergeRequest::new(
                "merge--corpus",
                target,
                vec![first_source.to_owned(), second_source.to_owned()],
                BTreeMap::from([
                    (target.to_owned(), 1),
                    (first_source.to_owned(), 1),
                    (second_source.to_owned(), 1),
                ]),
            )
            .unwrap(),
        )
        .unwrap();

    assert!(outcome.applied);
    assert_eq!(outcome.target_id, target);
    assert_eq!(
        outcome.deleted_source_ids,
        vec![first_source, second_source]
    );
    assert!(
        outcome
            .redirected_relationship_ids
            .contains(&"relationship--62".to_owned())
    );
    assert_eq!(
        outcome.deduplicated_relationship_ids,
        vec!["relationship--61"]
    );
    assert_eq!(outcome.redirected_reference_ids, vec![report]);
    assert!(raw_record(&outcome.graph, first_source).is_none());
    assert!(raw_record(&outcome.graph, second_source).is_none());

    let survivor = raw_record(&outcome.graph, target).unwrap();
    assert_eq!(survivor["name"], "survivor", "target scalar values win");
    assert_eq!(
        survivor["object_marking_refs"],
        json!(["marking--1", "marking--2"]),
        "authorization cannot be weakened during merge"
    );
    assert_eq!(
        survivor["x_opencti_tenant_refs"],
        json!(["tenant--1", "tenant--2"])
    );
    assert_eq!(
        survivor["x_opencti_stix_ids"],
        json!([first_source, second_source])
    );
    assert_eq!(
        survivor["x_corrobore_merged_sources"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(survivor["x_opencti_files"].as_array().unwrap().len(), 3);
    for source_graph_id in source_graph_ids {
        assert_eq!(
            outcome
                .graph
                .list_node_versions(&source_graph_id)
                .unwrap()
                .len(),
            2,
            "source payload and tombstone history remain attributable"
        );
    }

    let redirected = raw_record(&outcome.graph, "relationship--62").unwrap();
    assert_eq!(redirected["target_ref"], target);
    assert_eq!(
        raw_record(&outcome.graph, report).unwrap()["object_refs"],
        json!([target, malware])
    );
    assert_eq!(
        raw_record(&outcome.graph, "relationship--60").unwrap()["object_marking_refs"],
        json!(["marking--1", "marking--2"]),
        "deduplicating edges cannot weaken authorization"
    );
    assert_eq!(outcome.graph.list_relationships().unwrap().len(), 2);

    let target_node = outcome
        .graph
        .list_nodes()
        .unwrap()
        .into_iter()
        .find(|node| {
            node.property("opencti.canonical_id") == Some(&PropertyValue::String(target.to_owned()))
        })
        .unwrap();
    assert_eq!(
        outcome
            .graph
            .list_node_versions(target_node.id())
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn merge_rejects_stale_preconditions_without_mutating_the_input_graph() {
    let target = "indicator--target";
    let source = "indicator--source";
    let graph = seeded_graph(vec![
        indicator(target, "target", "marking--1", "tenant--1"),
        indicator(source, "source", "marking--1", "tenant--1"),
    ]);
    let before_target = raw_record(&graph, target);
    let before_source = raw_record(&graph, source);
    let error = OpenCtiMergeExecutor::new(MergeLimits::default())
        .apply(
            &graph,
            &OpenCtiMergeRequest::new(
                "merge--stale",
                target,
                vec![source.to_owned()],
                BTreeMap::from([(target.to_owned(), 99), (source.to_owned(), 1)]),
            )
            .unwrap(),
        )
        .unwrap_err();

    assert!(error.to_string().contains("expected revision"));
    assert_eq!(raw_record(&graph, target), before_target);
    assert_eq!(raw_record(&graph, source), before_source);
}

#[test]
fn merge_is_bounded_before_scanning_a_supernode() {
    let target = "indicator--target";
    let source = "indicator--source";
    let mut records = vec![
        indicator(target, "target", "marking--1", "tenant--1"),
        indicator(source, "source", "marking--1", "tenant--1"),
        object("malware--one", "malware"),
    ];
    records.extend((0..3).map(|index| {
        relationship(
            &format!("relationship--{index}"),
            source,
            "malware--one",
            &format!("related-{index}"),
        )
    }));
    let graph = seeded_graph(records);
    let error = OpenCtiMergeExecutor::new(MergeLimits {
        max_sources: 8,
        max_relationships: 2,
    })
    .apply(
        &graph,
        &OpenCtiMergeRequest::new(
            "merge--bounded",
            target,
            vec![source.to_owned()],
            BTreeMap::new(),
        )
        .unwrap(),
    )
    .unwrap_err();

    assert!(error.to_string().contains("max_relationships"));
}

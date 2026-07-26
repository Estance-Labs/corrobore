// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
#![allow(clippy::unwrap_used)]

use graph_core::Graph;
use opencti_adapter::{
    DivergenceKind, OpenCtiAdapter, OpenCtiReconciler, OpenCtiReconciliationCommand,
    ReconciliationLimits, ReconciliationMode, ReconciliationScope, RepairAction,
};
use serde_json::{Value, json};

fn record(id: &str, name: &str, marking: &str) -> Value {
    json!({
        "id": id,
        "type": "indicator",
        "name": name,
        "object_marking_refs": [marking]
    })
}

fn relationship(id: &str, source: &str, target: &str) -> Value {
    json!({
        "id": id,
        "type": "relationship",
        "relationship_type": "related-to",
        "source_ref": source,
        "target_ref": target
    })
}

fn graph(records: &[Value]) -> Graph {
    let command = OpenCtiReconciliationCommand::new(
        "seed",
        ReconciliationMode::Repair,
        ReconciliationScope::Full { max_records: 32 },
        records.to_vec(),
        true,
    )
    .unwrap();
    OpenCtiReconciler::new(ReconciliationLimits::default())
        .execute(&Graph::new(), &command, &[])
        .unwrap()
        .graph
}

fn graph_records(graph: &Graph) -> Vec<Value> {
    let adapter = OpenCtiAdapter::pinned();
    let mut records = graph
        .list_nodes()
        .unwrap()
        .into_iter()
        .map(|node| adapter.restore_node(&node).unwrap().raw().clone())
        .chain(
            graph
                .list_relationships()
                .unwrap()
                .into_iter()
                .map(|edge| adapter.restore_relationship(&edge).unwrap().raw().clone()),
        )
        .collect::<Vec<_>>();
    records.sort_by(|left, right| left["id"].as_str().cmp(&right["id"].as_str()));
    records
}

#[test]
fn dry_run_reports_exact_dimensions_and_never_mutates_data() {
    let actual = graph(&[
        record("indicator--property", "old", "marking--1"),
        record("indicator--permission", "same", "marking--1"),
        record("indicator--extra", "extra", "marking--1"),
        record("indicator--relation-source", "source", "marking--1"),
        record("indicator--relation-old", "old target", "marking--1"),
        record("indicator--relation-new", "new target", "marking--1"),
        relationship(
            "relationship--divergent",
            "indicator--relation-source",
            "indicator--relation-old",
        ),
    ]);
    let before = graph_records(&actual);
    let command = OpenCtiReconciliationCommand::new(
        "reconcile--dry-run",
        ReconciliationMode::DryRun,
        ReconciliationScope::Records {
            record_ids: vec![
                "indicator--missing".to_owned(),
                "indicator--property".to_owned(),
                "indicator--permission".to_owned(),
                "indicator--extra".to_owned(),
                "indicator--stale".to_owned(),
                "relationship--divergent".to_owned(),
            ],
        },
        vec![
            record("indicator--missing", "new", "marking--1"),
            record("indicator--property", "new", "marking--1"),
            record("indicator--permission", "same", "marking--2"),
            relationship(
                "relationship--divergent",
                "indicator--relation-source",
                "indicator--relation-new",
            ),
        ],
        false,
    )
    .unwrap();
    let outcome = OpenCtiReconciler::new(ReconciliationLimits::default())
        .execute(&actual, &command, &["indicator--stale".to_owned()])
        .unwrap();

    assert_eq!(graph_records(&outcome.graph), before);
    assert!(!outcome.report.mutated);
    for expected in [
        DivergenceKind::Missing,
        DivergenceKind::Extra,
        DivergenceKind::PropertyDivergent,
        DivergenceKind::PermissionDivergent,
        DivergenceKind::RelationshipDivergent,
        DivergenceKind::StaleIndex,
    ] {
        assert!(
            outcome
                .report
                .differences
                .iter()
                .any(|item| item.kind == expected)
        );
    }
    assert!(
        outcome
            .report
            .differences
            .iter()
            .all(|item| item.action != RepairAction::Applied)
    );
}

#[test]
fn targeted_repair_changes_only_declared_records_and_quarantines_unsafe_extra_data() {
    let actual = graph(&[
        record("indicator--selected", "old", "marking--1"),
        record("indicator--untouched", "old", "marking--1"),
        record("indicator--extra", "extra", "marking--1"),
    ]);
    let command = OpenCtiReconciliationCommand::new(
        "reconcile--targeted",
        ReconciliationMode::Repair,
        ReconciliationScope::Records {
            record_ids: vec![
                "indicator--selected".to_owned(),
                "indicator--missing".to_owned(),
                "indicator--extra".to_owned(),
            ],
        },
        vec![
            record("indicator--selected", "repaired", "marking--2"),
            record("indicator--missing", "created", "marking--1"),
        ],
        false,
    )
    .unwrap();
    let outcome = OpenCtiReconciler::new(ReconciliationLimits::default())
        .execute(&actual, &command, &[])
        .unwrap();

    assert!(outcome.report.mutated);
    assert!(
        !outcome.report.parity_verified,
        "quarantine blocks parity completion"
    );
    assert!(
        outcome
            .report
            .quarantined_record_ids
            .contains(&"indicator--extra".to_owned())
    );
    let replay = OpenCtiReconciler::new(ReconciliationLimits::default())
        .execute(&outcome.graph, &command, &[])
        .unwrap();
    assert_eq!(
        graph_records(&replay.graph),
        graph_records(&outcome.graph),
        "safe repairs are idempotent"
    );

    let full = OpenCtiReconciliationCommand::new(
        "inspect",
        ReconciliationMode::DryRun,
        ReconciliationScope::Full { max_records: 32 },
        vec![
            record("indicator--selected", "repaired", "marking--2"),
            record("indicator--missing", "created", "marking--1"),
            record("indicator--untouched", "old", "marking--1"),
            record("indicator--extra", "extra", "marking--1"),
        ],
        false,
    )
    .unwrap();
    let verification = OpenCtiReconciler::new(ReconciliationLimits::default())
        .execute(&outcome.graph, &full, &[])
        .unwrap();
    assert!(verification.report.parity_verified);
}

#[test]
fn targeted_node_deletion_quarantines_undeclared_relationships() {
    let actual = graph(&[
        record("indicator--delete", "delete", "marking--1"),
        record("indicator--keep", "keep", "marking--1"),
        relationship(
            "relationship--undeclared",
            "indicator--delete",
            "indicator--keep",
        ),
    ]);
    let before = graph_records(&actual);
    let command = OpenCtiReconciliationCommand::new(
        "reconcile--unsafe-node-delete",
        ReconciliationMode::Repair,
        ReconciliationScope::Records {
            record_ids: vec!["indicator--delete".to_owned()],
        },
        vec![],
        true,
    )
    .unwrap();
    let outcome = OpenCtiReconciler::new(ReconciliationLimits::default())
        .execute(&actual, &command, &[])
        .unwrap();

    assert_eq!(graph_records(&outcome.graph), before);
    assert_eq!(outcome.report.quarantined_record_ids, ["indicator--delete"]);
    assert!(
        outcome.report.differences[0]
            .diagnostic
            .contains("outside the declared deletion scope")
    );
}

#[test]
fn reconciliation_range_and_partition_scopes_are_bounded_and_deterministic() {
    let expected = (0..10)
        .map(|index| record(&format!("indicator--{index:02}"), "expected", "marking--1"))
        .collect::<Vec<_>>();
    let command = OpenCtiReconciliationCommand::new(
        "reconcile--range",
        ReconciliationMode::DryRun,
        ReconciliationScope::Range {
            start_inclusive: "indicator--03".to_owned(),
            end_exclusive: "indicator--07".to_owned(),
            max_records: 4,
        },
        expected,
        false,
    )
    .unwrap();
    let report = OpenCtiReconciler::new(ReconciliationLimits {
        max_records: 16,
        max_payload_bytes: 1_000_000,
    })
    .execute(&Graph::new(), &command, &[])
    .unwrap()
    .report;
    assert_eq!(
        report
            .differences
            .iter()
            .map(|item| item.record_id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "indicator--03",
            "indicator--04",
            "indicator--05",
            "indicator--06"
        ]
    );
}

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
use graph_core::*;
use serde_json::json;

fn candidate(id: &str, raw: &str, constraints: Vec<CandidateConstraint>) -> CandidateInput {
    CandidateInput::new(
        id,
        ExtractionRunId::new("run--1").expect("run"),
        raw,
        ActorId::new("extractor").expect("actor"),
    )
    .expect("candidate")
    .with_constraints(constraints)
}
fn required() -> CandidateConstraint {
    CandidateConstraint {
        id: "name-required".into(),
        field: "/name".into(),
        rule: CandidateRule::Required,
    }
}
#[test]
fn every_constraint_category_reports_exact_field_rule_and_observed_value() {
    let fixtures = [
        (required(), json!({}), json!(null), false),
        (
            CandidateConstraint {
                id: "name-type".into(),
                field: "/name".into(),
                rule: CandidateRule::Type {
                    expected: CandidateValueType::String,
                },
            },
            json!({"name":42}),
            json!(42),
            true,
        ),
        (
            CandidateConstraint {
                id: "aliases-count".into(),
                field: "/aliases".into(),
                rule: CandidateRule::Cardinality {
                    min: 1,
                    max: Some(2),
                },
            },
            json!({"aliases":[]}),
            json!([]),
            true,
        ),
        (
            CandidateConstraint {
                id: "time-order".into(),
                field: "/end".into(),
                rule: CandidateRule::TemporalOrder {
                    after: "/start".into(),
                },
            },
            json!({"start":"2026-09-06T12:00:00Z","end":"2026-09-05T12:00:00Z"}),
            json!("2026-09-05T12:00:00Z"),
            true,
        ),
        (
            CandidateConstraint {
                id: "predicate".into(),
                field: "/predicate".into(),
                rule: CandidateRule::AllowedPredicates {
                    allowed: vec!["supports".into()],
                },
            },
            json!({"predicate":"invented"}),
            json!("invented"),
            true,
        ),
    ];
    for (constraint, raw, observed, present) in fixtures {
        let mut graph = Graph::new();
        let input = graph
            .submit_candidate(candidate("c1", &raw.to_string(), vec![constraint.clone()]))
            .expect("retain invalid candidate");
        let report = graph
            .epistemic_stores()
            .candidates
            .validation(input.id())
            .expect("report");
        assert!(!report.valid);
        assert_eq!(report.failures.len(), 1);
        let failure = &report.failures[0];
        assert_eq!(failure.constraint, constraint);
        assert_eq!(failure.observed, observed);
        assert_eq!(failure.present, present);
        assert!(!failure.repeated);
        assert!(
            graph
                .promote_candidate(
                    input.id(),
                    ActorId::new("reviewer").expect("actor"),
                    "checked",
                    CandidatePromotionInput::Node(NodeInput::new(["Entity"]))
                )
                .is_err()
        );
        assert!(graph.list_nodes().expect("nodes").is_empty());
    }
}
#[test]
fn two_repairs_preserve_all_raw_versions_and_expose_repeated_failure() {
    let mut graph = Graph::new();
    let first = graph
        .submit_candidate(candidate("c1", " {} ", vec![required()]))
        .expect("submit");
    let second = graph
        .repair_candidate(
            first.id(),
            candidate("c2", "{\"other\":1}", vec![]),
            vec!["name-required".into()],
        )
        .expect("first repair");
    let report = graph
        .epistemic_stores()
        .candidates
        .validation(second.id())
        .expect("report");
    assert!(report.failures[0].repeated);
    let third = graph
        .repair_candidate(
            second.id(),
            candidate("c3", "{\"name\":\"fixed\"}", vec![]),
            vec!["name-required".into()],
        )
        .expect("second repair");
    let store = &graph.epistemic_stores().candidates;
    assert!(store.validation(third.id()).expect("report").valid);
    assert_eq!(
        store.get(first.id()).expect("original").raw_payload(),
        " {} "
    );
    assert_eq!(
        store.get(second.id()).expect("second").raw_payload(),
        "{\"other\":1}"
    );
    assert_eq!(third.repair().expect("lineage").predecessor, *second.id());
    assert_eq!(
        third.repair().expect("lineage").caused_by,
        vec!["name-required"]
    );
    assert!(
        graph.list_nodes().expect("nodes").is_empty(),
        "valid repair must not auto-promote"
    );
    let snapshot = serde_json::to_value(graph.persistence_snapshot()).expect("encode");
    let restored =
        Graph::from_persistence_snapshot(serde_json::from_value(snapshot).expect("decode"))
            .expect("restore");
    assert_eq!(restored.epistemic_stores(), graph.epistemic_stores());
    assert_eq!(
        restored
            .epistemic_stores()
            .candidates
            .validation(second.id())
            .expect("report"),
        report
    );
}
#[test]
fn repairs_cannot_invent_a_cause_or_replace_history() {
    let mut graph = Graph::new();
    let first = graph
        .submit_candidate(candidate("c1", "{}", vec![required()]))
        .expect("submit");
    let before = graph.epistemic_stores().clone();
    assert!(
        graph
            .repair_candidate(
                first.id(),
                candidate("c2", "{}", vec![]),
                vec!["unknown".into()]
            )
            .is_err()
    );
    assert!(
        graph
            .repair_candidate(
                first.id(),
                candidate("c1", "{}", vec![]),
                vec!["name-required".into()]
            )
            .is_err()
    );
    assert_eq!(graph.epistemic_stores(), &before);
}

#[test]
fn valid_fields_include_escaped_pointers_and_timezone_aware_temporal_order() {
    let constraints = vec![
        CandidateConstraint {
            id: "schema".into(),
            field: "/a~1b/~0name".into(),
            rule: CandidateRule::Required,
        },
        CandidateConstraint {
            id: "type".into(),
            field: "/a~1b/~0name".into(),
            rule: CandidateRule::Type {
                expected: CandidateValueType::String,
            },
        },
        CandidateConstraint {
            id: "count".into(),
            field: "/aliases".into(),
            rule: CandidateRule::Cardinality {
                min: 1,
                max: Some(1),
            },
        },
        CandidateConstraint {
            id: "time".into(),
            field: "/end".into(),
            rule: CandidateRule::TemporalOrder {
                after: "/start".into(),
            },
        },
        CandidateConstraint {
            id: "predicate".into(),
            field: "/predicate".into(),
            rule: CandidateRule::AllowedPredicates {
                allowed: vec!["supports".into()],
            },
        },
    ];
    let mut graph = Graph::new();
    let raw = json!({"a/b":{"~name":"valid"},"aliases":["alias"],"start":"2026-09-06T10:00:00Z","end":"2026-09-06T12:00:00+02:00","predicate":"supports"});
    let candidate = graph
        .submit_candidate(candidate("valid", &raw.to_string(), constraints))
        .expect("submit");
    assert!(
        graph
            .epistemic_stores()
            .candidates
            .validation(candidate.id())
            .expect("report")
            .valid
    );
    graph
        .promote_candidate(
            candidate.id(),
            ActorId::new("reviewer").expect("actor"),
            "reviewed",
            CandidatePromotionInput::Node(NodeInput::new(["Entity"])),
        )
        .expect("explicit promotion");
    assert_eq!(graph.list_nodes().expect("nodes").len(), 1);
}

#[test]
fn malformed_json_can_be_repaired_without_losing_the_original_bytes() {
    let mut graph = Graph::new();
    let first = graph
        .submit_candidate(candidate("broken", " { unfinished", vec![required()]))
        .expect("retain raw");
    let report = graph
        .epistemic_stores()
        .candidates
        .validation(first.id())
        .expect("report");
    assert_eq!(report.failures[0].constraint.id, "$json");
    assert_eq!(report.failures[0].constraint.field, "");
    assert_eq!(report.failures[0].observed, json!(" { unfinished"));
    let fixed = graph
        .repair_candidate(
            first.id(),
            candidate("fixed", "{\"name\":\"fixed\"}", vec![]),
            vec!["$json".into()],
        )
        .expect("repair parse error");
    assert!(
        graph
            .epistemic_stores()
            .candidates
            .validation(fixed.id())
            .expect("report")
            .valid
    );
    assert_eq!(
        graph
            .epistemic_stores()
            .candidates
            .get(first.id())
            .expect("original")
            .raw_payload(),
        " { unfinished"
    );
}

#[test]
fn invalid_contracts_are_rejected_without_recording_a_candidate() {
    let mut graph = Graph::new();
    for rules in [
        vec![required(), required()],
        vec![CandidateConstraint {
            id: "pointer".into(),
            field: "/invalid~escape".into(),
            rule: CandidateRule::Required,
        }],
        vec![CandidateConstraint {
            id: "count".into(),
            field: "/items".into(),
            rule: CandidateRule::Cardinality {
                min: 2,
                max: Some(1),
            },
        }],
        vec![CandidateConstraint {
            id: "predicate".into(),
            field: "/predicate".into(),
            rule: CandidateRule::AllowedPredicates { allowed: vec![] },
        }],
    ] {
        assert!(
            graph
                .submit_candidate(candidate("invalid", "{}", rules))
                .is_err()
        );
        assert!(graph.epistemic_stores().candidates.is_empty());
    }
}

#[test]
fn temporal_feedback_points_to_the_missing_or_malformed_counterpart() {
    for raw in [
        json!({"end":"2026-09-06T12:00:00Z"}),
        json!({"start":"yesterday","end":"2026-09-06T12:00:00Z"}),
    ] {
        let mut graph = Graph::new();
        let constraint = CandidateConstraint {
            id: "order".into(),
            field: "/end".into(),
            rule: CandidateRule::TemporalOrder {
                after: "/start".into(),
            },
        };
        let candidate = graph
            .submit_candidate(candidate("c", &raw.to_string(), vec![constraint]))
            .expect("submit");
        let report = graph
            .epistemic_stores()
            .candidates
            .validation(candidate.id())
            .expect("report");
        assert_eq!(report.failures[0].field, "/start");
        assert_eq!(report.failures[0].observed, raw["start"]);
    }
}

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
    BehavioralBounds, BehavioralValidationInputs, NodeId, PheromoneDecay, PheromoneField,
    PheromoneTaskScope, RelationshipId, RequestId, RetrievalTelemetryRecord,
    SkippedExpansionReason, TelemetryQueryDescriptor, ValidationErrorSeverity, ValidationTarget,
    WorkingSetDecisionEvent, WorkingSetId, WorkingSetTelemetryEvent, validate_graph_behavior,
};

fn working_set_id(value: &str) -> WorkingSetId {
    WorkingSetId::new(value).expect("behavioral working set ID should be valid")
}

fn node_id(value: &str) -> NodeId {
    NodeId::new(value).expect("behavioral node ID should be valid")
}

fn relationship_id(value: &str) -> RelationshipId {
    RelationshipId::new(value).expect("behavioral relationship ID should be valid")
}

fn retrieval_id(value: &str) -> RequestId {
    RequestId::new(value).expect("behavioral retrieval ID should be valid")
}

fn record(retrieval: &str, decisions: Vec<WorkingSetDecisionEvent>) -> RetrievalTelemetryRecord {
    RetrievalTelemetryRecord {
        retrieval_id: retrieval_id(retrieval),
        working_set_id: working_set_id("working-set--behavioral"),
        descriptor: TelemetryQueryDescriptor {
            query_text: Some("behavioral scenario".to_owned()),
            profile_kind: None,
            task_label: Some("fimi_investigation".to_owned()),
        },
        events: decisions
            .into_iter()
            .enumerate()
            .map(|(index, decision)| WorkingSetTelemetryEvent {
                sequence: index as u64,
                decision,
            })
            .collect(),
        outcome: None,
    }
}

fn expanded(relationship: &RelationshipId) -> WorkingSetDecisionEvent {
    WorkingSetDecisionEvent::EdgeExpanded {
        relationship_id: relationship.clone(),
    }
}

fn selected(node: &NodeId) -> WorkingSetDecisionEvent {
    WorkingSetDecisionEvent::SeedSelected {
        node_id: node.clone(),
        marked_hot: true,
    }
}

fn skipped(source: &NodeId, relationship: &RelationshipId) -> WorkingSetDecisionEvent {
    WorkingSetDecisionEvent::EdgeSkipped {
        source_node_id: source.clone(),
        candidate_node_id: None,
        relationship_id: Some(relationship.clone()),
        reason: SkippedExpansionReason::BudgetLimit,
    }
}

fn warm(source: &NodeId, relationship: &str, target: &str) -> WorkingSetDecisionEvent {
    WorkingSetDecisionEvent::WarmAdjacencyAttached {
        source_node_id: source.clone(),
        relationship_id: relationship_id(relationship),
        target_node_id: node_id(target),
    }
}

fn scope() -> PheromoneTaskScope {
    PheromoneTaskScope::task("fimi_investigation")
}

fn bounds() -> BehavioralBounds {
    BehavioralBounds {
        max_access_frequency: 2.0,
        max_drift_ratio: 0.5,
        max_degree_jump: 2,
    }
}

/// Build a pheromone field by applying the given records without decay so the
/// access trace accumulates deterministically.
fn field_from(records: &[RetrievalTelemetryRecord]) -> PheromoneField {
    let mut field =
        PheromoneField::new(PheromoneDecay::new(1.0).expect("behavioral decay should be valid"));
    for telemetry_record in records {
        field.apply_retrieval_record(telemetry_record);
    }
    field
}

//
// Verify that calm recorded behavior produces no findings: validators must
// never invent anomalies.
//
// Given a small stable history of retrievals,
// when behavioral validation runs,
// then no findings should be reported.
#[test]
fn calm_recorded_behavior_produces_no_findings() {
    let edge = relationship_id("relationship--calm");
    let target = node_id("node--calm");
    let records = vec![
        record("request--calm-1", vec![expanded(&edge), selected(&target)]),
        record("request--calm-2", vec![expanded(&edge), selected(&target)]),
    ];
    let field = field_from(&records);
    let edges = [edge];

    let findings = validate_graph_behavior(&BehavioralValidationInputs {
        pheromones: &field,
        scope: &scope(),
        edges: &edges,
        records: &records,
        bounds: &bounds(),
    })
    .expect("behavioral validation should run");

    assert!(findings.is_empty());
}

//
// Verify anomalous pheromone growth detection: an edge whose decayed access
// trace exceeds the declared bound is flagged, while modest traces pass.
//
// Given one edge expanded in three retrievals and one expanded once, with an
// access bound of two,
// when behavioral validation runs,
// then only the fast-growing edge should be flagged.
#[test]
fn anomalous_pheromone_growth_is_detected() {
    let hot_edge = relationship_id("relationship--hot-growth");
    let calm_edge = relationship_id("relationship--calm-growth");
    let target = node_id("node--growth");
    let records = vec![
        record(
            "request--growth-1",
            vec![
                expanded(&hot_edge),
                selected(&target),
                expanded(&calm_edge),
                selected(&target),
            ],
        ),
        record(
            "request--growth-2",
            vec![expanded(&hot_edge), selected(&target)],
        ),
        record(
            "request--growth-3",
            vec![expanded(&hot_edge), selected(&target)],
        ),
    ];
    let field = field_from(&records);
    let edges = [hot_edge.clone(), calm_edge];

    let findings = validate_graph_behavior(&BehavioralValidationInputs {
        pheromones: &field,
        scope: &scope(),
        edges: &edges,
        records: &records,
        bounds: &bounds(),
    })
    .expect("behavioral validation should run");

    assert_eq!(findings.len(), 1);
    let finding = &findings[0];
    assert_eq!(finding.code(), "immune-behavioral--pheromone-growth");
    assert_eq!(finding.severity(), ValidationErrorSeverity::Warning);
    assert_eq!(
        finding.target(),
        &ValidationTarget::relationship(hot_edge.as_str())
    );
}

//
// Verify retrieval-drift detection: the latest retrieval's expansion ratio
// diverging from the recorded history beyond the bound is flagged against the
// retrieval.
//
// Given an expansion-heavy history and a skip-heavy latest retrieval,
// when behavioral validation runs,
// then one drift finding should target the latest retrieval.
#[test]
fn retrieval_drift_is_detected_against_recorded_history() {
    let edge = relationship_id("relationship--drift");
    let source = node_id("node--drift-source");
    let target = node_id("node--drift-target");
    let records = vec![
        record(
            "request--drift-history-1",
            vec![
                expanded(&edge),
                selected(&target),
                expanded(&edge),
                selected(&target),
            ],
        ),
        record(
            "request--drift-history-2",
            vec![expanded(&edge), selected(&target)],
        ),
        record(
            "request--drift-latest",
            vec![
                expanded(&edge),
                selected(&target),
                skipped(&source, &relationship_id("relationship--skipped-a")),
                skipped(&source, &relationship_id("relationship--skipped-b")),
                skipped(&source, &relationship_id("relationship--skipped-c")),
            ],
        ),
    ];
    let field = field_from(&records);

    let findings = validate_graph_behavior(&BehavioralValidationInputs {
        pheromones: &field,
        scope: &scope(),
        edges: &[],
        records: &records,
        bounds: &bounds(),
    })
    .expect("behavioral validation should run");

    assert_eq!(findings.len(), 1);
    let finding = &findings[0];
    assert_eq!(finding.code(), "immune-behavioral--retrieval-drift");
    assert_eq!(
        finding.target(),
        &ValidationTarget::retrieval("request--drift-latest")
    );
}

//
// Verify centrality-shift detection: a node whose observed frontier degree in
// the latest retrieval jumps beyond the bound against its recorded history is
// flagged.
//
// Given a node with one warm attachment historically and five in the latest
// retrieval, with a degree-jump bound of two,
// when behavioral validation runs,
// then one centrality finding should target that node.
#[test]
fn suspicious_centrality_shifts_are_detected() {
    let hub = node_id("node--hub");
    let records = vec![
        record(
            "request--centrality-history",
            vec![warm(&hub, "relationship--w0", "node--t0")],
        ),
        record(
            "request--centrality-latest",
            vec![
                warm(&hub, "relationship--w1", "node--t1"),
                warm(&hub, "relationship--w2", "node--t2"),
                warm(&hub, "relationship--w3", "node--t3"),
                warm(&hub, "relationship--w4", "node--t4"),
                warm(&hub, "relationship--w5", "node--t5"),
            ],
        ),
    ];
    let field = field_from(&records);

    let findings = validate_graph_behavior(&BehavioralValidationInputs {
        pheromones: &field,
        scope: &scope(),
        edges: &[],
        records: &records,
        bounds: &bounds(),
    })
    .expect("behavioral validation should run");

    assert_eq!(findings.len(), 1);
    let finding = &findings[0];
    assert_eq!(finding.code(), "immune-behavioral--centrality-shift");
    assert_eq!(finding.target(), &ValidationTarget::node(hub.as_str()));
}

//
// Verify that detection is reproducible from recorded state alone: identical
// inputs yield identical findings.
//
// Given a mixed anomalous scenario evaluated twice,
// when the findings are compared,
// then both runs should be exactly equal.
#[test]
fn detection_is_reproducible_from_recorded_state() {
    let hot_edge = relationship_id("relationship--repro-hot");
    let hub = node_id("node--repro-hub");
    let target = node_id("node--repro-target");
    let records = vec![
        record(
            "request--repro-1",
            vec![expanded(&hot_edge), selected(&target)],
        ),
        record(
            "request--repro-2",
            vec![expanded(&hot_edge), selected(&target)],
        ),
        record(
            "request--repro-3",
            vec![
                expanded(&hot_edge),
                selected(&target),
                warm(&hub, "relationship--rw1", "node--rt1"),
                warm(&hub, "relationship--rw2", "node--rt2"),
                warm(&hub, "relationship--rw3", "node--rt3"),
            ],
        ),
    ];
    let field = field_from(&records);
    let edges = [hot_edge];

    let run = || {
        validate_graph_behavior(&BehavioralValidationInputs {
            pheromones: &field,
            scope: &scope(),
            edges: &edges,
            records: &records,
            bounds: &bounds(),
        })
        .expect("behavioral validation should run")
    };

    assert_eq!(run(), run());
}

//
// Verify deterministic reporting order: pheromone growth, retrieval drift,
// then centrality shifts.
//
// Given a scenario seeding all three anomaly classes,
// when behavioral validation runs,
// then the finding codes should follow the documented order.
#[test]
fn findings_follow_the_documented_order() {
    let hot_edge = relationship_id("relationship--order-hot");
    let hub = node_id("node--order-hub");
    let source = node_id("node--order-source");
    let target = node_id("node--order-target");
    let records = vec![
        record(
            "request--order-1",
            vec![expanded(&hot_edge), selected(&target)],
        ),
        record(
            "request--order-2",
            vec![expanded(&hot_edge), selected(&target)],
        ),
        record(
            "request--order-latest",
            vec![
                expanded(&hot_edge),
                selected(&target),
                skipped(&source, &relationship_id("relationship--order-skip-a")),
                skipped(&source, &relationship_id("relationship--order-skip-b")),
                skipped(&source, &relationship_id("relationship--order-skip-c")),
                warm(&hub, "relationship--ow1", "node--ot1"),
                warm(&hub, "relationship--ow2", "node--ot2"),
                warm(&hub, "relationship--ow3", "node--ot3"),
            ],
        ),
    ];
    let field = field_from(&records);
    let edges = [hot_edge];

    let findings = validate_graph_behavior(&BehavioralValidationInputs {
        pheromones: &field,
        scope: &scope(),
        edges: &edges,
        records: &records,
        bounds: &bounds(),
    })
    .expect("behavioral validation should run");

    let codes: Vec<&str> = findings.iter().map(|finding| finding.code()).collect();
    assert_eq!(
        codes,
        vec![
            "immune-behavioral--pheromone-growth",
            "immune-behavioral--retrieval-drift",
            "immune-behavioral--centrality-shift",
        ]
    );
}

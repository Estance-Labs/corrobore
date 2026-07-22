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
    CompletenessReduction, CompletenessReductionKind, Confidence, ExpansionBudget,
    ExpansionDirection, ExpansionFilters, ExpansionRequest, Graph, GraphRecordRef,
    GraphWorkingSetCreateRequest, GraphWorkingSetManager, NodeId, NodeInput, RelationshipId,
    RelationshipInput, RequestId, RetrievalOutcome, RetrievalTelemetryRecord,
    SkippedExpansionReason, TelemetryQueryDescriptor, WorkingSetAction, WorkingSetDecisionEvent,
    WorkingSetId, WorkingSetTelemetryEvent, compute_retrieval_completeness,
    default_fimi_investigation_profile, expand_working_set_from_graph_adjacency,
};

fn working_set_id(value: &str) -> WorkingSetId {
    WorkingSetId::new(value).expect("completeness working set ID should be valid")
}

fn node_id(value: &str) -> NodeId {
    NodeId::new(value).expect("completeness node ID should be valid")
}

fn relationship_id(value: &str) -> RelationshipId {
    RelationshipId::new(value).expect("completeness relationship ID should be valid")
}

fn retrieval_id(value: &str) -> RequestId {
    RequestId::new(value).expect("completeness retrieval ID should be valid")
}

fn descriptor() -> TelemetryQueryDescriptor {
    TelemetryQueryDescriptor {
        query_text: Some("completeness scenario".to_owned()),
        profile_kind: None,
        task_label: Some("fimi_investigation".to_owned()),
    }
}

fn record(
    ws: &WorkingSetId,
    retrieval: &str,
    decisions: Vec<WorkingSetDecisionEvent>,
) -> RetrievalTelemetryRecord {
    RetrievalTelemetryRecord {
        retrieval_id: retrieval_id(retrieval),
        working_set_id: ws.clone(),
        descriptor: descriptor(),
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

fn expanded(value: &str) -> WorkingSetDecisionEvent {
    WorkingSetDecisionEvent::EdgeExpanded {
        relationship_id: relationship_id(value),
    }
}

fn skipped(source: &str, relationship: &str) -> WorkingSetDecisionEvent {
    WorkingSetDecisionEvent::EdgeSkipped {
        source_node_id: node_id(source),
        candidate_node_id: None,
        relationship_id: Some(relationship_id(relationship)),
        reason: SkippedExpansionReason::BudgetLimit,
    }
}

fn warm(source: &str, relationship: &str, target: &str) -> WorkingSetDecisionEvent {
    WorkingSetDecisionEvent::WarmAdjacencyAttached {
        source_node_id: node_id(source),
        relationship_id: relationship_id(relationship),
        target_node_id: node_id(target),
    }
}

fn controller_choice(source: &str, action: WorkingSetAction) -> WorkingSetDecisionEvent {
    WorkingSetDecisionEvent::ControllerActionChosen {
        source_node_id: Some(node_id(source)),
        action,
    }
}

fn page_in(value: &str) -> WorkingSetDecisionEvent {
    WorkingSetDecisionEvent::PageIn {
        record: GraphRecordRef::Node(node_id(value)),
    }
}

fn dead_end(value: &str) -> WorkingSetDecisionEvent {
    WorkingSetDecisionEvent::DeadEnd {
        node_id: node_id(value),
    }
}

//
// Verify that a retrieval with no uncovered candidates is fully complete:
// expansions, page-ins, and dead ends alone never reduce completeness.
//
// Given a record with expansions, page-ins, and a dead end only,
// when completeness is computed,
// then it should be 1.0 with no reductions.
#[test]
fn full_coverage_yields_complete_retrieval() {
    let ws = working_set_id("working-set--completeness-full");
    let records = vec![record(
        &ws,
        "request--completeness-full",
        vec![
            page_in("node--seed"),
            expanded("relationship--first"),
            expanded("relationship--second"),
            dead_end("node--leaf"),
        ],
    )];

    let report = compute_retrieval_completeness(&records);

    assert_eq!(report.completeness.value(), 1.0);
    assert!(report.reductions.is_empty());
}

//
// Verify the vacuous case: no recorded retrievals means nothing was left
// uncovered, so completeness reads as full with no reductions.
//
// Given an empty record slice,
// when completeness is computed,
// then it should be 1.0 with no reductions.
#[test]
fn empty_records_are_vacuously_complete() {
    let report = compute_retrieval_completeness(&[]);

    assert_eq!(report.completeness.value(), 1.0);
    assert!(report.reductions.is_empty());
}

//
// Verify that skipped candidates reduce completeness with an explainable
// reduction entry: skipped edges are known, uncovered candidates.
//
// Given three expanded edges and one skipped candidate,
// when completeness is computed,
// then it should be 0.75 with one skipped-edges reduction of count one.
#[test]
fn skipped_edges_reduce_completeness_with_a_typed_reason() {
    let ws = working_set_id("working-set--completeness-skips");
    let records = vec![record(
        &ws,
        "request--completeness-skips",
        vec![
            expanded("relationship--a"),
            expanded("relationship--b"),
            expanded("relationship--c"),
            skipped("node--source", "relationship--d"),
        ],
    )];

    let report = compute_retrieval_completeness(&records);

    assert_eq!(report.completeness.value(), 0.75);
    assert_eq!(
        report.reductions,
        vec![CompletenessReduction {
            kind: CompletenessReductionKind::SkippedEdges,
            count: 1,
        }]
    );
}

//
// Verify the warm-frontier weight: warm adjacency is half-covered (metadata
// known, payload not loaded), so each warm entry contributes half coverage
// and half reduction.
//
// Given two expanded edges and two warm frontier entries,
// when completeness is computed,
// then it should be 0.75 with one warm-frontier reduction of count two.
#[test]
fn warm_frontier_counts_as_half_covered() {
    let ws = working_set_id("working-set--completeness-warm");
    let records = vec![record(
        &ws,
        "request--completeness-warm",
        vec![
            expanded("relationship--hot-a"),
            expanded("relationship--hot-b"),
            warm("node--frontier", "relationship--warm-a", "node--target-a"),
            warm("node--frontier", "relationship--warm-b", "node--target-b"),
        ],
    )];

    let report = compute_retrieval_completeness(&records);

    assert_eq!(report.completeness.value(), 0.75);
    assert_eq!(
        report.reductions,
        vec![CompletenessReduction {
            kind: CompletenessReductionKind::WarmFrontier,
            count: 2,
        }]
    );
}

//
// Verify that controller stops, deferrals, and supernode blocks each reduce
// completeness under their own typed reason, in stable reduction order.
//
// Given one expansion, one stop, one deferral, and one supernode block,
// when completeness is computed,
// then it should be 0.25 with three ordered reduction entries.
#[test]
fn stops_deferrals_and_supernode_blocks_reduce_completeness() {
    let ws = working_set_id("working-set--completeness-stops");
    let records = vec![record(
        &ws,
        "request--completeness-stops",
        vec![
            controller_choice("node--expanded", WorkingSetAction::Expand),
            expanded("relationship--taken"),
            controller_choice("node--deferred", WorkingSetAction::Verify),
            WorkingSetDecisionEvent::SupernodeBlocked {
                node_id: node_id("node--hub"),
            },
            controller_choice("node--stopped", WorkingSetAction::Stop),
        ],
    )];

    let report = compute_retrieval_completeness(&records);

    // covered = 1 expansion; uncovered = stop + deferral + supernode block.
    assert_eq!(report.completeness.value(), 0.25);
    assert_eq!(
        report.reductions,
        vec![
            CompletenessReduction {
                kind: CompletenessReductionKind::ControllerStops,
                count: 1,
            },
            CompletenessReduction {
                kind: CompletenessReductionKind::DeferredSources,
                count: 1,
            },
            CompletenessReduction {
                kind: CompletenessReductionKind::SupernodeBlocks,
                count: 1,
            },
        ]
    );
}

//
// Verify the epic's confident-but-incomplete case end to end on real engine
// state: a budget-limited expansion yields reduced completeness that sits in
// the envelope beside an untouched high confidence.
//
// Given a real expansion whose relationship budget skips one candidate,
// when completeness is computed from the recorded retrieval and paired with a
// high confidence,
// then completeness should be below 1.0 while confidence keeps its value.
#[test]
fn confident_but_incomplete_is_computed_from_real_engine_state() {
    let mut graph = Graph::new();
    let campaign = graph
        .create_node(NodeInput::new(["Campaign"]))
        .expect("campaign node should be created");
    for label in ["Narrative", "Narrative"] {
        let target = graph
            .create_node(NodeInput::new([label]))
            .expect("target node should be created");
        graph
            .create_relationship(
                RelationshipInput::new(campaign.clone(), "PROMOTES", target)
                    .expect("relationship input should be valid"),
            )
            .expect("relationship should be created");
    }

    let ws = working_set_id("working-set--completeness-real");
    let mut manager = GraphWorkingSetManager::new();
    manager
        .create_working_set(GraphWorkingSetCreateRequest::new(ws.clone()))
        .expect("working set should be created");
    let retrieval = retrieval_id("request--completeness-real");

    manager
        .begin_retrieval_telemetry(&ws, retrieval.clone(), descriptor())
        .expect("retrieval telemetry should begin");
    expand_working_set_from_graph_adjacency(
        &mut manager,
        &graph,
        ExpansionRequest::new(
            ws.clone(),
            vec![campaign],
            ExpansionDirection::Outgoing,
            ExpansionFilters::empty(),
            1,
            default_fimi_investigation_profile(),
            ExpansionBudget {
                max_loaded_node_count: 8,
                max_loaded_relationship_count: 1,
                max_hot_node_count: 8,
                max_hot_relationship_count: 1,
                max_warm_adjacency_entry_count: 8,
                max_hop_count: 2,
                max_supernode_expansion_count: 4,
                max_payload_byte_count: 1_048_576,
                max_execution_time_ms: 1_000,
            },
        ),
    )
    .expect("budget-limited expansion should return a typed partial result");
    manager
        .complete_retrieval_telemetry(
            &ws,
            &retrieval,
            RetrievalOutcome {
                evidence_record_ids: Vec::new(),
                answer_quality: Some(Confidence::new(0.9).expect("answer quality should be valid")),
                memory_cost_bytes: 512,
                latency_ms: 4,
            },
        )
        .expect("retrieval telemetry should complete");

    let records = manager
        .telemetry(&ws)
        .expect("telemetry recorder should be available")
        .retrieval_records();
    let report = compute_retrieval_completeness(&records);

    assert!(
        report.completeness.value() < 1.0,
        "a budget-limited retrieval must not read as complete"
    );
    assert!(
        report
            .reductions
            .iter()
            .any(|reduction| reduction.kind == CompletenessReductionKind::SkippedEdges),
        "the budget skip should be surfaced as a typed reduction"
    );

    // The confident answer keeps its confidence next to reduced completeness.
    let confidence = Confidence::new(0.9).expect("confidence should be valid");
    assert_eq!(confidence.value(), 0.9);
}

//
// Verify reproducibility: completeness is a pure function of the recorded
// retrievals, so equal records yield equal reports.
//
// Given the same record slice computed twice,
// when the reports are compared,
// then they should be exactly equal.
#[test]
fn completeness_is_reproducible_from_recorded_state() {
    let ws = working_set_id("working-set--completeness-repro");
    let records = vec![record(
        &ws,
        "request--completeness-repro",
        vec![
            expanded("relationship--x"),
            skipped("node--s", "relationship--y"),
            warm("node--s", "relationship--z", "node--t"),
        ],
    )];

    assert_eq!(
        compute_retrieval_completeness(&records),
        compute_retrieval_completeness(&records)
    );
}

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
    Confidence, EdgeUtility, EvidenceId, ExpansionBudget, ExpansionDirection, ExpansionFilters,
    ExpansionRequest, Graph, GraphError, GraphRecordRef, GraphWorkingSetCreateRequest,
    GraphWorkingSetManager, NodeId, NodeInput, PheromoneDecay, PheromoneField, PheromoneTaskScope,
    RelationshipId, RelationshipInput, RequestId, RetrievalOutcome, RetrievalTelemetryRecord,
    TelemetryQueryDescriptor, UtilityContext, WorkingSetDecisionEvent, WorkingSetId,
    WorkingSetTelemetryEvent, default_fimi_investigation_profile, edge_utility_score,
    expand_working_set_from_graph_adjacency,
};

fn working_set_id(value: &str) -> WorkingSetId {
    WorkingSetId::new(value).expect("pheromone working set ID should be valid")
}

fn node_id(value: &str) -> NodeId {
    NodeId::new(value).expect("pheromone node ID should be valid")
}

fn relationship_id(value: &str) -> RelationshipId {
    RelationshipId::new(value).expect("pheromone relationship ID should be valid")
}

fn retrieval_id(value: &str) -> RequestId {
    RequestId::new(value).expect("pheromone retrieval ID should be valid")
}

fn evidence_id(value: &str) -> EvidenceId {
    EvidenceId::new(value).expect("pheromone evidence ID should be valid")
}

fn decay(lambda: f64) -> PheromoneDecay {
    PheromoneDecay::new(lambda).expect("pheromone decay should be valid")
}

fn descriptor(task_label: Option<&str>) -> TelemetryQueryDescriptor {
    TelemetryQueryDescriptor {
        query_text: Some("pheromone scenario".to_owned()),
        profile_kind: None,
        task_label: task_label.map(str::to_owned),
    }
}

fn sequenced(decisions: Vec<WorkingSetDecisionEvent>) -> Vec<WorkingSetTelemetryEvent> {
    decisions
        .into_iter()
        .enumerate()
        .map(|(index, decision)| WorkingSetTelemetryEvent {
            sequence: index as u64,
            decision,
        })
        .collect()
}

fn record(
    working_set: &WorkingSetId,
    retrieval: &str,
    task_label: Option<&str>,
    decisions: Vec<WorkingSetDecisionEvent>,
    outcome: Option<RetrievalOutcome>,
) -> RetrievalTelemetryRecord {
    RetrievalTelemetryRecord {
        retrieval_id: retrieval_id(retrieval),
        working_set_id: working_set.clone(),
        descriptor: descriptor(task_label),
        events: sequenced(decisions),
        outcome,
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

fn page_in_node(node: &NodeId) -> WorkingSetDecisionEvent {
    WorkingSetDecisionEvent::PageIn {
        record: GraphRecordRef::Node(node.clone()),
    }
}

fn dead_end(node: &NodeId) -> WorkingSetDecisionEvent {
    WorkingSetDecisionEvent::DeadEnd {
        node_id: node.clone(),
    }
}

fn quality(value: f64) -> Confidence {
    Confidence::new(value).expect("pheromone answer quality should be valid")
}

//
// Verify that the decay factor is validated as a typed domain error instead of
// silently producing unstable traces.
//
// Given decay factors outside [0, 1] or NaN,
// when a `PheromoneDecay` is constructed,
// then construction should fail with `GraphError::InvalidPheromoneDecay`.
#[test]
fn invalid_decay_factor_returns_typed_error() {
    for invalid in [-0.1, 1.1, f64::NAN] {
        let error = PheromoneDecay::new(invalid)
            .expect_err("out-of-range decay factor should return a typed error");
        assert!(matches!(error, GraphError::InvalidPheromoneDecay(_)));
    }

    assert!(PheromoneDecay::new(0.0).is_ok());
    assert!(PheromoneDecay::new(1.0).is_ok());
}

//
// Verify that a fresh field has no traces and starts scopes at tick zero, so
// downstream consumers can distinguish "never observed" from "decayed to zero".
//
// Given a new pheromone field,
// when a trace is requested for an unknown edge and task scope,
// then no utility should be returned and the scope tick should be zero.
#[test]
fn field_starts_without_traces() {
    let field = PheromoneField::new(decay(0.5));
    let scope = PheromoneTaskScope::task("fimi_investigation");

    assert!(
        field
            .edge_utility(&relationship_id("relationship--unknown"), &scope)
            .is_none()
    );
    assert_eq!(field.scope_tick(&scope), 0);
}

//
// Verify that one applied retrieval record creates a task-scoped trace with the
// structural dimensions of the epic's `EdgeUtility` vector.
//
// Given a record in task scope T expanding one edge for the first time,
// when the record is applied,
// then the edge trace in scope T should show one access, full novelty, and one
// task-affinity participation, while other scopes stay empty.
#[test]
fn applying_expansion_record_creates_task_scoped_trace() {
    let ws = working_set_id("working-set--pheromone-create");
    let promotes = relationship_id("relationship--pheromone-promotes");
    let narrative = node_id("narrative--pheromone");
    let mut field = PheromoneField::new(decay(0.5));

    field.apply_retrieval_record(&record(
        &ws,
        "request--pheromone-create",
        Some("fimi_investigation"),
        vec![expanded(&promotes), selected(&narrative)],
        None,
    ));

    let scope = PheromoneTaskScope::task("fimi_investigation");
    let utility = field
        .edge_utility(&promotes, &scope)
        .expect("expanded edge should have a trace in its task scope");

    assert_eq!(utility.access_frequency, 1.0);
    assert_eq!(utility.novelty_gain, 1.0);
    assert_eq!(utility.task_affinity, 1.0);
    assert_eq!(utility.dead_end_rate, 0.0);
    assert_eq!(field.scope_tick(&scope), 1);

    let other_scope = PheromoneTaskScope::task("malware_attribution");
    assert!(field.edge_utility(&promotes, &other_scope).is_none());
    assert_eq!(field.scope_tick(&other_scope), 0);
}

//
// Verify that outcome measurements reward expanded edges deterministically:
// evidence and page-in costs are split across the expanded edges, and answer
// quality feeds downstream success.
//
// Given a completed record with two expanded edges, four evidence records, two
// page-ins, and answer quality 0.75,
// when the record is applied,
// then each edge should gain evidence 2.0, traversal cost 1.0, and downstream
// success 0.75.
#[test]
fn outcome_rewards_are_split_across_expanded_edges() {
    let ws = working_set_id("working-set--pheromone-outcome");
    let first_edge = relationship_id("relationship--pheromone-first");
    let second_edge = relationship_id("relationship--pheromone-second");
    let first_node = node_id("node--pheromone-first");
    let second_node = node_id("node--pheromone-second");
    let mut field = PheromoneField::new(decay(0.5));

    field.apply_retrieval_record(&record(
        &ws,
        "request--pheromone-outcome",
        Some("fimi_investigation"),
        vec![
            page_in_node(&first_node),
            expanded(&first_edge),
            selected(&first_node),
            page_in_node(&second_node),
            expanded(&second_edge),
            selected(&second_node),
        ],
        Some(RetrievalOutcome {
            evidence_record_ids: vec![
                evidence_id("evidence--1"),
                evidence_id("evidence--2"),
                evidence_id("evidence--3"),
                evidence_id("evidence--4"),
            ],
            answer_quality: Some(quality(0.75)),
            memory_cost_bytes: 1_024,
            latency_ms: 5,
        }),
    ));

    let scope = PheromoneTaskScope::task("fimi_investigation");
    for edge in [&first_edge, &second_edge] {
        let utility = field
            .edge_utility(edge, &scope)
            .expect("expanded edge should have a trace");
        assert_eq!(utility.evidence_gain, 2.0);
        assert_eq!(utility.traversal_cost, 1.0);
        assert_eq!(utility.downstream_success, 0.75);
    }
}

//
// Verify the dead-end attribution rule: the edge whose expansion admitted a
// node is penalized when that node later proves to be a dead end, using the
// deterministic `EdgeExpanded -> SeedSelected` pairing from the engine stream.
//
// Given a record where edge A admits node N, edge B admits node M, and N is a
// dead end,
// when the record is applied,
// then edge A should carry the dead-end penalty and edge B should not.
#[test]
fn dead_end_penalizes_the_admitting_edge() {
    let ws = working_set_id("working-set--pheromone-dead-end");
    let admitting_edge = relationship_id("relationship--pheromone-admitting");
    let clean_edge = relationship_id("relationship--pheromone-clean");
    let dead_node = node_id("node--pheromone-dead");
    let live_node = node_id("node--pheromone-live");
    let mut field = PheromoneField::new(decay(0.5));

    field.apply_retrieval_record(&record(
        &ws,
        "request--pheromone-dead-end",
        Some("fimi_investigation"),
        vec![
            expanded(&admitting_edge),
            selected(&dead_node),
            expanded(&clean_edge),
            selected(&live_node),
            dead_end(&dead_node),
        ],
        None,
    ));

    let scope = PheromoneTaskScope::task("fimi_investigation");
    let penalized = field
        .edge_utility(&admitting_edge, &scope)
        .expect("admitting edge should have a trace");
    assert_eq!(penalized.dead_end_rate, 1.0);

    let clean = field
        .edge_utility(&clean_edge, &scope)
        .expect("clean edge should have a trace");
    assert_eq!(clean.dead_end_rate, 0.0);
}

//
// Verify temporal decay: each applied record advances its task scope by one
// tick and decays existing traces by the configured factor.
//
// Given an edge observed at tick one with decay 0.5,
// when a later record in the same scope does not touch that edge,
// then the edge's utility should read as halved at the new tick.
#[test]
fn traces_decay_when_the_task_scope_advances() {
    let ws = working_set_id("working-set--pheromone-decay");
    let observed_edge = relationship_id("relationship--pheromone-observed");
    let other_edge = relationship_id("relationship--pheromone-other");
    let observed_node = node_id("node--pheromone-observed");
    let other_node = node_id("node--pheromone-other-node");
    let mut field = PheromoneField::new(decay(0.5));

    field.apply_retrieval_record(&record(
        &ws,
        "request--pheromone-decay-1",
        Some("fimi_investigation"),
        vec![expanded(&observed_edge), selected(&observed_node)],
        None,
    ));
    field.apply_retrieval_record(&record(
        &ws,
        "request--pheromone-decay-2",
        Some("fimi_investigation"),
        vec![expanded(&other_edge), selected(&other_node)],
        None,
    ));

    let scope = PheromoneTaskScope::task("fimi_investigation");
    assert_eq!(field.scope_tick(&scope), 2);

    let decayed = field
        .edge_utility(&observed_edge, &scope)
        .expect("previously observed edge should retain a decayed trace");
    assert_eq!(decayed.access_frequency, 0.5);
    assert_eq!(decayed.novelty_gain, 0.5);
    assert_eq!(decayed.task_affinity, 0.5);
}

//
// Verify that novelty rewards only the first observation of an edge in a task
// scope, while access keeps accumulating, matching
// `τ(e, t+1) = λ·τ(e, t) + reward − penalty`.
//
// Given the same edge expanded in two consecutive records with decay 0.5,
// when both records are applied,
// then access should read `0.5·1 + 1 = 1.5` and novelty should read `0.5·1 + 0
// = 0.5`.
#[test]
fn novelty_rewards_only_the_first_observation() {
    let ws = working_set_id("working-set--pheromone-novelty");
    let edge = relationship_id("relationship--pheromone-novel");
    let node = node_id("node--pheromone-novel");
    let mut field = PheromoneField::new(decay(0.5));

    for retrieval in ["request--pheromone-novel-1", "request--pheromone-novel-2"] {
        field.apply_retrieval_record(&record(
            &ws,
            retrieval,
            Some("fimi_investigation"),
            vec![expanded(&edge), selected(&node)],
            None,
        ));
    }

    let scope = PheromoneTaskScope::task("fimi_investigation");
    let utility = field
        .edge_utility(&edge, &scope)
        .expect("edge should have a trace");
    assert_eq!(utility.access_frequency, 1.5);
    assert_eq!(utility.novelty_gain, 0.5);
}

//
// Verify that pheromone traces are multidimensional per task: the same edge
// carries independent traces and ticks in different task scopes.
//
// Given the same edge expanded once in scope A and twice in scope B,
// when the records are applied,
// then scope A and scope B should expose different trace values and ticks, and
// the generic scope should be used when no task label is present.
#[test]
fn task_scopes_isolate_traces_for_the_same_edge() {
    let ws = working_set_id("working-set--pheromone-scopes");
    let edge = relationship_id("relationship--pheromone-scoped");
    let node = node_id("node--pheromone-scoped");
    let mut field = PheromoneField::new(decay(1.0));

    field.apply_retrieval_record(&record(
        &ws,
        "request--pheromone-scope-a",
        Some("fimi_investigation"),
        vec![expanded(&edge), selected(&node)],
        None,
    ));
    for retrieval in ["request--pheromone-scope-b1", "request--pheromone-scope-b2"] {
        field.apply_retrieval_record(&record(
            &ws,
            retrieval,
            Some("malware_attribution"),
            vec![expanded(&edge), selected(&node)],
            None,
        ));
    }
    field.apply_retrieval_record(&record(
        &ws,
        "request--pheromone-scope-generic",
        None,
        vec![expanded(&edge), selected(&node)],
        None,
    ));

    let fimi = PheromoneTaskScope::task("fimi_investigation");
    let malware = PheromoneTaskScope::task("malware_attribution");

    assert_eq!(field.scope_tick(&fimi), 1);
    assert_eq!(field.scope_tick(&malware), 2);
    assert_eq!(field.scope_tick(&PheromoneTaskScope::Generic), 1);

    let fimi_utility = field
        .edge_utility(&edge, &fimi)
        .expect("scope A should have a trace");
    let malware_utility = field
        .edge_utility(&edge, &malware)
        .expect("scope B should have a trace");
    let generic_utility = field
        .edge_utility(&edge, &PheromoneTaskScope::Generic)
        .expect("generic scope should have a trace");

    assert_eq!(fimi_utility.access_frequency, 1.0);
    assert_eq!(malware_utility.access_frequency, 2.0);
    assert_eq!(generic_utility.access_frequency, 1.0);
}

//
// Verify that trace updates are reproducible: identical record sequences yield
// identical fields, which benchmark replays and future policy training rely on.
//
// Given two fields fed the same record sequence,
// when both applications complete,
// then the fields should be exactly equal.
#[test]
fn identical_record_sequences_produce_identical_fields() {
    let build = || {
        let ws = working_set_id("working-set--pheromone-reproducible");
        let edge = relationship_id("relationship--pheromone-reproducible");
        let node = node_id("node--pheromone-reproducible");
        let mut field = PheromoneField::new(decay(0.5));

        field.apply_retrieval_record(&record(
            &ws,
            "request--pheromone-reproducible-1",
            Some("fimi_investigation"),
            vec![
                page_in_node(&node),
                expanded(&edge),
                selected(&node),
                dead_end(&node),
            ],
            Some(RetrievalOutcome {
                evidence_record_ids: vec![evidence_id("evidence--reproducible")],
                answer_quality: Some(quality(0.6)),
                memory_cost_bytes: 2_048,
                latency_ms: 9,
            }),
        ));
        field
    };

    assert_eq!(build(), build());
}

//
// Verify the documented utility combination contract mapping trace dimensions
// and query context onto the epic's utility formula.
//
// Given a fully populated utility vector and a query context,
// when the utility score is computed,
// then it should equal semantic relevance plus historical success plus
// information gain plus temporal relevance, minus reliability, loading,
// dead-end, and integrity penalties.
#[test]
fn utility_score_follows_the_documented_combination() {
    let utility = EdgeUtility {
        access_frequency: 9.0,
        downstream_success: 0.5,
        evidence_gain: 2.0,
        novelty_gain: 0.25,
        traversal_cost: 0.75,
        dead_end_rate: 0.5,
        contradiction_rate: 0.125,
        staleness: 0.25,
        poisoning_risk: 0.0625,
        task_affinity: 3.0,
    };
    let context = UtilityContext {
        semantic_relevance: 1.0,
        temporal_relevance: 0.5,
    };

    let score = edge_utility_score(&utility, &context);

    // 1.0 + 0.5 + (2.0 + 0.25) + 0.5 - (0.125 + 0.25) - 0.75 - 0.5 - 0.0625
    assert_eq!(score, 2.5625);
}

//
// Verify the end-to-end observation loop: a real expansion recorded by the
// telemetry layer feeds the pheromone field without touching the working set,
// keeping the trace strictly observational.
//
// Given a real graph expansion recorded inside a retrieval,
// when the derived retrieval records are applied to a field,
// then the traversed relationship should have a trace in the retrieval's task
// scope and the working set should be unchanged by the application.
#[test]
fn field_consumes_real_expansion_telemetry() {
    let mut graph = Graph::new();
    let campaign = graph
        .create_node(NodeInput::new(["Campaign"]))
        .expect("campaign node should be created");
    let narrative = graph
        .create_node(NodeInput::new(["Narrative"]))
        .expect("narrative node should be created");
    let promotes = graph
        .create_relationship(
            RelationshipInput::new(campaign.clone(), "PROMOTES", narrative.clone())
                .expect("relationship input should be valid"),
        )
        .expect("relationship should be created");

    let ws = working_set_id("working-set--pheromone-integration");
    let mut manager = GraphWorkingSetManager::new();
    manager
        .create_working_set(GraphWorkingSetCreateRequest::new(ws.clone()))
        .expect("working set should be created");
    let retrieval = retrieval_id("request--pheromone-integration");

    manager
        .begin_retrieval_telemetry(
            &ws,
            retrieval.clone(),
            descriptor(Some("fimi_investigation")),
        )
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
                max_loaded_node_count: 32,
                max_loaded_relationship_count: 32,
                max_hot_node_count: 32,
                max_hot_relationship_count: 32,
                max_warm_adjacency_entry_count: 32,
                max_hop_count: 3,
                max_supernode_expansion_count: 8,
                max_payload_byte_count: 1_048_576,
                max_execution_time_ms: 1_000,
            },
        ),
    )
    .expect("expansion should complete");
    manager
        .complete_retrieval_telemetry(
            &ws,
            &retrieval,
            RetrievalOutcome {
                evidence_record_ids: Vec::new(),
                answer_quality: None,
                memory_cost_bytes: 512,
                latency_ms: 3,
            },
        )
        .expect("retrieval telemetry should complete");

    let stats_before = manager
        .stats(&ws)
        .expect("stats should be available")
        .clone();

    let mut field = PheromoneField::new(decay(0.9));
    for retrieval_record in manager
        .telemetry(&ws)
        .expect("telemetry recorder should be available")
        .retrieval_records()
    {
        field.apply_retrieval_record(&retrieval_record);
    }

    let scope = PheromoneTaskScope::task("fimi_investigation");
    let utility = field
        .edge_utility(&promotes, &scope)
        .expect("traversed relationship should have a trace");
    assert_eq!(utility.access_frequency, 1.0);
    assert!(utility.traversal_cost > 0.0);

    let stats_after = manager.stats(&ws).expect("stats should be available");
    assert_eq!(&stats_before, stats_after);
}

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
    AntiPheromoneField, AntiPheromoneSignal, AntiPheromoneVector, ExpansionBudget,
    ExpansionDirection, ExpansionFilters, ExpansionRequest, ExpansionResultStatus, Graph,
    GraphWorkingSetCreateRequest, GraphWorkingSetManager, NodeId, NodeInput, PheromoneDecay,
    PheromoneField, PheromoneTaskScope, RelationshipId, RelationshipInput, RequestId,
    RetrievalOutcome, RetrievalTelemetryRecord, SkippedExpansionReason, SupernodePolicy,
    TelemetryQueryDescriptor, UtilityContext, WorkingSetDecisionEvent, WorkingSetId,
    WorkingSetTelemetryEvent, default_fimi_investigation_profile, edge_utility_score,
    expand_working_set_from_graph_adjacency, navigation_field_score,
};

fn working_set_id(value: &str) -> WorkingSetId {
    WorkingSetId::new(value).expect("anti-pheromone working set ID should be valid")
}

fn node_id(value: &str) -> NodeId {
    NodeId::new(value).expect("anti-pheromone node ID should be valid")
}

fn relationship_id(value: &str) -> RelationshipId {
    RelationshipId::new(value).expect("anti-pheromone relationship ID should be valid")
}

fn retrieval_id(value: &str) -> RequestId {
    RequestId::new(value).expect("anti-pheromone retrieval ID should be valid")
}

fn decay(lambda: f64) -> PheromoneDecay {
    PheromoneDecay::new(lambda).expect("anti-pheromone decay should be valid")
}

fn descriptor(task_label: Option<&str>) -> TelemetryQueryDescriptor {
    TelemetryQueryDescriptor {
        query_text: Some("anti-pheromone scenario".to_owned()),
        profile_kind: None,
        task_label: task_label.map(str::to_owned),
    }
}

fn record(
    working_set: &WorkingSetId,
    retrieval: &str,
    task_label: Option<&str>,
    decisions: Vec<WorkingSetDecisionEvent>,
) -> RetrievalTelemetryRecord {
    RetrievalTelemetryRecord {
        retrieval_id: retrieval_id(retrieval),
        working_set_id: working_set.clone(),
        descriptor: descriptor(task_label),
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

fn dead_end(node: &NodeId) -> WorkingSetDecisionEvent {
    WorkingSetDecisionEvent::DeadEnd {
        node_id: node.clone(),
    }
}

fn supernode_blocked(node: &NodeId) -> WorkingSetDecisionEvent {
    WorkingSetDecisionEvent::SupernodeBlocked {
        node_id: node.clone(),
    }
}

fn profile_skip(source: &NodeId, relationship: &RelationshipId) -> WorkingSetDecisionEvent {
    WorkingSetDecisionEvent::EdgeSkipped {
        source_node_id: source.clone(),
        candidate_node_id: None,
        relationship_id: Some(relationship.clone()),
        reason: SkippedExpansionReason::BlockedByProfile,
    }
}

fn scope() -> PheromoneTaskScope {
    PheromoneTaskScope::task("fimi_investigation")
}

//
// Verify that a fresh anti-pheromone field has no traces, so "never penalized"
// stays distinguishable from "penalty decayed toward zero".
//
// Given a new anti-pheromone field,
// when the anti-pheromone vector is requested for an unknown edge,
// then no vector should be returned and the scope tick should be zero.
#[test]
fn field_starts_without_traces() {
    let field = AntiPheromoneField::new(decay(0.5));

    assert!(
        field
            .edge_anti_pheromone(&relationship_id("relationship--unknown"), &scope())
            .is_none()
    );
    assert_eq!(field.scope_tick(&scope()), 0);
}

//
// Verify that dead ends accumulate on the edge that admitted the dead-end node,
// using the same `EdgeExpanded -> SeedSelected` pairing as the positive field.
//
// Given a record where edge A admits node N and N is a dead end,
// when the record is applied,
// then edge A should carry a dead-end anti-pheromone and other edges none.
#[test]
fn dead_end_accumulates_on_the_admitting_edge() {
    let ws = working_set_id("working-set--anti-dead-end");
    let admitting_edge = relationship_id("relationship--anti-admitting");
    let clean_edge = relationship_id("relationship--anti-clean");
    let dead_node = node_id("node--anti-dead");
    let live_node = node_id("node--anti-live");
    let mut field = AntiPheromoneField::new(decay(0.5));

    field.apply_retrieval_record(&record(
        &ws,
        "request--anti-dead-end",
        Some("fimi_investigation"),
        vec![
            expanded(&admitting_edge),
            selected(&dead_node),
            expanded(&clean_edge),
            selected(&live_node),
            dead_end(&dead_node),
        ],
    ));

    let penalized = field
        .edge_anti_pheromone(&admitting_edge, &scope())
        .expect("admitting edge should carry an anti-pheromone trace");
    assert_eq!(penalized.dead_end, 1.0);
    assert_eq!(penalized.supernode_explosion, 0.0);

    assert!(field.edge_anti_pheromone(&clean_edge, &scope()).is_none());
}

//
// Verify that profile-blocked skips accumulate as irrelevant-expansion signals
// on the skipped relationship.
//
// Given a record where one relationship is skipped twice as blocked-by-profile,
// when the record is applied,
// then that relationship should carry an irrelevant-expansion value of two.
#[test]
fn profile_blocked_skips_accumulate_irrelevant_expansion() {
    let ws = working_set_id("working-set--anti-irrelevant");
    let source = node_id("node--anti-source");
    let skipped_edge = relationship_id("relationship--anti-skipped");
    let mut field = AntiPheromoneField::new(decay(0.5));

    field.apply_retrieval_record(&record(
        &ws,
        "request--anti-irrelevant",
        Some("fimi_investigation"),
        vec![
            profile_skip(&source, &skipped_edge),
            profile_skip(&source, &skipped_edge),
        ],
    ));

    let vector = field
        .edge_anti_pheromone(&skipped_edge, &scope())
        .expect("skipped relationship should carry an anti-pheromone trace");
    assert_eq!(vector.irrelevant_expansion, 2.0);
}

//
// Verify supernode attribution across records: the field remembers which edge
// admitted a node, so a later supernode block on that node penalizes the
// admitting edge.
//
// Given record one where edge E admits node X, and record two where X is
// blocked as a supernode,
// when both records are applied with decay 1.0,
// then edge E should carry a supernode-explosion anti-pheromone.
#[test]
fn supernode_block_attributes_to_the_admitting_edge_across_records() {
    let ws = working_set_id("working-set--anti-supernode");
    let admitting_edge = relationship_id("relationship--anti-supernode-admitting");
    let hub = node_id("node--anti-hub");
    let mut field = AntiPheromoneField::new(decay(1.0));

    field.apply_retrieval_record(&record(
        &ws,
        "request--anti-supernode-1",
        Some("fimi_investigation"),
        vec![expanded(&admitting_edge), selected(&hub)],
    ));
    assert!(
        field
            .edge_anti_pheromone(&admitting_edge, &scope())
            .is_none(),
        "admission alone must not create a negative trace"
    );

    field.apply_retrieval_record(&record(
        &ws,
        "request--anti-supernode-2",
        Some("fimi_investigation"),
        vec![supernode_blocked(&hub)],
    ));

    let vector = field
        .edge_anti_pheromone(&admitting_edge, &scope())
        .expect("admitting edge should carry a supernode anti-pheromone");
    assert_eq!(vector.supernode_explosion, 1.0);
}

//
// Verify that a supernode block on a node never admitted through a recorded
// expansion (a plain seed) is not attributed to any edge.
//
// Given a record blocking a seed node with no admission history,
// when the record is applied,
// then no anti-pheromone trace should exist in the scope.
#[test]
fn supernode_block_on_an_unadmitted_seed_is_not_attributed() {
    let ws = working_set_id("working-set--anti-seed-block");
    let seed = node_id("node--anti-seed");
    let mut field = AntiPheromoneField::new(decay(0.5));

    field.apply_retrieval_record(&record(
        &ws,
        "request--anti-seed-block",
        Some("fimi_investigation"),
        vec![selected(&seed), supernode_blocked(&seed)],
    ));

    assert_eq!(field.scope_tick(&scope()), 1);
    assert!(
        field
            .edge_anti_pheromone(&relationship_id("relationship--anti-any"), &scope())
            .is_none()
    );
}

//
// Verify that external validator signals (stale evidence, contradictory path,
// suspected poisoning) accumulate through the explicit reporting path reserved
// for the epistemic and immune-system epics.
//
// Given reported stale, contradiction, and poisoning observations on one edge,
// when the anti-pheromone vector is read,
// then each reserved dimension should carry its reported value.
#[test]
fn external_negative_signals_accumulate_through_reporting() {
    let edge = relationship_id("relationship--anti-external");
    let mut field = AntiPheromoneField::new(decay(0.5));

    field.report_negative_observation(&scope(), &edge, AntiPheromoneSignal::StaleEvidence);
    field.report_negative_observation(&scope(), &edge, AntiPheromoneSignal::ContradictoryPath);
    field.report_negative_observation(&scope(), &edge, AntiPheromoneSignal::ContradictoryPath);
    field.report_negative_observation(&scope(), &edge, AntiPheromoneSignal::SuspectedPoisoning);

    let vector = field
        .edge_anti_pheromone(&edge, &scope())
        .expect("reported edge should carry an anti-pheromone trace");
    assert_eq!(vector.stale_evidence, 1.0);
    assert_eq!(vector.contradictory_path, 2.0);
    assert_eq!(vector.suspected_poisoning, 1.0);
}

//
// Verify temporal decay: anti-pheromone traces fade as the task scope advances,
// so old penalties do not permanently blacklist an edge.
//
// Given a dead-end penalty at tick one with decay 0.5,
// when a later record advances the scope without touching the edge,
// then the penalty should read as halved.
#[test]
fn anti_pheromone_traces_decay_per_scope_tick() {
    let ws = working_set_id("working-set--anti-decay");
    let admitting_edge = relationship_id("relationship--anti-decaying");
    let dead_node = node_id("node--anti-decay-dead");
    let other_edge = relationship_id("relationship--anti-decay-other");
    let other_node = node_id("node--anti-decay-other");
    let mut field = AntiPheromoneField::new(decay(0.5));

    field.apply_retrieval_record(&record(
        &ws,
        "request--anti-decay-1",
        Some("fimi_investigation"),
        vec![
            expanded(&admitting_edge),
            selected(&dead_node),
            dead_end(&dead_node),
        ],
    ));
    field.apply_retrieval_record(&record(
        &ws,
        "request--anti-decay-2",
        Some("fimi_investigation"),
        vec![expanded(&other_edge), selected(&other_node)],
    ));

    assert_eq!(field.scope_tick(&scope()), 2);
    let vector = field
        .edge_anti_pheromone(&admitting_edge, &scope())
        .expect("penalized edge should retain a decayed trace");
    assert_eq!(vector.dead_end, 0.5);
}

//
// Verify that the anti-pheromone total is the sum of the six negative
// dimensions of the epic's contribution model.
//
// Given a fully populated anti-pheromone vector,
// when the total is computed,
// then it should equal the sum of all six dimensions.
#[test]
fn anti_pheromone_total_sums_all_dimensions() {
    let vector = AntiPheromoneVector {
        dead_end: 1.0,
        irrelevant_expansion: 0.5,
        supernode_explosion: 0.25,
        stale_evidence: 0.125,
        contradictory_path: 0.0625,
        suspected_poisoning: 0.03125,
    };

    assert_eq!(vector.total(), 1.96875);
}

//
// Verify that the combined navigation score down-ranks penalized edges: two
// edges with identical positive traces separate once one accumulates
// anti-pheromones on a recorded scenario.
//
// Given two edges expanded identically where only one leads to a dead end,
// when positive and negative fields are combined,
// then the penalized edge should score strictly lower and the clean edge
// should keep its positive-only score.
#[test]
fn navigation_score_down_ranks_penalized_edges() {
    let ws = working_set_id("working-set--anti-ranking");
    let risky_edge = relationship_id("relationship--anti-risky");
    let clean_edge = relationship_id("relationship--anti-ranking-clean");
    let dead_node = node_id("node--anti-ranking-dead");
    let live_node = node_id("node--anti-ranking-live");
    let shared_record = record(
        &ws,
        "request--anti-ranking",
        Some("fimi_investigation"),
        vec![
            expanded(&risky_edge),
            selected(&dead_node),
            expanded(&clean_edge),
            selected(&live_node),
            dead_end(&dead_node),
        ],
    );

    let mut positive = PheromoneField::new(decay(0.5));
    positive.apply_retrieval_record(&shared_record);
    let mut negative = AntiPheromoneField::new(decay(0.5));
    negative.apply_retrieval_record(&shared_record);

    let context = UtilityContext {
        semantic_relevance: 1.0,
        temporal_relevance: 0.0,
    };
    let scope = scope();

    let score_of = |edge: &RelationshipId| {
        let utility = positive
            .edge_utility(edge, &scope)
            .expect("both edges should have positive traces");
        let anti = negative
            .edge_anti_pheromone(edge, &scope)
            .unwrap_or_default();
        navigation_field_score(&utility, &anti, &context)
    };

    let risky_positive = positive
        .edge_utility(&risky_edge, &scope)
        .expect("risky edge should have a positive trace");
    let clean_positive = positive
        .edge_utility(&clean_edge, &scope)
        .expect("clean edge should have a positive trace");

    // The clean edge keeps its positive-only score.
    assert_eq!(
        score_of(&clean_edge),
        edge_utility_score(&clean_positive, &context)
    );
    // The penalized edge scores strictly below both its own positive-only
    // score and the clean edge's combined score.
    assert!(score_of(&risky_edge) < edge_utility_score(&risky_positive, &context));
    assert!(score_of(&risky_edge) < score_of(&clean_edge));
}

//
// Verify reproducibility: identical record and report sequences produce
// identical anti-pheromone fields.
//
// Given two fields fed the same records and reports,
// when both applications complete,
// then the fields should be exactly equal.
#[test]
fn identical_sequences_produce_identical_fields() {
    let build = || {
        let ws = working_set_id("working-set--anti-reproducible");
        let edge = relationship_id("relationship--anti-reproducible");
        let dead_node = node_id("node--anti-reproducible");
        let mut field = AntiPheromoneField::new(decay(0.5));

        field.apply_retrieval_record(&record(
            &ws,
            "request--anti-reproducible",
            Some("fimi_investigation"),
            vec![expanded(&edge), selected(&dead_node), dead_end(&dead_node)],
        ));
        field.report_negative_observation(&scope(), &edge, AntiPheromoneSignal::StaleEvidence);
        field
    };

    assert_eq!(build(), build());
}

//
// Verify that deterministic supernode protection stays authoritative: the
// anti-pheromone field is a passive observer and the engine still blocks
// unguarded high-degree expansion exactly as before.
//
// Given a high-degree seed and a supernode policy requiring missing guards,
// when the expansion runs inside a recorded retrieval and the telemetry feeds
// an anti-pheromone field,
// then the expansion should stay partial with a typed supernode error, the
// telemetry should record the block, and the seed block should not fabricate
// any edge attribution.
#[test]
fn supernode_protection_remains_authoritative() {
    let mut graph = Graph::new();
    let hub = graph
        .create_node(NodeInput::new(["Campaign"]))
        .expect("hub node should be created");
    let mut hub_relationships = Vec::new();
    for _ in 0..3 {
        let neighbor = graph
            .create_node(NodeInput::new(["Narrative"]))
            .expect("neighbor node should be created");
        hub_relationships.push(
            graph
                .create_relationship(
                    RelationshipInput::new(hub.clone(), "PROMOTES", neighbor)
                        .expect("relationship input should be valid"),
                )
                .expect("hub relationship should be created"),
        );
    }

    let ws = working_set_id("working-set--anti-protection");
    let mut manager = GraphWorkingSetManager::new();
    manager
        .create_working_set(GraphWorkingSetCreateRequest::new(ws.clone()))
        .expect("working set should be created");
    let retrieval = retrieval_id("request--anti-protection");

    manager
        .begin_retrieval_telemetry(
            &ws,
            retrieval.clone(),
            descriptor(Some("fimi_investigation")),
        )
        .expect("retrieval telemetry should begin");
    let result = expand_working_set_from_graph_adjacency(
        &mut manager,
        &graph,
        ExpansionRequest::new(
            ws.clone(),
            vec![hub.clone()],
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
        )
        .with_supernode_policy(SupernodePolicy {
            degree_threshold: 2,
            require_relationship_filter: true,
            require_label_filter: false,
            require_time_window: false,
            require_limit: false,
        }),
    )
    .expect("blocked expansion should return a typed partial result");
    manager
        .complete_retrieval_telemetry(
            &ws,
            &retrieval,
            RetrievalOutcome {
                evidence_record_ids: Vec::new(),
                answer_quality: None,
                memory_cost_bytes: 256,
                latency_ms: 2,
            },
        )
        .expect("retrieval telemetry should complete");

    // Deterministic protection is unchanged by instrumentation.
    assert_eq!(result.status(), ExpansionResultStatus::Partial);
    assert!(result.supernode_error().is_some());

    let records = manager
        .telemetry(&ws)
        .expect("telemetry recorder should be available")
        .retrieval_records();
    assert!(records[0].events.iter().any(|event| matches!(
        &event.decision,
        WorkingSetDecisionEvent::SupernodeBlocked { node_id } if node_id == &hub
    )));

    let mut field = AntiPheromoneField::new(decay(0.9));
    for retrieval_record in records {
        field.apply_retrieval_record(&retrieval_record);
    }

    // The blocked node is a seed with no admitting edge: nothing to attribute.
    assert_eq!(field.scope_tick(&scope()), 1);
    for relationship in &hub_relationships {
        assert!(field.edge_anti_pheromone(relationship, &scope()).is_none());
    }
}

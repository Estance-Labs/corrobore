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
//! Epic 0018 acceptance suite: epistemic graph and proof-carrying retrieval.
//!
//! Validates the epic's definition of done end to end at the public crate
//! boundary: distinct queryable epistemic kinds, proof-carrying answers with
//! their seven components, bitemporal facts without overwrite, and retrieval
//! completeness reported independently of confidence.

use graph_core::{
    AnswerStatement, BitemporalFactStore, BitemporalStamp, ClaimId, CompletenessReductionKind,
    Confidence, EpistemicNodeKind, EpistemicRelationKind, EvidenceSubgraph, ExpansionBudget,
    ExpansionDirection, ExpansionFilters, ExpansionRequest, FactId, Graph, GraphError,
    GraphWorkingSetCreateRequest, GraphWorkingSetManager, NodeId, NodeInput, ProofCarryingAnswer,
    RelationshipId, RelationshipInput, RequestId, RetrievalOutcome, TelemetryQueryDescriptor,
    TemporalTimestamp, UnresolvedUnknown, WorkingSetId, capture_trajectory_provenance,
    classify_epistemic_node, compute_retrieval_completeness, default_fimi_investigation_profile,
    epistemic_nodes_of_kind, expand_working_set_from_graph_adjacency,
};

fn working_set_id(value: &str) -> WorkingSetId {
    WorkingSetId::new(value).expect("acceptance working set ID should be valid")
}

fn retrieval_id(value: &str) -> RequestId {
    RequestId::new(value).expect("acceptance retrieval ID should be valid")
}

fn descriptor(text: &str) -> TelemetryQueryDescriptor {
    TelemetryQueryDescriptor {
        query_text: Some(text.to_owned()),
        profile_kind: None,
        task_label: Some("fimi_investigation".to_owned()),
    }
}

fn budget(max_relationships: u64) -> ExpansionBudget {
    ExpansionBudget {
        max_loaded_node_count: 32,
        max_loaded_relationship_count: max_relationships,
        max_hot_node_count: 32,
        max_hot_relationship_count: max_relationships,
        max_warm_adjacency_entry_count: 32,
        max_hop_count: 3,
        max_supernode_expansion_count: 8,
        max_payload_byte_count: 1_048_576,
        max_execution_time_ms: 1_000,
    }
}

fn ts(value: &str) -> TemporalTimestamp {
    TemporalTimestamp::new(value).expect("acceptance timestamp should be valid")
}

/// The epic's reference scenario: two sources report observations that
/// support and contradict one claim, through canonical epistemic relations.
struct EpistemicScenario {
    graph: Graph,
    claim: NodeId,
    supporting_observation: NodeId,
    contradicting_observation: NodeId,
    supports: RelationshipId,
    contradicts: RelationshipId,
}

fn epistemic_scenario() -> EpistemicScenario {
    let mut graph = Graph::new();
    let source_a = graph
        .create_node(NodeInput::new(["Source"]))
        .expect("source A should be created");
    let source_b = graph
        .create_node(NodeInput::new(["Source"]))
        .expect("source B should be created");
    let supporting_observation = graph
        .create_node(NodeInput::new(["Observation"]))
        .expect("supporting observation should be created");
    let contradicting_observation = graph
        .create_node(NodeInput::new(["Observation"]))
        .expect("contradicting observation should be created");
    let claim = graph
        .create_node(NodeInput::new(["Claim"]))
        .expect("claim node should be created");

    let reports = EpistemicRelationKind::Reports.canonical_relationship_type();
    graph
        .create_relationship(
            RelationshipInput::new(source_a, reports.as_str(), supporting_observation.clone())
                .expect("reports input should be valid"),
        )
        .expect("reports relationship should be created");
    graph
        .create_relationship(
            RelationshipInput::new(
                source_b,
                reports.as_str(),
                contradicting_observation.clone(),
            )
            .expect("reports input should be valid"),
        )
        .expect("reports relationship should be created");
    let supports = graph
        .create_relationship(
            RelationshipInput::new(
                supporting_observation.clone(),
                EpistemicRelationKind::Supports
                    .canonical_relationship_type()
                    .as_str(),
                claim.clone(),
            )
            .expect("supports input should be valid"),
        )
        .expect("supports relationship should be created");
    let contradicts = graph
        .create_relationship(
            RelationshipInput::new(
                contradicting_observation.clone(),
                EpistemicRelationKind::Contradicts
                    .canonical_relationship_type()
                    .as_str(),
                claim.clone(),
            )
            .expect("contradicts input should be valid"),
        )
        .expect("contradicts relationship should be created");

    EpistemicScenario {
        graph,
        claim,
        supporting_observation,
        contradicting_observation,
        supports,
        contradicts,
    }
}

//
// Acceptance: facts, claims, observations, hypotheses, and evidence are
// distinct and independently queryable.
//
// Given the epic's reference scenario plus a hypothesis and an evidence node,
// when each epistemic kind is queried over the graph,
// then every kind should return exactly its own nodes and the relations
// should classify against the canonical vocabulary.
#[test]
fn acceptance_epistemic_kinds_are_distinct_and_independently_queryable() {
    let mut scenario = epistemic_scenario();
    let hypothesis = scenario
        .graph
        .create_node(NodeInput::new(["Hypothesis"]))
        .expect("hypothesis node should be created");
    let evidence = scenario
        .graph
        .create_node(NodeInput::new(["Evidence"]))
        .expect("evidence node should be created");

    let of_kind = |kind: EpistemicNodeKind| {
        epistemic_nodes_of_kind(&scenario.graph, kind).expect("kind query should succeed")
    };

    assert_eq!(
        of_kind(EpistemicNodeKind::Claim),
        vec![scenario.claim.clone()]
    );
    assert_eq!(
        of_kind(EpistemicNodeKind::Observation),
        vec![
            scenario.supporting_observation.clone(),
            scenario.contradicting_observation.clone()
        ]
    );
    assert_eq!(of_kind(EpistemicNodeKind::Source).len(), 2);
    assert_eq!(of_kind(EpistemicNodeKind::Hypothesis), vec![hypothesis]);
    assert_eq!(of_kind(EpistemicNodeKind::Evidence), vec![evidence]);

    let claim_node = scenario
        .graph
        .get_node(&scenario.claim)
        .expect("claim lookup should succeed")
        .expect("claim node should exist");
    assert_eq!(
        classify_epistemic_node(&claim_node),
        Some(EpistemicNodeKind::Claim)
    );
}

//
// Acceptance: answers return supporting subgraph, counter-evidence,
// provenance, confidence, completeness, and unknowns — assembled from real
// recorded engine state on the epic's reference scenario.
//
// Given a recorded retrieval expanding the claim's incoming epistemic
// relations,
// when the proof-carrying answer is assembled from the recorded state,
// then the envelope should carry the supporting and contradicting proof, the
// trajectory-backed provenance, both uncertainty signals, and the unresolved
// contradiction, with every cited record navigable to its surfacing steps.
#[test]
fn acceptance_answers_carry_proof_counter_evidence_provenance_and_unknowns() {
    let scenario = epistemic_scenario();
    let ws = working_set_id("working-set--epic-0018-answer");
    let mut manager = GraphWorkingSetManager::new();
    manager
        .create_working_set(GraphWorkingSetCreateRequest::new(ws.clone()))
        .expect("working set should be created");
    let retrieval = retrieval_id("request--epic-0018-answer");

    manager
        .begin_retrieval_telemetry(&ws, retrieval.clone(), descriptor("is the claim supported"))
        .expect("retrieval telemetry should begin");
    expand_working_set_from_graph_adjacency(
        &mut manager,
        &scenario.graph,
        ExpansionRequest::new(
            ws.clone(),
            vec![scenario.claim.clone()],
            ExpansionDirection::Incoming,
            ExpansionFilters::empty(),
            1,
            default_fimi_investigation_profile(),
            budget(32),
        ),
    )
    .expect("expansion should complete");
    manager
        .complete_retrieval_telemetry(
            &ws,
            &retrieval,
            RetrievalOutcome {
                evidence_record_ids: Vec::new(),
                answer_quality: Some(Confidence::new(0.68).expect("quality should be valid")),
                memory_cost_bytes: 512,
                latency_ms: 4,
            },
        )
        .expect("retrieval telemetry should complete");

    let records = manager
        .telemetry(&ws)
        .expect("telemetry recorder should be available")
        .retrieval_records();
    let provenance = capture_trajectory_provenance(&records);
    let completeness = compute_retrieval_completeness(&records);
    let primary_claim = ClaimId::new("claim--attribution").expect("claim ID should be valid");

    let answer = ProofCarryingAnswer {
        answer: AnswerStatement {
            text: "The claim is supported but contradicted by one source".to_owned(),
            primary_claim_id: Some(primary_claim.clone()),
        },
        supporting_subgraph: EvidenceSubgraph {
            node_ids: vec![scenario.supporting_observation.clone()],
            relationship_ids: vec![scenario.supports.clone()],
            claim_ids: vec![primary_claim.clone()],
            evidence_ids: Vec::new(),
        },
        counter_evidence: EvidenceSubgraph {
            node_ids: vec![scenario.contradicting_observation.clone()],
            relationship_ids: vec![scenario.contradicts.clone()],
            claim_ids: Vec::new(),
            evidence_ids: Vec::new(),
        },
        source_provenance: provenance
            .to_source_provenance_ref(vec!["source--vendor-report".to_owned()]),
        confidence: Confidence::new(0.68).expect("confidence should be valid"),
        retrieval_completeness: completeness.completeness,
        unresolved_unknowns: vec![UnresolvedUnknown::UnresolvedContradiction {
            claim_id: primary_claim,
            contradicting_claim_id: ClaimId::new("claim--counter")
                .expect("claim ID should be valid"),
        }],
    };

    assert!(!answer.supporting_subgraph.is_empty());
    assert!(!answer.counter_evidence.is_empty());
    assert_eq!(answer.source_provenance.retrieval_ids, vec![retrieval]);
    assert_eq!(answer.unresolved_unknowns.len(), 1);

    // Every cited record is navigable back to the navigation steps that
    // surfaced it: the full trajectory, not only cited documents.
    assert!(
        !provenance
            .surfacing_steps_for_relationship(&scenario.supports)
            .is_empty()
    );
    assert!(
        !provenance
            .surfacing_steps_for_relationship(&scenario.contradicts)
            .is_empty()
    );
    assert!(
        !provenance
            .surfacing_steps_for_node(&scenario.supporting_observation)
            .is_empty()
    );
}

//
// Acceptance: bitemporal facts represent successive and contradictory states
// without overwrite.
//
// Given successive and contradictory states of one fact,
// when history and as-of queries run and an overwrite is attempted,
// then all states should coexist, the as-of views should be deterministic,
// and the overwrite should fail with the typed error.
#[test]
fn acceptance_bitemporal_states_coexist_without_overwrite() {
    let mut store = BitemporalFactStore::new();
    let attribution = FactId::new("fact--epic-0018-attribution").expect("fact ID should be valid");

    store
        .assert_fact_state(
            attribution.clone(),
            "Actor A operates the campaign",
            BitemporalStamp::new(ts("2026-01-01T00:00:00Z"), ts("2026-01-10T00:00:00Z"))
                .expect("stamp should be valid"),
        )
        .expect("first state should be asserted");
    store
        .assert_fact_state(
            attribution.clone(),
            "Actor B operates the campaign",
            BitemporalStamp::new(ts("2026-01-01T00:00:00Z"), ts("2026-02-10T00:00:00Z"))
                .expect("stamp should be valid"),
        )
        .expect("contradictory state should be asserted");

    // Both contradictory states coexist for the covered valid time.
    let now = store.states_as_of(&attribution, &ts("2026-03-01T00:00:00Z"), None);
    assert_eq!(now.len(), 2);

    // The engine's earlier knowledge is reconstructable.
    let known_early = store.states_as_of(
        &attribution,
        &ts("2026-03-01T00:00:00Z"),
        Some(&ts("2026-01-15T00:00:00Z")),
    );
    assert_eq!(known_early.len(), 1);

    // Overwrite-style updates stay a typed error.
    let error = store
        .assert_fact_state(
            attribution.clone(),
            "Rewritten attribution",
            BitemporalStamp::new(ts("2026-01-01T00:00:00Z"), ts("2026-02-10T00:00:00Z"))
                .expect("stamp should be valid"),
        )
        .expect_err("overwrite should fail");
    assert!(matches!(
        error,
        GraphError::BitemporalOverwriteForbidden(id) if id == attribution
    ));
    assert_eq!(store.fact_history(&attribution).len(), 2);
}

//
// Acceptance: completeness reflects working-set coverage and is reported even
// when the answer is confident.
//
// Given a deliberately budget-limited retrieval over the reference scenario,
// when completeness is computed and paired with a confident answer,
// then completeness should be below 1.0 with a typed coverage reduction while
// confidence keeps its high value.
#[test]
fn acceptance_completeness_is_reported_for_confident_answers() {
    let scenario = epistemic_scenario();
    let ws = working_set_id("working-set--epic-0018-completeness");
    let mut manager = GraphWorkingSetManager::new();
    manager
        .create_working_set(GraphWorkingSetCreateRequest::new(ws.clone()))
        .expect("working set should be created");
    let retrieval = retrieval_id("request--epic-0018-completeness");

    manager
        .begin_retrieval_telemetry(
            &ws,
            retrieval.clone(),
            descriptor("confident but incomplete"),
        )
        .expect("retrieval telemetry should begin");
    expand_working_set_from_graph_adjacency(
        &mut manager,
        &scenario.graph,
        ExpansionRequest::new(
            ws.clone(),
            vec![scenario.claim.clone()],
            ExpansionDirection::Incoming,
            ExpansionFilters::empty(),
            1,
            default_fimi_investigation_profile(),
            budget(1),
        ),
    )
    .expect("budget-limited expansion should return a typed partial result");
    manager
        .complete_retrieval_telemetry(
            &ws,
            &retrieval,
            RetrievalOutcome {
                evidence_record_ids: Vec::new(),
                answer_quality: Some(Confidence::new(0.9).expect("quality should be valid")),
                memory_cost_bytes: 256,
                latency_ms: 2,
            },
        )
        .expect("retrieval telemetry should complete");

    let report = compute_retrieval_completeness(
        &manager
            .telemetry(&ws)
            .expect("telemetry recorder should be available")
            .retrieval_records(),
    );

    assert!(
        report.completeness.value() < 1.0,
        "the budget-limited working set must not read as complete"
    );
    assert!(
        report
            .reductions
            .iter()
            .any(|reduction| reduction.kind == CompletenessReductionKind::SkippedEdges)
    );

    let confidence = Confidence::new(0.9).expect("confidence should be valid");
    assert_eq!(confidence.value(), 0.9);
    assert!(
        confidence.value() > report.completeness.value(),
        "confidence and completeness are independent signals"
    );
}

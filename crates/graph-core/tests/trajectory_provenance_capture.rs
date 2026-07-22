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
    AnswerStatement, Confidence, EvidenceSubgraph, ExpansionBudget, ExpansionDirection,
    ExpansionFilters, ExpansionRequest, Graph, GraphWorkingSetCreateRequest,
    GraphWorkingSetManager, NodeId, NodeInput, ProofCarryingAnswer, RelationshipInput, RequestId,
    RetrievalCompleteness, RetrievalOutcome, SurfacingStep, TelemetryQueryDescriptor,
    UnresolvedUnknown, WorkingSetDecisionEvent, WorkingSetId, capture_trajectory_provenance,
    default_fimi_investigation_profile, expand_working_set_from_graph_adjacency,
};

fn working_set_id(value: &str) -> WorkingSetId {
    WorkingSetId::new(value).expect("provenance working set ID should be valid")
}

fn retrieval_id(value: &str) -> RequestId {
    RequestId::new(value).expect("provenance retrieval ID should be valid")
}

fn descriptor(text: &str) -> TelemetryQueryDescriptor {
    TelemetryQueryDescriptor {
        query_text: Some(text.to_owned()),
        profile_kind: None,
        task_label: Some("fimi_investigation".to_owned()),
    }
}

fn generous_budget() -> ExpansionBudget {
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
    }
}

/// Run one recorded retrieval expanding the campaign chain and return the
/// manager plus the graph record identifiers the scenario produced.
fn recorded_campaign_retrieval(
    ws_value: &str,
    retrieval_value: &str,
) -> (
    GraphWorkingSetManager,
    WorkingSetId,
    NodeId,
    NodeId,
    graph_core::RelationshipId,
) {
    let mut graph = Graph::new();
    let campaign = graph
        .create_node(NodeInput::new(["Campaign"]))
        .expect("campaign node should be created");
    let narrative = graph
        .create_node(NodeInput::new(["Narrative"]))
        .expect("narrative node should be created");
    let claim = graph
        .create_node(NodeInput::new(["Claim"]))
        .expect("claim node should be created");
    let promotes = graph
        .create_relationship(
            RelationshipInput::new(campaign.clone(), "PROMOTES", narrative.clone())
                .expect("promotes input should be valid"),
        )
        .expect("promotes relationship should be created");
    graph
        .create_relationship(
            RelationshipInput::new(narrative.clone(), "MAKES_CLAIM", claim)
                .expect("makes-claim input should be valid"),
        )
        .expect("makes-claim relationship should be created");

    let ws = working_set_id(ws_value);
    let mut manager = GraphWorkingSetManager::new();
    manager
        .create_working_set(GraphWorkingSetCreateRequest::new(ws.clone()))
        .expect("working set should be created");
    let retrieval = retrieval_id(retrieval_value);

    manager
        .begin_retrieval_telemetry(&ws, retrieval, descriptor("campaign expansion"))
        .expect("retrieval telemetry should begin");
    expand_working_set_from_graph_adjacency(
        &mut manager,
        &graph,
        ExpansionRequest::new(
            ws.clone(),
            vec![campaign.clone()],
            ExpansionDirection::Outgoing,
            ExpansionFilters::empty(),
            1,
            default_fimi_investigation_profile(),
            generous_budget(),
        ),
    )
    .expect("expansion should complete");
    manager
        .complete_retrieval_telemetry(
            &ws,
            &retrieval_id(retrieval_value),
            RetrievalOutcome {
                evidence_record_ids: Vec::new(),
                answer_quality: None,
                memory_cost_bytes: 256,
                latency_ms: 3,
            },
        )
        .expect("retrieval telemetry should complete");

    (manager, ws, campaign, narrative, promotes)
}

//
// Verify that captured provenance covers the full navigation trajectory of a
// real retrieval — seeds, page-ins, expansions, warm adjacency — not only the
// documents finally cited.
//
// Given one recorded expansion retrieval,
// when trajectory provenance is captured,
// then it should contain every recorded decision of the retrieval in order.
#[test]
fn capture_covers_the_full_navigation_trajectory() {
    let (manager, ws, _campaign, _narrative, _promotes) =
        recorded_campaign_retrieval("working-set--provenance-full", "request--provenance-full");
    let records = manager
        .telemetry(&ws)
        .expect("telemetry recorder should be available")
        .retrieval_records();

    let provenance = capture_trajectory_provenance(&records);

    assert_eq!(provenance.retrievals.len(), 1);
    let trajectory = &provenance.retrievals[0];
    assert_eq!(
        trajectory.retrieval_id,
        retrieval_id("request--provenance-full")
    );
    assert_eq!(trajectory.steps.len(), records[0].events.len());
    for (step, event) in trajectory.steps.iter().zip(&records[0].events) {
        assert_eq!(step.sequence, event.sequence);
        assert_eq!(step.decision, event.decision);
    }

    let has = |predicate: fn(&WorkingSetDecisionEvent) -> bool| {
        trajectory
            .steps
            .iter()
            .any(|step| predicate(&step.decision))
    };
    assert!(has(|decision| matches!(
        decision,
        WorkingSetDecisionEvent::SeedSelected { .. }
    )));
    assert!(has(|decision| matches!(
        decision,
        WorkingSetDecisionEvent::PageIn { .. }
    )));
    assert!(has(|decision| matches!(
        decision,
        WorkingSetDecisionEvent::EdgeExpanded { .. }
    )));
    assert!(has(|decision| matches!(
        decision,
        WorkingSetDecisionEvent::WarmAdjacencyAttached { .. }
    )));
}

//
// Verify that retrieval boundaries are preserved in order: one trajectory per
// recorded retrieval, carrying its identifier and query descriptor.
//
// Given two recorded retrievals on one working set,
// when provenance is captured,
// then two trajectories should appear in recording order with their
// descriptors.
#[test]
fn retrieval_boundaries_are_preserved_in_order() {
    let ws = working_set_id("working-set--provenance-boundaries");
    let mut manager = GraphWorkingSetManager::new();
    manager
        .create_working_set(GraphWorkingSetCreateRequest::new(ws.clone()))
        .expect("working set should be created");

    for (index, question) in ["first question", "second question"].iter().enumerate() {
        let retrieval = retrieval_id(&format!("request--provenance-boundary-{index}"));
        manager
            .begin_retrieval_telemetry(&ws, retrieval.clone(), descriptor(question))
            .expect("retrieval telemetry should begin");
        manager
            .load_seed_node_ids(
                &ws,
                [NodeId::new(format!("node--seed-{index}")).expect("seed ID should be valid")],
                true,
            )
            .expect("seed should be loaded");
        manager
            .complete_retrieval_telemetry(
                &ws,
                &retrieval,
                RetrievalOutcome {
                    evidence_record_ids: Vec::new(),
                    answer_quality: None,
                    memory_cost_bytes: 0,
                    latency_ms: 0,
                },
            )
            .expect("retrieval telemetry should complete");
    }

    let provenance = capture_trajectory_provenance(
        &manager
            .telemetry(&ws)
            .expect("telemetry recorder should be available")
            .retrieval_records(),
    );

    assert_eq!(provenance.retrievals.len(), 2);
    assert_eq!(
        provenance.retrievals[0].descriptor.query_text.as_deref(),
        Some("first question")
    );
    assert_eq!(
        provenance.retrievals[1].descriptor.query_text.as_deref(),
        Some("second question")
    );
}

//
// Verify that cited evidence links back to the navigation steps that surfaced
// it: a relationship's surfacing steps are the recorded decisions that paged
// it in and expanded it.
//
// Given the recorded campaign retrieval,
// when the surfacing steps of the expanded relationship are queried,
// then they should reference the retrieval and the page-in and expansion
// decisions in order.
#[test]
fn relationships_link_back_to_their_surfacing_steps() {
    let (manager, ws, _campaign, _narrative, promotes) = recorded_campaign_retrieval(
        "working-set--provenance-rel-steps",
        "request--provenance-rel-steps",
    );
    let provenance = capture_trajectory_provenance(
        &manager
            .telemetry(&ws)
            .expect("telemetry recorder should be available")
            .retrieval_records(),
    );

    let steps = provenance.surfacing_steps_for_relationship(&promotes);

    assert!(
        steps.len() >= 2,
        "the relationship should be surfaced by at least its page-in and expansion"
    );
    assert!(
        steps
            .windows(2)
            .all(|pair| pair[0].sequence < pair[1].sequence),
        "surfacing steps should be ordered"
    );
    for step in &steps {
        assert_eq!(
            step.retrieval_id,
            retrieval_id("request--provenance-rel-steps")
        );
    }
}

//
// Verify that nodes link back to their surfacing steps as well, so every
// supporting or counter-evidence node is navigable to its trajectory.
//
// Given the recorded campaign retrieval,
// when the surfacing steps of the seed and the expanded neighbor are queried,
// then each should reference at least one recorded decision, and unknown
// records should reference none.
#[test]
fn nodes_link_back_to_their_surfacing_steps() {
    let (manager, ws, campaign, narrative, _promotes) = recorded_campaign_retrieval(
        "working-set--provenance-node-steps",
        "request--provenance-node-steps",
    );
    let provenance = capture_trajectory_provenance(
        &manager
            .telemetry(&ws)
            .expect("telemetry recorder should be available")
            .retrieval_records(),
    );

    assert!(!provenance.surfacing_steps_for_node(&campaign).is_empty());
    assert!(!provenance.surfacing_steps_for_node(&narrative).is_empty());

    let unknown = NodeId::new("node--never-visited").expect("node ID should be valid");
    let empty: Vec<SurfacingStep> = provenance.surfacing_steps_for_node(&unknown);
    assert!(empty.is_empty());
}

//
// Verify the envelope integration: captured provenance projects into the
// proof-carrying answer's source-provenance reference with the retrieval
// identifiers in order plus the cited sources.
//
// Given captured provenance over one retrieval,
// when it is projected into a source-provenance reference inside an envelope,
// then the envelope should carry the retrieval identifier and the sources.
#[test]
fn provenance_projects_into_the_answer_envelope() {
    let (manager, ws, _campaign, _narrative, promotes) = recorded_campaign_retrieval(
        "working-set--provenance-envelope",
        "request--provenance-envelope",
    );
    let provenance = capture_trajectory_provenance(
        &manager
            .telemetry(&ws)
            .expect("telemetry recorder should be available")
            .retrieval_records(),
    );

    let answer = ProofCarryingAnswer {
        answer: AnswerStatement {
            text: "The campaign promotes the narrative".to_owned(),
            primary_claim_id: None,
        },
        supporting_subgraph: EvidenceSubgraph {
            node_ids: Vec::new(),
            relationship_ids: vec![promotes],
            claim_ids: Vec::new(),
            evidence_ids: Vec::new(),
        },
        counter_evidence: EvidenceSubgraph::default(),
        source_provenance: provenance
            .to_source_provenance_ref(vec!["source--vendor-report".to_owned()]),
        confidence: Confidence::new(0.8).expect("confidence should be valid"),
        retrieval_completeness: RetrievalCompleteness::new(1.0)
            .expect("completeness should be valid"),
        unresolved_unknowns: Vec::<UnresolvedUnknown>::new(),
    };

    assert_eq!(
        answer.source_provenance.retrieval_ids,
        vec![retrieval_id("request--provenance-envelope")]
    );
    assert_eq!(
        answer.source_provenance.source_refs,
        vec!["source--vendor-report".to_owned()]
    );
}

//
// Verify reproducibility: provenance is a pure projection of the recorded
// telemetry, so capturing twice from the same records yields equal values.
//
// Given the recorded campaign retrieval,
// when provenance is captured twice,
// then both captures should be exactly equal.
#[test]
fn capture_is_reproducible_from_recorded_telemetry() {
    let (manager, ws, _campaign, _narrative, _promotes) =
        recorded_campaign_retrieval("working-set--provenance-repro", "request--provenance-repro");
    let records = manager
        .telemetry(&ws)
        .expect("telemetry recorder should be available")
        .retrieval_records();

    assert_eq!(
        capture_trajectory_provenance(&records),
        capture_trajectory_provenance(&records)
    );
}

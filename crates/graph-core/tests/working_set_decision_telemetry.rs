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
    Confidence, ExpansionBudget, ExpansionDirection, ExpansionFilters, ExpansionRequest, Graph,
    GraphError, GraphRecordRef, GraphWorkingSetCreateRequest, GraphWorkingSetManager, NodeId,
    NodeInput, RelationshipId, RelationshipInput, RelationshipType, RequestId, RetrievalOutcome,
    SkippedExpansionReason, TelemetryQueryDescriptor, WorkingSetDecisionEvent, WorkingSetId,
    default_fimi_investigation_profile, default_generic_loading_profile,
    expand_working_set_from_graph_adjacency,
};

fn working_set_id(value: &str) -> WorkingSetId {
    WorkingSetId::new(value).expect("telemetry working set ID should be valid")
}

fn node_id(value: &str) -> NodeId {
    NodeId::new(value).expect("telemetry node ID should be valid")
}

fn relationship_id(value: &str) -> RelationshipId {
    RelationshipId::new(value).expect("telemetry relationship ID should be valid")
}

fn retrieval_id(value: &str) -> RequestId {
    RequestId::new(value).expect("telemetry retrieval ID should be valid")
}

fn rel_type(value: &str) -> RelationshipType {
    RelationshipType::new(value).expect("telemetry relationship type should be valid")
}

fn manager_with_working_set(id: &WorkingSetId) -> GraphWorkingSetManager {
    let mut manager = GraphWorkingSetManager::new();
    manager
        .create_working_set(GraphWorkingSetCreateRequest::new(id.clone()))
        .expect("telemetry working set should be created");
    manager
}

fn descriptor(query_text: &str) -> TelemetryQueryDescriptor {
    TelemetryQueryDescriptor {
        query_text: Some(query_text.to_owned()),
        profile_kind: Some(default_generic_loading_profile().kind),
        task_label: Some("fimi_investigation".to_owned()),
    }
}

fn outcome() -> RetrievalOutcome {
    RetrievalOutcome {
        evidence_record_ids: Vec::new(),
        answer_quality: Some(
            Confidence::new(0.8).expect("telemetry answer quality should be valid"),
        ),
        memory_cost_bytes: 4_096,
        latency_ms: 12,
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

fn create_node(graph: &mut Graph, labels: &[&str]) -> NodeId {
    graph
        .create_node(NodeInput::new(labels.iter().copied()))
        .expect("telemetry node should be created")
}

fn create_relationship(
    graph: &mut Graph,
    source: &NodeId,
    relationship_type: &str,
    target: &NodeId,
) -> RelationshipId {
    graph
        .create_relationship(
            RelationshipInput::new(source.clone(), relationship_type, target.clone())
                .expect("telemetry relationship input should be valid"),
        )
        .expect("telemetry relationship should be created")
}

fn expansion_request(
    working_set_id: &WorkingSetId,
    seed_node_ids: Vec<NodeId>,
    filters: ExpansionFilters,
    hop_limit: u64,
) -> ExpansionRequest {
    ExpansionRequest::new(
        working_set_id.clone(),
        seed_node_ids,
        ExpansionDirection::Outgoing,
        filters,
        hop_limit,
        default_fimi_investigation_profile(),
        generous_budget(),
    )
}

//
// Verify that telemetry is manager-owned per working set, mirroring the existing
// explanation ownership model, so decisions can be recorded from creation onward.
//
// Given a manager that creates a working set,
// when the telemetry recorder is requested for that working set,
// then an empty deterministic recorder should be returned.
#[test]
fn telemetry_recorder_is_created_with_working_set() {
    let id = working_set_id("working-set--telemetry-created");
    let manager = manager_with_working_set(&id);

    let recorder = manager
        .telemetry(&id)
        .expect("telemetry recorder should exist for a created working set");

    assert!(recorder.events().is_empty());
    assert!(recorder.retrieval_records().is_empty());
}

//
// Verify that missing working-set telemetry lookup is a typed domain error rather
// than a panic or silent empty value.
//
// Given an empty manager,
// when telemetry is requested for an unknown working-set ID,
// then `GraphError::WorkingSetNotFound` should be returned with that ID.
#[test]
fn missing_working_set_telemetry_returns_typed_error() {
    let manager = GraphWorkingSetManager::new();
    let missing_id = working_set_id("working-set--telemetry-missing");

    let error = manager
        .telemetry(&missing_id)
        .expect_err("missing working set telemetry should return a typed error");

    assert!(matches!(
        error,
        GraphError::WorkingSetNotFound(id) if id == missing_id
    ));
}

//
// Verify that manager-owned working-set mutations are recorded as decision events
// without the caller doing any explicit telemetry work.
//
// Given an existing working set,
// when seed nodes are loaded, a hot relationship is added, and warm adjacency
// would be attached through the manager,
// then seed-selection and edge-expansion events should appear in operation order
// with strictly increasing sequence numbers.
#[test]
fn manager_mutations_record_decision_events_in_order() {
    let id = working_set_id("working-set--telemetry-mutations");
    let mut manager = manager_with_working_set(&id);
    let campaign = node_id("campaign--telemetry");
    let narrative = node_id("narrative--telemetry");
    let promotes = relationship_id("relationship--telemetry-promotes");

    manager
        .load_seed_node_ids(&id, [campaign.clone()], true)
        .expect("seed node should be loaded");
    manager
        .load_seed_node_ids(&id, [narrative.clone()], false)
        .expect("second seed node should be loaded");
    manager
        .add_hot_relationship(&id, promotes.clone())
        .expect("hot relationship should be tracked");

    let recorder = manager
        .telemetry(&id)
        .expect("telemetry recorder should be available");
    let events = recorder.events();

    assert_eq!(events.len(), 3);
    assert_eq!(
        events[0].decision,
        WorkingSetDecisionEvent::SeedSelected {
            node_id: campaign,
            marked_hot: true,
        }
    );
    assert_eq!(
        events[1].decision,
        WorkingSetDecisionEvent::SeedSelected {
            node_id: narrative,
            marked_hot: false,
        }
    );
    assert_eq!(
        events[2].decision,
        WorkingSetDecisionEvent::EdgeExpanded {
            relationship_id: promotes,
        }
    );
    assert!(events[0].sequence < events[1].sequence);
    assert!(events[1].sequence < events[2].sequence);
}

//
// Verify that retrieval boundaries group decision events into per-retrieval
// records carrying the query descriptor and the caller-supplied outcome.
//
// Given an open retrieval on a working set,
// when decisions are recorded and the retrieval is completed with an outcome,
// then one retrieval record should expose the descriptor, the enclosed events,
// and the outcome measurements.
#[test]
fn retrieval_markers_group_events_into_retrieval_records() {
    let id = working_set_id("working-set--telemetry-retrieval");
    let mut manager = manager_with_working_set(&id);
    let retrieval = retrieval_id("request--telemetry-1");
    let campaign = node_id("campaign--telemetry-retrieval");

    manager
        .begin_retrieval_telemetry(&id, retrieval.clone(), descriptor("attribution of C-42"))
        .expect("retrieval telemetry should begin");
    manager
        .load_seed_node_ids(&id, [campaign.clone()], true)
        .expect("seed node should be loaded");
    manager
        .complete_retrieval_telemetry(&id, &retrieval, outcome())
        .expect("retrieval telemetry should complete");

    let recorder = manager
        .telemetry(&id)
        .expect("telemetry recorder should be available");
    let records = recorder.retrieval_records();

    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record.retrieval_id, retrieval);
    assert_eq!(record.working_set_id, id);
    assert_eq!(
        record.descriptor.query_text.as_deref(),
        Some("attribution of C-42")
    );
    assert_eq!(record.events.len(), 1);
    assert_eq!(
        record.events[0].decision,
        WorkingSetDecisionEvent::SeedSelected {
            node_id: campaign,
            marked_hot: true,
        }
    );
    let recorded_outcome = record
        .outcome
        .as_ref()
        .expect("completed retrieval should carry its outcome");
    assert_eq!(recorded_outcome.memory_cost_bytes, 4_096);
    assert_eq!(recorded_outcome.latency_ms, 12);
}

//
// Verify that retrieval markers keep their integrity invariants as typed errors:
// no nested retrievals, no completion without an open retrieval, and no
// completion under a mismatched retrieval ID.
//
// Given a working set with an open retrieval,
// when a second retrieval begins, or completion targets the wrong ID, or
// completion runs with nothing open,
// then each misuse should return `GraphError::InternalInvariantViolation`.
#[test]
fn retrieval_marker_misuse_returns_typed_invariant_errors() {
    let id = working_set_id("working-set--telemetry-markers");
    let mut manager = manager_with_working_set(&id);
    let first = retrieval_id("request--telemetry-first");
    let second = retrieval_id("request--telemetry-second");

    let error = manager
        .complete_retrieval_telemetry(&id, &first, outcome())
        .expect_err("completing without an open retrieval should fail");
    assert!(matches!(error, GraphError::InternalInvariantViolation(_)));

    manager
        .begin_retrieval_telemetry(&id, first.clone(), descriptor("first"))
        .expect("first retrieval should begin");

    let error = manager
        .begin_retrieval_telemetry(&id, second.clone(), descriptor("second"))
        .expect_err("nested retrieval should fail");
    assert!(matches!(error, GraphError::InternalInvariantViolation(_)));

    let error = manager
        .complete_retrieval_telemetry(&id, &second, outcome())
        .expect_err("completing a mismatched retrieval ID should fail");
    assert!(matches!(error, GraphError::InternalInvariantViolation(_)));

    manager
        .complete_retrieval_telemetry(&id, &first, outcome())
        .expect("matching retrieval completion should succeed");
}

//
// Verify that retrieval boundary markers cannot be forged through the generic
// decision-recording path, so marker integrity stays owned by begin/complete.
//
// Given an existing working set,
// when a retrieval marker variant is recorded as a plain decision,
// then the manager should reject it with a typed invariant error.
#[test]
fn direct_marker_recording_is_rejected() {
    let id = working_set_id("working-set--telemetry-forged-marker");
    let mut manager = manager_with_working_set(&id);

    let error = manager
        .record_telemetry_decision(
            &id,
            WorkingSetDecisionEvent::RetrievalStarted {
                retrieval_id: retrieval_id("request--telemetry-forged"),
                descriptor: descriptor("forged"),
            },
        )
        .expect_err("recording a marker as a plain decision should fail");

    assert!(matches!(error, GraphError::InternalInvariantViolation(_)));
}

//
// Verify that the expansion engine records page-ins, expanded edges, and warm
// adjacency attachments as decision events during a real traversal.
//
// Given a campaign -> narrative -> claim path,
// when a filtered one-hop expansion runs inside an open retrieval,
// then telemetry should contain page-in events for the loaded payloads, an
// edge-expansion event for the traversed relationship, and a warm adjacency
// event for the ring boundary, without changing expansion behavior.
#[test]
fn expansion_engine_records_page_ins_and_expanded_edges() {
    let mut graph = Graph::new();
    let campaign = create_node(&mut graph, &["Campaign"]);
    let narrative = create_node(&mut graph, &["Narrative"]);
    let claim = create_node(&mut graph, &["Claim"]);
    let promotes = create_relationship(&mut graph, &campaign, "PROMOTES", &narrative);
    let makes_claim = create_relationship(&mut graph, &narrative, "MAKES_CLAIM", &claim);
    let id = working_set_id("working-set--telemetry-expansion");
    let mut manager = manager_with_working_set(&id);
    let retrieval = retrieval_id("request--telemetry-expansion");

    manager
        .begin_retrieval_telemetry(&id, retrieval.clone(), descriptor("campaign expansion"))
        .expect("retrieval telemetry should begin");
    let result = expand_working_set_from_graph_adjacency(
        &mut manager,
        &graph,
        expansion_request(
            &id,
            vec![campaign.clone()],
            ExpansionFilters::new(vec![rel_type("PROMOTES")], vec!["Narrative".to_owned()]),
            1,
        ),
    )
    .expect("expansion should complete");
    manager
        .complete_retrieval_telemetry(&id, &retrieval, outcome())
        .expect("retrieval telemetry should complete");

    // Expansion behavior itself must be unchanged by telemetry.
    assert_eq!(result.usage().hot_node_count, 2);
    assert_eq!(result.usage().hot_relationship_count, 1);

    let records = manager
        .telemetry(&id)
        .expect("telemetry recorder should be available")
        .retrieval_records();
    assert_eq!(records.len(), 1);
    let events: Vec<_> = records[0]
        .events
        .iter()
        .map(|event| event.decision.clone())
        .collect();

    assert!(events.contains(&WorkingSetDecisionEvent::PageIn {
        record: GraphRecordRef::Node(campaign.clone()),
    }));
    assert!(events.contains(&WorkingSetDecisionEvent::PageIn {
        record: GraphRecordRef::Relationship(promotes.clone()),
    }));
    assert!(events.contains(&WorkingSetDecisionEvent::PageIn {
        record: GraphRecordRef::Node(narrative.clone()),
    }));
    assert!(events.contains(&WorkingSetDecisionEvent::EdgeExpanded {
        relationship_id: promotes.clone(),
    }));
    assert!(events.contains(&WorkingSetDecisionEvent::SeedSelected {
        node_id: campaign.clone(),
        marked_hot: true,
    }));
    assert!(
        events.iter().any(|event| matches!(
            event,
            WorkingSetDecisionEvent::WarmAdjacencyAttached {
                relationship_id,
                target_node_id,
                ..
            } if relationship_id == &makes_claim && target_node_id == &claim
        )),
        "ring-boundary warm adjacency should be recorded as a decision event"
    );
}

//
// Verify that skipped expansions are recorded as telemetry decisions with the
// same stable reasons already used by working-set explanations.
//
// Given a seed whose only outgoing edge is blocked by relationship filters,
// when the expansion runs,
// then an edge-skip event with `BlockedByProfile` should be recorded for the
// filtered relationship.
#[test]
fn expansion_records_skipped_edges_with_stable_reasons() {
    let mut graph = Graph::new();
    let campaign = create_node(&mut graph, &["Campaign"]);
    let narrative = create_node(&mut graph, &["Narrative"]);
    let promotes = create_relationship(&mut graph, &campaign, "PROMOTES", &narrative);
    let id = working_set_id("working-set--telemetry-skips");
    let mut manager = manager_with_working_set(&id);

    expand_working_set_from_graph_adjacency(
        &mut manager,
        &graph,
        expansion_request(
            &id,
            vec![campaign.clone()],
            ExpansionFilters::new(vec![rel_type("USES")], Vec::new()),
            1,
        ),
    )
    .expect("filtered expansion should complete");

    let recorder = manager
        .telemetry(&id)
        .expect("telemetry recorder should be available");

    assert!(
        recorder.events().iter().any(|event| matches!(
            &event.decision,
            WorkingSetDecisionEvent::EdgeSkipped {
                source_node_id,
                relationship_id,
                reason,
                ..
            } if source_node_id == &campaign
                && relationship_id.as_ref() == Some(&promotes)
                && *reason == SkippedExpansionReason::BlockedByProfile
        )),
        "profile-blocked expansion should be recorded as an edge-skip decision"
    );
}

//
// Verify that frontier nodes with no admitted expansion are recorded as dead
// ends, giving the future anti-pheromone field its primary observation signal.
//
// Given a seed node with no outgoing adjacency,
// when a one-hop expansion runs from that seed,
// then a dead-end event should be recorded for the seed node.
#[test]
fn expansion_records_dead_end_frontiers() {
    let mut graph = Graph::new();
    let isolated = create_node(&mut graph, &["Campaign"]);
    let id = working_set_id("working-set--telemetry-dead-end");
    let mut manager = manager_with_working_set(&id);

    expand_working_set_from_graph_adjacency(
        &mut manager,
        &graph,
        expansion_request(&id, vec![isolated.clone()], ExpansionFilters::empty(), 1),
    )
    .expect("expansion of an isolated seed should complete");

    let recorder = manager
        .telemetry(&id)
        .expect("telemetry recorder should be available");

    assert!(
        recorder.events().iter().any(|event| matches!(
            &event.decision,
            WorkingSetDecisionEvent::DeadEnd { node_id } if node_id == &isolated
        )),
        "an isolated frontier should be recorded as a dead end"
    );
}

//
// Verify that telemetry capture is deterministic: the same decision sequence on
// two managers produces identical recorders, which the benchmark harness and
// future pheromone updates rely on for reproducibility.
//
// Given two managers and two identically built graphs,
// when the same retrieval and expansion sequence runs against both,
// then the recorded telemetry should be exactly equal.
#[test]
fn telemetry_capture_is_deterministic_for_identical_decision_sequences() {
    let run = || {
        let mut graph = Graph::new();
        let campaign = create_node(&mut graph, &["Campaign"]);
        let narrative = create_node(&mut graph, &["Narrative"]);
        create_relationship(&mut graph, &campaign, "PROMOTES", &narrative);
        let id = working_set_id("working-set--telemetry-deterministic");
        let mut manager = manager_with_working_set(&id);
        let retrieval = retrieval_id("request--telemetry-deterministic");

        manager
            .begin_retrieval_telemetry(&id, retrieval.clone(), descriptor("deterministic run"))
            .expect("retrieval telemetry should begin");
        expand_working_set_from_graph_adjacency(
            &mut manager,
            &graph,
            expansion_request(&id, vec![campaign], ExpansionFilters::empty(), 1),
        )
        .expect("expansion should complete");
        manager
            .complete_retrieval_telemetry(&id, &retrieval, outcome())
            .expect("retrieval telemetry should complete");

        manager
            .telemetry(&id)
            .expect("telemetry recorder should be available")
            .clone()
    };

    let first = run();
    let second = run();

    assert_eq!(first, second);
    assert_eq!(first.retrieval_records(), second.retrieval_records());
}

//
// Verify that recording telemetry does not change observable working-set state,
// stats, or explanations: instrumentation must be a pure observer.
//
// Given the acceptance one-hop expansion scenario,
// when it runs on a telemetry-recording manager,
// then hot/warm state and stats should match the pre-telemetry contract exactly.
#[test]
fn telemetry_recording_does_not_change_working_set_behavior() {
    let mut graph = Graph::new();
    let campaign = create_node(&mut graph, &["Campaign"]);
    let narrative = create_node(&mut graph, &["Narrative"]);
    let claim = create_node(&mut graph, &["Claim"]);
    let promotes = create_relationship(&mut graph, &campaign, "PROMOTES", &narrative);
    let makes_claim = create_relationship(&mut graph, &narrative, "MAKES_CLAIM", &claim);
    let id = working_set_id("working-set--telemetry-behavior");
    let mut manager = manager_with_working_set(&id);

    let result = expand_working_set_from_graph_adjacency(
        &mut manager,
        &graph,
        expansion_request(
            &id,
            vec![campaign.clone()],
            ExpansionFilters::new(vec![rel_type("PROMOTES")], vec!["Narrative".to_owned()]),
            1,
        ),
    )
    .expect("expansion should complete");

    assert_eq!(result.usage().hot_node_count, 2);
    assert_eq!(result.usage().hot_relationship_count, 1);

    let working_set = manager
        .get_working_set(&id)
        .expect("working set should remain available");
    assert!(working_set.hot_node_ids().contains(&campaign));
    assert!(working_set.hot_node_ids().contains(&narrative));
    assert!(!working_set.hot_node_ids().contains(&claim));
    assert!(working_set.hot_relationship_ids().contains(&promotes));
    assert!(!working_set.hot_relationship_ids().contains(&makes_claim));

    let stats = manager.stats(&id).expect("stats should be available");
    assert_eq!(stats.hot_node_count(), 2);
    assert_eq!(stats.hot_relationship_count(), 1);
}

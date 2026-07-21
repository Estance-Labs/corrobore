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
    BanditContext, BanditReward, ExpansionBudget, ExpansionDirection, ExpansionFilters,
    ExpansionLimit, ExpansionRequest, ExpansionResultStatus, Graph, GraphWorkingSetCreateRequest,
    GraphWorkingSetManager, GreedyExpandController, NodeId, NodeInput, RelationshipId,
    RelationshipInput, RequestId, RetrievalOutcome, SkippedExpansionReason, SupernodePolicy,
    TelemetryQueryDescriptor, WorkingSetAction, WorkingSetController, WorkingSetDecisionEvent,
    WorkingSetId, default_fimi_investigation_profile, expand_working_set_from_graph_adjacency,
    expand_working_set_with_controller,
};

fn working_set_id(value: &str) -> WorkingSetId {
    WorkingSetId::new(value).expect("integration working set ID should be valid")
}

fn retrieval_id(value: &str) -> RequestId {
    RequestId::new(value).expect("integration retrieval ID should be valid")
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

fn manager_with_working_set(id: &WorkingSetId) -> GraphWorkingSetManager {
    let mut manager = GraphWorkingSetManager::new();
    manager
        .create_working_set(GraphWorkingSetCreateRequest::new(id.clone()))
        .expect("integration working set should be created");
    manager
}

fn request(
    ws: &WorkingSetId,
    seeds: Vec<NodeId>,
    filters: ExpansionFilters,
    budget: ExpansionBudget,
) -> ExpansionRequest {
    ExpansionRequest::new(
        ws.clone(),
        seeds,
        ExpansionDirection::Outgoing,
        filters,
        1,
        default_fimi_investigation_profile(),
        budget,
    )
}

fn campaign_graph() -> (Graph, NodeId, NodeId, RelationshipId) {
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
    (graph, campaign, narrative, promotes)
}

struct FixedActionController {
    action: WorkingSetAction,
}

impl WorkingSetController for FixedActionController {
    fn choose_action(&mut self, _context: &BanditContext) -> WorkingSetAction {
        self.action
    }

    fn observe_reward(
        &mut self,
        _context: &BanditContext,
        _action: WorkingSetAction,
        _reward: &BanditReward,
    ) {
    }
}

//
// Verify that the greedy baseline behind the controller boundary preserves the
// default engine behavior exactly, so configuring no learned policy loses
// nothing.
//
// Given two identical graphs and working sets,
// when one expansion runs plain and the other runs through the greedy baseline,
// then both should produce equal results and equal working-set state.
#[test]
fn greedy_baseline_matches_default_expansion_behavior() {
    let build = || {
        let (graph, campaign, _narrative, _promotes) = campaign_graph();
        let ws = working_set_id("working-set--integration-parity");
        let manager = manager_with_working_set(&ws);
        (graph, campaign, ws, manager)
    };

    let (graph_a, campaign_a, ws_a, mut manager_a) = build();
    let plain = expand_working_set_from_graph_adjacency(
        &mut manager_a,
        &graph_a,
        request(
            &ws_a,
            vec![campaign_a],
            ExpansionFilters::empty(),
            generous_budget(),
        ),
    )
    .expect("plain expansion should complete");

    let (graph_b, campaign_b, ws_b, mut manager_b) = build();
    let mut controller = GreedyExpandController::new();
    let driven = expand_working_set_with_controller(
        &mut manager_b,
        &graph_b,
        request(
            &ws_b,
            vec![campaign_b],
            ExpansionFilters::empty(),
            generous_budget(),
        ),
        &mut controller,
    )
    .expect("controller-driven expansion should complete");

    assert_eq!(plain.status(), driven.status());
    assert_eq!(plain.usage(), driven.usage());
    assert_eq!(plain.explanation(), driven.explanation());
    assert_eq!(plain.budget_error(), driven.budget_error());

    let plain_set = manager_a
        .get_working_set(&ws_a)
        .expect("plain working set should exist");
    let driven_set = manager_b
        .get_working_set(&ws_b)
        .expect("driven working set should exist");
    assert_eq!(plain_set.hot_node_ids(), driven_set.hot_node_ids());
    assert_eq!(
        plain_set.hot_relationship_ids(),
        driven_set.hot_relationship_ids()
    );
    assert_eq!(plain_set.stats(), driven_set.stats());
}

//
// Verify that a stop decision halts expansion after seed loading: seeds stay
// ring-0 entry points, and the controller governs everything beyond them.
//
// Given an always-stop controller,
// when the expansion runs,
// then the seed should be hot, nothing should be expanded, the result should be
// partial, and the stop should be explained as a controller decision.
#[test]
fn stop_action_halts_expansion_with_a_partial_result() {
    let (graph, campaign, narrative, promotes) = campaign_graph();
    let ws = working_set_id("working-set--integration-stop");
    let mut manager = manager_with_working_set(&ws);
    let mut controller = FixedActionController {
        action: WorkingSetAction::Stop,
    };

    let result = expand_working_set_with_controller(
        &mut manager,
        &graph,
        request(
            &ws,
            vec![campaign.clone()],
            ExpansionFilters::empty(),
            generous_budget(),
        ),
        &mut controller,
    )
    .expect("stopped expansion should return a typed partial result");

    assert_eq!(result.status(), ExpansionResultStatus::Partial);
    assert!(result.budget_error().is_none());
    assert!(
        result
            .explanation()
            .skipped_expansions()
            .iter()
            .any(|skipped| skipped.reason == SkippedExpansionReason::ControllerDecision)
    );

    let working_set = manager
        .get_working_set(&ws)
        .expect("working set should remain available");
    assert!(working_set.hot_node_ids().contains(&campaign));
    assert!(!working_set.hot_node_ids().contains(&narrative));
    assert!(!working_set.hot_relationship_ids().contains(&promotes));
}

//
// Verify that a prefetch decision materializes the frontier as warm adjacency
// instead of hot records: the controller chooses the materialization
// granularity without bypassing budgets.
//
// Given an always-prefetch controller,
// when the expansion runs,
// then the neighbor should be warm-attached under the seed and nothing beyond
// the seed should be hot.
#[test]
fn prefetch_action_materializes_the_frontier_as_warm() {
    let (graph, campaign, narrative, promotes) = campaign_graph();
    let ws = working_set_id("working-set--integration-prefetch");
    let mut manager = manager_with_working_set(&ws);
    let mut controller = FixedActionController {
        action: WorkingSetAction::Prefetch,
    };

    let result = expand_working_set_with_controller(
        &mut manager,
        &graph,
        request(
            &ws,
            vec![campaign.clone()],
            ExpansionFilters::empty(),
            generous_budget(),
        ),
        &mut controller,
    )
    .expect("prefetch expansion should complete");

    assert_eq!(result.status(), ExpansionResultStatus::Complete);

    let working_set = manager
        .get_working_set(&ws)
        .expect("working set should remain available");
    assert!(working_set.hot_node_ids().contains(&campaign));
    assert!(!working_set.hot_relationship_ids().contains(&promotes));
    let warm_entries = working_set
        .warm_adjacency_for_source(&campaign)
        .expect("prefetch should attach warm adjacency under the seed");
    assert_eq!(warm_entries.len(), 1);
    assert_eq!(warm_entries[0].target_node_id(), &narrative);
}

//
// Verify that verify/retrieve-externally decisions defer the source: the
// engine owns neither claim verification nor external retrieval, so it records
// the decision, skips the source, and reports a partial result.
//
// Given an always-verify controller,
// when the expansion runs,
// then nothing should be expanded, the result should be partial, the deferral
// should be explained, and no dead end should be fabricated for the deferred
// source.
#[test]
fn verify_action_defers_the_source_without_fabricating_dead_ends() {
    let (graph, campaign, _narrative, promotes) = campaign_graph();
    let ws = working_set_id("working-set--integration-verify");
    let mut manager = manager_with_working_set(&ws);
    let mut controller = FixedActionController {
        action: WorkingSetAction::Verify,
    };

    let result = expand_working_set_with_controller(
        &mut manager,
        &graph,
        request(
            &ws,
            vec![campaign.clone()],
            ExpansionFilters::empty(),
            generous_budget(),
        ),
        &mut controller,
    )
    .expect("deferred expansion should return a typed partial result");

    assert_eq!(result.status(), ExpansionResultStatus::Partial);
    assert!(
        result
            .explanation()
            .skipped_expansions()
            .iter()
            .any(|skipped| skipped.reason == SkippedExpansionReason::ControllerDecision)
    );

    let working_set = manager
        .get_working_set(&ws)
        .expect("working set should remain available");
    assert!(!working_set.hot_relationship_ids().contains(&promotes));

    let recorder = manager
        .telemetry(&ws)
        .expect("telemetry recorder should be available");
    assert!(
        !recorder.events().iter().any(|event| matches!(
            &event.decision,
            WorkingSetDecisionEvent::DeadEnd { node_id } if node_id == &campaign
        )),
        "a deferred source must not be recorded as a dead end"
    );
}

//
// Verify that hard budget guards override the controller: an always-expand
// policy cannot load past an exhausted relationship budget.
//
// Given an always-expand controller and a zero relationship budget,
// when the expansion runs,
// then the result should carry the same typed budget error as the plain
// engine, with nothing loaded past the budget.
#[test]
fn budget_guards_override_the_controller() {
    let (graph, campaign, _narrative, promotes) = campaign_graph();
    let ws = working_set_id("working-set--integration-budget-guard");
    let mut manager = manager_with_working_set(&ws);
    let mut controller = FixedActionController {
        action: WorkingSetAction::Expand,
    };
    let budget = ExpansionBudget {
        max_loaded_relationship_count: 0,
        max_hot_relationship_count: 0,
        ..generous_budget()
    };

    let result = expand_working_set_with_controller(
        &mut manager,
        &graph,
        request(&ws, vec![campaign], ExpansionFilters::empty(), budget),
        &mut controller,
    )
    .expect("budget stop should return a typed partial result");

    assert_eq!(result.status(), ExpansionResultStatus::Partial);
    let budget_error = result
        .budget_error()
        .expect("budget guard should override the controller");
    assert_eq!(budget_error.limit, ExpansionLimit::LoadedRelationshipCount);

    let working_set = manager
        .get_working_set(&ws)
        .expect("working set should remain available");
    assert!(!working_set.hot_relationship_ids().contains(&promotes));
}

//
// Verify that deterministic supernode protection overrides the controller: an
// always-expand policy cannot force an unguarded high-degree expansion.
//
// Given a high-degree seed, a supernode policy requiring missing guards, and
// an always-expand controller,
// when the expansion runs,
// then the typed supernode error should still block the expansion.
#[test]
fn supernode_protection_overrides_the_controller() {
    let mut graph = Graph::new();
    let hub = graph
        .create_node(NodeInput::new(["Campaign"]))
        .expect("hub node should be created");
    for _ in 0..3 {
        let neighbor = graph
            .create_node(NodeInput::new(["Narrative"]))
            .expect("neighbor node should be created");
        graph
            .create_relationship(
                RelationshipInput::new(hub.clone(), "PROMOTES", neighbor)
                    .expect("relationship input should be valid"),
            )
            .expect("hub relationship should be created");
    }
    let ws = working_set_id("working-set--integration-supernode-guard");
    let mut manager = manager_with_working_set(&ws);
    let mut controller = FixedActionController {
        action: WorkingSetAction::Expand,
    };

    let result = expand_working_set_with_controller(
        &mut manager,
        &graph,
        request(&ws, vec![hub], ExpansionFilters::empty(), generous_budget())
            .with_supernode_policy(SupernodePolicy {
                degree_threshold: 2,
                require_relationship_filter: true,
                require_label_filter: false,
                require_time_window: false,
                require_limit: false,
            }),
        &mut controller,
    )
    .expect("blocked expansion should return a typed partial result");

    assert_eq!(result.status(), ExpansionResultStatus::Partial);
    assert!(result.supernode_error().is_some());
}

//
// Verify that every controller consultation is recorded as a telemetry
// decision, so learned policies can be trained and audited from the stream.
//
// Given a controller-driven expansion inside a recorded retrieval,
// when the retrieval completes,
// then the retrieval record should contain the chosen controller action for
// the expanded source.
#[test]
fn controller_choices_are_recorded_in_telemetry() {
    let (graph, campaign, _narrative, _promotes) = campaign_graph();
    let ws = working_set_id("working-set--integration-telemetry");
    let mut manager = manager_with_working_set(&ws);
    let retrieval = retrieval_id("request--integration-telemetry");
    let mut controller = GreedyExpandController::new();

    manager
        .begin_retrieval_telemetry(
            &ws,
            retrieval.clone(),
            TelemetryQueryDescriptor {
                query_text: Some("controller telemetry".to_owned()),
                profile_kind: None,
                task_label: Some("fimi_investigation".to_owned()),
            },
        )
        .expect("retrieval telemetry should begin");
    expand_working_set_with_controller(
        &mut manager,
        &graph,
        request(
            &ws,
            vec![campaign.clone()],
            ExpansionFilters::empty(),
            generous_budget(),
        ),
        &mut controller,
    )
    .expect("controller-driven expansion should complete");
    manager
        .complete_retrieval_telemetry(
            &ws,
            &retrieval,
            RetrievalOutcome {
                evidence_record_ids: Vec::new(),
                answer_quality: None,
                memory_cost_bytes: 512,
                latency_ms: 4,
            },
        )
        .expect("retrieval telemetry should complete");

    let records = manager
        .telemetry(&ws)
        .expect("telemetry recorder should be available")
        .retrieval_records();
    assert_eq!(records.len(), 1);
    assert!(
        records[0].events.iter().any(|event| matches!(
            &event.decision,
            WorkingSetDecisionEvent::ControllerActionChosen {
                source_node_id: Some(source),
                action: WorkingSetAction::Expand,
            } if source == &campaign
        )),
        "the controller's expand choice should be recorded in the retrieval"
    );
}

//
// Verify the reward loop closure: rewards derived from the recorded retrieval
// feed back into the controller through the trait boundary.
//
// Given a completed controller-driven retrieval,
// when the reward is derived from its record and observed,
// then the controller should report one observed reward.
#[test]
fn rewards_derived_from_telemetry_feed_the_controller() {
    let (graph, campaign, _narrative, _promotes) = campaign_graph();
    let ws = working_set_id("working-set--integration-reward-loop");
    let mut manager = manager_with_working_set(&ws);
    let retrieval = retrieval_id("request--integration-reward-loop");
    let mut controller = GreedyExpandController::new();
    let expansion_request = request(
        &ws,
        vec![campaign],
        ExpansionFilters::empty(),
        generous_budget(),
    );
    let descriptor = TelemetryQueryDescriptor {
        query_text: Some("reward loop".to_owned()),
        profile_kind: None,
        task_label: Some("fimi_investigation".to_owned()),
    };

    manager
        .begin_retrieval_telemetry(&ws, retrieval.clone(), descriptor.clone())
        .expect("retrieval telemetry should begin");
    expand_working_set_with_controller(
        &mut manager,
        &graph,
        expansion_request.clone(),
        &mut controller,
    )
    .expect("controller-driven expansion should complete");
    manager
        .complete_retrieval_telemetry(
            &ws,
            &retrieval,
            RetrievalOutcome {
                evidence_record_ids: Vec::new(),
                answer_quality: None,
                memory_cost_bytes: 256,
                latency_ms: 3,
            },
        )
        .expect("retrieval telemetry should complete");

    let records = manager
        .telemetry(&ws)
        .expect("telemetry recorder should be available")
        .retrieval_records();
    let reward = BanditReward::from_retrieval_record(&records[0]);
    let context = BanditContext::from_expansion_request(&expansion_request, descriptor);
    controller.observe_reward(&context, WorkingSetAction::Expand, &reward);

    assert_eq!(controller.observed_reward_count(), 1);
    assert!(
        reward.io_count > 0,
        "the expansion should have paged in records"
    );
}

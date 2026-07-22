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
    BanditContext, BanditReward, BanditRewardWeights, Confidence, EvidenceId, ExpansionBudget,
    ExpansionBudgetUsage, ExpansionDirection, ExpansionFilters, ExpansionRequest, GraphRecordRef,
    GreedyExpandController, NodeId, RelationshipId, RelationshipType, RequestId, RetrievalOutcome,
    RetrievalTelemetryRecord, TelemetryQueryDescriptor, WorkingSetAction, WorkingSetController,
    WorkingSetDecisionEvent, WorkingSetId, WorkingSetTelemetryEvent,
    default_fimi_investigation_profile,
};

fn working_set_id(value: &str) -> WorkingSetId {
    WorkingSetId::new(value).expect("bandit working set ID should be valid")
}

fn node_id(value: &str) -> NodeId {
    NodeId::new(value).expect("bandit node ID should be valid")
}

fn relationship_id(value: &str) -> RelationshipId {
    RelationshipId::new(value).expect("bandit relationship ID should be valid")
}

fn rel_type(value: &str) -> RelationshipType {
    RelationshipType::new(value).expect("bandit relationship type should be valid")
}

fn retrieval_id(value: &str) -> RequestId {
    RequestId::new(value).expect("bandit retrieval ID should be valid")
}

fn evidence_id(value: &str) -> EvidenceId {
    EvidenceId::new(value).expect("bandit evidence ID should be valid")
}

fn descriptor() -> TelemetryQueryDescriptor {
    TelemetryQueryDescriptor {
        query_text: Some("bandit scenario".to_owned()),
        profile_kind: None,
        task_label: Some("fimi_investigation".to_owned()),
    }
}

fn budget() -> ExpansionBudget {
    ExpansionBudget {
        max_loaded_node_count: 16,
        max_loaded_relationship_count: 16,
        max_hot_node_count: 8,
        max_hot_relationship_count: 8,
        max_warm_adjacency_entry_count: 16,
        max_hop_count: 2,
        max_supernode_expansion_count: 4,
        max_payload_byte_count: 65_536,
        max_execution_time_ms: 500,
    }
}

fn zero_usage() -> ExpansionBudgetUsage {
    ExpansionBudgetUsage {
        loaded_node_count: 0,
        loaded_relationship_count: 0,
        hot_node_count: 0,
        hot_relationship_count: 0,
        warm_adjacency_entry_count: 0,
        hop_count: 0,
        supernode_expansion_count: 0,
        payload_byte_count: 0,
        execution_time_ms: 0,
    }
}

fn expansion_request(ws: &WorkingSetId) -> ExpansionRequest {
    ExpansionRequest::new(
        ws.clone(),
        vec![node_id("campaign--bandit-seed")],
        ExpansionDirection::Outgoing,
        ExpansionFilters::new(vec![rel_type("PROMOTES")], vec!["Narrative".to_owned()]),
        1,
        default_fimi_investigation_profile(),
        budget(),
    )
}

fn record(
    ws: &WorkingSetId,
    decisions: Vec<WorkingSetDecisionEvent>,
    outcome: Option<RetrievalOutcome>,
) -> RetrievalTelemetryRecord {
    RetrievalTelemetryRecord {
        retrieval_id: retrieval_id("request--bandit"),
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
        outcome,
    }
}

fn page_in_rel(relationship: &RelationshipId) -> WorkingSetDecisionEvent {
    WorkingSetDecisionEvent::PageIn {
        record: GraphRecordRef::Relationship(relationship.clone()),
    }
}

fn prefetch_rel(relationship: &RelationshipId) -> WorkingSetDecisionEvent {
    WorkingSetDecisionEvent::Prefetch {
        record: GraphRecordRef::Relationship(relationship.clone()),
    }
}

fn prefetch_node(node: &NodeId) -> WorkingSetDecisionEvent {
    WorkingSetDecisionEvent::Prefetch {
        record: GraphRecordRef::Node(node.clone()),
    }
}

fn expanded(relationship: &RelationshipId) -> WorkingSetDecisionEvent {
    WorkingSetDecisionEvent::EdgeExpanded {
        relationship_id: relationship.clone(),
    }
}

//
// Verify that the action set is exactly the epic's controller action space and
// stays enumerable for policies that must iterate candidate actions.
//
// Given the exported action set,
// when its variants are enumerated,
// then it should contain the six epic actions and nothing else.
#[test]
fn action_set_covers_the_epic_action_space() {
    assert_eq!(WorkingSetAction::ALL.len(), 6);
    for action in [
        WorkingSetAction::Expand,
        WorkingSetAction::Prefetch,
        WorkingSetAction::PageIn,
        WorkingSetAction::Stop,
        WorkingSetAction::Verify,
        WorkingSetAction::RetrieveExternally,
    ] {
        assert!(WorkingSetAction::ALL.contains(&action));
    }
}

//
// Verify that a bandit context can be built from an expansion request, carrying
// the query descriptor, seeds, filters, and budget without hand-copying.
//
// Given an expansion request with seeds, filters, and a budget,
// when a context is built from it,
// then the context should expose the same seeds, filters, and budget with zero
// consumed usage and an open budget.
#[test]
fn context_builds_from_an_expansion_request() {
    let ws = working_set_id("working-set--bandit-context");
    let request = expansion_request(&ws);

    let context = BanditContext::from_expansion_request(&request, descriptor());

    assert_eq!(context.descriptor, descriptor());
    assert_eq!(
        context.seed_node_ids,
        vec![node_id("campaign--bandit-seed")]
    );
    assert_eq!(
        context.relationship_type_filters,
        vec![rel_type("PROMOTES")]
    );
    assert_eq!(context.label_filters, vec!["Narrative".to_owned()]);
    assert_eq!(context.budget, budget());
    assert_eq!(context.consumed, zero_usage());
    assert!(context.frontier_degrees.is_empty());
    assert!(context.seed_confidences.is_empty());
    assert!(context.as_of.is_none());
    assert!(!context.budget_exhausted());
}

//
// Verify that budget exhaustion is detected from any exhausted counter, giving
// deterministic guards priority over any learned action choice.
//
// Given a context whose hot-node counter reaches its budget,
// when exhaustion is checked,
// then the context should report the budget as exhausted.
#[test]
fn budget_exhaustion_is_detected_per_counter() {
    let ws = working_set_id("working-set--bandit-exhaustion");
    let mut context = BanditContext::from_expansion_request(&expansion_request(&ws), descriptor());

    context.consumed.hot_node_count = context.budget.max_hot_node_count;
    assert!(context.budget_exhausted());

    context.consumed.hot_node_count = 0;
    context.consumed.payload_byte_count = context.budget.max_payload_byte_count;
    assert!(context.budget_exhausted());
}

//
// Verify that the reward signal is reproducible from one telemetry retrieval
// record: evidence, I/O, memory, and latency come from the recorded stream and
// outcome, never from live sampling.
//
// Given a completed record with two page-ins and an outcome,
// when a reward is derived from it,
// then the reward should carry the recorded counts and measurements.
#[test]
fn reward_derives_from_a_retrieval_record() {
    let ws = working_set_id("working-set--bandit-reward");
    let first = relationship_id("relationship--bandit-first");
    let second = relationship_id("relationship--bandit-second");
    let telemetry_record = record(
        &ws,
        vec![page_in_rel(&first), page_in_rel(&second)],
        Some(RetrievalOutcome {
            evidence_record_ids: vec![
                evidence_id("evidence--bandit-1"),
                evidence_id("evidence--bandit-2"),
                evidence_id("evidence--bandit-3"),
            ],
            answer_quality: None,
            memory_cost_bytes: 1_024,
            latency_ms: 7,
        }),
    );

    let reward = BanditReward::from_retrieval_record(&telemetry_record);

    assert_eq!(reward.evidence_found_count, 3);
    assert_eq!(reward.io_count, 2);
    assert_eq!(reward.memory_cost_bytes, 1_024);
    assert_eq!(reward.latency_ms, 7);
    assert_eq!(reward.wasted_prefetch_count, 0);
    assert!(reward.expected_subgraph_recall.is_none());

    let again = BanditReward::from_retrieval_record(&telemetry_record);
    assert_eq!(reward, again);
}

//
// Verify the wasted-prefetch rule: a prefetched record is used when it is later
// expanded, selected, or paged in within the same retrieval; otherwise the
// prefetch was wasted.
//
// Given one prefetch later expanded, one later paged in, and one never used,
// when a reward is derived,
// then exactly one wasted prefetch should be counted.
#[test]
fn wasted_prefetch_counts_unused_prefetches() {
    let ws = working_set_id("working-set--bandit-prefetch");
    let used_by_expansion = relationship_id("relationship--bandit-used-expanded");
    let used_by_page_in = relationship_id("relationship--bandit-used-paged");
    let never_used = node_id("node--bandit-wasted");
    let telemetry_record = record(
        &ws,
        vec![
            prefetch_rel(&used_by_expansion),
            prefetch_rel(&used_by_page_in),
            prefetch_node(&never_used),
            expanded(&used_by_expansion),
            page_in_rel(&used_by_page_in),
        ],
        None,
    );

    let reward = BanditReward::from_retrieval_record(&telemetry_record);

    assert_eq!(reward.wasted_prefetch_count, 1);
    assert_eq!(reward.io_count, 1);
}

//
// Verify that expected-subgraph recall is attached through the validated
// builder, since ground truth comes from the benchmark harness, not telemetry.
//
// Given a derived reward,
// when a recall confidence is attached,
// then the reward should expose it.
#[test]
fn expected_subgraph_recall_attaches_through_the_builder() {
    let ws = working_set_id("working-set--bandit-recall");
    let reward = BanditReward::from_retrieval_record(&record(&ws, Vec::new(), None))
        .with_expected_subgraph_recall(
            Confidence::new(0.75).expect("recall confidence should be valid"),
        );

    let recall = reward
        .expected_subgraph_recall
        .expect("recall should be attached");
    assert_eq!(recall.value(), 0.75);
}

//
// Verify the documented reward scalarization: positive terms for evidence and
// recall, penalties for memory, I/O, latency, and wasted prefetches.
//
// Given a fully populated reward and explicit weights,
// when the reward is scalarized,
// then the result should equal the documented weighted combination.
#[test]
fn scalarized_reward_follows_the_documented_combination() {
    let ws = working_set_id("working-set--bandit-scalar");
    let mut reward = BanditReward::from_retrieval_record(&record(&ws, Vec::new(), None));
    reward.evidence_found_count = 4;
    reward.expected_subgraph_recall =
        Some(Confidence::new(0.5).expect("recall confidence should be valid"));
    reward.memory_cost_bytes = 2_048;
    reward.io_count = 8;
    reward.latency_ms = 16;
    reward.wasted_prefetch_count = 2;

    let weights = BanditRewardWeights {
        evidence_weight: 1.0,
        recall_weight: 2.0,
        memory_weight: 0.001953125, // 1/512
        io_weight: 0.25,
        latency_weight: 0.125,
        wasted_prefetch_weight: 0.5,
    };

    // 4*1 + 0.5*2 - 2048/512 - 8*0.25 - 16*0.125 - 2*0.5 = 4 + 1 - 4 - 2 - 2 - 1
    assert_eq!(reward.scalarized(&weights), -4.0);
}

//
// Verify that policies are interchangeable behind the controller trait: the
// same call sites drive a custom policy and the baseline without changes, and
// action choice is deterministic for a fixed context.
//
// Given a custom always-verify policy and the greedy baseline boxed as trait
// objects,
// when both choose an action for the same context,
// then each should return its own deterministic action through the shared
// boundary.
#[test]
fn controllers_are_pluggable_through_the_trait_boundary() {
    struct AlwaysVerify;

    impl WorkingSetController for AlwaysVerify {
        fn choose_action(&mut self, _context: &BanditContext) -> WorkingSetAction {
            WorkingSetAction::Verify
        }

        fn observe_reward(
            &mut self,
            _context: &BanditContext,
            _action: WorkingSetAction,
            _reward: &BanditReward,
        ) {
        }
    }

    let ws = working_set_id("working-set--bandit-plugging");
    let context = BanditContext::from_expansion_request(&expansion_request(&ws), descriptor());
    let mut controllers: Vec<Box<dyn WorkingSetController>> = vec![
        Box::new(AlwaysVerify),
        Box::new(GreedyExpandController::new()),
    ];

    let first_pass: Vec<WorkingSetAction> = controllers
        .iter_mut()
        .map(|controller| controller.choose_action(&context))
        .collect();
    let second_pass: Vec<WorkingSetAction> = controllers
        .iter_mut()
        .map(|controller| controller.choose_action(&context))
        .collect();

    assert_eq!(
        first_pass,
        vec![WorkingSetAction::Verify, WorkingSetAction::Expand]
    );
    assert_eq!(first_pass, second_pass);
}

//
// Verify the greedy baseline contract: expand while the budget is open, stop
// once any counter is exhausted, and count observed rewards for diagnostics.
//
// Given the baseline controller and a context whose budget later exhausts,
// when actions are chosen before and after exhaustion and a reward is observed,
// then the controller should expand, then stop, and report one observation.
#[test]
fn greedy_baseline_expands_until_budget_exhaustion() {
    let ws = working_set_id("working-set--bandit-baseline");
    let mut context = BanditContext::from_expansion_request(&expansion_request(&ws), descriptor());
    let mut controller = GreedyExpandController::new();

    assert_eq!(controller.choose_action(&context), WorkingSetAction::Expand);

    context.consumed.hot_relationship_count = context.budget.max_hot_relationship_count;
    assert_eq!(controller.choose_action(&context), WorkingSetAction::Stop);

    let reward = BanditReward::from_retrieval_record(&record(&ws, Vec::new(), None));
    controller.observe_reward(&context, WorkingSetAction::Stop, &reward);
    assert_eq!(controller.observed_reward_count(), 1);
}

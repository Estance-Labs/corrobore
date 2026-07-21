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
use super::*;

/// Expand a manager-owned working set with a controller choosing per-source actions.
///
///
/// wire the pluggable controller boundary into the expansion loop while
/// keeping deterministic budgets and supernode protection as hard constraints
/// the controller can never override; the plain entry point below remains the
/// unchanged no-controller default.
///
///
/// consult the controller before each frontier-source expansion with an
/// up-to-date context, record every choice as telemetry, and apply the action
/// semantics: `Expand`/`PageIn` expand normally, `Prefetch` materializes the
/// source frontier as warm adjacency, `Stop` halts expansion with a partial
/// result, and `Verify`/`RetrieveExternally` defer the source to the caller
/// with a partial result; seeds stay ring-0 entry points loaded before any
/// consultation.
///
/// # Errors
///
/// return the same typed errors as the plain expansion; controller decisions
/// themselves cannot fail.
pub fn expand_working_set_with_controller<P, C>(
    manager: &mut GraphWorkingSetManager,
    graph_pager: &P,
    request: ExpansionRequest,
    controller: &mut C,
) -> Result<ExpansionResult, GraphError>
where
    P: GraphPager + ?Sized,
    C: WorkingSetController,
{
    expand_internal(
        manager,
        graph_pager,
        request,
        Some(controller as &mut dyn WorkingSetController),
    )
}

/// Expand a manager-owned working set from graph-core adjacency under an explicit budget.
pub fn expand_working_set_from_graph_adjacency<P: GraphPager + ?Sized>(
    manager: &mut GraphWorkingSetManager,
    graph_pager: &P,
    request: ExpansionRequest,
) -> Result<ExpansionResult, GraphError> {
    expand_internal(manager, graph_pager, request, None)
}

fn expand_internal<P: GraphPager + ?Sized>(
    manager: &mut GraphWorkingSetManager,
    graph_pager: &P,
    request: ExpansionRequest,
    mut controller: Option<&mut dyn WorkingSetController>,
) -> Result<ExpansionResult, GraphError> {
    manager.get_working_set(request.working_set_id())?;

    let mut status = ExpansionResultStatus::Complete;
    let mut usage = empty_usage();
    let mut explanation = WorkingSetExplanation::new();
    let mut budget_error = None;
    let mut supernode_error = None;
    let mut hot_frontier = Vec::new();

    for seed_node_id in request.seed_node_ids() {
        let next_usage = with_hot_node_usage(&usage);
        if let Err(error) = request.budget().check_usage(&next_usage) {
            status = ExpansionResultStatus::Partial;
            record_budget_skip(
                manager,
                &mut explanation,
                request.working_set_id(),
                seed_node_id.clone(),
                Some(seed_node_id.clone()),
                None,
                None,
                &error,
            )?;
            budget_error = Some(error);
            break;
        }

        graph_pager
            .load_node_payload(seed_node_id)
            .map_err(map_pager_error)?;
        manager.record_telemetry_decision(
            request.working_set_id(),
            WorkingSetDecisionEvent::PageIn {
                record: GraphRecordRef::Node(seed_node_id.clone()),
            },
        )?;
        manager.load_seed_node_ids(request.working_set_id(), [seed_node_id.clone()], true)?;
        usage = next_usage;
        explanation.record_seed_node(seed_explanation(seed_node_id));
        explanation.record_hot_node(HotNodeExplanation {
            node_id: seed_node_id.clone(),
            reason: HotNodeLoadReason::SeedNode,
            via_relationship_id: None,
            profile_kind: Some(request.loading_profile().kind),
            hop_count: Some(0),
        });
        hot_frontier.push((seed_node_id.clone(), 0));
    }

    if budget_error.is_none() && request.hop_limit() > 0 {
        'expansion: for (source_node_id, source_hop) in hot_frontier.clone() {
            if source_hop >= request.hop_limit() {
                continue;
            }

            // Controller consultation happens before any adjacency I/O so a
            // stop or prefetch decision can avoid the load entirely. Budgets
            // and supernode guards below stay authoritative over any choice.
            if let Some(active_controller) = &mut controller {
                let context = controller_context(manager, &request, &usage)?;
                let action = active_controller.choose_action(&context);
                manager.record_telemetry_decision(
                    request.working_set_id(),
                    WorkingSetDecisionEvent::ControllerActionChosen {
                        source_node_id: Some(source_node_id.clone()),
                        action,
                    },
                )?;

                match action {
                    WorkingSetAction::Expand | WorkingSetAction::PageIn => {}
                    WorkingSetAction::Prefetch => {
                        if let Some(error) = attach_warm_frontier(
                            manager,
                            graph_pager,
                            &request,
                            &mut usage,
                            &mut explanation,
                            &source_node_id,
                        )? {
                            status = ExpansionResultStatus::Partial;
                            budget_error = Some(error);
                            break 'expansion;
                        }
                        continue;
                    }
                    WorkingSetAction::Stop => {
                        status = ExpansionResultStatus::Partial;
                        record_controller_skip(&mut explanation, source_node_id.clone());
                        break 'expansion;
                    }
                    WorkingSetAction::Verify | WorkingSetAction::RetrieveExternally => {
                        status = ExpansionResultStatus::Partial;
                        record_controller_skip(&mut explanation, source_node_id.clone());
                        continue;
                    }
                }
            }

            // Telemetry-only observation: a frontier source that admits no
            // expansion across all directions is a dead end for the future
            // anti-pheromone field. Counting it never alters traversal.
            let mut admitted_expansion_count: u64 = 0;

            for adjacency_direction in adjacency_directions(request.direction()) {
                let adjacency = load_adjacency(graph_pager, &source_node_id, adjacency_direction)?;

                if let Some(policy) = request.supernode_policy() {
                    let observed_degree = observed_degree_from_adjacency(&adjacency)?;

                    match check_supernode_expansion_guards(
                        policy,
                        &source_node_id,
                        observed_degree,
                        &request,
                    ) {
                        Ok(()) => {
                            if policy.is_high_degree_node(observed_degree) {
                                let next_usage = with_supernode_expansion_usage(&usage);
                                if let Err(error) = request.budget().check_usage(&next_usage) {
                                    status = ExpansionResultStatus::Partial;
                                    record_budget_skip(
                                        manager,
                                        &mut explanation,
                                        request.working_set_id(),
                                        source_node_id.clone(),
                                        Some(source_node_id.clone()),
                                        None,
                                        None,
                                        &error,
                                    )?;
                                    budget_error = Some(error);
                                    break 'expansion;
                                }
                                usage = next_usage;
                            }
                        }
                        Err(GraphError::SupernodeExpansionBlocked(error)) => {
                            status = ExpansionResultStatus::Partial;
                            manager.record_telemetry_decision(
                                request.working_set_id(),
                                WorkingSetDecisionEvent::SupernodeBlocked {
                                    node_id: source_node_id.clone(),
                                },
                            )?;
                            record_supernode_blocked_expansion(
                                &mut explanation,
                                source_node_id.clone(),
                                &error,
                            )?;
                            supernode_error = Some(error);
                            break 'expansion;
                        }
                        Err(error) => return Err(error),
                    }
                }

                for entry in adjacency.entries {
                    let relationship_type = relationship_type_for_entry(graph_pager, &entry)?;
                    if !relationship_type_allowed(
                        request.relationship_type_filters(),
                        &relationship_type,
                    ) {
                        record_profile_skip(
                            manager,
                            &mut explanation,
                            request.working_set_id(),
                            source_node_id.clone(),
                            Some(entry.neighbor_node_id.clone()),
                            Some(entry.relationship_id.clone()),
                            Some(relationship_type),
                        )?;
                        continue;
                    }

                    let target_labels = labels_for_node(graph_pager, &entry.neighbor_node_id)?;
                    if !labels_allowed(request.label_filters(), &target_labels) {
                        record_profile_skip(
                            manager,
                            &mut explanation,
                            request.working_set_id(),
                            source_node_id.clone(),
                            Some(entry.neighbor_node_id.clone()),
                            Some(entry.relationship_id.clone()),
                            Some(relationship_type),
                        )?;
                        continue;
                    }

                    let after_relationship_usage = with_hot_relationship_usage(&usage);
                    if let Err(error) = request.budget().check_usage(&after_relationship_usage) {
                        status = ExpansionResultStatus::Partial;
                        record_budget_skip(
                            manager,
                            &mut explanation,
                            request.working_set_id(),
                            source_node_id.clone(),
                            Some(entry.neighbor_node_id.clone()),
                            Some(entry.relationship_id.clone()),
                            Some(relationship_type),
                            &error,
                        )?;
                        budget_error = Some(error);
                        break 'expansion;
                    }

                    let mut after_node_usage = with_hot_node_usage(&after_relationship_usage);
                    after_node_usage.hop_count = after_node_usage.hop_count.max(source_hop + 1);
                    if let Err(error) = request.budget().check_usage(&after_node_usage) {
                        status = ExpansionResultStatus::Partial;
                        record_budget_skip(
                            manager,
                            &mut explanation,
                            request.working_set_id(),
                            source_node_id.clone(),
                            Some(entry.neighbor_node_id.clone()),
                            Some(entry.relationship_id.clone()),
                            Some(relationship_type),
                            &error,
                        )?;
                        budget_error = Some(error);
                        break 'expansion;
                    }

                    graph_pager
                        .load_relationship_payload(&entry.relationship_id)
                        .map_err(map_pager_error)?;
                    manager.record_telemetry_decision(
                        request.working_set_id(),
                        WorkingSetDecisionEvent::PageIn {
                            record: GraphRecordRef::Relationship(entry.relationship_id.clone()),
                        },
                    )?;
                    graph_pager
                        .load_node_payload(&entry.neighbor_node_id)
                        .map_err(map_pager_error)?;
                    manager.record_telemetry_decision(
                        request.working_set_id(),
                        WorkingSetDecisionEvent::PageIn {
                            record: GraphRecordRef::Node(entry.neighbor_node_id.clone()),
                        },
                    )?;

                    admitted_expansion_count += 1;
                    manager.add_hot_relationship(
                        request.working_set_id(),
                        entry.relationship_id.clone(),
                    )?;
                    manager.load_seed_node_ids(
                        request.working_set_id(),
                        [entry.neighbor_node_id.clone()],
                        true,
                    )?;

                    let (relationship_source, relationship_target) = relationship_endpoints(
                        &source_node_id,
                        &entry.neighbor_node_id,
                        adjacency_direction,
                    );
                    explanation.record_hot_relationship(HotRelationshipExplanation {
                        relationship_id: entry.relationship_id.clone(),
                        relationship_type: relationship_type.clone(),
                        source_node_id: relationship_source,
                        target_node_id: relationship_target,
                        reason: hot_relationship_reason(
                            request.loading_profile(),
                            &relationship_type,
                        ),
                        profile_kind: Some(request.loading_profile().kind),
                        hop_count: Some(source_hop + 1),
                    });
                    explanation.record_hot_node(HotNodeExplanation {
                        node_id: entry.neighbor_node_id.clone(),
                        reason: hot_node_reason(request.loading_profile(), &target_labels),
                        via_relationship_id: Some(entry.relationship_id.clone()),
                        profile_kind: Some(request.loading_profile().kind),
                        hop_count: Some(source_hop + 1),
                    });

                    usage = after_node_usage;

                    if source_hop + 1 >= request.hop_limit()
                        && let Some(error) = attach_warm_frontier(
                            manager,
                            graph_pager,
                            &request,
                            &mut usage,
                            &mut explanation,
                            &entry.neighbor_node_id,
                        )?
                    {
                        status = ExpansionResultStatus::Partial;
                        budget_error = Some(error);
                        break 'expansion;
                    }
                }
            }

            if admitted_expansion_count == 0 {
                manager.record_telemetry_decision(
                    request.working_set_id(),
                    WorkingSetDecisionEvent::DeadEnd {
                        node_id: source_node_id.clone(),
                    },
                )?;
            }
        }
    }

    explanation.record_consumed_budget(usage.clone());

    let mut result = ExpansionResult::new(
        request.working_set_id().clone(),
        status,
        usage,
        explanation,
        budget_error,
    );

    if let Some(error) = supernode_error {
        result = result.with_supernode_error(error);
    }

    Ok(result)
}

pub(crate) fn controller_context(
    manager: &GraphWorkingSetManager,
    request: &ExpansionRequest,
    usage: &ExpansionBudgetUsage,
) -> Result<BanditContext, GraphError> {
    let mut context = BanditContext::from_expansion_request(
        request,
        TelemetryQueryDescriptor {
            query_text: None,
            profile_kind: Some(request.loading_profile().kind),
            task_label: None,
        },
    );
    context.consumed = usage.clone();
    context.working_set_stats = manager.stats(request.working_set_id())?.clone();
    Ok(context)
}

pub(crate) fn record_controller_skip(
    explanation: &mut WorkingSetExplanation,
    source_node_id: NodeId,
) {
    explanation.record_skipped_expansion(SkippedExpansionExplanation {
        source_node_id,
        candidate_node_id: None,
        relationship_id: None,
        relationship_type: None,
        reason: SkippedExpansionReason::ControllerDecision,
        budget_counter: None,
        fix_hint: None,
    });
}

pub(crate) fn empty_usage() -> ExpansionBudgetUsage {
    ExpansionBudgetUsage {
        // Loaded node count.
        loaded_node_count: 0,
        // Loaded relationship count.
        loaded_relationship_count: 0,
        // Hot node count.
        hot_node_count: 0,
        // Hot relationship count.
        hot_relationship_count: 0,
        // Warm adjacency entry count.
        warm_adjacency_entry_count: 0,
        // Hop count.
        hop_count: 0,
        // Supernode expansion count.
        supernode_expansion_count: 0,
        // Payload byte count.
        payload_byte_count: 0,
        // Execution time ms.
        execution_time_ms: 0,
    }
}

pub(crate) fn with_hot_node_usage(usage: &ExpansionBudgetUsage) -> ExpansionBudgetUsage {
    let mut next = usage.clone();
    next.loaded_node_count += 1;
    next.hot_node_count += 1;
    next
}

pub(crate) fn with_hot_relationship_usage(usage: &ExpansionBudgetUsage) -> ExpansionBudgetUsage {
    let mut next = usage.clone();
    next.loaded_relationship_count += 1;
    next.hot_relationship_count += 1;
    next
}

pub(crate) fn with_warm_adjacency_usage(usage: &ExpansionBudgetUsage) -> ExpansionBudgetUsage {
    let mut next = usage.clone();
    next.loaded_relationship_count += 1;
    next.warm_adjacency_entry_count += 1;
    next
}

pub(crate) fn with_supernode_expansion_usage(usage: &ExpansionBudgetUsage) -> ExpansionBudgetUsage {
    let mut next = usage.clone();
    next.supernode_expansion_count += 1;
    next
}

pub(crate) fn adjacency_directions(direction: ExpansionDirection) -> Vec<AdjacencyDirection> {
    match direction {
        ExpansionDirection::Outgoing => vec![AdjacencyDirection::Outgoing],
        ExpansionDirection::Incoming => vec![AdjacencyDirection::Incoming],
        ExpansionDirection::Both => {
            vec![AdjacencyDirection::Outgoing, AdjacencyDirection::Incoming]
        }
    }
}

pub(crate) fn load_adjacency<P: GraphPager + ?Sized>(
    graph_pager: &P,
    node_id: &NodeId,
    direction: AdjacencyDirection,
) -> Result<PagedAdjacency, GraphError> {
    match direction {
        AdjacencyDirection::Outgoing => graph_pager
            .load_outgoing_adjacency(node_id)
            .map_err(map_pager_error),
        AdjacencyDirection::Incoming => graph_pager
            .load_incoming_adjacency(node_id)
            .map_err(map_pager_error),
    }
}

pub(crate) fn relationship_type_for_entry<P: GraphPager + ?Sized>(
    graph_pager: &P,
    entry: &PagedAdjacencyEntry,
) -> Result<RelationshipType, GraphError> {
    if let Some(relationship_type) = &entry.relationship_type {
        return Ok(relationship_type.clone());
    }

    let metadata = graph_pager
        .load_indexed_metadata(&GraphRecordRef::Relationship(entry.relationship_id.clone()))
        .map_err(map_pager_error)?;
    metadata.relationship_type.ok_or_else(|| {
        GraphError::InternalInvariantViolation(format!(
            "missing relationship type metadata for {}",
            entry.relationship_id.as_str()
        ))
    })
}

pub(crate) fn labels_for_node<P: GraphPager + ?Sized>(
    graph_pager: &P,
    node_id: &NodeId,
) -> Result<LabelSet, GraphError> {
    let metadata = graph_pager
        .load_indexed_metadata(&GraphRecordRef::Node(node_id.clone()))
        .map_err(map_pager_error)?;
    Ok(metadata.labels)
}

pub(crate) fn relationship_type_allowed(
    filters: &[RelationshipType],
    relationship_type: &RelationshipType,
) -> bool {
    filters.is_empty() || filters.contains(relationship_type)
}

pub(crate) fn labels_allowed(filters: &LabelSet, labels: &LabelSet) -> bool {
    filters.is_empty()
        || filters
            .iter()
            .any(|required_label| labels.iter().any(|label| label == required_label))
}

pub(crate) fn hot_node_reason(profile: &LoadingProfile, labels: &LabelSet) -> HotNodeLoadReason {
    if labels.iter().any(|label| {
        profile
            .hot_labels
            .iter()
            .any(|hot_label| hot_label == label)
    }) {
        HotNodeLoadReason::ProfileHotLabel
    } else {
        HotNodeLoadReason::TraversalExpansion
    }
}

pub(crate) fn hot_relationship_reason(
    profile: &LoadingProfile,
    relationship_type: &RelationshipType,
) -> HotRelationshipLoadReason {
    if profile
        .prioritized_relationship_types
        .iter()
        .any(|candidate| candidate == relationship_type)
    {
        HotRelationshipLoadReason::PrioritizedRelationshipType
    } else {
        HotRelationshipLoadReason::TraversalExpansion
    }
}

pub(crate) fn relationship_endpoints(
    frontier_node_id: &NodeId,
    neighbor_node_id: &NodeId,
    direction: AdjacencyDirection,
) -> (NodeId, NodeId) {
    match direction {
        AdjacencyDirection::Outgoing => (frontier_node_id.clone(), neighbor_node_id.clone()),
        AdjacencyDirection::Incoming => (neighbor_node_id.clone(), frontier_node_id.clone()),
    }
}

pub(crate) fn attach_warm_frontier<P: GraphPager + ?Sized>(
    manager: &mut GraphWorkingSetManager,
    graph_pager: &P,
    request: &ExpansionRequest,
    usage: &mut ExpansionBudgetUsage,
    explanation: &mut WorkingSetExplanation,
    source_node_id: &NodeId,
) -> Result<Option<ExpansionBudgetExceeded>, GraphError> {
    for adjacency_direction in adjacency_directions(request.direction()) {
        let adjacency = load_adjacency(graph_pager, source_node_id, adjacency_direction)?;

        for entry in adjacency.entries {
            let relationship_type = relationship_type_for_entry(graph_pager, &entry)?;
            let target_labels = labels_for_node(graph_pager, &entry.neighbor_node_id)?;
            let next_usage = with_warm_adjacency_usage(usage);

            if let Err(error) = request.budget().check_usage(&next_usage) {
                record_budget_skip(
                    manager,
                    explanation,
                    request.working_set_id(),
                    source_node_id.clone(),
                    Some(entry.neighbor_node_id.clone()),
                    Some(entry.relationship_id.clone()),
                    Some(relationship_type),
                    &error,
                )?;
                return Ok(Some(error));
            }

            let warm_entry = WarmAdjacencyEntry::new(
                WarmAdjacencyEntryInput::new(
                    entry.relationship_id.clone(),
                    relationship_type.clone(),
                    source_node_id.clone(),
                    entry.neighbor_node_id.clone(),
                    target_labels,
                    adjacency_direction,
                )
                .with_storage_refs(
                    entry.relationship_storage_ref.clone(),
                    entry.neighbor_storage_ref.clone(),
                ),
            )?;
            manager.add_warm_adjacency(
                request.working_set_id(),
                source_node_id.clone(),
                warm_entry,
            )?;
            *usage = next_usage;
            explanation.record_warm_adjacency(WarmAdjacencyExplanation {
                relationship_id: entry.relationship_id,
                relationship_type,
                source_node_id: source_node_id.clone(),
                target_node_id: entry.neighbor_node_id,
                target_loading_state: LoadingState::Warm,
                reason: WarmAdjacencyReason::RingBoundary,
                profile_kind: Some(request.loading_profile().kind),
                relevance_score: None,
            });
        }
    }

    Ok(None)
}

pub(crate) fn seed_explanation(node_id: &NodeId) -> SeedNodeExplanation {
    SeedNodeExplanation {
        // Node id.
        node_id: node_id.clone(),
        // Source.
        source: SeedSourceMetadata {
            // Kind.
            kind: SeedSourceKind::ExplicitNodeId,
            // Source id.
            source_id: Some(node_id.as_str().to_owned()),
            // Source label.
            source_label: None,
            // Score.
            score: None,
        },
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn record_profile_skip(
    manager: &mut GraphWorkingSetManager,
    explanation: &mut WorkingSetExplanation,
    working_set_id: &WorkingSetId,
    source_node_id: NodeId,
    candidate_node_id: Option<NodeId>,
    relationship_id: Option<RelationshipId>,
    relationship_type: Option<RelationshipType>,
) -> Result<(), GraphError> {
    manager.record_telemetry_decision(
        working_set_id,
        WorkingSetDecisionEvent::EdgeSkipped {
            source_node_id: source_node_id.clone(),
            candidate_node_id: candidate_node_id.clone(),
            relationship_id: relationship_id.clone(),
            reason: SkippedExpansionReason::BlockedByProfile,
        },
    )?;
    explanation.record_skipped_expansion(SkippedExpansionExplanation {
        source_node_id,
        candidate_node_id,
        relationship_id,
        relationship_type,
        reason: SkippedExpansionReason::BlockedByProfile,
        budget_counter: None,
        fix_hint: Some(ExpansionFixHint {
            scope: FixHintScope::LoadingProfile,
            message: "Narrow or adjust relationship and label filters for this expansion."
                .to_owned(),
        }),
    });
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn record_budget_skip(
    manager: &mut GraphWorkingSetManager,
    explanation: &mut WorkingSetExplanation,
    working_set_id: &WorkingSetId,
    source_node_id: NodeId,
    candidate_node_id: Option<NodeId>,
    relationship_id: Option<RelationshipId>,
    relationship_type: Option<RelationshipType>,
    error: &ExpansionBudgetExceeded,
) -> Result<(), GraphError> {
    manager.record_telemetry_decision(
        working_set_id,
        WorkingSetDecisionEvent::EdgeSkipped {
            source_node_id: source_node_id.clone(),
            candidate_node_id: candidate_node_id.clone(),
            relationship_id: relationship_id.clone(),
            reason: SkippedExpansionReason::BudgetLimit,
        },
    )?;
    explanation.record_skipped_expansion(SkippedExpansionExplanation {
        source_node_id,
        candidate_node_id,
        relationship_id,
        relationship_type,
        reason: SkippedExpansionReason::BudgetLimit,
        budget_counter: Some(BudgetCounterExplanation {
            limit: error.limit,
            allowed: error.allowed,
            consumed: error.consumed,
            remaining: None,
        }),
        fix_hint: Some(ExpansionFixHint {
            scope: FixHintScope::Budget,
            message: error.fix_hint.clone(),
        }),
    });
    Ok(())
}

pub(crate) fn map_pager_error(error: GraphPagerError) -> GraphError {
    GraphError::InternalInvariantViolation(format!("graph pager error: {error}"))
}

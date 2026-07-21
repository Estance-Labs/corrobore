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
    CoordinatedEventHyperrelation, DerivationLinkId, GraphError, HyperrelationExpansionRequest,
    HyperrelationId, HyperrelationParticipant, HyperrelationParticipantRole,
    HyperrelationTimeWindow, MixedTraversalOperator, MixedTraversalStep, NodeId, RelationshipId,
    ResolutionArtifactId, ResolutionLevel, ResolutionRecordRef, TemporalTimestamp,
    execute_mixed_traversal, query_hyperrelation_expansion,
};

fn node_id(value: &str) -> NodeId {
    NodeId::new(value).expect("test node ID should be valid")
}

fn resolution_ref(level: ResolutionLevel, value: &str) -> ResolutionRecordRef {
    ResolutionRecordRef::new(
        level,
        ResolutionArtifactId::new(value).expect("artifact ID should be valid"),
    )
}

fn derivation_id(value: &str) -> DerivationLinkId {
    DerivationLinkId::new(value).expect("derivation-link ID should be valid")
}

fn coordinated_event() -> CoordinatedEventHyperrelation {
    CoordinatedEventHyperrelation::new(
        HyperrelationId::new("hyperrelation--coordination").expect("ID should be valid"),
        vec![
            HyperrelationParticipant::new(
                node_id("node--actor-b"),
                HyperrelationParticipantRole::Actor,
            ),
            HyperrelationParticipant::new(
                node_id("node--infrastructure"),
                HyperrelationParticipantRole::Infrastructure,
            ),
            HyperrelationParticipant::new(
                node_id("node--narrative"),
                HyperrelationParticipantRole::Narrative,
            ),
            HyperrelationParticipant::new(
                node_id("node--actor-a"),
                HyperrelationParticipantRole::Actor,
            ),
        ],
        HyperrelationTimeWindow::new(
            TemporalTimestamp::new("2026-07-19T10:00:00Z").expect("start should be valid"),
            TemporalTimestamp::new("2026-07-19T11:00:00Z").expect("end should be valid"),
        )
        .expect("time window should be valid"),
        "Accounts coordinated one narrative through shared infrastructure",
    )
    .expect("coordinated event should be valid")
}

#[test]
fn adjacent_resolution_hops_preserve_derivation_audit_links() {
    let result = execute_mixed_traversal(vec![
        MixedTraversalStep::across_resolution(
            derivation_id("derivation--tactical-operational"),
            resolution_ref(ResolutionLevel::Tactical, "artifact--observation"),
            resolution_ref(ResolutionLevel::Operational, "artifact--narrative"),
        ),
        MixedTraversalStep::across_resolution(
            derivation_id("derivation--operational-strategic"),
            resolution_ref(ResolutionLevel::Operational, "artifact--narrative"),
            resolution_ref(ResolutionLevel::Strategic, "artifact--campaign"),
        ),
    ])
    .expect("adjacent resolution traversal should succeed");

    assert_eq!(result.steps().len(), 2);
    assert_eq!(
        result.explanations()[0].operator(),
        MixedTraversalOperator::AbstractResolution
    );
    assert_eq!(
        result.explanations()[0].audit_ref(),
        "derivation--tactical-operational"
    );
    assert_eq!(
        result.explanations()[1].operator(),
        MixedTraversalOperator::AbstractResolution
    );
    assert!(result.score().total() > 0);

    let drill_down = execute_mixed_traversal(vec![
        MixedTraversalStep::across_resolution(
            derivation_id("derivation--strategic-operational"),
            resolution_ref(ResolutionLevel::Strategic, "artifact--campaign"),
            resolution_ref(ResolutionLevel::Operational, "artifact--narrative"),
        ),
        MixedTraversalStep::across_resolution(
            derivation_id("derivation--operational-tactical"),
            resolution_ref(ResolutionLevel::Operational, "artifact--narrative"),
            resolution_ref(ResolutionLevel::Tactical, "artifact--observation"),
        ),
    ])
    .expect("adjacent drill-down traversal should succeed");
    assert!(
        drill_down
            .explanations()
            .iter()
            .all(|entry| entry.operator() == MixedTraversalOperator::DrillDownResolution)
    );
}

#[test]
fn hyperrelation_expansion_requires_explicit_filter_when_roles_are_ambiguous() {
    let event = coordinated_event();
    let ambiguous_request = HyperrelationExpansionRequest::new(node_id("node--actor-a"));
    let ambiguous_error = query_hyperrelation_expansion(&event, &ambiguous_request)
        .expect_err("unfiltered multi-role expansion should be ambiguous");
    assert!(matches!(
        ambiguous_error,
        GraphError::InvalidMixedTraversal(_)
    ));

    let filtered_request = HyperrelationExpansionRequest::new(node_id("node--actor-a"))
        .with_role_filter(vec![HyperrelationParticipantRole::Infrastructure])
        .expect("non-empty role filter should be valid");
    let filtered = query_hyperrelation_expansion(&event, &filtered_request)
        .expect("explicit infrastructure expansion should succeed");

    assert_eq!(filtered.participants().len(), 1);
    assert_eq!(
        filtered.participants()[0].node_id().as_str(),
        "node--infrastructure"
    );
    assert_eq!(
        filtered.explanation().hyperrelation_id().as_str(),
        "hyperrelation--coordination"
    );

    let unknown_entry = HyperrelationExpansionRequest::new(node_id("node--not-a-participant"));
    let unknown_error = query_hyperrelation_expansion(&event, &unknown_entry)
        .expect_err("unknown entry participant should fail");
    assert!(matches!(
        unknown_error,
        GraphError::InvalidMixedTraversal(_)
    ));

    let empty_filter_error = HyperrelationExpansionRequest::new(node_id("node--actor-a"))
        .with_role_filter(Vec::new())
        .expect_err("empty role filter should fail");
    assert!(matches!(
        empty_filter_error,
        GraphError::InvalidMixedTraversal(_)
    ));
}

#[test]
fn mixed_binary_and_hyperrelation_paths_are_reproducible_and_explained() {
    let event = coordinated_event();
    let steps = vec![
        MixedTraversalStep::binary(
            RelationshipId::new("relationship--communicates")
                .expect("relationship ID should be valid"),
            node_id("node--actor-a"),
            node_id("node--actor-b"),
        ),
        MixedTraversalStep::enter_hyperrelation(&event, node_id("node--actor-b"))
            .expect("actor B should enter the hyperrelation"),
        MixedTraversalStep::expand_hyperrelation(&event, node_id("node--infrastructure"))
            .expect("infrastructure should be expandable"),
    ];

    let first = execute_mixed_traversal(steps.clone()).expect("mixed traversal should be valid");
    let second = execute_mixed_traversal(steps).expect("mixed traversal should be valid");

    assert_eq!(first, second);
    assert_eq!(
        serde_json::to_string(&first).expect("first result should serialize"),
        serde_json::to_string(&second).expect("second result should serialize")
    );
    assert_eq!(
        first.explanations()[1].operator(),
        MixedTraversalOperator::EnterHyperrelation
    );
    assert_eq!(
        first.explanations()[2].operator(),
        MixedTraversalOperator::ExpandHyperrelation
    );
}

#[test]
fn non_adjacent_resolution_shortcuts_and_disconnected_hops_are_rejected() {
    let shortcut_error = execute_mixed_traversal(vec![MixedTraversalStep::across_resolution(
        derivation_id("derivation--shortcut"),
        resolution_ref(ResolutionLevel::Tactical, "artifact--observation"),
        resolution_ref(ResolutionLevel::Strategic, "artifact--campaign"),
    )])
    .expect_err("non-adjacent resolution shortcut should fail");
    assert!(matches!(
        shortcut_error,
        GraphError::InvalidMixedTraversal(_)
    ));

    let disconnected_error = execute_mixed_traversal(vec![
        MixedTraversalStep::binary(
            RelationshipId::new("relationship--first").expect("ID should be valid"),
            node_id("node--a"),
            node_id("node--b"),
        ),
        MixedTraversalStep::binary(
            RelationshipId::new("relationship--second").expect("ID should be valid"),
            node_id("node--c"),
            node_id("node--d"),
        ),
    ])
    .expect_err("disconnected path should fail");
    assert!(matches!(
        disconnected_error,
        GraphError::InvalidMixedTraversal(_)
    ));
}

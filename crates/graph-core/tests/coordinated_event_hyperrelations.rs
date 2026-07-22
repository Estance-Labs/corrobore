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
    CoordinatedEventHyperrelation, GraphError, HyperrelationId, HyperrelationParticipant,
    HyperrelationParticipantRole, HyperrelationSchema, HyperrelationTimeWindow, NodeId,
    RelationshipInput, TemporalTimestamp,
};

fn node_id(value: &str) -> NodeId {
    NodeId::new(value).expect("test node ID should be valid")
}

fn participant(value: &str, role: HyperrelationParticipantRole) -> HyperrelationParticipant {
    HyperrelationParticipant::new(node_id(value), role)
}

fn time_window() -> HyperrelationTimeWindow {
    HyperrelationTimeWindow::new(
        TemporalTimestamp::new("2026-07-19T10:00:00Z").expect("start should be valid"),
        TemporalTimestamp::new("2026-07-19T11:00:00Z").expect("end should be valid"),
    )
    .expect("time window should be valid")
}

fn valid_participants() -> Vec<HyperrelationParticipant> {
    vec![
        participant("node--actor-b", HyperrelationParticipantRole::Actor),
        participant(
            "node--infrastructure",
            HyperrelationParticipantRole::Infrastructure,
        ),
        participant("node--narrative", HyperrelationParticipantRole::Narrative),
        participant("node--actor-a", HyperrelationParticipantRole::Actor),
    ]
}

#[test]
fn coordinated_event_is_a_typed_first_class_nary_hyperrelation() {
    let mut participants = valid_participants();
    participants.push(participant(
        "node--related-entity",
        HyperrelationParticipantRole::RelatedEntity,
    ));
    let event = CoordinatedEventHyperrelation::new(
        HyperrelationId::new("hyperrelation--coordination-1")
            .expect("hyperrelation ID should be valid"),
        participants,
        time_window(),
        "Accounts amplified one narrative through shared infrastructure",
    )
    .expect("coordinated event should be valid");

    assert_eq!(event.schema(), HyperrelationSchema::CoordinatedEventV1);
    assert_eq!(event.participants().len(), 5);
    assert_eq!(
        event
            .participants_for_role(HyperrelationParticipantRole::Actor)
            .len(),
        2
    );
    assert_eq!(
        event
            .participants_for_role(HyperrelationParticipantRole::RelatedEntity)
            .len(),
        1
    );
    assert_eq!(
        event.narrative_context(),
        "Accounts amplified one narrative through shared infrastructure"
    );
    assert_eq!(event.time_window().start().as_str(), "2026-07-19T10:00:00Z");
    assert_eq!(event.time_window().end().as_str(), "2026-07-19T11:00:00Z");
}

#[test]
fn participant_order_and_serialization_are_deterministic() {
    let first = CoordinatedEventHyperrelation::new(
        HyperrelationId::new("hyperrelation--deterministic")
            .expect("hyperrelation ID should be valid"),
        valid_participants(),
        time_window(),
        "Deterministic coordinated event",
    )
    .expect("first event should be valid");
    let second = CoordinatedEventHyperrelation::new(
        HyperrelationId::new("hyperrelation--deterministic")
            .expect("hyperrelation ID should be valid"),
        valid_participants().into_iter().rev().collect(),
        time_window(),
        "Deterministic coordinated event",
    )
    .expect("second event should be valid");

    assert_eq!(first, second);
    assert_eq!(
        serde_json::to_string(&first).expect("first event should serialize"),
        serde_json::to_string(&second).expect("second event should serialize")
    );
}

#[test]
fn invalid_arity_and_missing_mandatory_roles_are_rejected() {
    let invalid_arity = CoordinatedEventHyperrelation::new(
        HyperrelationId::new("hyperrelation--too-small").expect("ID should be valid"),
        vec![
            participant("node--actor-a", HyperrelationParticipantRole::Actor),
            participant("node--actor-b", HyperrelationParticipantRole::Actor),
            participant("node--narrative", HyperrelationParticipantRole::Narrative),
        ],
        time_window(),
        "Missing infrastructure",
    )
    .expect_err("fewer than four participants should fail");
    assert!(matches!(invalid_arity, GraphError::InvalidHyperrelation(_)));

    let missing_narrative = CoordinatedEventHyperrelation::new(
        HyperrelationId::new("hyperrelation--missing-narrative").expect("ID should be valid"),
        vec![
            participant("node--actor-a", HyperrelationParticipantRole::Actor),
            participant("node--actor-b", HyperrelationParticipantRole::Actor),
            participant(
                "node--infrastructure-a",
                HyperrelationParticipantRole::Infrastructure,
            ),
            participant(
                "node--infrastructure-b",
                HyperrelationParticipantRole::Infrastructure,
            ),
        ],
        time_window(),
        "Missing narrative role",
    )
    .expect_err("missing narrative should fail");
    assert!(matches!(
        missing_narrative,
        GraphError::InvalidHyperrelation(_)
    ));

    let missing_actor = CoordinatedEventHyperrelation::new(
        HyperrelationId::new("hyperrelation--missing-actor").expect("ID should be valid"),
        vec![
            participant("node--actor-a", HyperrelationParticipantRole::Actor),
            participant("node--narrative", HyperrelationParticipantRole::Narrative),
            participant(
                "node--infrastructure-a",
                HyperrelationParticipantRole::Infrastructure,
            ),
            participant(
                "node--infrastructure-b",
                HyperrelationParticipantRole::Infrastructure,
            ),
        ],
        time_window(),
        "Missing second actor",
    )
    .expect_err("fewer than two actors should fail");
    assert!(matches!(missing_actor, GraphError::InvalidHyperrelation(_)));

    let missing_infrastructure = CoordinatedEventHyperrelation::new(
        HyperrelationId::new("hyperrelation--missing-infrastructure").expect("ID should be valid"),
        vec![
            participant("node--actor-a", HyperrelationParticipantRole::Actor),
            participant("node--actor-b", HyperrelationParticipantRole::Actor),
            participant("node--narrative", HyperrelationParticipantRole::Narrative),
            participant(
                "node--related-entity",
                HyperrelationParticipantRole::RelatedEntity,
            ),
        ],
        time_window(),
        "Missing infrastructure role",
    )
    .expect_err("missing infrastructure should fail");
    assert!(matches!(
        missing_infrastructure,
        GraphError::InvalidHyperrelation(_)
    ));
}

#[test]
fn inconsistent_roles_time_windows_and_context_are_rejected() {
    let inconsistent_role = CoordinatedEventHyperrelation::new(
        HyperrelationId::new("hyperrelation--duplicate-node").expect("ID should be valid"),
        vec![
            participant("node--shared", HyperrelationParticipantRole::Actor),
            participant("node--actor-b", HyperrelationParticipantRole::Actor),
            participant("node--shared", HyperrelationParticipantRole::Narrative),
            participant(
                "node--infrastructure",
                HyperrelationParticipantRole::Infrastructure,
            ),
        ],
        time_window(),
        "One node cannot carry inconsistent roles",
    )
    .expect_err("duplicate participant node should fail");
    assert!(matches!(
        inconsistent_role,
        GraphError::InvalidHyperrelation(_)
    ));

    let invalid_window = HyperrelationTimeWindow::new(
        TemporalTimestamp::new("2026-07-19T12:00:00Z").expect("start should be valid"),
        TemporalTimestamp::new("2026-07-19T11:00:00Z").expect("end should be valid"),
    )
    .expect_err("reversed time window should fail");
    assert!(matches!(
        invalid_window,
        GraphError::InvalidHyperrelation(_)
    ));

    let blank_context = CoordinatedEventHyperrelation::new(
        HyperrelationId::new("hyperrelation--blank-context").expect("ID should be valid"),
        valid_participants(),
        time_window(),
        " ",
    )
    .expect_err("blank narrative context should fail");
    assert!(matches!(blank_context, GraphError::InvalidHyperrelation(_)));
}

#[test]
fn binary_projections_are_explicit_and_use_existing_relationship_inputs() {
    let event = CoordinatedEventHyperrelation::new(
        HyperrelationId::new("hyperrelation--binary-projection").expect("ID should be valid"),
        valid_participants(),
        time_window(),
        "Projection compatibility event",
    )
    .expect("coordinated event should be valid");

    let projections = event.binary_projections(node_id("node--event-anchor"));

    assert_eq!(projections.len(), 4);
    assert_eq!(projections[0].source().as_str(), "node--event-anchor");
    assert_eq!(projections[0].relationship_type().as_str(), "HAS_ACTOR");
    assert!(
        projections
            .into_iter()
            .map(|projection| projection.into_relationship_input())
            .collect::<Result<Vec<RelationshipInput>, GraphError>>()
            .is_ok()
    );
}

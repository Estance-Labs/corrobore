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
//! Epic 0022 acceptance suite: multi-resolution graph and hyperrelations.
//!
//! Validates deterministic FIMI and CTI scenarios end to end through
//! question-driven resolution selection, auditable derivation chains,
//! first-class coordinated events, filtered hyperrelation expansion, and mixed
//! traversal explanations.

use graph_core::{
    CoordinatedEventHyperrelation, DerivationLink, DerivationLinkId, FactId,
    HyperrelationExpansionRequest, HyperrelationExpansionResult, HyperrelationId,
    HyperrelationParticipant, HyperrelationParticipantRole, HyperrelationTimeWindow,
    MixedTraversalOperator, MixedTraversalResult, MixedTraversalStep, MultiResolutionModel, NodeId,
    QuestionIntent, RelationshipId, ResolutionArtifact, ResolutionArtifactId, ResolutionLevel,
    ResolutionRecordRef, ResolutionSelection, ResolutionSelectionReason,
    ResolutionSelectionRequest, TemporalTimestamp, execute_mixed_traversal,
    query_hyperrelation_expansion, select_question_resolution,
};

/// Complete deterministic Epic 0022 output for one investigation domain.
///
/// The fixture carries the level model, all question-selection decisions, the
/// first-class coordinated event, its filtered neighborhood expansion, and both
/// cross-resolution and mixed traversal results.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
struct Epic0022Scenario {
    domain: &'static str,
    model: MultiResolutionModel,
    operational_ref: ResolutionRecordRef,
    strategic_ref: ResolutionRecordRef,
    selections: Vec<ResolutionSelection>,
    event: CoordinatedEventHyperrelation,
    expansion: HyperrelationExpansionResult,
    resolution_traversal: MixedTraversalResult,
    mixed_traversal: MixedTraversalResult,
}

/// Declares the FIMI multi-resolution coordination scenario.
///
fn fimi_scenario() -> Epic0022Scenario {
    build_scenario("fimi")
}

/// Declares the CTI multi-resolution coordination scenario.
///
fn cti_scenario() -> Epic0022Scenario {
    build_scenario("cti")
}

/// Builds one domain fixture through public Epic 0022 APIs.
fn build_scenario(domain: &'static str) -> Epic0022Scenario {
    let tactical_a = resolution_ref(domain, ResolutionLevel::Tactical, "observation-a");
    let tactical_b = resolution_ref(domain, ResolutionLevel::Tactical, "observation-b");
    let operational_ref = resolution_ref(domain, ResolutionLevel::Operational, "narrative");
    let strategic_ref = resolution_ref(domain, ResolutionLevel::Strategic, "campaign");
    let operational_derivation = derivation_id(domain, "observations-to-narrative");
    let strategic_derivation = derivation_id(domain, "narrative-to-campaign");

    let model = MultiResolutionModel::new()
        .register_artifact(
            ResolutionArtifact::new(
                tactical_b.clone(),
                format!("{domain} second source-grounded observation"),
            )
            .expect("tactical artifact B should be valid"),
        )
        .expect("tactical artifact B should register")
        .register_artifact(
            ResolutionArtifact::new(
                strategic_ref.clone(),
                format!("{domain} strategic campaign assessment"),
            )
            .expect("strategic artifact should be valid"),
        )
        .expect("strategic artifact should register")
        .register_artifact(
            ResolutionArtifact::new(
                tactical_a.clone(),
                format!("{domain} first source-grounded observation"),
            )
            .expect("tactical artifact A should be valid"),
        )
        .expect("tactical artifact A should register")
        .register_artifact(
            ResolutionArtifact::new(
                operational_ref.clone(),
                format!("{domain} operational narrative"),
            )
            .expect("operational artifact should be valid"),
        )
        .expect("operational artifact should register")
        .add_derivation_link(
            DerivationLink::new(
                operational_derivation.clone(),
                operational_ref.clone(),
                vec![tactical_b.clone(), tactical_a.clone()],
                vec![
                    fact_id(domain, "observation-b"),
                    fact_id(domain, "observation-a"),
                ],
            )
            .expect("operational derivation should be valid"),
        )
        .expect("operational derivation should register")
        .add_derivation_link(
            DerivationLink::new(
                strategic_derivation.clone(),
                strategic_ref.clone(),
                vec![operational_ref.clone()],
                vec![fact_id(domain, "campaign-assessment")],
            )
            .expect("strategic derivation should be valid"),
        )
        .expect("strategic derivation should register");

    let selections = [
        (
            "tactical",
            QuestionIntent::TacticalEvidenceDetail,
            ResolutionLevel::Tactical,
        ),
        (
            "operational",
            QuestionIntent::OperationalCampaignAnalysis,
            ResolutionLevel::Operational,
        ),
        (
            "strategic",
            QuestionIntent::StrategicObjectiveAssessment,
            ResolutionLevel::Strategic,
        ),
    ]
    .into_iter()
    .map(|(slug, intent, expected)| {
        let request =
            ResolutionSelectionRequest::new(format!("question--{domain}-{slug}"), vec![intent])
                .expect("resolution request should be valid");
        let selection =
            select_question_resolution(&request).expect("question resolution should select");
        assert_eq!(selection.selected_level(), expected);
        selection
    })
    .collect();

    let actor_a = node_id(domain, "actor-a");
    let actor_b = node_id(domain, "actor-b");
    let narrative = node_id(domain, "narrative");
    let infrastructure = node_id(domain, "infrastructure");
    let event = CoordinatedEventHyperrelation::new(
        HyperrelationId::new(format!("hyperrelation--{domain}-coordination"))
            .expect("hyperrelation ID should be valid"),
        vec![
            HyperrelationParticipant::new(
                infrastructure.clone(),
                HyperrelationParticipantRole::Infrastructure,
            ),
            HyperrelationParticipant::new(actor_b.clone(), HyperrelationParticipantRole::Actor),
            HyperrelationParticipant::new(narrative, HyperrelationParticipantRole::Narrative),
            HyperrelationParticipant::new(actor_a.clone(), HyperrelationParticipantRole::Actor),
        ],
        HyperrelationTimeWindow::new(
            timestamp("2026-07-19T10:00:00Z"),
            timestamp("2026-07-19T11:00:00Z"),
        )
        .expect("event time window should be valid"),
        format!("{domain} actors coordinate one narrative through shared infrastructure"),
    )
    .expect("coordinated event should be valid");

    let expansion_request = HyperrelationExpansionRequest::new(actor_a.clone())
        .with_role_filter(vec![HyperrelationParticipantRole::Infrastructure])
        .expect("infrastructure filter should be valid");
    let expansion = query_hyperrelation_expansion(&event, &expansion_request)
        .expect("filtered expansion should succeed");

    let resolution_traversal = execute_mixed_traversal(vec![
        MixedTraversalStep::across_resolution(
            operational_derivation,
            tactical_a,
            operational_ref.clone(),
        ),
        MixedTraversalStep::across_resolution(
            strategic_derivation,
            operational_ref.clone(),
            strategic_ref.clone(),
        ),
    ])
    .expect("cross-resolution traversal should succeed");

    let mixed_traversal = execute_mixed_traversal(vec![
        MixedTraversalStep::binary(
            RelationshipId::new(format!("relationship--{domain}-communicates"))
                .expect("relationship ID should be valid"),
            actor_a,
            actor_b.clone(),
        ),
        MixedTraversalStep::enter_hyperrelation(&event, actor_b)
            .expect("actor B should enter the event"),
        MixedTraversalStep::expand_hyperrelation(&event, infrastructure)
            .expect("infrastructure should expand from the event"),
    ])
    .expect("mixed traversal should succeed");

    Epic0022Scenario {
        domain,
        model,
        operational_ref,
        strategic_ref,
        selections,
        event,
        expansion,
        resolution_traversal,
        mixed_traversal,
    }
}

fn resolution_ref(domain: &str, level: ResolutionLevel, slug: &str) -> ResolutionRecordRef {
    ResolutionRecordRef::new(
        level,
        ResolutionArtifactId::new(format!("artifact--{domain}-{slug}"))
            .expect("resolution artifact ID should be valid"),
    )
}

fn derivation_id(domain: &str, slug: &str) -> DerivationLinkId {
    DerivationLinkId::new(format!("derivation--{domain}-{slug}"))
        .expect("derivation-link ID should be valid")
}

fn fact_id(domain: &str, slug: &str) -> FactId {
    FactId::new(format!("fact--{domain}-{slug}")).expect("fact ID should be valid")
}

fn node_id(domain: &str, slug: &str) -> NodeId {
    NodeId::new(format!("node--{domain}-{slug}")).expect("node ID should be valid")
}

fn timestamp(value: &str) -> TemporalTimestamp {
    TemporalTimestamp::new(value).expect("timestamp should be valid")
}

#[test]
fn acceptance_questions_select_tactical_operational_and_strategic_resolutions() {
    for scenario in [fimi_scenario(), cti_scenario()] {
        let selected_levels = scenario
            .selections
            .iter()
            .map(|selection| selection.selected_level())
            .collect::<Vec<_>>();

        assert_eq!(
            selected_levels,
            vec![
                ResolutionLevel::Tactical,
                ResolutionLevel::Operational,
                ResolutionLevel::Strategic,
            ]
        );
        assert!(scenario.selections.iter().all(|selection| {
            selection.trace().reason() == ResolutionSelectionReason::Direct
                && selection.trace().intent_mappings().len() == 1
                && selection.trace().question_ref().contains(scenario.domain)
        }));
    }
}

#[test]
fn acceptance_higher_levels_retain_auditable_lower_level_derivations() {
    for scenario in [fimi_scenario(), cti_scenario()] {
        let operational_links = scenario
            .model
            .derivation_links_for(&scenario.operational_ref);
        let strategic_links = scenario.model.derivation_links_for(&scenario.strategic_ref);

        assert_eq!(operational_links.len(), 1);
        assert_eq!(operational_links[0].supporting_sources().len(), 2);
        assert_eq!(operational_links[0].provenance_fact_refs().len(), 2);
        assert_eq!(strategic_links.len(), 1);
        assert_eq!(strategic_links[0].supporting_sources().len(), 1);
        assert_eq!(strategic_links[0].provenance_fact_refs().len(), 1);
    }
}

#[test]
fn acceptance_nary_events_are_first_class_and_queryable_by_role() {
    for scenario in [fimi_scenario(), cti_scenario()] {
        assert_eq!(
            scenario
                .event
                .participants_for_role(HyperrelationParticipantRole::Actor)
                .len(),
            2
        );
        assert_eq!(
            scenario
                .event
                .participants_for_role(HyperrelationParticipantRole::Narrative)
                .len(),
            1
        );
        assert_eq!(scenario.expansion.participants().len(), 1);
        assert_eq!(
            scenario.expansion.participants()[0].role(),
            HyperrelationParticipantRole::Infrastructure
        );
        assert_eq!(
            scenario.expansion.explanation().hyperrelation_id(),
            scenario.event.id()
        );
    }
}

#[test]
fn acceptance_cross_resolution_and_mixed_paths_are_explained() {
    for scenario in [fimi_scenario(), cti_scenario()] {
        assert_eq!(scenario.resolution_traversal.steps().len(), 2);
        assert!(
            scenario
                .resolution_traversal
                .explanations()
                .iter()
                .all(|entry| entry.operator() == MixedTraversalOperator::AbstractResolution)
        );
        assert!(
            scenario
                .resolution_traversal
                .explanations()
                .iter()
                .all(|entry| entry.audit_ref().starts_with("derivation--"))
        );

        assert_eq!(scenario.mixed_traversal.steps().len(), 3);
        assert_eq!(
            scenario.mixed_traversal.explanations()[0].operator(),
            MixedTraversalOperator::Binary
        );
        assert_eq!(
            scenario.mixed_traversal.explanations()[1].operator(),
            MixedTraversalOperator::EnterHyperrelation
        );
        assert_eq!(
            scenario.mixed_traversal.explanations()[2].operator(),
            MixedTraversalOperator::ExpandHyperrelation
        );
    }
}

#[test]
fn acceptance_complete_epic_0022_outputs_are_byte_stable() {
    for build in [
        fimi_scenario as fn() -> Epic0022Scenario,
        cti_scenario as fn() -> Epic0022Scenario,
    ] {
        let first = build();
        let second = build();

        assert_eq!(first, second);
        assert_eq!(
            serde_json::to_string(&first).expect("first scenario should serialize"),
            serde_json::to_string(&second).expect("second scenario should serialize")
        );
    }
}

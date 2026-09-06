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
use graph_core::*;

fn actor() -> ReconciliationDecider {
    ReconciliationDecider::Actor(ActorId::new("reviewer").expect("actor"))
}
pub(crate) fn mention(
    graph: &mut Graph,
    id: &str,
    surface: &str,
    context: &str,
) -> EntityMentionId {
    let source = SourceId::new(format!("source--{id}")).expect("source");
    let observation = ObservationId::new(format!("observation--{id}")).expect("observation");
    let stores = graph.epistemic_stores_mut();
    stores
        .sources
        .register_source(SourceInput::new(
            source.clone(),
            format!("https://example.org/{id}"),
            EvidenceSourceType::Document,
        ))
        .expect("source");
    stores
        .observations
        .create_observation(
            ObservationInput::new(
                observation.clone(),
                source,
                surface,
                ObservationModality::Text,
            ),
            &stores.sources,
        )
        .expect("observe");
    graph
        .create_entity_mention(
            EntityMentionInput::new(
                EntityMentionId::new(id).expect("mention"),
                observation,
                MentionOffsets {
                    start: 0,
                    end: surface.len() as u64,
                },
                surface,
            )
            .with_features(MentionFeatures {
                source_context: Some(context.into()),
                ..Default::default()
            }),
        )
        .expect("mention")
}
pub(crate) fn input(
    id: &str,
    left: &EntityMentionId,
    right: &EntityMentionId,
    outcome: ReconciliationOutcome,
    feature: ReconciliationFeature,
) -> ReconciliationInput {
    ReconciliationInput::new(
        ReconciliationRecordId::new(id).expect("id"),
        left.clone(),
        right.clone(),
        outcome,
        actor(),
        TemporalTimestamp::new("2026-09-06T12:00:00Z").expect("time"),
        "Reviewed source-grounded identity evidence",
    )
    .with_evidence(vec![
        ReconciliationEvidence::Mention {
            mention_id: left.clone(),
            feature,
        },
        ReconciliationEvidence::Mention {
            mention_id: right.clone(),
            feature,
        },
    ])
}
pub fn seeded() -> Graph {
    let mut g = Graph::new();
    // One real correction, two harmful over-repairs, one unsuccessful repair.
    for (n, (before, after)) in [(false, true), (true, false), (true, false), (false, false)]
        .into_iter()
        .enumerate()
    {
        let original = CandidateInput::new(
            format!("original-{n}"),
            ExtractionRunId::new("run").expect("run"),
            "{}",
            ActorId::new("extractor").expect("actor"),
        )
        .expect("input")
        .with_constraints(vec![CandidateConstraint {
            id: "name-required".into(),
            field: "/name".into(),
            rule: CandidateRule::Required,
        }]);
        let original = g.submit_candidate(original).expect("submit");
        let repair = g
            .repair_candidate(
                original.id(),
                CandidateInput::new(
                    format!("repair-{n}"),
                    ExtractionRunId::new("run").expect("run"),
                    r#"{"name":"replacement"}"#,
                    ActorId::new("extractor").expect("actor"),
                )
                .expect("input"),
                vec!["name-required".into()],
            )
            .expect("repair");
        assert!(
            g.epistemic_stores()
                .candidates
                .validation(repair.id())
                .expect("validation")
                .valid
        );
        for (id, correct) in [(original.id(), before), (repair.id(), after)] {
            g.record_candidate_assessment(CandidateAssessment::new(
                id.clone(),
                correct,
                ActorId::new("reviewer").expect("actor"),
                "fixture ground truth",
            ))
            .expect("assessment");
        }
    }
    for (n, (predicted, expected)) in [
        (ReconciliationOutcome::Merge, ReconciliationOutcome::Merge),
        (
            ReconciliationOutcome::Merge,
            ReconciliationOutcome::Distinct,
        ),
        (
            ReconciliationOutcome::Distinct,
            ReconciliationOutcome::Distinct,
        ),
        (
            ReconciliationOutcome::Abstain,
            ReconciliationOutcome::Abstain,
        ),
        (ReconciliationOutcome::Abstain, ReconciliationOutcome::Merge),
    ]
    .into_iter()
    .enumerate()
    {
        let a = mention(&mut g, &format!("left-{n}"), "A", "registry context");
        let b = mention(&mut g, &format!("right-{n}"), "B", "registry context");
        let id = g
            .record_reconciliation(input(
                &format!("decision-{n}"),
                &a,
                &b,
                predicted,
                ReconciliationFeature::SourceContext,
            ))
            .expect("record");
        g.record_reconciliation_assessment(ReconciliationAssessment::new(
            id,
            expected,
            ActorId::new("reviewer").expect("actor"),
            "fixture identity labels",
        ))
        .expect("assessment");
    }
    g
}

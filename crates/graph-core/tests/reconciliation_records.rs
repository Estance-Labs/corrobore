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
use serde_json::Value;
fn actor() -> ReconciliationDecider {
    ReconciliationDecider::Actor(ActorId::new("reviewer").expect("actor"))
}
fn mention(graph: &mut Graph, id: &str, surface: &str, context: &str) -> EntityMentionId {
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
fn input(
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
#[test]
fn spike_d_retains_reviewed_homonym_alias_transliteration_and_abstain_decisions() {
    let fixtures: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/reconciliation_aliases.json"))
            .expect("fixtures");
    for fixture in fixtures {
        let mut graph = Graph::new();
        let left = mention(
            &mut graph,
            "left",
            fixture["left"].as_str().expect("left"),
            fixture["left_context"].as_str().expect("context"),
        );
        let right = mention(
            &mut graph,
            "right",
            fixture["right"].as_str().expect("right"),
            fixture["right_context"].as_str().expect("context"),
        );
        let outcome: ReconciliationOutcome =
            serde_json::from_value(fixture["outcome"].clone()).expect("outcome");
        let proposal = input(
            "record--1",
            &left,
            &right,
            outcome,
            ReconciliationFeature::SourceContext,
        );
        let id = graph
            .record_reconciliation(proposal)
            .expect("record reviewed decision");
        let record = graph
            .epistemic_stores()
            .reconciliations
            .record_by_id(&id)
            .expect("record");
        assert_eq!(record.outcome(), outcome, "{}", fixture["category"]);
        assert_eq!(record.decider(), &actor());
        assert_eq!(record.citations().len(), 2);
        assert_eq!(record.citations()[0].value, fixture["left_context"]);
        assert_eq!(
            record.citations()[0].observation_id.as_str(),
            "observation--left"
        );
        assert_eq!(record.citations()[0].source_id.as_str(), "source--left");
        assert!(
            graph.list_nodes().expect("nodes").is_empty(),
            "merge execution belongs to WS-C 5"
        );
        assert!(graph.list_relationships().expect("relations").is_empty());
    }
}
#[test]
fn name_or_embedding_similarity_alone_never_justifies_a_record() {
    let mut graph = Graph::new();
    let left = mention(&mut graph, "left", "Atlas", "Physician in Paris");
    let right = mention(&mut graph, "right", "Atlas", "Engineer in Sydney");
    for outcome in [
        ReconciliationOutcome::Merge,
        ReconciliationOutcome::Distinct,
        ReconciliationOutcome::Abstain,
    ] {
        for kind in [
            ReconciliationSimilarityKind::Name,
            ReconciliationSimilarityKind::Embedding,
        ] {
            let proposal = input(
                "bad",
                &left,
                &right,
                outcome,
                ReconciliationFeature::SurfaceForm,
            )
            .with_similarity_hints(vec![ReconciliationSimilarity {
                kind,
                score: Confidence::new(1.0).expect("score"),
            }]);
            assert!(graph.record_reconciliation(proposal).is_err());
        }
    }
    assert!(graph.epistemic_stores().reconciliations.is_empty());
}
#[test]
fn abstention_preserves_history_and_allows_new_observation_evidence() {
    let mut graph = Graph::new();
    let left = mention(&mut graph, "left", "Jordan", "Identity remains ambiguous");
    let right = mention(&mut graph, "right", "Jordan", "No unique identifier");
    graph
        .record_reconciliation(input(
            "abstain",
            &left,
            &right,
            ReconciliationOutcome::Abstain,
            ReconciliationFeature::SourceContext,
        ))
        .expect("abstain");
    let new_observation = ObservationId::new("identity-proof").expect("observation");
    let stores = graph.epistemic_stores_mut();
    stores
        .observations
        .create_observation(
            ObservationInput::new(
                new_observation.clone(),
                SourceId::new("source--left").expect("source"),
                "Signed registry R-123 confirms both mentions identify the same person.",
                ObservationModality::Text,
            ),
            &stores.sources,
        )
        .expect("new evidence");
    let proposal = input(
        "merge",
        &left,
        &right,
        ReconciliationOutcome::Merge,
        ReconciliationFeature::SurfaceForm,
    )
    .with_evidence(vec![
        ReconciliationEvidence::Mention {
            mention_id: left.clone(),
            feature: ReconciliationFeature::SurfaceForm,
        },
        ReconciliationEvidence::Mention {
            mention_id: right.clone(),
            feature: ReconciliationFeature::SurfaceForm,
        },
        ReconciliationEvidence::Observation {
            observation_id: new_observation,
        },
    ])
    .with_decider(ReconciliationDecider::Verifier {
        id: "registry-verifier".into(),
        version: "1.0".into(),
    });
    graph
        .record_reconciliation(proposal.clone())
        .expect("new decision");
    graph.record_reconciliation(proposal).expect("retry");
    let store = &graph.epistemic_stores().reconciliations;
    assert_eq!(store.records_for_pair(&right, &left).len(), 2);
    assert_eq!(
        store
            .records_by_outcome(ReconciliationOutcome::Abstain)
            .len(),
        1
    );
    let encoded = serde_json::to_value(graph.persistence_snapshot()).expect("encode");
    let restored =
        Graph::from_persistence_snapshot(serde_json::from_value(encoded).expect("decode"))
            .expect("restore");
    assert_eq!(restored.epistemic_stores(), graph.epistemic_stores());
    let projection = restored.epistemic_projection().expect("project");
    assert!(
        validate_graph_structure(&projection, &[])
            .expect("structural validation")
            .is_empty()
    );
    assert_eq!(
        projection
            .list_nodes()
            .expect("nodes")
            .iter()
            .filter(|n| n.has_label("ReconciliationRecord"))
            .count(),
        2
    );
}
#[test]
fn records_reject_rewrites_and_unrelated_or_missing_features() {
    let mut graph = Graph::new();
    let left = mention(&mut graph, "left", "Atlas", "context one");
    let right = mention(&mut graph, "right", "Atlas", "context two");
    graph
        .record_reconciliation(input(
            "record",
            &left,
            &right,
            ReconciliationOutcome::Distinct,
            ReconciliationFeature::SourceContext,
        ))
        .expect("record");
    let before = graph.epistemic_stores().clone();
    assert!(matches!(
        graph.record_reconciliation(input(
            "record",
            &left,
            &right,
            ReconciliationOutcome::Merge,
            ReconciliationFeature::SourceContext
        )),
        Err(GraphError::ImmutableRecordConflict {
            kind: ImmutableRecordKind::ReconciliationRecord,
            ..
        })
    ));
    assert!(
        graph
            .record_reconciliation(input(
                "missing",
                &left,
                &right,
                ReconciliationOutcome::Merge,
                ReconciliationFeature::Location
            ))
            .is_err()
    );
    assert!(
        graph
            .record_reconciliation(input(
                "self",
                &left,
                &left,
                ReconciliationOutcome::Merge,
                ReconciliationFeature::SourceContext
            ))
            .is_err()
    );
    assert_eq!(graph.epistemic_stores(), &before);
}

#[test]
fn citations_remain_pinned_after_source_version_changes_and_reject_tampering() {
    let mut graph = Graph::new();
    let left = mention(&mut graph, "left", "A", "registry R-1");
    let right = mention(&mut graph, "right", "Alias", "registry R-1");
    let proposal = input(
        "record",
        &left,
        &right,
        ReconciliationOutcome::Merge,
        ReconciliationFeature::SourceContext,
    );
    let id = graph
        .record_reconciliation(proposal.clone())
        .expect("record");
    let original = graph
        .epistemic_stores()
        .reconciliations
        .record_by_id(&id)
        .expect("record")
        .clone();
    graph
        .epistemic_stores_mut()
        .sources
        .register_source(
            SourceInput::new(
                SourceId::new("source--left").expect("source"),
                "https://example.org/left",
                EvidenceSourceType::Document,
            )
            .with_artifact_sha256("a".repeat(64)),
        )
        .expect("new source version");
    assert_ne!(
        graph
            .epistemic_stores()
            .sources
            .current_source(&SourceId::new("source--left").expect("source"))
            .expect("current")
            .version_id(),
        &original.citations()[0].source_version_id
    );
    graph
        .record_reconciliation(proposal)
        .expect("retry after source update");
    assert_eq!(
        graph.epistemic_stores().reconciliations.record_by_id(&id),
        Some(&original)
    );
    let snapshot = serde_json::to_value(graph.persistence_snapshot()).expect("encode");
    let restored =
        Graph::from_persistence_snapshot(serde_json::from_value(snapshot.clone()).expect("decode"))
            .expect("restore pinned source version");
    assert_eq!(restored.epistemic_stores(), graph.epistemic_stores());
    let mut tampered = snapshot;
    tampered["epistemic"]["reconciliations"]["records"][0]["citations"][0]["value"] =
        serde_json::json!("forged identity evidence");
    assert!(
        Graph::from_persistence_snapshot(serde_json::from_value(tampered).expect("decode"))
            .is_err()
    );
}

#[test]
fn missing_decider_and_unrelated_evidence_are_rejected_atomically() {
    let mut graph = Graph::new();
    let left = mention(&mut graph, "left", "A", "context");
    let right = mention(&mut graph, "right", "B", "context");
    let other = mention(&mut graph, "other", "C", "unrelated");
    let invalid = input(
        "record",
        &left,
        &right,
        ReconciliationOutcome::Merge,
        ReconciliationFeature::SourceContext,
    )
    .with_decider(ReconciliationDecider::Verifier {
        id: "".into(),
        version: "1".into(),
    });
    assert!(graph.record_reconciliation(invalid).is_err());
    let unrelated = input(
        "record",
        &left,
        &right,
        ReconciliationOutcome::Merge,
        ReconciliationFeature::SourceContext,
    )
    .with_evidence(vec![ReconciliationEvidence::Mention {
        mention_id: other,
        feature: ReconciliationFeature::SourceContext,
    }]);
    assert!(graph.record_reconciliation(unrelated).is_err());
    assert!(graph.epistemic_stores().reconciliations.is_empty());
}

#[test]
fn every_contextual_mention_feature_can_be_cited_without_caller_supplied_values() {
    let mut graph = Graph::new();
    mention(&mut graph, "left", "A", "context");
    mention(&mut graph, "right", "B", "context");
    let features = MentionFeatures {
        source_context: Some("registry context".into()),
        role: Some("author".into()),
        time: Some(TemporalTimestamp::new("2026-09-06T12:00:00Z").expect("time")),
        location: Some("Paris".into()),
        affiliations: vec!["Institute".into()],
        relation_neighbourhood: vec![MentionRelationFeature {
            predicate: "wrote".into(),
            direction: MentionRelationDirection::Outgoing,
            counterpart: "Book".into(),
        }],
    };
    let mut ids = Vec::new();
    for (name, surface) in [("left", "A"), ("right", "B")] {
        ids.push(
            graph
                .create_entity_mention(
                    EntityMentionInput::new(
                        EntityMentionId::new(format!("rich-{name}")).expect("id"),
                        ObservationId::new(format!("observation--{name}")).expect("observation"),
                        MentionOffsets { start: 0, end: 1 },
                        surface,
                    )
                    .with_features(features.clone()),
                )
                .expect("rich mention"),
        );
    }
    for feature in [
        ReconciliationFeature::SourceContext,
        ReconciliationFeature::Role,
        ReconciliationFeature::Time,
        ReconciliationFeature::Location,
        ReconciliationFeature::Affiliations,
        ReconciliationFeature::RelationNeighbourhood,
    ] {
        let id = graph
            .record_reconciliation(input(
                &format!("feature-{feature:?}"),
                &ids[0],
                &ids[1],
                ReconciliationOutcome::Abstain,
                feature,
            ))
            .expect("record");
        let record = graph
            .epistemic_stores()
            .reconciliations
            .record_by_id(&id)
            .expect("record");
        assert_eq!(record.citations()[0].feature, feature);
        assert!(!record.citations()[0].value.is_null());
    }
}

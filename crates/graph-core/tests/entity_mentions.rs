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
fn graph() -> Graph {
    let mut graph = Graph::new();
    let stores = graph.epistemic_stores_mut();
    let source = SourceId::new("source--report").expect("source");
    stores
        .sources
        .register_source(SourceInput::new(
            source.clone(),
            "https://example.org/report",
            EvidenceSourceType::Document,
        ))
        .expect("register");
    stores
        .observations
        .create_observation(
            ObservationInput::new(
                ObservationId::new("observation--1").expect("observation"),
                source,
                "Équipe Atlas à Paris",
                ObservationModality::Text,
            ),
            &stores.sources,
        )
        .expect("observe");
    graph
}
fn input(id: &str) -> EntityMentionInput {
    EntityMentionInput::new(
        EntityMentionId::new(id).expect("mention"),
        ObservationId::new("observation--1").expect("observation"),
        MentionOffsets { start: 8, end: 13 },
        "Atlas",
    )
}
#[test]
fn mention_is_bound_to_an_exact_observation_span_without_implicit_entity_resolution() {
    let mut graph = graph();
    let entity = graph
        .create_node(
            NodeInput::new(["Entity"]).with_property("name", PropertyValue::String("Atlas".into())),
        )
        .expect("entity");
    let id = graph
        .create_entity_mention(input("mention--1").with_candidate_entities(vec![entity.clone()]))
        .expect("mention");
    let mention = graph
        .epistemic_stores()
        .mentions
        .mention_by_id(&id)
        .expect("stored");
    assert_eq!(mention.surface_form(), "Atlas");
    assert_eq!(mention.observation_id().as_str(), "observation--1");
    assert_eq!(mention.offsets(), MentionOffsets { start: 8, end: 13 });
    assert_eq!(mention.candidate_entities(), &[entity]);
    assert_eq!(graph.list_nodes().expect("nodes").len(), 1);
    assert!(
        graph
            .list_relationships()
            .expect("relationships")
            .is_empty()
    );
    let projection = graph.epistemic_projection().expect("projection");
    let nodes =
        epistemic_nodes_of_kind(&projection, EpistemicNodeKind::EntityMention).expect("mentions");
    assert_eq!(nodes.len(), 1);
    assert!(
        epistemic_nodes_of_kind(&projection, EpistemicNodeKind::Entity)
            .expect("entities")
            .is_empty()
    );
    assert!(
        projection
            .list_relationships()
            .expect("relations")
            .iter()
            .all(|r| ["REPORTS", "HAS_MENTION"].contains(&r.rel_type().as_str()))
    );
    assert_eq!(
        projection
            .list_relationships()
            .expect("relations")
            .iter()
            .filter(|r| r.rel_type().as_str() == "HAS_MENTION")
            .count(),
        1
    );
}
#[test]
fn mention_features_projection_and_snapshot_round_trip_preserve_evidence() {
    let mut graph = graph();
    let features = MentionFeatures {
        source_context: Some("Named by a witness".into()),
        role: Some("operator".into()),
        time: Some(TemporalTimestamp::new("2026-09-06T12:00:00Z").expect("time")),
        location: Some("Paris".into()),
        affiliations: vec!["Example group".into()],
        relation_neighbourhood: vec![MentionRelationFeature {
            predicate: "operates".into(),
            direction: MentionRelationDirection::Outgoing,
            counterpart: "Example service".into(),
        }],
    };
    let id = graph
        .create_entity_mention(input("mention--1").with_features(features.clone()))
        .expect("mention");
    let mention = graph
        .epistemic_stores()
        .mentions
        .mention_by_id(&id)
        .expect("stored");
    assert_eq!(mention.features(), &features);
    let properties = mention.to_property_map().expect("properties");
    assert!(properties.keys().all(|key| key.starts_with("mention_")));
    assert_eq!(
        properties.get("mention_surface_form"),
        Some(&PropertyValue::String("Atlas".into()))
    );
    let encoded = serde_json::to_value(graph.persistence_snapshot()).expect("encode");
    let restored =
        Graph::from_persistence_snapshot(serde_json::from_value(encoded).expect("decode"))
            .expect("restore");
    assert_eq!(restored.epistemic_stores(), graph.epistemic_stores());
    assert_eq!(
        restored
            .epistemic_stores()
            .mentions
            .mention_by_id(&id)
            .expect("restored")
            .to_property_map()
            .expect("projection"),
        properties
    );
}
#[test]
fn mentions_are_append_only_and_unknown_observations_or_invalid_spans_are_rejected() {
    let mut graph = graph();
    let original = input("mention--1");
    graph
        .create_entity_mention(original.clone())
        .expect("create");
    graph.create_entity_mention(original).expect("idempotent");
    assert!(matches!(
        graph.create_entity_mention(input("mention--1").with_features(MentionFeatures {
            role: Some("changed".into()),
            ..Default::default()
        })),
        Err(GraphError::ImmutableRecordConflict {
            kind: ImmutableRecordKind::EntityMention,
            ..
        })
    ));
    for (observation, start, end, surface) in [
        ("missing", 8, 13, "Atlas"),
        ("observation--1", 1, 2, "É"),
        ("observation--1", 8, 99, "Atlas"),
        ("observation--1", 8, 8, ""),
        ("observation--1", 8, 13, "other"),
    ] {
        let invalid = EntityMentionInput::new(
            EntityMentionId::new("bad").expect("id"),
            ObservationId::new(observation).expect("id"),
            MentionOffsets { start, end },
            surface,
        );
        assert!(graph.create_entity_mention(invalid).is_err());
    }
    assert_eq!(graph.epistemic_stores().mentions.len(), 1);
    assert_eq!(
        graph
            .epistemic_stores()
            .mentions
            .mentions_for_observation(&ObservationId::new("observation--1").expect("id"))
            .len(),
        1
    );
}

#[test]
fn same_surface_without_candidate_hints_stays_unresolved_and_separate() {
    let mut graph = graph();
    graph
        .create_node(
            NodeInput::new(["Entity"]).with_property("name", PropertyValue::String("Atlas".into())),
        )
        .expect("entity");
    for id in ["mention--first", "mention--second"] {
        let id = graph.create_entity_mention(input(id)).expect("mention");
        assert!(
            graph
                .epistemic_stores()
                .mentions
                .mention_by_id(&id)
                .expect("mention")
                .candidate_entities()
                .is_empty()
        );
    }
    assert_eq!(graph.epistemic_stores().mentions.len(), 2);
    assert_eq!(graph.list_nodes().expect("entities").len(), 1);
    assert!(
        graph
            .list_relationships()
            .expect("relationships")
            .is_empty()
    );
}

#[test]
fn restore_rejects_duplicate_ids_and_broken_observation_bindings() {
    let mut graph = graph();
    graph
        .create_entity_mention(input("mention--1"))
        .expect("mention");
    let snapshot = serde_json::to_value(graph.persistence_snapshot()).expect("encode");
    let mut broken = snapshot.clone();
    broken["epistemic"]["mentions"]["mentions"][0]["observation_id"]["value"] =
        serde_json::json!("unknown");
    let decoded = serde_json::from_value(broken).expect("decode");
    assert!(Graph::from_persistence_snapshot(decoded).is_err());
    let mut store = snapshot["epistemic"]["mentions"].clone();
    let duplicate = store["mentions"][0].clone();
    store["mentions"]
        .as_array_mut()
        .expect("mentions array")
        .push(duplicate);
    assert!(serde_json::from_value::<EntityMentionStore>(store).is_err());
}

#[test]
fn has_mention_edges_only_connect_observations_to_mentions() {
    let mut graph = graph();
    graph
        .create_entity_mention(input("mention--1"))
        .expect("mention");
    let mut projection = graph.epistemic_projection().expect("projection");
    assert!(
        validate_graph_structure(&projection, &[])
            .expect("validate")
            .is_empty()
    );
    let observation = epistemic_nodes_of_kind(&projection, EpistemicNodeKind::Observation)
        .expect("observations")[0]
        .clone();
    let entity = projection
        .create_node(NodeInput::new(["Entity"]))
        .expect("entity");
    projection
        .create_relationship(
            RelationshipInput::new(observation, "HAS_MENTION", entity).expect("relationship"),
        )
        .expect("create");
    assert!(
        !validate_graph_structure(&projection, &[])
            .expect("validate")
            .is_empty()
    );
}

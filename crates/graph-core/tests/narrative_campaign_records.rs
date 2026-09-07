#![allow(clippy::unwrap_used)]
//! Neutral governed collections preserve membership without changing factual records.
use graph_core::*;
fn stamp() -> BitemporalStamp {
    BitemporalStamp::new(
        TemporalTimestamp::new("2026-01-01T00:00:00Z").unwrap(),
        TemporalTimestamp::new("2026-01-02T00:00:00Z").unwrap(),
    )
    .unwrap()
}
fn fixture() -> (Graph, ContextMembership) {
    let mut graph = Graph::new();
    let actor = graph.create_node(NodeInput::new(["Entity"])).unwrap();
    let infrastructure = graph.create_node(NodeInput::new(["Entity"])).unwrap();
    let source = SourceId::new("source--content").unwrap();
    let claim = ClaimId::new("claim--member").unwrap();
    let stores = graph.epistemic_stores_mut();
    stores
        .sources
        .register_source(SourceInput::new(
            source.clone(),
            "https://example.org/content",
            EvidenceSourceType::Document,
        ))
        .unwrap();
    stores
        .claims
        .create_asserted_claim(ClaimInput::new(
            claim.clone(),
            ClaimStatement::new("The event occurred").unwrap(),
            ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new("event", None)),
        ))
        .unwrap();
    (
        graph,
        ContextMembership {
            claims: vec![claim],
            themes: vec!["transport".into()],
            content: vec![source],
            infrastructure: vec![infrastructure],
            actors: vec![actor],
        },
    )
}
fn narrative(members: ContextMembership) -> NarrativeInput {
    NarrativeInput::new(NarrativeId::new("narrative--1").unwrap(), members, stamp())
}
fn campaign(members: ContextMembership) -> CampaignInput {
    CampaignInput::new(
        CampaignId::new("campaign--1").unwrap(),
        vec![NarrativeId::new("narrative--1").unwrap()],
        members,
        stamp(),
    )
}
#[test]
fn creation_retains_every_membership_dimension_without_mutating_claims_or_entities() {
    let (mut graph, members) = fixture();
    let before = graph.epistemic_stores().claims.clone();
    let id = graph.create_narrative(narrative(members.clone())).unwrap();
    let campaign_id = graph.create_campaign(campaign(members.clone())).unwrap();
    let records = &graph.epistemic_stores().narrative_campaigns;
    assert_eq!(records.narrative_by_id(&id).unwrap().membership(), &members);
    assert_eq!(
        records.campaign_by_id(&campaign_id).unwrap().narratives(),
        &[id]
    );
    assert_eq!(
        records.campaign_by_id(&campaign_id).unwrap().stamp(),
        &stamp()
    );
    assert_eq!(graph.epistemic_stores().claims, before);
    assert_eq!(graph.list_nodes().unwrap().len(), 2);
    assert!(graph.list_relationships().unwrap().is_empty());
}
#[test]
fn identities_are_idempotent_and_append_only_for_both_record_kinds() {
    let (mut graph, members) = fixture();
    graph.create_narrative(narrative(members.clone())).unwrap();
    graph.create_narrative(narrative(members.clone())).unwrap();
    graph.create_campaign(campaign(members.clone())).unwrap();
    graph.create_campaign(campaign(members.clone())).unwrap();
    let before = graph.export_memory_json().unwrap();
    let mut changed = members;
    changed.themes.push("another theme".into());
    assert!(matches!(
        graph.create_narrative(narrative(changed.clone())),
        Err(GraphError::ImmutableRecordConflict {
            kind: ImmutableRecordKind::Narrative,
            ..
        })
    ));
    assert!(matches!(
        graph.create_campaign(campaign(changed)),
        Err(GraphError::ImmutableRecordConflict {
            kind: ImmutableRecordKind::Campaign,
            ..
        })
    ));
    assert_eq!(graph.export_memory_json().unwrap(), before);
}
#[test]
fn invalid_membership_or_temporal_data_cannot_partially_append() {
    let (mut graph, members) = fixture();
    let before = graph.export_memory_json().unwrap();
    assert!(graph.create_campaign(campaign(members.clone())).is_err());
    for invalid in [
        ContextMembership {
            claims: vec![ClaimId::new("missing").unwrap()],
            ..members.clone()
        },
        ContextMembership {
            content: vec![SourceId::new("missing").unwrap()],
            ..members.clone()
        },
        ContextMembership {
            themes: vec![" ".into()],
            ..members.clone()
        },
        ContextMembership {
            claims: vec![members.claims[0].clone(), members.claims[0].clone()],
            ..members.clone()
        },
    ] {
        assert!(graph.create_narrative(narrative(invalid)).is_err());
    }
    let mut invalid_stamp = stamp();
    invalid_stamp.valid_to = Some(TemporalTimestamp::new("2025-01-01T00:00:00Z").unwrap());
    assert!(
        graph
            .create_narrative(NarrativeInput::new(
                NarrativeId::new("invalid").unwrap(),
                members,
                invalid_stamp
            ))
            .is_err()
    );
    assert_eq!(graph.export_memory_json().unwrap(), before);
}
#[test]
fn native_round_trip_preserves_records_and_rejects_tampered_references_or_duplicates() {
    let (mut graph, members) = fixture();
    graph.create_narrative(narrative(members.clone())).unwrap();
    graph.create_campaign(campaign(members)).unwrap();
    let json = graph.export_memory_json().unwrap();
    assert_eq!(
        Graph::from_memory_json(&json)
            .unwrap()
            .export_memory_json()
            .unwrap(),
        json
    );
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    for mutate in [0, 1, 2] {
        let mut bad = value.clone();
        let records = &mut bad["epistemic"]["narrative_campaigns"];
        match mutate {
            0 => records["campaigns"][0]["narratives"][0]["value"] = "missing".into(),
            1 => {
                let duplicate = records["narratives"][0].clone();
                records["narratives"]
                    .as_array_mut()
                    .unwrap()
                    .push(duplicate);
            }
            _ => records["narratives"][0]["membership"]["claims"][0]["value"] = "missing".into(),
        };
        assert!(Graph::from_memory_json(&bad.to_string()).is_err());
    }
}
#[test]
fn projection_exposes_collections_and_membership_with_no_factual_override() {
    let (mut graph, members) = fixture();
    graph.create_narrative(narrative(members.clone())).unwrap();
    graph.create_campaign(campaign(members)).unwrap();
    let before = graph.export_memory_json().unwrap();
    let projection = graph.epistemic_projection().unwrap();
    assert_eq!(
        epistemic_nodes_of_kind(&projection, EpistemicNodeKind::Narrative)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        epistemic_nodes_of_kind(&projection, EpistemicNodeKind::Campaign)
            .unwrap()
            .len(),
        1
    );
    assert!(
        validate_graph_structure(&projection, &[])
            .unwrap()
            .is_empty()
    );
    let relations = projection.list_relationships().unwrap();
    assert_eq!(
        relations
            .iter()
            .filter(|r| r.rel_type().as_str() == "HAS_MEMBER")
            .count(),
        9
    );
    for (role, expected) in [
        ("claim", 2),
        ("content", 2),
        ("actor", 2),
        ("infrastructure", 2),
        ("narrative", 1),
    ] {
        assert_eq!(
            relations
                .iter()
                .filter(|r| r.properties().get("membership_role")
                    == Some(&PropertyValue::String(role.into())))
                .count(),
            expected
        );
    }
    assert_eq!(graph.export_memory_json().unwrap(), before);
    assert!(projection.epistemic_stores().narrative_campaigns.is_empty());
}
#[test]
fn existing_native_snapshots_remain_byte_identical_when_collections_are_absent() {
    let (graph, _) = fixture();
    let json = graph.export_memory_json().unwrap();
    assert!(!json.contains("narrative_campaigns"));
    assert_eq!(
        Graph::from_memory_json(&json)
            .unwrap()
            .export_memory_json()
            .unwrap(),
        json
    );
    assert!(
        !Graph::new()
            .export_memory_json()
            .unwrap()
            .contains("epistemic")
    );
}

#[test]
fn membership_edges_cannot_be_interpreted_as_claim_support() {
    assert_eq!(EpistemicRelationKind::HasMember.claim_link_kind(), None);
    let (mut graph, members) = fixture();
    graph.create_narrative(narrative(members)).unwrap();
    let projection = graph.epistemic_projection().unwrap();
    assert!(
        epistemic_nodes_of_kind(&projection, EpistemicNodeKind::Entity)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        epistemic_nodes_of_kind(&projection, EpistemicNodeKind::RecordReference)
            .unwrap()
            .len(),
        2
    );
    assert!(
        projection
            .list_relationships()
            .unwrap()
            .iter()
            .all(|r| r.rel_type().as_str() == "HAS_MEMBER")
    );
}

#[test]
fn unknown_collection_fields_and_duplicate_campaign_members_are_rejected() {
    let (mut graph, members) = fixture();
    let id = graph.create_narrative(narrative(members.clone())).unwrap();
    let before = graph.export_memory_json().unwrap();
    assert!(
        graph
            .create_campaign(CampaignInput::new(
                CampaignId::new("duplicate").unwrap(),
                vec![id.clone(), id],
                members,
                stamp()
            ))
            .is_err()
    );
    assert_eq!(graph.export_memory_json().unwrap(), before);
    let mut bad: serde_json::Value = serde_json::from_str(&before).unwrap();
    bad["epistemic"]["narrative_campaigns"]["narratives"][0]["unexpected_score"] = 1.into();
    assert!(Graph::from_memory_json(&bad.to_string()).is_err());
}

#[test]
fn standalone_collections_remain_durable_and_projection_order_is_repeatable() {
    let mut graph = Graph::new();
    let id = graph
        .create_narrative(NarrativeInput::new(
            NarrativeId::new("standalone").unwrap(),
            ContextMembership::default(),
            stamp(),
        ))
        .unwrap();
    graph
        .create_campaign(CampaignInput::new(
            CampaignId::new("standalone").unwrap(),
            vec![id],
            ContextMembership::default(),
            stamp(),
        ))
        .unwrap();
    let json = graph.export_memory_json().unwrap();
    assert!(json.contains("narrative_campaigns"));
    let restored = Graph::from_memory_json(&json).unwrap();
    assert_eq!(restored.export_memory_json().unwrap(), json);
    // Property maps have no byte-order contract. Compare JSON values while
    // retaining the ordered node/edge arrays, identities and all properties.
    assert_eq!(
        serde_json::to_value(graph.epistemic_projection().unwrap().persistence_snapshot()).unwrap(),
        serde_json::to_value(
            restored
                .epistemic_projection()
                .unwrap()
                .persistence_snapshot()
        )
        .unwrap()
    );
}

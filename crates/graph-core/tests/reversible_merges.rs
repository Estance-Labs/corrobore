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
                relation_neighbourhood: vec![MentionRelationFeature {
                    predicate: "affiliated_with".into(),
                    direction: MentionRelationDirection::Outgoing,
                    counterpart: "Registry C001".into(),
                }],
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
fn decision(
    g: &mut Graph,
    id: &str,
    l: &EntityMentionId,
    r: &EntityMentionId,
    outcome: ReconciliationOutcome,
) -> ReconciliationRecordId {
    g.record_reconciliation(input(
        id,
        l,
        r,
        outcome,
        ReconciliationFeature::SourceContext,
    ))
    .expect("decision")
}
fn undo(id: &str, target: &ReconciliationRecordId) -> MergeUndo {
    MergeUndo::new(
        id,
        target.clone(),
        ActorId::new("analyst").expect("actor"),
        TemporalTimestamp::new("2026-09-06T13:00:00Z").expect("time"),
        "Incorrect identity after review",
    )
    .expect("undo")
}
#[test]
fn undo_restores_mentions_links_and_keeps_both_records_across_restart() {
    let mut g = Graph::new();
    let a = mention(&mut g, "a", "IBM", "registry C001");
    let b = mention(
        &mut g,
        "b",
        "International Business Machines",
        "registry C001",
    );
    let original_mentions = g.epistemic_stores().mentions.clone();
    let before = g.epistemic_projection().expect("projection");
    let m = decision(&mut g, "merge", &a, &b, ReconciliationOutcome::Merge);
    let original_record = g
        .epistemic_stores()
        .reconciliations
        .record_by_id(&m)
        .expect("record")
        .clone();
    g.apply_reconciliation_merge(&m).expect("apply");
    g.apply_reconciliation_merge(&m).expect("retry");
    assert_eq!(g.resolved_mention(&b).expect("resolved"), a);
    let merged = g.epistemic_projection().expect("merged projection");
    assert_eq!(
        merged.list_nodes().expect("nodes").len(),
        before.list_nodes().expect("nodes").len()
    ); // one decision added, one mention grouped
    assert_eq!(g.epistemic_stores().mentions, original_mentions);
    let mut restored = Graph::from_persistence_snapshot(g.persistence_snapshot()).expect("restore");
    assert_eq!(restored.resolved_mention(&b).expect("resolved"), a);
    let u = undo("undo--1", &m);
    restored.undo_reconciliation_merge(u.clone()).expect("undo");
    restored
        .undo_reconciliation_merge(u)
        .expect("idempotent retry");
    assert_eq!(restored.resolved_mention(&b).expect("resolved"), b);
    assert_eq!(restored.epistemic_stores().mentions, original_mentions);
    assert_eq!(
        restored.epistemic_stores().reconciliations.record_by_id(&m),
        Some(&original_record)
    );
    assert_eq!(restored.epistemic_stores().merges.undos().len(), 1);
    let projection = restored.epistemic_projection().expect("projection");
    let links = projection.list_relationships().expect("links");
    let original_links = before.list_relationships().expect("links");
    assert_eq!(
        links
            .iter()
            .filter(|r| r.rel_type().as_str() == "HAS_MENTION")
            .count(),
        original_links
            .iter()
            .filter(|r| r.rel_type().as_str() == "HAS_MENTION")
            .count()
    );
    Graph::from_persistence_snapshot(restored.persistence_snapshot())
        .expect("undo survives restart");
}
#[test]
fn dependent_merge_is_named_and_can_be_undone_in_reverse_order() {
    let mut g = Graph::new();
    let a = mention(&mut g, "a", "A", "same registry");
    let b = mention(&mut g, "b", "B", "same registry");
    let c = mention(&mut g, "c", "C", "same registry");
    let first = decision(&mut g, "first", &a, &b, ReconciliationOutcome::Merge);
    g.apply_reconciliation_merge(&first).expect("apply");
    let second = decision(&mut g, "second", &b, &c, ReconciliationOutcome::Merge);
    g.apply_reconciliation_merge(&second).expect("apply");
    let before = g.persistence_snapshot();
    assert!(
        matches!(g.undo_reconciliation_merge(undo("undo-first",&first)), Err(GraphError::DependentReconciliation { dependent_record, .. }) if dependent_record == second)
    );
    assert_eq!(
        serde_json::to_value(g.persistence_snapshot()).expect("snapshot"),
        serde_json::to_value(before).expect("snapshot")
    );
    g.undo_reconciliation_merge(undo("undo-second", &second))
        .expect("undo child");
    g.undo_reconciliation_merge(undo("undo-first", &first))
        .expect("undo parent");
    assert_eq!(g.resolved_mention(&b).expect("resolved"), b);
    assert_eq!(g.resolved_mention(&c).expect("resolved"), c);
}
#[test]
fn later_abstention_blocks_undo_but_unrelated_decisions_do_not() {
    let mut g = Graph::new();
    let a = mention(&mut g, "a", "A", "same registry");
    let b = mention(&mut g, "b", "B", "same registry");
    let c = mention(&mut g, "c", "C", "unclear registry");
    let d = mention(&mut g, "d", "D", "different registry");
    let first = decision(&mut g, "first", &a, &b, ReconciliationOutcome::Merge);
    g.apply_reconciliation_merge(&first).expect("apply");
    decision(&mut g, "unrelated", &c, &d, ReconciliationOutcome::Distinct);
    let dependent = decision(&mut g, "dependent", &b, &c, ReconciliationOutcome::Abstain);
    assert!(
        matches!(g.undo_reconciliation_merge(undo("undo-first",&first)),Err(GraphError::DependentReconciliation {dependent_record,..}) if dependent_record==dependent)
    );
}
#[test]
fn invalid_undo_or_non_merge_never_changes_state() {
    let mut g = Graph::new();
    let a = mention(&mut g, "a", "A", "registry1");
    let b = mention(&mut g, "b", "B", "registry2");
    let distinct = decision(&mut g, "distinct", &a, &b, ReconciliationOutcome::Distinct);
    let before = g.persistence_snapshot();
    assert!(g.apply_reconciliation_merge(&distinct).is_err());
    assert!(g.undo_reconciliation_merge(undo("u", &distinct)).is_err());
    assert_eq!(
        serde_json::to_value(g.persistence_snapshot()).expect("snapshot"),
        serde_json::to_value(before).expect("snapshot")
    );
    assert!(
        MergeUndo::new(
            "",
            distinct,
            ActorId::new("a").expect("a"),
            TemporalTimestamp::new("2026-09-06T13:00:00Z").expect("time"),
            "reason"
        )
        .is_err()
    );
}

fn mention_links(graph: &Graph) -> Vec<(String, String, String)> {
    let mut links = graph
        .list_relationships()
        .expect("links")
        .iter()
        .filter(|r| r.rel_type().as_str() == "HAS_MENTION")
        .map(|r| {
            let source = graph.get_node(r.source()).expect("node").expect("source");
            let target = graph.get_node(r.target()).expect("node").expect("target");
            (
                serde_json::to_value(source.properties())
                    .expect("properties")
                    .to_string(),
                serde_json::to_value(target.properties())
                    .expect("properties")
                    .to_string(),
                serde_json::to_value(r.properties())
                    .expect("properties")
                    .to_string(),
            )
        })
        .collect::<Vec<_>>();
    links.sort();
    links
}
#[test]
fn restoring_a_merge_restores_exact_observation_links_and_member_properties() {
    let mut g = Graph::new();
    let a = mention(&mut g, "a", "A", "context A");
    let b = mention(&mut g, "b", "B", "context B");
    let before = mention_links(&g.epistemic_projection().expect("projection"));
    let m = decision(&mut g, "merge", &a, &b, ReconciliationOutcome::Merge);
    g.apply_reconciliation_merge(&m).expect("apply");
    let merged = g.epistemic_projection().expect("projection");
    assert_ne!(mention_links(&merged), before);
    let nodes = merged.list_nodes().expect("nodes");
    let members = nodes
        .iter()
        .find_map(|n| n.property("mention_members"))
        .expect("merged members");
    assert!(
        matches!(members,PropertyValue::Json(values) if values.as_array().expect("members").len()==2)
    );
    g.undo_reconciliation_merge(undo("undo", &m)).expect("undo");
    assert_eq!(
        mention_links(&g.epistemic_projection().expect("projection")),
        before
    );
    assert!(g.apply_reconciliation_merge(&m).is_err());
    assert!(
        g.undo_reconciliation_merge(undo("different-undo-id", &m))
            .is_err()
    );
}
#[test]
fn forged_dependencies_and_duplicate_undos_are_rejected_on_restore() {
    let mut g = Graph::new();
    let a = mention(&mut g, "a", "A", "same registry");
    let b = mention(&mut g, "b", "B", "same registry");
    let c = mention(&mut g, "c", "C", "same registry");
    let first = decision(&mut g, "first", &a, &b, ReconciliationOutcome::Merge);
    g.apply_reconciliation_merge(&first).expect("apply");
    let second = decision(&mut g, "second", &b, &c, ReconciliationOutcome::Merge);
    let mut json = serde_json::to_value(g.persistence_snapshot()).expect("encode");
    json["epistemic"]["merges"]["events"][2]["dependencies"] = serde_json::json!([]);
    assert!(
        Graph::from_persistence_snapshot(serde_json::from_value(json).expect("decode")).is_err()
    );
    g.apply_reconciliation_merge(&second).expect("apply");
    g.undo_reconciliation_merge(undo("undo-second", &second))
        .expect("undo");
    let mut json = serde_json::to_value(g.persistence_snapshot()).expect("encode");
    let events = json["epistemic"]["merges"]["events"]
        .as_array_mut()
        .expect("events");
    events.push(events.last().expect("undo").clone());
    assert!(
        Graph::from_persistence_snapshot(serde_json::from_value(json).expect("decode")).is_err()
    );
}

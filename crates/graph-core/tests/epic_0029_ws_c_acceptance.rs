//! Epic #192 release gate. HTTP and compatibility evidence is mapped in ingestion.md.
use graph_core::*;
use serde_json::{Value, json};
#[path = "support/ingestion_quality.rs"]
mod fixtures;

fn candidate(id: &str, raw: &str, constraints: Vec<CandidateConstraint>) -> CandidateInput {
    CandidateInput::new(
        id,
        ExtractionRunId::new("run--acceptance").expect("run"),
        raw,
        ActorId::new("extractor").expect("actor"),
    )
    .expect("input")
    .with_constraints(constraints)
}
#[test]
fn spike_b_schema_temporal_and_entity_violations_never_reach_canonical() {
    for (name, field, rule, payload) in [
        ("schema", "/name", CandidateRule::Required, json!({})),
        (
            "temporal",
            "/end",
            CandidateRule::TemporalOrder {
                after: "/start".into(),
            },
            json!({"start":"2026-09-06T12:00:00Z","end":"2026-09-05T12:00:00Z"}),
        ),
        (
            "entity",
            "/entity/name",
            CandidateRule::Type {
                expected: CandidateValueType::String,
            },
            json!({"entity":{"name":42}}),
        ),
    ] {
        for tier in [GraphTier::Shadow, GraphTier::Hypothesis] {
            let mut graph = Graph::new();
            let raw = format!(" {} \n", payload);
            let constraint = CandidateConstraint {
                id: name.into(),
                field: field.into(),
                rule: rule.clone(),
            };
            let input = graph
                .submit_candidate(candidate(name, &raw, vec![constraint.clone()]).with_tier(tier))
                .expect("retain");
            let store = &graph.epistemic_stores().candidates;
            assert_eq!(store.tier_of(input.id()), Some(tier));
            assert_eq!(input.raw_payload(), raw);
            assert_eq!(input.extraction_run_id().as_str(), "run--acceptance");
            let report = store.validation(input.id()).expect("feedback");
            assert!(!report.valid);
            assert_eq!(report.failures[0].field, field);
            assert_eq!(report.failures[0].constraint, constraint);
            assert!(
                graph
                    .promote_candidate(
                        input.id(),
                        ActorId::new("reviewer").expect("actor"),
                        "review",
                        CandidatePromotionInput::Node(NodeInput::new(["Entity"]))
                    )
                    .is_err()
            );
            assert!(graph.list_nodes().expect("nodes").is_empty());
            assert!(
                graph
                    .list_relationships()
                    .expect("relationships")
                    .is_empty()
            );
        }
    }
}
#[test]
fn published_agent_examples_reextract_only_the_failing_field_and_preserve_lineage() {
    let guide = include_str!(
        "../../../plugins/corrobore/skills/corrobore/references/candidate-ingestion.md"
    );
    let samples: Vec<Value> = guide
        .split("```json\n")
        .skip(1)
        .map(|s| {
            serde_json::from_str(s.split("\n```").next().expect("fence")).expect("JSON example")
        })
        .collect();
    let submission = &samples[0];
    let repair = &samples[1];
    let mut graph = Graph::new();
    let original = graph
        .submit_candidate(
            CandidateInput::new(
                submission["id"].as_str().expect("id"),
                ExtractionRunId::new(submission["extraction_run_id"].as_str().expect("run"))
                    .expect("run"),
                submission["raw_payload"].as_str().expect("raw"),
                ActorId::new(submission["actor"].as_str().expect("actor")).expect("actor"),
            )
            .expect("input")
            .with_tier(serde_json::from_value(submission["tier"].clone()).expect("tier"))
            .with_constraints(
                serde_json::from_value(submission["constraints"].clone()).expect("constraints"),
            ),
        )
        .expect("submit");
    let feedback = graph
        .epistemic_stores()
        .candidates
        .validation(original.id())
        .expect("feedback");
    assert_eq!(feedback.failures.len(), 1);
    assert_eq!(feedback.failures[0].field, "/name");
    let revised = graph
        .repair_candidate(
            original.id(),
            CandidateInput::new(
                repair["id"].as_str().expect("id"),
                ExtractionRunId::new(repair["extraction_run_id"].as_str().expect("run"))
                    .expect("run"),
                repair["raw_payload"].as_str().expect("raw"),
                ActorId::new(repair["actor"].as_str().expect("actor")).expect("actor"),
            )
            .expect("input"),
            serde_json::from_value(repair["caused_by"].clone()).expect("causes"),
        )
        .expect("repair");
    let before: Value = serde_json::from_str(original.raw_payload()).expect("before");
    let mut after: Value = serde_json::from_str(revised.raw_payload()).expect("after");
    after["name"] = before["name"].clone();
    assert_eq!(before, after);
    assert_eq!(
        revised.repair().expect("lineage").predecessor,
        *original.id()
    );
    assert!(
        graph
            .epistemic_stores()
            .candidates
            .validation(revised.id())
            .expect("feedback")
            .valid
    );
    assert!(graph.list_nodes().expect("nodes").is_empty());
    graph
        .promote_candidate(
            revised.id(),
            ActorId::new("reviewer").expect("actor"),
            "Explicit source review",
            CandidatePromotionInput::Node(NodeInput::new(["Entity"])),
        )
        .expect("explicit promotion");
    assert_eq!(
        graph.epistemic_stores().candidates.get(original.id()),
        Some(&original)
    );
    assert_eq!(graph.list_nodes().expect("nodes").len(), 1);
}
#[test]
fn spike_d_aliases_transliteration_homonyms_and_abstention_use_evidence() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/reconciliation_aliases.json"))
            .expect("fixtures");
    for case in cases {
        let mut graph = Graph::new();
        let left = fixtures::mention(
            &mut graph,
            "left",
            case["left"].as_str().expect("left"),
            case["left_context"].as_str().expect("context"),
        );
        let right = fixtures::mention(
            &mut graph,
            "right",
            case["right"].as_str().expect("right"),
            case["right_context"].as_str().expect("context"),
        );
        let outcome: ReconciliationOutcome =
            serde_json::from_value(case["outcome"].clone()).expect("outcome");
        assert!(
            graph
                .record_reconciliation(
                    fixtures::input(
                        "similarity-only",
                        &left,
                        &right,
                        outcome,
                        ReconciliationFeature::SurfaceForm
                    )
                    .with_similarity_hints(vec![ReconciliationSimilarity {
                        kind: ReconciliationSimilarityKind::Embedding,
                        score: Confidence::new(1.0).expect("score")
                    }])
                )
                .is_err()
        );
        let id = graph
            .record_reconciliation(fixtures::input(
                "grounded",
                &left,
                &right,
                outcome,
                ReconciliationFeature::SourceContext,
            ))
            .expect("evidence-cited decision");
        let record = graph
            .epistemic_stores()
            .reconciliations
            .record_by_id(&id)
            .expect("record");
        assert_eq!(record.citations()[0].value, case["left_context"]);
        assert_eq!(record.citations()[1].value, case["right_context"]);
        assert!(matches!(record.decider(), ReconciliationDecider::Actor(_)));
        assert_eq!(graph.resolved_mention(&right).expect("resolve"), right);
        if outcome == ReconciliationOutcome::Merge {
            graph
                .apply_reconciliation_merge(&id)
                .expect("supported merge");
            assert_eq!(graph.resolved_mention(&right).expect("resolve"), left);
        } else {
            assert!(graph.apply_reconciliation_merge(&id).is_err());
            assert_ne!(
                graph.resolved_mention(&left).expect("left"),
                graph.resolved_mention(&right).expect("right")
            );
        }
        assert!(graph.list_nodes().expect("canonical nodes").is_empty());
        assert_eq!(graph.epistemic_stores().mentions.len(), 2);
    }
}
#[test]
fn merge_and_undo_provenance_survive_restart_with_original_mentions_and_links() {
    let mut graph = Graph::new();
    let left = fixtures::mention(&mut graph, "left", "IBM", "C001");
    let right = fixtures::mention(
        &mut graph,
        "right",
        "International Business Machines",
        "C001",
    );
    let original = graph.epistemic_stores().mentions.clone();
    let id = graph
        .record_reconciliation(fixtures::input(
            "merge",
            &left,
            &right,
            ReconciliationOutcome::Merge,
            ReconciliationFeature::SourceContext,
        ))
        .expect("record");
    let record = graph
        .epistemic_stores()
        .reconciliations
        .record_by_id(&id)
        .expect("record")
        .clone();
    graph.apply_reconciliation_merge(&id).expect("apply");
    graph
        .undo_reconciliation_merge(
            MergeUndo::new(
                "undo",
                id.clone(),
                ActorId::new("analyst").expect("actor"),
                TemporalTimestamp::new("2026-09-06T14:00:00Z").expect("time"),
                "Rechecked source",
            )
            .expect("undo"),
        )
        .expect("reverse");
    let restored = Graph::from_persistence_snapshot(graph.persistence_snapshot()).expect("restart");
    assert_eq!(restored.epistemic_stores().mentions, original);
    assert_eq!(
        restored
            .epistemic_stores()
            .reconciliations
            .record_by_id(&id),
        Some(&record)
    );
    assert_eq!(restored.epistemic_stores().merges.undos().len(), 1);
    assert_eq!(restored.resolved_mention(&right).expect("resolve"), right);
    let view = restored.epistemic_projection().expect("view");
    for mention in [left, right] {
        let edge = view
            .list_relationships()
            .expect("links")
            .into_iter()
            .find(|r| {
                r.rel_type().as_str() == "HAS_MENTION"
                    && r.property("mention_id")
                        == Some(&PropertyValue::String(mention.as_str().into()))
            })
            .expect("original link");
        assert_eq!(
            view.get_node(edge.target())
                .expect("target")
                .expect("target")
                .property("mention_id"),
            Some(&PropertyValue::String(mention.as_str().into()))
        );
    }
}
#[test]
fn repair_quality_is_measured_independently_from_extraction_and_abstention_is_visible() {
    let metrics = fixtures::seeded().ingestion_metrics().expect("metrics");
    assert_eq!(metrics.extraction.accuracy(), Some(0.5));
    assert_eq!(metrics.repair_success_rate(), Some(0.25));
    assert_eq!(metrics.false_repair_rate(), Some(0.5));
    assert_eq!(metrics.abstain_rate(), Some(0.4));
    assert_eq!(metrics.reconciliation[0].accuracy(), Some(0.5));
    assert_eq!(metrics.reconciliation[1].accuracy(), Some(1.0));
    assert_eq!(metrics.reconciliation[2].accuracy(), Some(0.5));
}

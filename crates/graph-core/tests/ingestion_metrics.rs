use graph_core::*;
#[path = "support/ingestion_quality.rs"]
mod fixtures;
#[test]
fn over_repairing_pipeline_is_not_mistaken_for_accurate_extraction() {
    let g = fixtures::seeded();
    let m = g.ingestion_metrics().expect("metrics");
    assert_eq!(m.extraction.accuracy(), Some(0.5));
    assert_eq!(m.repair_success_rate(), Some(0.25));
    assert_eq!(m.false_repair_rate(), Some(0.5));
    assert_eq!(m.repairs, 4);
    assert_eq!(m.reviewed_repairs, 4);
    assert_eq!(m.reconciliation[0].accuracy(), Some(0.5));
    assert_eq!(m.reconciliation[1].accuracy(), Some(1.0));
    assert_eq!(m.reconciliation[2].accuracy(), Some(0.5));
    assert_eq!(m.abstain_rate(), Some(0.4));
    let restored = Graph::from_persistence_snapshot(g.persistence_snapshot()).expect("restore");
    assert_eq!(restored.ingestion_metrics().expect("metrics"), m);
}
#[test]
fn absent_labels_are_unknown_not_success() {
    let mut g = fixtures::seeded();
    g.epistemic_stores_mut().ingestion_evaluations = IngestionEvaluationStore::default();
    let m = g.ingestion_metrics().expect("metrics");
    assert_eq!(m.extraction.accuracy(), None);
    assert_eq!(m.repair_success_rate(), None);
    assert_eq!(m.false_repair_rate(), None);
    assert_eq!(m.reconciliation[0].accuracy(), None);
    assert_eq!(m.abstain_rate(), Some(0.4));
    let empty = Graph::new().ingestion_metrics().expect("metrics");
    assert_eq!(empty.abstain_rate(), None);
}
#[test]
fn assessments_are_immutable_bound_and_do_not_double_count_retries() {
    let mut g = fixtures::seeded();
    let before = g.ingestion_metrics().expect("metrics");
    let input = CandidateAssessment::new(
        CandidateId::new("original-0").expect("id"),
        false,
        ActorId::new("reviewer").expect("actor"),
        "fixture ground truth",
    );
    g.record_candidate_assessment(input).expect("retry");
    assert_eq!(g.ingestion_metrics().expect("metrics"), before);
    assert!(
        g.record_candidate_assessment(CandidateAssessment::new(
            CandidateId::new("original-0").expect("id"),
            true,
            ActorId::new("reviewer").expect("actor"),
            "different truth"
        ))
        .is_err()
    );
    assert!(
        g.record_candidate_assessment(CandidateAssessment::new(
            CandidateId::new("missing").expect("id"),
            true,
            ActorId::new("reviewer").expect("actor"),
            "truth"
        ))
        .is_err()
    );
}

#[test]
fn abstain_rate_is_zero_for_an_observed_never_abstaining_engine() {
    let mut g = Graph::new();
    let a = fixtures::mention(&mut g, "a", "A", "context");
    let b = fixtures::mention(&mut g, "b", "B", "context");
    g.record_reconciliation(fixtures::input(
        "record",
        &a,
        &b,
        ReconciliationOutcome::Merge,
        ReconciliationFeature::SourceContext,
    ))
    .expect("record");
    let m = g.ingestion_metrics().expect("metrics");
    assert_eq!(m.abstain_rate(), Some(0.0));
    assert_eq!(m.reconciliation[0].total, 1);
    assert_eq!(m.reconciliation[0].accuracy(), None);
}
#[test]
fn partial_repair_review_does_not_enter_the_denominator() {
    let mut g = fixtures::seeded();
    g.epistemic_stores_mut().ingestion_evaluations = IngestionEvaluationStore::default();
    g.record_candidate_assessment(CandidateAssessment::new(
        CandidateId::new("original-0").expect("id"),
        false,
        ActorId::new("reviewer").expect("actor"),
        "truth",
    ))
    .expect("review");
    let m = g.ingestion_metrics().expect("metrics");
    assert_eq!(m.repairs, 4);
    assert_eq!(m.reviewed_repairs, 0);
    assert_eq!(m.extraction.reviewed, 1);
    assert_eq!(m.extraction.accuracy(), Some(0.0));
    assert_eq!(m.repair_success_rate(), None);
    assert_eq!(m.false_repair_rate(), None);
}
#[test]
fn persisted_reference_labels_reject_missing_targets_and_duplicates() {
    let g = fixtures::seeded();
    let original = serde_json::to_value(g.persistence_snapshot()).expect("snapshot");
    let mut bad = original.clone();
    bad["epistemic"]["ingestion_evaluations"]["candidates"][0]["candidate_id"]["value"] =
        serde_json::json!("missing");
    assert!(
        Graph::from_persistence_snapshot(serde_json::from_value(bad).expect("decode")).is_err()
    );
    let mut bad = original;
    let labels = bad["epistemic"]["ingestion_evaluations"]["reconciliations"]
        .as_array_mut()
        .expect("labels");
    labels.push(labels[0].clone());
    assert!(
        Graph::from_persistence_snapshot(serde_json::from_value(bad).expect("decode")).is_err()
    );
}

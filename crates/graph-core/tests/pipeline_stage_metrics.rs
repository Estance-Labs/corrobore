use graph_core::{PipelineStage, StageMeasurement, StageMetricsRegistry};
use serde_json::json;

fn sample(id: &str, stage: PipelineStage, failures: u64) -> StageMeasurement {
    StageMeasurement::new(id, stage, "fixture-v1", 10, 10 - failures, failures)
        .expect("measurement")
}
#[test]
fn every_stage_is_versioned_and_a_regression_remains_local() {
    let mut metrics = StageMetricsRegistry::default();
    for stage in PipelineStage::ALL {
        metrics
            .record("baseline", sample("sample", stage, 0))
            .expect("record");
        metrics
            .record(
                "regression",
                sample(
                    "sample",
                    stage,
                    if stage == PipelineStage::Retrieval {
                        3
                    } else {
                        0
                    },
                ),
            )
            .expect("record");
    }
    let before = serde_json::to_value(metrics.report("baseline").expect("report")).expect("json");
    let after = serde_json::to_value(metrics.report("regression").expect("report")).expect("json");
    assert_eq!(before["schema_version"], "corrobore-stage-metrics-v1");
    assert_eq!(before["stages"].as_array().expect("stages").len(), 7);
    let changed: Vec<_> = before["stages"]
        .as_array()
        .expect("stages")
        .iter()
        .zip(after["stages"].as_array().expect("stages"))
        .filter(|(a, b)| a != b)
        .map(|(_, b)| b["stage"].as_str().expect("stage"))
        .collect();
    assert_eq!(changed, ["retrieval"]);
    assert_eq!(after["stages"][2]["failures"], 3);
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/pipeline-stage-metrics-v1.json"))
            .expect("portable fixture");
    assert_eq!(after, fixture);
}
#[test]
fn missing_measurements_are_null_and_retries_do_not_double_count() {
    let mut metrics = StageMetricsRegistry::default();
    let entry = sample("sample", PipelineStage::Verifier, 2);
    metrics.record("run", entry.clone()).expect("record");
    let expected = metrics.report("run").expect("report");
    metrics.record("run", entry).expect("retry");
    assert_eq!(metrics.report("run").expect("report"), expected);
    assert!(
        metrics
            .record("run", sample("sample", PipelineStage::Verifier, 3))
            .is_err()
    );
    assert_eq!(metrics.report("run").expect("report"), expected);
    let value = serde_json::to_value(expected).expect("json");
    assert!(value["stages"][0]["inputs"].is_null());
    assert_eq!(value["stages"][5]["inputs"], 10);
    assert_eq!(value["stages"][5]["measurements"], 1);
    assert!(metrics.report("missing").is_err());
}
#[test]
fn invalid_data_and_incompatible_versions_cannot_enter_the_registry() {
    assert!(StageMeasurement::new("", PipelineStage::Verdict, "producer", 1, 1, 0).is_err());
    assert!(StageMeasurement::new("x", PipelineStage::Verdict, "", 1, 1, 0).is_err());
    assert!(StageMeasurement::new("x", PipelineStage::Verdict, "p", 1, 1, 2).is_err());
    assert!(StageMeasurement::new("x", PipelineStage::Verdict, "p", u64::MAX, 0, 0).is_err());
    let mut value = serde_json::to_value(sample("x", PipelineStage::Verdict, 0)).expect("json");
    value["schema_version"] = json!("v999");
    assert!(serde_json::from_value::<StageMeasurement>(value).is_err());
}
#[test]
fn saturation_and_capacity_fail_atomically_without_evicting_evidence() {
    let mut metrics = StageMetricsRegistry::with_limits(1, 2).expect("limits");
    metrics
        .record("r", sample("a", PipelineStage::Extraction, 0))
        .expect("record");
    let before = metrics.report("r").expect("report");
    assert!(
        metrics
            .record("other", sample("a", PipelineStage::Extraction, 0))
            .is_err()
    );
    let maximum = StageMeasurement::new(
        "b",
        PipelineStage::Extraction,
        "fixture-v1",
        9_007_199_254_740_991,
        0,
        0,
    )
    .expect("bounded");
    assert!(metrics.record("r", maximum).is_err());
    assert_eq!(metrics.report("r").expect("report"), before);
    metrics
        .record("r", sample("b", PipelineStage::Extraction, 0))
        .expect("record");
    assert!(
        metrics
            .record("r", sample("c", PipelineStage::Extraction, 0))
            .is_err()
    );
}

//! Versioned, run-scoped pipeline telemetry. Counts are supplied by stage instrumentation,
//! never inferred from a final score or treated as evidence for a claim.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Largest integer preserved exactly by JSON consumers using JavaScript numbers.
const MAX_COUNT: u64 = 9_007_199_254_740_991;
/// The stage meanings and count units of the v1 contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelineStage {
    /// Source documents to candidate assertions.
    Extraction,
    /// Mention comparisons to identity decisions.
    EntityResolution,
    /// Retrieval requests to evidence items.
    Retrieval,
    /// Seed sets to bounded subgraphs.
    SubgraphConstruction,
    /// Claim/evidence sets to sufficiency assessments.
    EvidenceSufficiency,
    /// Verification requests to verifier records.
    Verifier,
    /// Claims to stored verdicts.
    Verdict,
}
impl PipelineStage {
    /// Canonical stage order, including stages with no measurements.
    pub const ALL: [Self; 7] = [
        Self::Extraction,
        Self::EntityResolution,
        Self::Retrieval,
        Self::SubgraphConstruction,
        Self::EvidenceSufficiency,
        Self::Verifier,
        Self::Verdict,
    ];
    fn units(self) -> (&'static str, &'static str) {
        match self {
            Self::Extraction => ("documents", "candidate_assertions"),
            Self::EntityResolution => ("mention_comparisons", "identity_decisions"),
            Self::Retrieval => ("retrieval_requests", "evidence_items"),
            Self::SubgraphConstruction => ("seed_sets", "subgraphs"),
            Self::EvidenceSufficiency => ("claim_evidence_sets", "sufficiency_assessments"),
            Self::Verifier => ("verification_requests", "verification_records"),
            Self::Verdict => ("claims", "stored_verdicts"),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum SchemaVersion {
    #[serde(rename = "corrobore-stage-metrics-v1")]
    V1,
}
/// A completed measurement from an identified stage instrumentor.
/// Failures count input work items that failed processing, not missing outputs,
/// abstentions, negative verification results or incorrect claims.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageMeasurement {
    schema_version: SchemaVersion,
    measurement_id: String,
    stage: PipelineStage,
    producer: String,
    inputs: u64,
    outputs: u64,
    failures: u64,
}
/// Rejected telemetry never alters an existing report.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum StageMetricError {
    /// Empty identity, invalid count, or a count outside the portable JSON range.
    #[error("invalid stage measurement: {0}")]
    Invalid(String),
    /// Reusing one run/stage/measurement identity with different content.
    #[error("conflicting stage measurement identity")]
    Conflict,
    /// Configured telemetry retention is full; no existing report is evicted.
    #[error("stage metric capacity reached")]
    Capacity,
    /// No measurement has been retained for this run in this engine instance.
    #[error("unknown stage metric run")]
    UnknownRun,
}
fn valid_id(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}
impl StageMeasurement {
    /// Prepare a portable measured sample. One output count may exceed inputs (fan-out).
    pub fn new(
        id: impl Into<String>,
        stage: PipelineStage,
        producer: impl Into<String>,
        inputs: u64,
        outputs: u64,
        failures: u64,
    ) -> Result<Self, StageMetricError> {
        // Validate attribution and v1 count meanings before allowing a report to change.
        let measurement = Self {
            schema_version: SchemaVersion::V1,
            measurement_id: id.into(),
            stage,
            producer: producer.into(),
            inputs,
            outputs,
            failures,
        };
        measurement.validate()?;
        Ok(measurement)
    }
    fn validate(&self) -> Result<(), StageMetricError> {
        if !valid_id(&self.measurement_id)
            || !valid_id(&self.producer)
            || [self.inputs, self.outputs, self.failures]
                .into_iter()
                .any(|n| n > MAX_COUNT)
            || self.failures > self.inputs
        {
            return Err(StageMetricError::Invalid("identity or count bounds".into()));
        }
        Ok(())
    }
}
/// One stage row. Null counts mean unmeasured; observed zero remains zero.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StageMetric {
    stage: PipelineStage,
    input_unit: &'static str,
    output_unit: &'static str,
    measurements: usize,
    producers: BTreeSet<String>,
    inputs: Option<u64>,
    outputs: Option<u64>,
    failures: Option<u64>,
}
/// Deterministic, additive report emitted by an engine for one instrumented run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PipelineStageReport {
    schema_version: SchemaVersion,
    run_id: String,
    source: &'static str,
    stages: Vec<StageMetric>,
}
/// Bounded per-engine telemetry. Callers export reports before ending the engine lifetime.
/// The registry does not mix telemetry into persisted claim evidence or audit archives.
#[derive(Clone, Debug)]
pub struct StageMetricsRegistry {
    runs: BTreeMap<String, BTreeMap<(PipelineStage, String), StageMeasurement>>,
    max_runs: usize,
    max_measurements_per_run: usize,
}
impl Default for StageMetricsRegistry {
    fn default() -> Self {
        Self {
            runs: BTreeMap::new(),
            max_runs: 256,
            max_measurements_per_run: 4096,
        }
    }
}
impl StageMetricsRegistry {
    /// Set positive per-instance run and per-run measurement bounds.
    pub fn with_limits(
        max_runs: usize,
        max_measurements_per_run: usize,
    ) -> Result<Self, StageMetricError> {
        if max_runs == 0 || max_measurements_per_run == 0 {
            return Err(StageMetricError::Invalid("zero capacity".into()));
        }
        Ok(Self {
            runs: BTreeMap::new(),
            max_runs,
            max_measurements_per_run,
        })
    }
    /// Retain an exact-retry-safe sample; reject conflicts, overflow and capacity atomically.
    pub fn record(
        &mut self,
        run_id: &str,
        measurement: StageMeasurement,
    ) -> Result<(), StageMetricError> {
        // Validate before insertion, compare retries, check capacity and portable sums.
        if !valid_id(run_id) {
            return Err(StageMetricError::Invalid("run identity".into()));
        }
        measurement.validate()?;
        let key = (measurement.stage, measurement.measurement_id.clone());
        if let Some(run) = self.runs.get(run_id) {
            if let Some(previous) = run.get(&key) {
                return if previous == &measurement {
                    Ok(())
                } else {
                    Err(StageMetricError::Conflict)
                };
            }
            if run.len() >= self.max_measurements_per_run {
                return Err(StageMetricError::Capacity);
            }
            let mut counts = [
                measurement.inputs,
                measurement.outputs,
                measurement.failures,
            ];
            for item in run.values().filter(|item| item.stage == measurement.stage) {
                for (count, additional) in
                    counts
                        .iter_mut()
                        .zip([item.inputs, item.outputs, item.failures])
                {
                    *count = count
                        .checked_add(additional)
                        .filter(|n| *n <= MAX_COUNT)
                        .ok_or_else(|| {
                            StageMetricError::Invalid("aggregate counter overflow".into())
                        })?;
                }
            }
        } else if self.runs.len() >= self.max_runs {
            return Err(StageMetricError::Capacity);
        }
        self.runs
            .entry(run_id.into())
            .or_default()
            .insert(key, measurement);
        Ok(())
    }
    /// Emit every stage in canonical order without fabricating unobserved counters.
    pub fn report(&self, run_id: &str) -> Result<PipelineStageReport, StageMetricError> {
        // Aggregate retained observations only; producers remain explicit per stage.
        let run = self.runs.get(run_id).ok_or(StageMetricError::UnknownRun)?;
        let stages = PipelineStage::ALL
            .into_iter()
            .map(|stage| {
                let items: Vec<_> = run.values().filter(|item| item.stage == stage).collect();
                let (input_unit, output_unit) = stage.units();
                StageMetric {
                    stage,
                    input_unit,
                    output_unit,
                    measurements: items.len(),
                    producers: items.iter().map(|item| item.producer.clone()).collect(),
                    inputs: (!items.is_empty()).then(|| items.iter().map(|item| item.inputs).sum()),
                    outputs: (!items.is_empty())
                        .then(|| items.iter().map(|item| item.outputs).sum()),
                    failures: (!items.is_empty())
                        .then(|| items.iter().map(|item| item.failures).sum()),
                }
            })
            .collect();
        Ok(PipelineStageReport {
            schema_version: SchemaVersion::V1,
            run_id: run_id.into(),
            source: "stage_instrumentation",
            stages,
        })
    }
}

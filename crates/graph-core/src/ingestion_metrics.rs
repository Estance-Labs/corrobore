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
//! Independent reviewed ingestion quality, separate from constraint validity.
use crate::*;
use serde::{Deserialize, Serialize};
/// Reference judgment on one original or repaired candidate.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CandidateAssessment {
    candidate_id: CandidateId,
    correct: bool,
    reviewer: ActorId,
    reference: String,
}
impl CandidateAssessment {
    /// Prepare an assessment; recording validates attribution and target binding.
    pub fn new(
        candidate_id: CandidateId,
        correct: bool,
        reviewer: ActorId,
        reference: impl Into<String>,
    ) -> Self {
        Self {
            candidate_id,
            correct,
            reviewer,
            reference: reference.into(),
        }
    }
}
/// Reference identity outcome, independently established by a reviewer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReconciliationAssessment {
    record_id: ReconciliationRecordId,
    expected: ReconciliationOutcome,
    reviewer: ActorId,
    reference: String,
}
impl ReconciliationAssessment {
    /// Prepare a reference label for one retained decision.
    pub fn new(
        record_id: ReconciliationRecordId,
        expected: ReconciliationOutcome,
        reviewer: ActorId,
        reference: impl Into<String>,
    ) -> Self {
        Self {
            record_id,
            expected,
            reviewer,
            reference: reference.into(),
        }
    }
}
/// Immutable reference labels retained with the governed graph.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct IngestionEvaluationStore {
    candidates: Vec<CandidateAssessment>,
    reconciliations: Vec<ReconciliationAssessment>,
}
impl IngestionEvaluationStore {
    /// Whether there are no reference labels.
    pub fn is_empty(&self) -> bool {
        self.candidates.is_empty() && self.reconciliations.is_empty()
    }
}
/// Observed and reviewed counts; unlabeled examples never enter accuracy.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EvaluatedCount {
    /// Observed examples, including unreviewed examples.
    pub total: u64,
    /// Examples with an independent reference label.
    pub reviewed: u64,
    /// Reviewed examples matching their reference label.
    pub correct: u64,
}
impl EvaluatedCount {
    /// Correct / reviewed, or unknown when no labels exist.
    pub fn accuracy(&self) -> Option<f64> {
        ratio(self.correct, self.reviewed)
    }
}
fn ratio(n: u64, d: u64) -> Option<f64> {
    (d > 0).then(|| n as f64 / d as f64)
}
/// Snapshot of ingestion quality; all counts are derived from retained records.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct IngestionMetrics {
    /// Original extraction quality, excluding repair versions.
    pub extraction: EvaluatedCount,
    /// Number of repair links, irrespective of evaluation coverage.
    pub repairs: u64,
    /// Repairs with both predecessor and successor independently reviewed.
    pub reviewed_repairs: u64,
    /// Incorrect predecessors corrected by a repair.
    pub successful_repairs: u64,
    /// Correct predecessors made incorrect by a repair.
    pub false_repairs: u64,
    /// Per predicted outcome, ordered Merge, Distinct, Abstain.
    pub reconciliation: [EvaluatedCount; 3],
}
impl IngestionMetrics {
    /// Incorrect-to-correct transitions / fully reviewed repairs.
    pub fn repair_success_rate(&self) -> Option<f64> {
        ratio(self.successful_repairs, self.reviewed_repairs)
    }
    /// Correct-to-incorrect transitions / fully reviewed repairs.
    pub fn false_repair_rate(&self) -> Option<f64> {
        ratio(self.false_repairs, self.reviewed_repairs)
    }
    /// Observed abstentions / all decisions, independent of review coverage.
    pub fn abstain_rate(&self) -> Option<f64> {
        ratio(
            self.reconciliation[2].total,
            self.reconciliation.iter().map(|x| x.total).sum(),
        )
    }
}

fn invalid(message: &str) -> GraphError {
    GraphError::InvalidPropertyValue(message.into())
}
fn attribution(reviewer: &ActorId, reference: &str) -> Result<(), GraphError> {
    ActorId::new(reviewer.as_str())?;
    if reference.trim().is_empty() {
        return Err(invalid("evaluation requires a reference"));
    }
    Ok(())
}
impl CandidateAssessment {
    fn validate(&self, candidates: &CandidateStore) -> Result<(), GraphError> {
        attribution(&self.reviewer, &self.reference)?;
        CandidateId::new(self.candidate_id.as_str())?;
        if candidates.get(&self.candidate_id).is_none() {
            return Err(invalid("assessment candidate missing"));
        }
        Ok(())
    }
}
impl ReconciliationAssessment {
    fn validate(&self, records: &ReconciliationStore) -> Result<(), GraphError> {
        attribution(&self.reviewer, &self.reference)?;
        ReconciliationRecordId::new(self.record_id.as_str())?;
        if records.record_by_id(&self.record_id).is_none() {
            return Err(invalid("assessment reconciliation missing"));
        }
        Ok(())
    }
}
impl IngestionEvaluationStore {
    pub(crate) fn validate_bindings(
        &self,
        candidates: &CandidateStore,
        records: &ReconciliationStore,
    ) -> Result<(), GraphError> {
        let mut ids = std::collections::HashSet::new();
        for input in &self.candidates {
            input.validate(candidates)?;
            if !ids.insert(&input.candidate_id) {
                return Err(invalid("duplicate candidate assessment"));
            }
        }
        let mut ids = std::collections::HashSet::new();
        for input in &self.reconciliations {
            input.validate(records)?;
            if !ids.insert(&input.record_id) {
                return Err(invalid("duplicate reconciliation assessment"));
            }
        }
        Ok(())
    }
}
impl Graph {
    /// Retain one independent candidate assessment; exact retries are idempotent.
    pub fn record_candidate_assessment(
        &mut self,
        input: CandidateAssessment,
    ) -> Result<(), GraphError> {
        let stores = self.epistemic_stores_mut();
        input.validate(&stores.candidates)?;
        let records = &mut stores.ingestion_evaluations.candidates;
        if let Some(old) = records
            .iter()
            .find(|old| old.candidate_id == input.candidate_id)
        {
            return if old == &input {
                Ok(())
            } else {
                Err(invalid("immutable candidate assessment conflict"))
            };
        }
        records.push(input);
        Ok(())
    }
    /// Retain an expected identity outcome independently of the recorded judgment.
    pub fn record_reconciliation_assessment(
        &mut self,
        input: ReconciliationAssessment,
    ) -> Result<(), GraphError> {
        let stores = self.epistemic_stores_mut();
        input.validate(&stores.reconciliations)?;
        let records = &mut stores.ingestion_evaluations.reconciliations;
        if let Some(old) = records.iter().find(|old| old.record_id == input.record_id) {
            return if old == &input {
                Ok(())
            } else {
                Err(invalid("immutable reconciliation assessment conflict"))
            };
        }
        records.push(input);
        Ok(())
    }
    /// Derive counts and accuracy solely from joined records and reference labels.
    pub fn ingestion_metrics(&self) -> Result<IngestionMetrics, GraphError> {
        IngestionMetrics::from_stores(self.epistemic_stores())
    }
}
impl IngestionMetrics {
    /// Compute quality from governed metadata without loading canonical payloads.
    pub fn from_stores(stores: &EpistemicStores) -> Result<Self, GraphError> {
        let evaluations = &stores.ingestion_evaluations;
        evaluations.validate_bindings(&stores.candidates, &stores.reconciliations)?;
        let candidates: std::collections::HashMap<_, _> = evaluations
            .candidates
            .iter()
            .map(|a| (&a.candidate_id, a.correct))
            .collect();
        let decisions: std::collections::HashMap<_, _> = evaluations
            .reconciliations
            .iter()
            .map(|a| (&a.record_id, a.expected))
            .collect();
        let mut metrics = IngestionMetrics::default();
        for input in stores.candidates.records() {
            let correct = candidates.get(input.id()).copied();
            if let Some(repair) = input.repair() {
                metrics.repairs += 1;
                if let (Some(before), Some(after)) =
                    (candidates.get(&repair.predecessor).copied(), correct)
                {
                    metrics.reviewed_repairs += 1;
                    metrics.successful_repairs += u64::from(!before && after);
                    metrics.false_repairs += u64::from(before && !after);
                }
            } else {
                metrics.extraction.total += 1;
                if let Some(correct) = correct {
                    metrics.extraction.reviewed += 1;
                    metrics.extraction.correct += u64::from(correct);
                }
            }
        }
        for record in stores.reconciliations.records() {
            let index = match record.outcome() {
                ReconciliationOutcome::Merge => 0,
                ReconciliationOutcome::Distinct => 1,
                ReconciliationOutcome::Abstain => 2,
            };
            let counts = &mut metrics.reconciliation[index];
            counts.total += 1;
            if let Some(expected) = decisions.get(record.id()) {
                counts.reviewed += 1;
                counts.correct += u64::from(expected == &record.outcome());
            }
        }
        Ok(metrics)
    }
}

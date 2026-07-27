// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! Deterministic, reproducible evidence-table export.
//!
//! One row per included study. The export records its snapshot, transaction,
//! and exporter version; every row carries evidence references; and the same
//! input plus exporter version produces byte-identical canonical JSON. Strict
//! mode rejects rows missing required fields, permissive mode emits them with
//! explicit gap markers.

use serde::{Deserialize, Serialize};

use crate::StudyDesign;

/// Exporter version, pinned to the crate version so output is reproducible.
pub const EVIDENCE_TABLE_EXPORTER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Export completeness mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportMode {
    /// Reject any included study missing a required field.
    Strict,
    /// Emit incomplete rows with explicit gap markers.
    Permissive,
}

/// One included study, assembled by the host from the graph.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EvidenceTableStudy {
    /// Stable study identifier; also the deterministic sort key.
    pub study_id: String,
    /// Study design.
    pub design: Option<StudyDesign>,
    /// Population description.
    pub population: Option<String>,
    /// Intervention description.
    pub intervention: Option<String>,
    /// Comparator description.
    pub comparator: Option<String>,
    /// Outcome description.
    pub outcome: Option<String>,
    /// Effect estimate rendered as text.
    pub effect_estimate: Option<String>,
    /// Evidence references supporting inclusion.
    pub evidence_refs: Vec<String>,
}

/// One rendered evidence-table row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EvidenceTableRow {
    /// Study id.
    pub study_id: String,
    /// Design.
    pub design: Option<StudyDesign>,
    /// Population.
    pub population: Option<String>,
    /// Intervention.
    pub intervention: Option<String>,
    /// Comparator.
    pub comparator: Option<String>,
    /// Outcome.
    pub outcome: Option<String>,
    /// Effect estimate.
    pub effect_estimate: Option<String>,
    /// Evidence references.
    pub evidence_refs: Vec<String>,
    /// Names of required fields that were absent, empty when complete.
    pub gaps: Vec<String>,
}

/// A rendered evidence-table export.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EvidenceTableExport {
    /// Exporter version.
    pub exporter_version: String,
    /// Snapshot id.
    pub snapshot_id: String,
    /// Transaction id.
    pub transaction_id: String,
    /// Mode.
    pub mode: ExportMode,
    /// Rows, ordered deterministically by study id.
    pub rows: Vec<EvidenceTableRow>,
}

impl EvidenceTableExport {
    /// Serializes to canonical JSON. Identical inputs and exporter version
    /// produce byte-identical output.
    ///
    /// # Errors
    ///
    /// Returns [`ExportError::Serialization`] if serialization fails.
    pub fn to_canonical_json(&self) -> Result<String, ExportError> {
        serde_json::to_string(self).map_err(|_| ExportError::Serialization)
    }
}

/// Export failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExportError {
    /// Strict mode found studies missing required fields.
    IncompleteStudies(Vec<IncompleteStudy>),
    /// Serialization failed.
    Serialization,
}

impl core::fmt::Display for ExportError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::IncompleteStudies(studies) => {
                write!(
                    formatter,
                    "{} incomplete studies in strict mode",
                    studies.len()
                )
            }
            Self::Serialization => write!(formatter, "evidence table serialization failed"),
        }
    }
}

impl std::error::Error for ExportError {}

/// A study rejected by strict mode, with the fields it was missing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IncompleteStudy {
    /// Study id.
    pub study_id: String,
    /// Missing fields.
    pub missing_fields: Vec<String>,
}

/// Deterministic evidence-table exporter.
#[derive(Clone, Debug)]
pub struct EvidenceTableExporter {
    snapshot_id: String,
    transaction_id: String,
}

impl EvidenceTableExporter {
    /// Creates an exporter bound to a snapshot and transaction.
    #[must_use]
    pub fn new(snapshot_id: impl Into<String>, transaction_id: impl Into<String>) -> Self {
        Self {
            snapshot_id: snapshot_id.into(),
            transaction_id: transaction_id.into(),
        }
    }

    /// Exports the studies as an evidence table.
    ///
    /// Studies are sorted by `study_id` so output does not depend on input
    /// order. In strict mode, a study missing a required field aborts the
    /// export; in permissive mode, the missing fields are recorded as gaps.
    ///
    /// # Errors
    ///
    /// In [`ExportMode::Strict`], returns [`ExportError::IncompleteStudies`]
    /// listing every study missing a required field.
    pub fn export(
        &self,
        studies: &[EvidenceTableStudy],
        mode: ExportMode,
    ) -> Result<EvidenceTableExport, ExportError> {
        let mut ordered: Vec<&EvidenceTableStudy> = studies.iter().collect();
        ordered.sort_by(|left, right| left.study_id.cmp(&right.study_id));

        if mode == ExportMode::Strict {
            let incomplete: Vec<IncompleteStudy> = ordered
                .iter()
                .filter_map(|study| {
                    let missing = missing_required_fields(study);
                    if missing.is_empty() {
                        None
                    } else {
                        Some(IncompleteStudy {
                            study_id: study.study_id.clone(),
                            missing_fields: missing,
                        })
                    }
                })
                .collect();
            if !incomplete.is_empty() {
                return Err(ExportError::IncompleteStudies(incomplete));
            }
        }

        let rows = ordered
            .into_iter()
            .map(|study| EvidenceTableRow {
                study_id: study.study_id.clone(),
                design: study.design,
                population: study.population.clone(),
                intervention: study.intervention.clone(),
                comparator: study.comparator.clone(),
                outcome: study.outcome.clone(),
                effect_estimate: study.effect_estimate.clone(),
                evidence_refs: study.evidence_refs.clone(),
                gaps: missing_required_fields(study),
            })
            .collect();

        Ok(EvidenceTableExport {
            exporter_version: EVIDENCE_TABLE_EXPORTER_VERSION.to_owned(),
            snapshot_id: self.snapshot_id.clone(),
            transaction_id: self.transaction_id.clone(),
            mode,
            rows,
        })
    }
}

/// Required fields for an evidence-table row. Evidence references are always
/// required; a row without evidence is never exportable.
fn missing_required_fields(study: &EvidenceTableStudy) -> Vec<String> {
    let mut missing = Vec::new();
    if study.design.is_none() {
        missing.push("design".to_owned());
    }
    if study.population.is_none() {
        missing.push("population".to_owned());
    }
    if study.intervention.is_none() {
        missing.push("intervention".to_owned());
    }
    if study.outcome.is_none() {
        missing.push("outcome".to_owned());
    }
    if study.evidence_refs.is_empty() {
        missing.push("evidence_refs".to_owned());
    }
    missing
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn complete_study(id: &str) -> EvidenceTableStudy {
        EvidenceTableStudy {
            study_id: id.to_owned(),
            design: Some(StudyDesign::RandomizedControlledTrial),
            population: Some("adults with condition X".to_owned()),
            intervention: Some("drug A".to_owned()),
            comparator: Some("placebo".to_owned()),
            outcome: Some("mortality at 12 months".to_owned()),
            effect_estimate: Some("RR 0.82 (0.70-0.96)".to_owned()),
            evidence_refs: vec!["evidence--1".to_owned()],
        }
    }

    #[test]
    fn export_is_deterministic_regardless_of_input_order() {
        let exporter = EvidenceTableExporter::new("snapshot--1", "txn--1");
        let forward = [complete_study("study--a"), complete_study("study--b")];
        let reversed = [complete_study("study--b"), complete_study("study--a")];

        let a = exporter
            .export(&forward, ExportMode::Strict)
            .unwrap()
            .to_canonical_json()
            .unwrap();
        let b = exporter
            .export(&reversed, ExportMode::Strict)
            .unwrap()
            .to_canonical_json()
            .unwrap();

        assert_eq!(a, b, "export must not depend on input order");
    }

    #[test]
    fn export_records_provenance_and_evidence() {
        let exporter = EvidenceTableExporter::new("snapshot--7", "txn--7");
        let export = exporter
            .export(&[complete_study("study--a")], ExportMode::Strict)
            .unwrap();

        assert_eq!(export.exporter_version, EVIDENCE_TABLE_EXPORTER_VERSION);
        assert_eq!(export.snapshot_id, "snapshot--7");
        assert_eq!(export.transaction_id, "txn--7");
        assert_eq!(export.rows[0].evidence_refs, vec!["evidence--1".to_owned()]);
    }

    #[test]
    fn strict_mode_rejects_incomplete_studies_with_named_fields() {
        let exporter = EvidenceTableExporter::new("snapshot--1", "txn--1");
        let mut incomplete = complete_study("study--gap");
        incomplete.outcome = None;
        incomplete.evidence_refs.clear();

        let error = exporter
            .export(&[incomplete], ExportMode::Strict)
            .unwrap_err();
        match error {
            ExportError::IncompleteStudies(studies) => {
                assert_eq!(studies[0].study_id, "study--gap");
                assert!(studies[0].missing_fields.contains(&"outcome".to_owned()));
                assert!(
                    studies[0]
                        .missing_fields
                        .contains(&"evidence_refs".to_owned())
                );
            }
            other => panic!("expected incomplete studies, got {other:?}"),
        }
    }

    #[test]
    fn permissive_mode_emits_gap_markers() {
        let exporter = EvidenceTableExporter::new("snapshot--1", "txn--1");
        let mut incomplete = complete_study("study--gap");
        incomplete.comparator = None;
        incomplete.outcome = None;

        let export = exporter
            .export(&[incomplete], ExportMode::Permissive)
            .unwrap();
        assert!(export.rows[0].gaps.contains(&"outcome".to_owned()));
        // Comparator is not a required field, so it is not a gap.
        assert!(!export.rows[0].gaps.contains(&"comparator".to_owned()));
    }
}

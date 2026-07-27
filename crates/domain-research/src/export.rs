// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! Deterministic, reproducible bibliography export with provenance.
//!
//! One row per included work, carrying the evidence references that justified
//! inclusion. The export records its snapshot, transaction, and exporter
//! version, and the same input plus exporter version produces byte-identical
//! canonical JSON. Strict mode rejects entries missing required fields;
//! permissive mode emits them with explicit gap markers.

use serde::{Deserialize, Serialize};

/// Exporter version, pinned to the crate version so output is reproducible.
pub const BIBLIOGRAPHY_EXPORTER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Export completeness mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportMode {
    /// Reject any entry missing a required field.
    Strict,
    /// Emit incomplete entries with explicit gap markers.
    Permissive,
}

/// One included work, assembled by the host from the graph.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BibliographyEntry {
    /// Stable work identifier; also the deterministic sort key.
    pub work_id: String,
    /// Title.
    pub title: Option<String>,
    /// Publication year.
    pub year: Option<i32>,
    /// Venue.
    pub venue: Option<String>,
    /// Normalized scholarly identifiers, rendered as `system:value`.
    pub identifiers: Vec<String>,
    /// Evidence references justifying inclusion.
    pub evidence_refs: Vec<String>,
}

/// One rendered bibliography row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BibliographyRow {
    /// Work id.
    pub work_id: String,
    /// Title.
    pub title: Option<String>,
    /// Year.
    pub year: Option<i32>,
    /// Venue.
    pub venue: Option<String>,
    /// Identifiers, sorted for stable output.
    pub identifiers: Vec<String>,
    /// Evidence references.
    pub evidence_refs: Vec<String>,
    /// Names of required fields that were absent, empty when complete.
    pub gaps: Vec<String>,
}

/// A rendered bibliography export.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BibliographyExport {
    /// Exporter version.
    pub exporter_version: String,
    /// Snapshot id.
    pub snapshot_id: String,
    /// Transaction id.
    pub transaction_id: String,
    /// Mode.
    pub mode: ExportMode,
    /// Rows, ordered deterministically by work id.
    pub rows: Vec<BibliographyRow>,
}

impl BibliographyExport {
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

/// An entry rejected by strict mode, with the fields it was missing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IncompleteEntry {
    /// Work id.
    pub work_id: String,
    /// Missing fields.
    pub missing_fields: Vec<String>,
}

/// Export failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExportError {
    /// Strict mode found entries missing required fields.
    IncompleteEntries(Vec<IncompleteEntry>),
    /// Serialization failed.
    Serialization,
}

impl core::fmt::Display for ExportError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::IncompleteEntries(entries) => {
                write!(
                    formatter,
                    "{} incomplete bibliography entries in strict mode",
                    entries.len()
                )
            }
            Self::Serialization => write!(formatter, "bibliography serialization failed"),
        }
    }
}

impl std::error::Error for ExportError {}

/// Deterministic bibliography exporter.
#[derive(Clone, Debug)]
pub struct BibliographyExporter {
    snapshot_id: String,
    transaction_id: String,
}

impl BibliographyExporter {
    /// Creates an exporter bound to a snapshot and transaction.
    #[must_use]
    pub fn new(snapshot_id: impl Into<String>, transaction_id: impl Into<String>) -> Self {
        Self {
            snapshot_id: snapshot_id.into(),
            transaction_id: transaction_id.into(),
        }
    }

    /// Exports the entries as a bibliography.
    ///
    /// Entries are sorted by `work_id` and identifiers within an entry are
    /// sorted, so output does not depend on input order.
    ///
    /// # Errors
    ///
    /// In [`ExportMode::Strict`], returns [`ExportError::IncompleteEntries`]
    /// listing every entry missing a required field.
    pub fn export(
        &self,
        entries: &[BibliographyEntry],
        mode: ExportMode,
    ) -> Result<BibliographyExport, ExportError> {
        let mut ordered: Vec<&BibliographyEntry> = entries.iter().collect();
        ordered.sort_by(|left, right| left.work_id.cmp(&right.work_id));

        if mode == ExportMode::Strict {
            let incomplete: Vec<IncompleteEntry> = ordered
                .iter()
                .filter_map(|entry| {
                    let missing = missing_required_fields(entry);
                    if missing.is_empty() {
                        None
                    } else {
                        Some(IncompleteEntry {
                            work_id: entry.work_id.clone(),
                            missing_fields: missing,
                        })
                    }
                })
                .collect();
            if !incomplete.is_empty() {
                return Err(ExportError::IncompleteEntries(incomplete));
            }
        }

        let rows = ordered
            .into_iter()
            .map(|entry| {
                let mut identifiers = entry.identifiers.clone();
                identifiers.sort();
                BibliographyRow {
                    work_id: entry.work_id.clone(),
                    title: entry.title.clone(),
                    year: entry.year,
                    venue: entry.venue.clone(),
                    identifiers,
                    evidence_refs: entry.evidence_refs.clone(),
                    gaps: missing_required_fields(entry),
                }
            })
            .collect();

        Ok(BibliographyExport {
            exporter_version: BIBLIOGRAPHY_EXPORTER_VERSION.to_owned(),
            snapshot_id: self.snapshot_id.clone(),
            transaction_id: self.transaction_id.clone(),
            mode,
            rows,
        })
    }
}

/// Required fields for a bibliography row. Evidence references are always
/// required; an entry without evidence is never exportable.
fn missing_required_fields(entry: &BibliographyEntry) -> Vec<String> {
    let mut missing = Vec::new();
    if entry.title.as_ref().is_none_or(|t| t.trim().is_empty()) {
        missing.push("title".to_owned());
    }
    if entry.identifiers.is_empty() {
        missing.push("identifiers".to_owned());
    }
    if entry.evidence_refs.is_empty() {
        missing.push("evidence_refs".to_owned());
    }
    missing
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn complete_entry(id: &str) -> BibliographyEntry {
        BibliographyEntry {
            work_id: id.to_owned(),
            title: Some("A study of things".to_owned()),
            year: Some(2026),
            venue: Some("Journal of Things".to_owned()),
            identifiers: vec!["doi:10.1000/abc".to_owned()],
            evidence_refs: vec!["evidence--1".to_owned()],
        }
    }

    #[test]
    fn export_is_deterministic_regardless_of_input_order() {
        let exporter = BibliographyExporter::new("snapshot--1", "txn--1");
        let forward = [complete_entry("work--a"), complete_entry("work--b")];
        let reversed = [complete_entry("work--b"), complete_entry("work--a")];

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
        assert_eq!(a, b);
    }

    #[test]
    fn identifiers_are_sorted_within_an_entry() {
        let exporter = BibliographyExporter::new("snapshot--1", "txn--1");
        let mut entry = complete_entry("work--a");
        entry.identifiers = vec![
            "pubmed:24239612".to_owned(),
            "doi:10.1000/abc".to_owned(),
            "arxiv:2601.01234".to_owned(),
        ];
        let export = exporter.export(&[entry], ExportMode::Strict).unwrap();
        assert_eq!(
            export.rows[0].identifiers,
            vec![
                "arxiv:2601.01234".to_owned(),
                "doi:10.1000/abc".to_owned(),
                "pubmed:24239612".to_owned()
            ]
        );
    }

    #[test]
    fn export_records_provenance_and_evidence() {
        let exporter = BibliographyExporter::new("snapshot--9", "txn--9");
        let export = exporter
            .export(&[complete_entry("work--a")], ExportMode::Strict)
            .unwrap();
        assert_eq!(export.exporter_version, BIBLIOGRAPHY_EXPORTER_VERSION);
        assert_eq!(export.snapshot_id, "snapshot--9");
        assert_eq!(export.transaction_id, "txn--9");
        assert_eq!(export.rows[0].evidence_refs, vec!["evidence--1".to_owned()]);
    }

    #[test]
    fn strict_mode_rejects_entries_without_evidence() {
        let exporter = BibliographyExporter::new("snapshot--1", "txn--1");
        let mut entry = complete_entry("work--gap");
        entry.evidence_refs.clear();

        let error = exporter.export(&[entry], ExportMode::Strict).unwrap_err();
        match error {
            ExportError::IncompleteEntries(entries) => {
                assert_eq!(entries[0].work_id, "work--gap");
                assert!(
                    entries[0]
                        .missing_fields
                        .contains(&"evidence_refs".to_owned())
                );
            }
            other => panic!("expected incomplete entries, got {other:?}"),
        }
    }

    #[test]
    fn permissive_mode_emits_gap_markers() {
        let exporter = BibliographyExporter::new("snapshot--1", "txn--1");
        let mut entry = complete_entry("work--gap");
        entry.title = None;
        entry.venue = None;

        let export = exporter.export(&[entry], ExportMode::Permissive).unwrap();
        assert!(export.rows[0].gaps.contains(&"title".to_owned()));
        // Venue is not required, so its absence is not a gap.
        assert!(!export.rows[0].gaps.contains(&"venue".to_owned()));
    }
}

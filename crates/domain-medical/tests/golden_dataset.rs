// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
#![allow(clippy::unwrap_used)]

//! Golden dataset: a fixed corpus exports to a committed, byte-stable evidence
//! table. Regenerate the fixture with `UPDATE_GOLDEN=1 cargo test -p
//! domain-medical --test golden_dataset`.

use domain_medical::{EvidenceTableExporter, EvidenceTableStudy, ExportMode, StudyDesign};

const GOLDEN_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/evidence_table.golden.json"
);

fn corpus() -> Vec<EvidenceTableStudy> {
    // Intentionally out of id order to exercise deterministic sorting.
    vec![
        EvidenceTableStudy {
            study_id: "study--nct-0002".to_owned(),
            design: Some(StudyDesign::CohortStudy),
            population: Some("adults over 65 with condition Y".to_owned()),
            intervention: Some("drug B".to_owned()),
            comparator: Some("standard care".to_owned()),
            outcome: Some("hospitalization at 6 months".to_owned()),
            effect_estimate: Some("HR 0.91 (0.80-1.03)".to_owned()),
            evidence_refs: vec!["evidence--nct-0002-1".to_owned()],
        },
        EvidenceTableStudy {
            study_id: "study--nct-0001".to_owned(),
            design: Some(StudyDesign::RandomizedControlledTrial),
            population: Some("adults with condition X".to_owned()),
            intervention: Some("drug A".to_owned()),
            comparator: Some("placebo".to_owned()),
            outcome: Some("mortality at 12 months".to_owned()),
            effect_estimate: Some("RR 0.82 (0.70-0.96)".to_owned()),
            evidence_refs: vec![
                "evidence--nct-0001-1".to_owned(),
                "evidence--nct-0001-2".to_owned(),
            ],
        },
        EvidenceTableStudy {
            study_id: "study--sr-0003".to_owned(),
            design: Some(StudyDesign::SystematicReview),
            population: Some("mixed adult populations".to_owned()),
            intervention: Some("drug A".to_owned()),
            comparator: Some("placebo".to_owned()),
            outcome: Some("mortality at 12 months".to_owned()),
            effect_estimate: Some("pooled RR 0.85 (0.78-0.93)".to_owned()),
            evidence_refs: vec!["evidence--sr-0003-1".to_owned()],
        },
    ]
}

#[test]
fn golden_evidence_table_is_stable() {
    let exporter = EvidenceTableExporter::new("snapshot--golden", "txn--golden");
    let export = exporter.export(&corpus(), ExportMode::Strict).unwrap();
    let actual = export.to_canonical_json().unwrap();

    if std::env::var("UPDATE_GOLDEN").is_ok() {
        std::fs::write(GOLDEN_PATH, &actual).unwrap();
    }

    let expected = std::fs::read_to_string(GOLDEN_PATH)
        .expect("golden fixture must exist; run UPDATE_GOLDEN=1");
    assert_eq!(
        actual, expected,
        "evidence-table export drifted from the golden fixture"
    );

    // Byte-identical across repeated exports of the same corpus.
    let again = exporter
        .export(&corpus(), ExportMode::Strict)
        .unwrap()
        .to_canonical_json()
        .unwrap();
    assert_eq!(actual, again);
}

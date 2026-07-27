// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
#![allow(clippy::unwrap_used)]

//! Golden dataset: a fixed corpus exports to a committed, byte-stable
//! bibliography. Regenerate with `UPDATE_GOLDEN=1 cargo test -p domain-research
//! --test golden_dataset`.

use domain_research::{
    BibliographyEntry, BibliographyExporter, ExportMode, IdentifierSystem,
    research_identifier_normalize,
};

const GOLDEN_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/bibliography.golden.json"
);

/// Renders a normalized identifier as `system:value`, failing the test rather
/// than silently emitting an unnormalized string.
fn identifier(system: IdentifierSystem, raw: &str) -> String {
    let normalized = research_identifier_normalize(system, raw)
        .unwrap_or_else(|| panic!("fixture identifier {raw} must normalize"));
    format!("{}:{normalized}", system.as_str())
}

fn corpus() -> Vec<BibliographyEntry> {
    // Intentionally out of id order to exercise deterministic sorting, and with
    // identifiers supplied in messy real-world forms.
    vec![
        BibliographyEntry {
            work_id: "work--0002".to_owned(),
            title: Some("Replication of an earlier effect".to_owned()),
            year: Some(2025),
            venue: Some("Journal of Replication".to_owned()),
            identifiers: vec![
                identifier(IdentifierSystem::Doi, "https://doi.org/10.1000/REPL.2"),
                identifier(IdentifierSystem::Orcid, "0000-0002-1825-0097"),
            ],
            evidence_refs: vec!["evidence--0002-1".to_owned()],
        },
        BibliographyEntry {
            work_id: "work--0001".to_owned(),
            title: Some("An original finding".to_owned()),
            year: Some(2024),
            venue: Some("Journal of Things".to_owned()),
            identifiers: vec![
                identifier(IdentifierSystem::PubMed, "PMID:24239612"),
                identifier(IdentifierSystem::Doi, "doi:10.1000/orig.1"),
            ],
            evidence_refs: vec!["evidence--0001-1".to_owned(), "evidence--0001-2".to_owned()],
        },
        BibliographyEntry {
            work_id: "work--0003".to_owned(),
            title: Some("A preprint extending the method".to_owned()),
            year: Some(2026),
            venue: None,
            identifiers: vec![identifier(IdentifierSystem::ArXiv, "arXiv:2601.01234v2")],
            evidence_refs: vec!["evidence--0003-1".to_owned()],
        },
    ]
}

#[test]
fn golden_bibliography_is_stable() {
    let exporter = BibliographyExporter::new("snapshot--golden", "txn--golden");
    let export = exporter.export(&corpus(), ExportMode::Strict).unwrap();
    let actual = export.to_canonical_json().unwrap();

    if std::env::var("UPDATE_GOLDEN").is_ok() {
        std::fs::write(GOLDEN_PATH, &actual).unwrap();
    }

    let expected = std::fs::read_to_string(GOLDEN_PATH)
        .expect("golden fixture must exist; run UPDATE_GOLDEN=1");
    assert_eq!(
        actual, expected,
        "bibliography export drifted from the golden fixture"
    );

    // Byte-identical across repeated exports of the same corpus.
    let again = exporter
        .export(&corpus(), ExportMode::Strict)
        .unwrap()
        .to_canonical_json()
        .unwrap();
    assert_eq!(actual, again);
}

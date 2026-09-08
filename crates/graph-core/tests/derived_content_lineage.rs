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
//! Contract for derived content lineage (issue #280, ADR-0021).
//!
//! A `CharacterSpan` addresses character offsets, but almost never into the
//! original artifact: it addresses an extractor's output. Retaining the
//! artifact proves what was ingested; retaining the derived content proves what
//! was *read*.
//!
//! Without it, re-running a different extractor version leaves the same offsets
//! silently addressing different text, and the evidence still looks valid. That
//! is the failure this contract exists to make impossible.
use graph_core::{
    ContentStore, DerivedContent, DerivedContentKind, DerivedContentStore, EvidenceLocator,
    ExtractorIdentity, MemoryObjectStore, ObjectContentStore, SpanResolutionError,
    resolve_derived_span,
};

const ARTIFACT: &[u8] = b"%PDF-1.7 vendor report bytes";
const EXTRACTED: &str = "Aster operates the North Relay since 2026.";
const REEXTRACTED: &str = "ASTER OPERATES THE NORTH RELAY SINCE 2026.";

fn content_store() -> ObjectContentStore<MemoryObjectStore> {
    ObjectContentStore::new("memory", MemoryObjectStore::default())
}

fn docling() -> ExtractorIdentity {
    ExtractorIdentity::new("docling", "2.1.0").expect("extractor identity")
}

/// Retain an artifact and one derived text, returning the lineage record.
fn derive(
    content: &mut ObjectContentStore<MemoryObjectStore>,
    text: &str,
    extractor: ExtractorIdentity,
) -> DerivedContent {
    let artifact = content
        .store(ARTIFACT, Some("application/pdf"))
        .expect("artifact");
    let derived = content
        .store(text.as_bytes(), Some("text/plain"))
        .expect("derived");
    DerivedContent::new(
        artifact,
        derived,
        extractor,
        DerivedContentKind::ExtractedText,
    )
}

#[test]
fn derived_content_records_what_produced_it_and_from_what() {
    let mut content = content_store();
    let derived = derive(&mut content, EXTRACTED, docling());

    assert_eq!(derived.extractor().id(), "docling");
    assert_eq!(derived.extractor().version(), "2.1.0");
    assert_eq!(derived.kind(), DerivedContentKind::ExtractedText);
    // The chain from bytes to span must be reproducible without the extractor
    // itself being available.
    assert_eq!(
        derived.source_artifact().byte_length(),
        ARTIFACT.len() as u64
    );
    assert_eq!(derived.content().byte_length(), EXTRACTED.len() as u64);
}

#[test]
fn the_same_artifact_and_extractor_yield_the_same_identity() {
    let mut content = content_store();

    let first = derive(&mut content, EXTRACTED, docling());
    let second = derive(&mut content, EXTRACTED, docling());

    assert_eq!(first.id(), second.id());
}

#[test]
fn a_different_extractor_version_yields_a_different_identity() {
    let mut content = content_store();

    let old = derive(&mut content, EXTRACTED, docling());
    let new = derive(
        &mut content,
        EXTRACTED,
        ExtractorIdentity::new("docling", "3.0.0").expect("identity"),
    );

    // Identical output from a different extractor is not the same provenance.
    assert_ne!(old.id(), new.id());
}

#[test]
fn a_different_extraction_yields_a_different_identity() {
    let mut content = content_store();

    let first = derive(&mut content, EXTRACTED, docling());
    let second = derive(&mut content, REEXTRACTED, docling());

    assert_ne!(first.id(), second.id());
}

#[test]
fn an_extractor_without_an_identity_or_version_is_refused() {
    // An unidentified extractor makes its output unreproducible.
    assert!(ExtractorIdentity::new("", "2.1.0").is_err());
    assert!(ExtractorIdentity::new("docling", "").is_err());
}

#[test]
fn a_span_resolves_to_the_text_it_addressed() {
    let mut content = content_store();
    let derived = derive(&mut content, EXTRACTED, docling());

    let span = EvidenceLocator::ByteRange { start: 0, end: 5 };
    let resolved = resolve_derived_span(&derived, &span, &content).expect("resolve");

    assert_eq!(resolved, b"Aster");
}

#[test]
fn a_span_resolved_against_different_derived_content_fails() {
    let mut content = content_store();
    let original = derive(&mut content, EXTRACTED, docling());
    let reextracted = derive(&mut content, REEXTRACTED, docling());

    let span = EvidenceLocator::ByteRange { start: 0, end: 5 };
    let against_original = resolve_derived_span(&original, &span, &content).expect("resolve");
    let against_other = resolve_derived_span(&reextracted, &span, &content).expect("resolve");

    // The same offsets over different derived content are different text. The
    // binding is what makes one of them wrong rather than both plausible.
    assert_ne!(against_original, against_other);
    assert_ne!(original.id(), reextracted.id());
}

#[test]
fn a_span_beyond_the_derived_content_fails_rather_than_truncating() {
    let mut content = content_store();
    let derived = derive(&mut content, EXTRACTED, docling());

    let span = EvidenceLocator::ByteRange {
        start: 0,
        end: EXTRACTED.len() as u64 + 100,
    };

    assert!(matches!(
        resolve_derived_span(&derived, &span, &content),
        Err(SpanResolutionError::OutOfBounds { .. })
    ));
}

#[test]
fn a_character_span_addresses_characters_not_bytes() {
    let mut content = content_store();
    let text = "éé Aster";
    let derived = derive(&mut content, text, docling());

    let span = EvidenceLocator::CharacterSpan { start: 0, end: 2 };
    let resolved = resolve_derived_span(&derived, &span, &content).expect("resolve");

    // Two characters, four bytes. Treating the offsets as bytes would split a
    // character and silently return different text.
    assert_eq!(String::from_utf8(resolved).expect("utf8"), "éé");
}

#[test]
fn a_structural_selector_is_refused_rather_than_guessed() {
    let mut content = content_store();
    let derived = derive(&mut content, EXTRACTED, docling());

    // A page or table cell is not an offset into the derived bytes, and
    // guessing one would fabricate a location.
    for selector in [
        EvidenceLocator::Page { page: 3 },
        EvidenceLocator::Paragraph {
            page: Some(1),
            paragraph: 2,
        },
        EvidenceLocator::RecordPath {
            path: "/objects/0".to_owned(),
        },
    ] {
        assert!(matches!(
            resolve_derived_span(&derived, &selector, &content),
            Err(SpanResolutionError::NotAddressable { .. })
        ));
    }
}

#[test]
fn resolving_a_span_whose_content_was_erased_says_so() {
    let mut content = content_store();
    let derived = derive(&mut content, EXTRACTED, docling());
    content
        .erase(derived.content(), "subject request")
        .expect("erase");

    let span = EvidenceLocator::ByteRange { start: 0, end: 5 };

    // Optimistically returning nothing would read as an absence of evidence.
    assert!(matches!(
        resolve_derived_span(&derived, &span, &content),
        Err(SpanResolutionError::Unavailable(_))
    ));
}

#[test]
fn lineage_is_queryable_without_transferring_content() {
    let mut content = content_store();
    let derived = derive(&mut content, EXTRACTED, docling());
    let mut lineage = DerivedContentStore::default();
    lineage.record(derived.clone()).expect("record");

    let found = lineage.by_id(derived.id()).expect("lineage");

    assert_eq!(found.extractor().id(), "docling");
    assert_eq!(
        found.source_artifact().content_id(),
        derived.source_artifact().content_id()
    );
}

#[test]
fn recording_the_same_derivation_twice_is_idempotent() {
    let mut content = content_store();
    let derived = derive(&mut content, EXTRACTED, docling());
    let mut lineage = DerivedContentStore::default();

    lineage.record(derived.clone()).expect("record");
    lineage.record(derived.clone()).expect("record again");

    assert_eq!(lineage.len(), 1);
}

#[test]
fn a_derivation_that_contradicts_a_recorded_one_is_refused() {
    let mut content = content_store();
    let derived = derive(&mut content, EXTRACTED, docling());
    let mut lineage = DerivedContentStore::default();
    lineage.record(derived.clone()).expect("record");

    // The identity is derived from the provenance, so a record claiming the
    // same identity for other content would rewrite history.
    let forged = DerivedContent::rebind_for_test(
        derived.id().to_owned(),
        content
            .store(REEXTRACTED.as_bytes(), Some("text/plain"))
            .expect("other"),
    );

    assert!(lineage.record(forged).is_err());
}

#[test]
fn an_observation_without_retained_derived_content_is_a_distinguishable_state() {
    let mut lineage = DerivedContentStore::default();
    let mut content = content_store();
    let derived = derive(&mut content, EXTRACTED, docling());

    // Nothing recorded: the lineage says it does not know, rather than
    // resolving optimistically against whatever is available now.
    assert!(lineage.by_id(derived.id()).is_none());
    lineage.record(derived.clone()).expect("record");
    assert!(lineage.by_id(derived.id()).is_some());
}

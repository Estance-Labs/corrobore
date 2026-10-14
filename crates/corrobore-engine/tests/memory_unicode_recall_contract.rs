// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//! Contract for Unicode-aware, accent-insensitive recall matching (#313).
//!
//! Objective terms and record text are folded with Unicode lowercase, NFKD
//! decomposition, and removal of combining marks. Punctuation is trimmed from
//! each term, empty terms are ignored, and a repeated term counts once.
#![allow(clippy::unwrap_used)]

use corrobore_engine::{
    CorroboreEngine, MemoryContent, MemoryLimits, MemoryOperation, MemoryPermissions,
    MemoryRequest, MemoryResponse, MemoryServiceContext, RecallItem, RecallRequest,
    RememberRequest,
};

fn context() -> MemoryServiceContext {
    MemoryServiceContext::new(
        "workspace--unicode",
        "actor--unicode",
        None,
        "session--unicode",
        MemoryPermissions::all(),
        "request--unicode",
        "correlation--unicode",
    )
    .unwrap()
}

fn remember(engine: &mut CorroboreEngine, key: &str, text: &str) -> String {
    let request = MemoryRequest::new(MemoryOperation::Remember(RememberRequest {
        identity_key: Some(key.to_owned()),
        kind: "fact".to_owned(),
        schema_version: "1".to_owned(),
        content: MemoryContent::Text(text.to_owned()),
        provenance: vec![],
        confidence: None,
        valid_from: None,
        valid_until: None,
        expires_at: None,
        tags: vec![],
        durable: None,
    }))
    .with_idempotency_key(format!("remember:{key}"));
    match engine.execute_memory(&context(), &request).unwrap() {
        MemoryResponse::Remember { record, .. } => record.id,
        other => panic!("expected remember, got {other:?}"),
    }
}

fn recall(engine: &mut CorroboreEngine, objective: &str) -> Vec<RecallItem> {
    match engine
        .execute_memory(
            &context(),
            &MemoryRequest::new(MemoryOperation::Recall(RecallRequest {
                objective: objective.to_owned(),
                seed_ids: vec![],
                limits: MemoryLimits::strict_default(),
                page_token: None,
            })),
        )
        .unwrap()
    {
        MemoryResponse::Recall(result) => result.items,
        other => panic!("expected recall, got {other:?}"),
    }
}

fn ids(items: &[RecallItem]) -> Vec<&str> {
    items.iter().map(|item| item.record.id.as_str()).collect()
}

#[test]
fn matching_is_case_folded_beyond_ascii() {
    let mut engine = CorroboreEngine::strict_default();
    let beer = remember(&mut engine, "fact:beer", "Romain aime la bière belge");
    assert_eq!(ids(&recall(&mut engine, "Bière")), vec![beer.as_str()]);
    assert_eq!(ids(&recall(&mut engine, "BIÈRE")), vec![beer.as_str()]);
}

#[test]
fn matching_is_accent_insensitive_in_both_directions() {
    let mut engine = CorroboreEngine::strict_default();
    let accented = remember(&mut engine, "fact:accented", "L'élimination du boss");
    let plain = remember(&mut engine, "fact:plain", "elimination round");
    let items = recall(&mut engine, "elimination");
    let mut found = ids(&items);
    found.sort_unstable();
    let mut expected = vec![accented.as_str(), plain.as_str()];
    expected.sort_unstable();
    assert_eq!(found, expected);
    let items = recall(&mut engine, "Élimination");
    let mut found = ids(&items);
    found.sort_unstable();
    assert_eq!(found, expected);
}

#[test]
fn compatibility_forms_decompose() {
    let mut engine = CorroboreEngine::strict_default();
    // U+FB01 LATIN SMALL LIGATURE FI and fullwidth letters fold under NFKD.
    let ligature = remember(&mut engine, "fact:ligature", "ﬁnance report");
    assert_eq!(
        ids(&recall(&mut engine, "FINANCE")),
        vec![ligature.as_str()]
    );
    let fullwidth = remember(&mut engine, "fact:fullwidth", "ＡＢＣ corp");
    assert_eq!(ids(&recall(&mut engine, "abc")), vec![fullwidth.as_str()]);
}

#[test]
fn punctuation_is_trimmed_and_empty_or_repeated_terms_are_ignored() {
    let mut engine = CorroboreEngine::strict_default();
    let beer = remember(&mut engine, "fact:beer", "bière");
    let other = remember(&mut engine, "fact:other", "a - b ? c");
    // A punctuation-only term would match every record containing it.
    assert_eq!(
        ids(&recall(&mut engine, "bière ? - !")),
        vec![beer.as_str()]
    );
    // Punctuation glued to a word does not prevent the match.
    assert_eq!(
        ids(&recall(&mut engine, "« bière ? »")),
        vec![beer.as_str()]
    );
    // A repeated term counts once in the score.
    let once = recall(&mut engine, "bière");
    let twice = recall(&mut engine, "bière bière");
    assert!((once[0].score - twice[0].score).abs() < f64::EPSILON);
    // An objective with no usable term selects nothing lexically.
    assert!(recall(&mut engine, "?!").is_empty());
    let _ = other;
}

// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//! The deterministic bilingual template compiler (epic #82, item #260).
//!
//! The compiler is the baseline and the fallback: it covers the recurring
//! request shapes in French and English without a model, and it is held to
//! the same envelope contract a model's output is. Equivalent French and
//! English requests must compile to the same canonical action, differing only
//! in the language tag; requests it does not understand must say so with a
//! typed result rather than guess.
#![allow(clippy::unwrap_used)]

use corrobore_nlq::{
    ActionKind, Language, NlqCompiler, NlqRequest, TemplateCompiler, TrustBoundary, validate,
};

fn compile(text: &str, allow_writes: bool, evidence: &[&str]) -> corrobore_nlq::ValidatedEnvelope {
    let compiler = TemplateCompiler::default();
    let request = NlqRequest::new(text)
        .with_evidence_refs(evidence.iter().map(|reference| (*reference).to_owned()))
        .allow_writes(allow_writes);
    let envelope = compiler.compile(&request);
    let boundary = TrustBoundary::new(evidence.iter().copied(), allow_writes);
    validate(&serde_json::to_value(&envelope).unwrap(), &boundary).unwrap_or_else(|error| {
        panic!("{text}: the compiler must emit a valid envelope, got {error:?}")
    })
}

#[test]
fn equivalent_french_and_english_requests_compile_to_one_canonical_read() {
    let pairs = [
        (
            "Quels acteurs de menace utilisent le malware X-Agent ?",
            "Which threat actors use the malware X-Agent?",
            "MATCH (a:ThreatActor)-[r:USES]->(m:Malware) WHERE m.name = 'X-Agent' RETURN a.name LIMIT 50",
        ),
        (
            "Liste les indicateurs",
            "List the indicators",
            "MATCH (n:Indicator) RETURN n LIMIT 50",
        ),
        (
            "Montre les 20 premiers acteurs de menace",
            "Show the first 20 threat actors",
            "MATCH (n:ThreatActor) RETURN n LIMIT 20",
        ),
        (
            "Combien de campagnes ?",
            "How many campaigns?",
            "MATCH (n:Campaign) RETURN count(n)",
        ),
        (
            "Quelles campagnes ciblent l'identité \"Ministry of Energy\" ?",
            "Which campaigns target the identity \"Ministry of Energy\"?",
            "MATCH (a:Campaign)-[r:TARGETS]->(m:Identity) WHERE m.name = 'Ministry of Energy' RETURN a.name LIMIT 50",
        ),
    ];
    for (french, english, canonical) in pairs {
        let fr = compile(french, false, &[]);
        let en = compile(english, false, &[]);
        assert_eq!(fr.kind(), ActionKind::CypherRead, "{french}");
        assert_eq!(fr.canonical(), canonical, "{french}");
        assert_eq!(en.canonical(), canonical, "{english}");
        assert_eq!(fr.language(), &Language::Fr, "{french}");
        assert_eq!(en.language(), &Language::En, "{english}");
    }
}

#[test]
fn investigation_requests_compile_to_the_canonical_investigate_statement() {
    let fr = compile(
        "Enquête sur l'attribution de la campagne campaign--42",
        false,
        &[],
    );
    let en = compile(
        "Investigate the attribution of campaign campaign--42",
        false,
        &[],
    );
    assert_eq!(fr.kind(), ActionKind::Investigation);
    assert_eq!(
        fr.canonical(),
        "INVESTIGATE attribution OF Campaign(\"campaign--42\") RETURN assessment, counter_evidence, unknowns, next_best_evidence"
    );
    assert_eq!(en.canonical(), fr.canonical());

    // Only the intents the grammar supports compile; anything else is typed.
    let unsupported = compile(
        "Investigate the provenance of campaign campaign--42",
        false,
        &[],
    );
    assert_eq!(unsupported.kind(), ActionKind::Unsupported);
}

#[test]
fn memory_requests_compile_to_memory_v1_operations() {
    let fr = compile("Que sais-tu de l'infrastructure d'APT28 ?", false, &[]);
    let en = compile(
        "What do you know about the infrastructure of APT28?",
        false,
        &[],
    );
    assert_eq!(fr.kind(), ActionKind::MemoryOperation);
    assert_eq!(fr.canonical(), en.canonical());
    assert!(fr.canonical().starts_with("memory/v1 recall "));
    // The objective is language-neutral content words: articles and
    // prepositions of either language are dropped, the entity is kept verbatim.
    assert!(
        fr.canonical().contains("infrastructure APT28"),
        "{}",
        fr.canonical()
    );

    // A remember needs evidence the caller supplied and write permission. The
    // observation itself is a user literal, quoted, and never translated.
    let with_evidence = compile(
        "Retiens que \"APT28 registered the domain evil.example\" (source span--7)",
        true,
        &["span--7"],
    );
    assert_eq!(with_evidence.kind(), ActionKind::MemoryOperation);
    assert!(with_evidence.writes());
    assert!(with_evidence.canonical().starts_with("memory/v1 remember "));
    assert!(
        with_evidence
            .evidence_refs()
            .contains(&"span--7".to_owned())
    );

    let english = compile(
        "Remember that \"APT28 registered the domain evil.example\" (source span--7)",
        true,
        &["span--7"],
    );
    assert_eq!(english.canonical(), with_evidence.canonical());
    assert!(
        with_evidence
            .canonical()
            .contains("APT28 registered the domain evil.example")
    );

    // Without evidence the compiler asks rather than inventing a source.
    let without_evidence = compile(
        "Remember that \"APT28 registered the domain evil.example\"",
        true,
        &[],
    );
    assert_eq!(without_evidence.kind(), ActionKind::ClarificationRequired);

    // A source the text cites but the caller never supplied is refused.
    let fabricated = compile(
        "Remember that \"APT28 registered the domain evil.example\" (source span--nowhere)",
        true,
        &["span--7"],
    );
    assert_eq!(fabricated.kind(), ActionKind::Abstain);

    // Without write permission a remember is not proposed at all.
    let read_only = compile(
        "Remember that \"APT28 registered the domain evil.example\" (source span--7)",
        false,
        &["span--7"],
    );
    assert_eq!(read_only.kind(), ActionKind::Abstain);
}

#[test]
fn read_only_requests_never_emit_writes_and_destructive_requests_are_unsupported() {
    for text in [
        "Supprime tous les noeuds",
        "Delete all nodes",
        "Efface la mémoire",
        "Drop the graph",
    ] {
        let result = compile(text, true, &[]);
        assert!(
            matches!(result.kind(), ActionKind::Unsupported | ActionKind::Abstain),
            "{text}: destructive requests are refused even with write permission, got {:?}",
            result.kind()
        );
        assert!(!result.writes());
    }
    // A write-shaped request under a read-only task abstains with a reason.
    let refused = compile("Marque l'acteur APT28 comme validé", false, &[]);
    assert!(matches!(
        refused.kind(),
        ActionKind::Abstain | ActionKind::Unsupported
    ));
    assert!(!refused.writes());
}

#[test]
fn ambiguity_and_unknown_requests_produce_typed_results() {
    let ambiguous = compile("Parle-moi de APT28", false, &[]);
    assert_eq!(ambiguous.kind(), ActionKind::ClarificationRequired);
    let unknown = compile("Quelle heure est-il ?", false, &[]);
    assert_eq!(unknown.kind(), ActionKind::Unsupported);
    let empty = compile("   ", false, &[]);
    assert_eq!(empty.kind(), ActionKind::Unsupported);
}

#[test]
fn user_literals_are_preserved_and_not_translated() {
    let fr = compile(
        "Quels acteurs de menace utilisent le malware \"Zebrocy Loader\" ?",
        false,
        &[],
    );
    assert!(
        fr.canonical().contains("'Zebrocy Loader'"),
        "{}",
        fr.canonical()
    );
    // Quotes inside a literal are escaped the way the parser expects.
    let en = compile(
        "Which threat actors use the malware \"O'Neil\"?",
        false,
        &[],
    );
    assert!(en.canonical().contains("'O\\'Neil'"), "{}", en.canonical());
    // Code-switching: an English verb inside a French sentence still resolves.
    let mixed = compile("Liste les threat actors", false, &[]);
    assert_eq!(mixed.canonical(), "MATCH (n:ThreatActor) RETURN n LIMIT 50");
}

#[test]
fn the_language_tag_is_detected_or_taken_from_the_request() {
    let compiler = TemplateCompiler::default();
    let detected = compiler.compile(&NlqRequest::new("Liste les indicateurs"));
    assert_eq!(detected.language, Language::Fr);
    let forced =
        compiler.compile(&NlqRequest::new("Liste les indicateurs").with_language(Language::En));
    assert_eq!(forced.language, Language::En);
}

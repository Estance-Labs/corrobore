// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//! The reproducible bilingual dataset generator, its leakage-safe splits and
//! the parser-backed evaluation harness (epic #82, items 2 and 3 of #260).
//!
//! Every example is derived from one semantic record, so its French and
//! English surfaces share a canonical expected action; adversarial and
//! abstention cases are part of the corpus, not an afterthought; splits are by
//! template family and entity set so surface paraphrases of a held-out family
//! cannot leak into training; and the harness measures the compiler against
//! the real parsers, per language.
#![allow(clippy::unwrap_used)]

use std::collections::BTreeSet;

use corrobore_nlq::{
    ActionKind, Language, NlqCompiler, TemplateCompiler,
    dataset::{Corpus, ExpectedOutcome, Split, generate, split},
    evaluation::evaluate,
};

fn corpus() -> Corpus {
    generate(42)
}

#[test]
fn the_generator_is_deterministic_and_carries_provenance() {
    let first = corpus();
    let second = generate(42);
    assert_eq!(first, second, "the same seed yields the same corpus");
    assert_ne!(
        first.manifest.content_hash,
        generate(43).manifest.content_hash
    );

    let manifest = &first.manifest;
    assert_eq!(manifest.generator_version, env!("CARGO_PKG_VERSION"));
    assert_eq!(manifest.license, "MIT");
    assert_eq!(manifest.seed, 42);
    assert!(manifest.families.len() >= 8, "{:?}", manifest.families);
    assert_eq!(manifest.example_count, first.examples.len());
    assert_eq!(manifest.content_hash.len(), 64, "SHA-256 hex");
    assert!(
        first.examples.len() >= 200,
        "a pilot corpus, got {}",
        first.examples.len()
    );
}

#[test]
fn every_example_is_paired_across_languages_on_one_semantic_record() {
    let corpus = corpus();
    let french: BTreeSet<_> = corpus
        .examples
        .iter()
        .filter(|example| example.language == Language::Fr)
        .map(|example| example.record_id.clone())
        .collect();
    let english: BTreeSet<_> = corpus
        .examples
        .iter()
        .filter(|example| example.language == Language::En)
        .map(|example| example.record_id.clone())
        .collect();
    assert_eq!(
        french, english,
        "each semantic record has a French and an English surface"
    );
    assert!(french.len() >= 100);

    for record in &french {
        let surfaces: Vec<_> = corpus
            .examples
            .iter()
            .filter(|e| &e.record_id == record)
            .collect();
        let expected: BTreeSet<_> = surfaces.iter().map(|e| e.expected.clone()).collect();
        assert_eq!(
            expected.len(),
            1,
            "record {record} has one expected outcome across languages"
        );
        let texts: BTreeSet<_> = surfaces.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(
            texts.len(),
            surfaces.len(),
            "record {record} surfaces differ"
        );
    }
    // Literals are shared verbatim between the two surfaces of a record.
    let example = corpus
        .examples
        .iter()
        .find(|e| matches!(&e.expected, ExpectedOutcome::Action { canonical, .. } if canonical.contains("m.name = '")))
        .unwrap();
    let sibling = corpus
        .examples
        .iter()
        .find(|e| e.record_id == example.record_id && e.language != example.language)
        .unwrap();
    let literal = example
        .text
        .split('"')
        .nth(1)
        .expect("the surface quotes its literal");
    assert!(
        sibling.text.contains(literal),
        "{} / {}",
        example.text,
        sibling.text
    );
}

#[test]
fn the_corpus_holds_adversarial_and_abstention_cases() {
    let corpus = corpus();
    let non_actions = corpus
        .examples
        .iter()
        .filter(|example| {
            matches!(
                example.expected,
                ExpectedOutcome::Terminal(
                    ActionKind::Abstain
                        | ActionKind::Unsupported
                        | ActionKind::ClarificationRequired
                )
            )
        })
        .count();
    let share = non_actions as f64 / corpus.examples.len() as f64;
    assert!(
        share >= 0.25,
        "at least a quarter must be ambiguous, unsupported or adversarial, got {share:.2}"
    );
    assert!(corpus.examples.iter().any(|example| example.adversarial));
    // Adversarial examples carry a prompt injection or a fabricated evidence
    // request and expect a non-action or a read without the injected effect.
    let injected = corpus
        .examples
        .iter()
        .find(|example| example.adversarial && example.text.to_lowercase().contains("ignore"))
        .expect("a prompt-injection example");
    assert!(!matches!(
        &injected.expected,
        ExpectedOutcome::Action { writes: true, .. }
    ));
}

#[test]
fn splits_are_disjoint_by_family_and_by_entity_set() {
    let corpus = corpus();
    let splits = split(&corpus, 42);
    let sets = [&splits.train, &splits.validation, &splits.golden];
    let total: usize = sets.iter().map(|set| set.len()).sum();
    assert_eq!(
        total,
        corpus.examples.len(),
        "every example lands in exactly one split"
    );
    assert!(!splits.golden.is_empty() && !splits.validation.is_empty());

    let families = |set: &[corrobore_nlq::dataset::Example]| -> BTreeSet<String> {
        set.iter().map(|example| example.family.clone()).collect()
    };
    let entities = |set: &[corrobore_nlq::dataset::Example]| -> BTreeSet<String> {
        set.iter()
            .flat_map(|example| example.entities.clone())
            .collect()
    };
    for (left, right) in [(0, 1), (0, 2), (1, 2)] {
        assert!(
            families(sets[left]).is_disjoint(&families(sets[right])),
            "template families leak between splits {left} and {right}"
        );
        assert!(
            entities(sets[left]).is_disjoint(&entities(sets[right])),
            "entity sets leak between splits {left} and {right}"
        );
    }
    // A record's two languages stay together, so a held-out record is held
    // out in both.
    for set in sets {
        let records: BTreeSet<_> = set.iter().map(|e| e.record_id.clone()).collect();
        for record in records {
            let count = set.iter().filter(|e| e.record_id == record).count();
            assert_eq!(
                count, 2,
                "record {record} must keep both surfaces in one split"
            );
        }
    }
    assert_eq!(
        split(&corpus, 42),
        splits,
        "splits are reproducible from the seed"
    );
    assert_eq!(splits.manifest.content_hash.len(), 64);
    assert_eq!(splits.manifest.corpus_hash, corpus.manifest.content_hash);
    assert!(matches!(
        splits.golden.first().map(|e| e.split),
        Some(Split::Golden)
    ));
}

#[test]
fn the_harness_measures_the_template_compiler_per_language() {
    let corpus = corpus();
    let compiler = TemplateCompiler::default();
    let report = evaluate(&compiler, &corpus.examples);

    for language in ["fr", "en"] {
        let metrics = report.per_language.get(language).expect(language);
        assert!(metrics.examples >= 100);
        // The template compiler is the baseline: every envelope it emits must be
        // schema-valid, and it must never invent evidence or emit a write for a
        // read-only request.
        assert_eq!(metrics.schema_valid_rate, 1.0, "{language}");
        assert_eq!(metrics.invented_evidence, 0, "{language}");
        assert_eq!(metrics.read_only_writes, 0, "{language}");
        assert_eq!(metrics.unsupported_recompiled, 0, "{language}");
        assert!(
            metrics.canonical_accuracy >= 0.95,
            "{language}: {}",
            metrics.canonical_accuracy
        );
        assert!(
            metrics.action_class_accuracy >= 0.95,
            "{language}: {}",
            metrics.action_class_accuracy
        );
        assert!(
            metrics.abstention_accuracy >= 0.95,
            "{language}: {}",
            metrics.abstention_accuracy
        );
    }
    let fr = &report.per_language["fr"];
    let en = &report.per_language["en"];
    assert!(
        (fr.canonical_accuracy - en.canonical_accuracy).abs() <= 0.02,
        "language parity"
    );
    assert_eq!(
        report.pair_agreement_rate, 1.0,
        "paired surfaces agree on the canonical action"
    );
    assert!(report.aggregate.examples == fr.examples + en.examples);

    // The report is a document other tooling can read and diff.
    let json = serde_json::to_value(&report).unwrap();
    assert!(json["per_language"]["fr"]["canonical_accuracy"].is_number());
    assert_eq!(json["compiler"], "template");
}

#[test]
fn the_harness_catches_a_compiler_that_misbehaves() {
    struct Reckless;
    impl NlqCompiler for Reckless {
        fn name(&self) -> &'static str {
            "reckless"
        }
        fn compile(&self, request: &corrobore_nlq::NlqRequest) -> corrobore_nlq::Envelope {
            // Emits a write for everything and cites evidence nobody supplied.
            let mut envelope = corrobore_nlq::Envelope::new(
                request.language().cloned().unwrap_or(Language::En),
                corrobore_nlq::Action::CypherWriteProposal {
                    query: "CREATE (n:Injected {k: 1})".to_owned(),
                },
                "compiled",
            );
            envelope.evidence_refs = vec!["span--nowhere".to_owned()];
            envelope
        }
    }
    let corpus = corpus();
    let report = evaluate(&Reckless, &corpus.examples);
    let fr = &report.per_language["fr"];
    assert!(fr.read_only_writes > 0);
    assert!(fr.invented_evidence > 0);
    assert!(fr.canonical_accuracy < 0.05);
    assert!(
        fr.schema_valid_rate < 1.0,
        "a refused envelope is not schema-valid for the harness"
    );
}

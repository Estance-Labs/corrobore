// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

#![allow(clippy::unwrap_used)]

use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

use opencti_access::{AccessContext, AccessMetadata};
use opencti_search::{
    FullTextDocument, FullTextFieldFilter, FullTextIndex, FullTextIndexSettings, FullTextMatchMode,
    FullTextQuery, FullTextRecordClass, FullTextSearchError, FullTextSearchReadiness,
};
use serde::Deserialize;

const CURSOR_KEY: &[u8] = b"issue-46-full-text-cursor-key-32-bytes";

fn root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "corrobore-opencti-search-{name}-{}",
        std::process::id()
    ))
}

fn settings() -> FullTextIndexSettings {
    FullTextIndexSettings {
        schema_version: "opencti-full-text-v1".to_owned(),
        cursor_key: CURSOR_KEY.to_vec(),
        writer_memory_bytes: 15_000_000,
        max_candidates: 10_000,
    }
}

fn system_access(policy_version: &str) -> AccessContext {
    AccessContext {
        subject_id: "system".to_owned(),
        roles: vec!["system".to_owned()],
        attributes: BTreeMap::from([("policy_version".to_owned(), policy_version.to_owned())]),
        ..AccessContext::default()
    }
}

fn clear_access(policy_version: &str) -> AccessContext {
    AccessContext {
        subject_id: "user--clear".to_owned(),
        marking_ids: vec!["marking--clear".to_owned()],
        attributes: BTreeMap::from([("policy_version".to_owned(), policy_version.to_owned())]),
        ..AccessContext::default()
    }
}

fn document(
    id: &str,
    kind: &str,
    revision: u64,
    fields: &[(&str, &[&str])],
    access: AccessMetadata,
) -> FullTextDocument {
    FullTextDocument {
        id: id.to_owned(),
        record_class: FullTextRecordClass::Object,
        kind: kind.to_owned(),
        revision,
        fields: fields
            .iter()
            .map(|(field, values)| {
                (
                    (*field).to_owned(),
                    values.iter().map(|value| (*value).to_owned()).collect(),
                )
            })
            .collect(),
        access,
    }
}

fn query(text: &str, mode: FullTextMatchMode) -> FullTextQuery {
    FullTextQuery {
        text: text.to_owned(),
        mode,
        fields: Vec::new(),
        kinds: Vec::new(),
        filters: Vec::new(),
        limit: 20,
        cursor: None,
    }
}

fn fixture_documents() -> Vec<FullTextDocument> {
    vec![
        document(
            "indicator--ipv4",
            "indicator",
            1,
            &[
                ("name", &["Documentation IPv4 indicator"]),
                ("pattern", &["ipv4-addr value 192.0.2.12"]),
            ],
            AccessMetadata {
                marking_ids: vec!["marking--clear".to_owned()],
                ..AccessMetadata::default()
            },
        ),
        document(
            "indicator--domain",
            "indicator",
            1,
            &[
                ("name", &["Documentation domain indicator"]),
                ("pattern", &["domain-name value malware.example.org"]),
            ],
            AccessMetadata {
                marking_ids: vec!["marking--amber".to_owned()],
                ..AccessMetadata::default()
            },
        ),
        document(
            "report--investigation",
            "report",
            1,
            &[
                ("name", &["Synthetic investigation report"]),
                (
                    "description",
                    &["Investigation of documentation network indicators"],
                ),
            ],
            AccessMetadata::default(),
        ),
    ]
}

fn ready_index(name: &str) -> FullTextIndex {
    let path = root(name);
    let _ = fs::remove_dir_all(&path);
    let index = FullTextIndex::open(path, settings()).unwrap();
    let outcome = index.rebuild(&fixture_documents()).unwrap();
    assert_eq!(outcome.readiness, FullTextSearchReadiness::Ready);
    index
}

#[test]
fn term_normalization_field_and_type_restrictions_match_the_opencti_subset() {
    let index = ready_index("term-field-type");

    let normalized = index
        .search(
            &FullTextQuery {
                text: "DOCUMENTATION".to_owned(),
                fields: vec!["name".to_owned()],
                kinds: vec!["indicator".to_owned()],
                ..query("DOCUMENTATION", FullTextMatchMode::Term)
            },
            &system_access("policy--v1"),
        )
        .unwrap();
    assert_eq!(normalized.total, 2);
    assert!(normalized.hits.iter().all(|hit| hit.kind == "indicator"));

    let wrong_field = index
        .search(
            &FullTextQuery {
                fields: vec!["description".to_owned()],
                ..query("192.0.2.12", FullTextMatchMode::Term)
            },
            &system_access("policy--v1"),
        )
        .unwrap();
    assert_eq!(wrong_field.total, 0);
}

#[test]
fn phrase_fuzzy_and_prefix_queries_cover_documented_user_shapes() {
    let index = ready_index("query-shapes");

    let phrase = index
        .search(
            &query("documentation domain indicator", FullTextMatchMode::Phrase),
            &system_access("policy--v1"),
        )
        .unwrap();
    assert_eq!(phrase.hits[0].id, "indicator--domain");

    let fuzzy = index
        .search(
            &query(
                "investigaton",
                FullTextMatchMode::Fuzzy {
                    distance: 1,
                    prefix: false,
                },
            ),
            &system_access("policy--v1"),
        )
        .unwrap();
    assert_eq!(fuzzy.hits[0].id, "report--investigation");

    let prefix = index
        .search(
            &query("malw", FullTextMatchMode::Prefix),
            &system_access("policy--v1"),
        )
        .unwrap();
    assert_eq!(prefix.hits[0].id, "indicator--domain");
}

#[test]
fn structured_filters_are_conjunctive_with_full_text_matching() {
    let index = ready_index("structured-filter");
    let page = index
        .search(
            &FullTextQuery {
                filters: vec![FullTextFieldFilter {
                    field: "pattern".to_owned(),
                    value: "ipv4-addr value 192.0.2.12".to_owned(),
                }],
                ..query("documentation", FullTextMatchMode::Term)
            },
            &system_access("policy--v1"),
        )
        .unwrap();

    assert_eq!(page.total, 1);
    assert_eq!(page.hits[0].id, "indicator--ipv4");
}

#[test]
fn access_filtering_applies_to_hits_counts_and_ranking_before_payload_reads() {
    let index = ready_index("authorization");
    let page = index
        .search(
            &query("documentation", FullTextMatchMode::Term),
            &clear_access("policy--v1"),
        )
        .unwrap();

    assert_eq!(page.total, 2);
    assert_eq!(
        page.hits
            .iter()
            .map(|hit| hit.id.as_str())
            .collect::<Vec<_>>(),
        ["indicator--ipv4", "report--investigation"]
    );
    assert!(
        !serde_json::to_string(&page)
            .unwrap()
            .contains("indicator--domain")
    );
}

#[test]
fn equal_scores_use_canonical_id_and_cursor_pages_are_generation_bound() {
    let path = root("cursor-generation");
    let _ = fs::remove_dir_all(&path);
    let index = FullTextIndex::open(path, settings()).unwrap();
    index
        .rebuild(&[
            document(
                "indicator--b",
                "indicator",
                1,
                &[("name", &["same token"])],
                AccessMetadata::default(),
            ),
            document(
                "indicator--a",
                "indicator",
                1,
                &[("name", &["same token"])],
                AccessMetadata::default(),
            ),
        ])
        .unwrap();
    let first = index
        .search(
            &FullTextQuery {
                limit: 1,
                ..query("same", FullTextMatchMode::Term)
            },
            &system_access("policy--v1"),
        )
        .unwrap();
    assert_eq!(first.hits[0].id, "indicator--a");
    let cursor = first.next_cursor.clone().expect("next cursor");

    let second = index
        .search(
            &FullTextQuery {
                limit: 1,
                cursor: Some(cursor.clone()),
                ..query("same", FullTextMatchMode::Term)
            },
            &system_access("policy--v1"),
        )
        .unwrap();
    assert_eq!(second.hits[0].id, "indicator--b");

    index
        .rebuild(&[document(
            "indicator--c",
            "indicator",
            2,
            &[("name", &["same token"])],
            AccessMetadata::default(),
        )])
        .unwrap();
    assert!(matches!(
        index.search(
            &FullTextQuery {
                limit: 1,
                cursor: Some(cursor),
                ..query("same", FullTextMatchMode::Term)
            },
            &system_access("policy--v1")
        ),
        Err(FullTextSearchError::IncompatibleCursor)
    ));
}

#[test]
fn policy_change_invalidates_an_existing_cursor() {
    let index = ready_index("cursor-policy");
    let first = index
        .search(
            &FullTextQuery {
                limit: 1,
                ..query("documentation", FullTextMatchMode::Term)
            },
            &clear_access("policy--v1"),
        )
        .unwrap();

    assert!(matches!(
        index.search(
            &FullTextQuery {
                limit: 1,
                cursor: first.next_cursor,
                ..query("documentation", FullTextMatchMode::Term)
            },
            &clear_access("policy--v2")
        ),
        Err(FullTextSearchError::IncompatibleCursor)
    ));
}

#[test]
fn update_delete_merge_and_replay_replace_the_visible_generation() {
    let path = root("mutation-lifecycle");
    let _ = fs::remove_dir_all(&path);
    let index = FullTextIndex::open(path, settings()).unwrap();
    index.rebuild(&fixture_documents()).unwrap();

    let replacement = vec![
        document(
            "indicator--ipv4",
            "indicator",
            2,
            &[("name", &["Updated survivor quasar"])],
            AccessMetadata::default(),
        ),
        document(
            "report--investigation",
            "report",
            1,
            &[("name", &["Synthetic investigation report"])],
            AccessMetadata::default(),
        ),
    ];
    index.rebuild(&replacement).unwrap();

    assert_eq!(
        index
            .search(
                &query("quasar", FullTextMatchMode::Term),
                &system_access("policy--v1")
            )
            .unwrap()
            .hits[0]
            .revision,
        2
    );
    assert_eq!(
        index
            .search(
                &query("malware", FullTextMatchMode::Term),
                &system_access("policy--v1")
            )
            .unwrap()
            .total,
        0
    );

    let replay = index.rebuild(&replacement).unwrap();
    assert!(!replay.generation_changed);
}

#[test]
fn missing_or_corrupt_index_is_detected_and_rebuild_restores_readiness() {
    let index = ready_index("corruption");
    fs::remove_file(index.index_path().join("meta.json")).unwrap();

    assert_eq!(
        index.inspect().readiness,
        FullTextSearchReadiness::RebuildRequired
    );
    assert!(matches!(
        index.search(
            &query("documentation", FullTextMatchMode::Term),
            &system_access("policy--v1")
        ),
        Err(FullTextSearchError::IndexNotReady)
    ));

    let rebuilt = index.rebuild(&fixture_documents()).unwrap();
    assert_eq!(rebuilt.readiness, FullTextSearchReadiness::Ready);
}

#[test]
fn rebuild_reports_progress_and_never_publishes_an_incomplete_index() {
    let path = root("resumable-rebuild");
    let _ = fs::remove_dir_all(&path);
    let index = FullTextIndex::open(path.clone(), settings()).unwrap();

    let interrupted = index
        .rebuild_with_checkpoint(&fixture_documents(), 1, Some(1))
        .unwrap();
    assert_eq!(interrupted.readiness, FullTextSearchReadiness::Building);
    assert_eq!(interrupted.processed_documents, 1);
    assert_eq!(interrupted.total_documents, 3);
    assert!(matches!(
        index.search(
            &query("documentation", FullTextMatchMode::Term),
            &system_access("policy--v1")
        ),
        Err(FullTextSearchError::IndexNotReady)
    ));

    let reopened = FullTextIndex::open(path, settings()).unwrap();
    let resumed = reopened
        .rebuild_with_checkpoint(&fixture_documents(), 1, None)
        .unwrap();
    assert_eq!(resumed.readiness, FullTextSearchReadiness::Ready);
    assert_eq!(resumed.processed_documents, 3);
}

#[test]
fn explicit_invalidation_blocks_stale_reads_until_the_generation_is_revalidated() {
    let index = ready_index("invalidation");
    index.invalidate().unwrap();

    assert_eq!(
        index.inspect().readiness,
        FullTextSearchReadiness::RebuildRequired
    );
    assert!(matches!(
        index.search(
            &query("documentation", FullTextMatchMode::Term),
            &system_access("policy--v1")
        ),
        Err(FullTextSearchError::IndexNotReady)
    ));

    let outcome = index.rebuild(&fixture_documents()).unwrap();
    assert_eq!(outcome.readiness, FullTextSearchReadiness::Ready);
    assert!(!outcome.generation_changed);
    assert_eq!(
        index
            .search(
                &query("documentation", FullTextMatchMode::Term),
                &system_access("policy--v1")
            )
            .unwrap()
            .total,
        3
    );
}

#[test]
fn small_profile_probe_records_latency_memory_and_disk_without_unbounded_candidates() {
    let path = root("small-profile");
    let _ = fs::remove_dir_all(&path);
    let index = FullTextIndex::open(path, settings()).unwrap();
    let documents = (0..2_000)
        .map(|number| {
            document(
                &format!("indicator--{number:05}"),
                "indicator",
                1,
                &[(
                    "name",
                    &[if number % 2 == 0 {
                        "documentation beacon"
                    } else {
                        "synthetic observation"
                    }],
                )],
                AccessMetadata::default(),
            )
        })
        .collect::<Vec<_>>();
    index.rebuild(&documents).unwrap();

    let started = Instant::now();
    let page = index
        .search(
            &FullTextQuery {
                limit: 100,
                ..query("documentation", FullTextMatchMode::Term)
            },
            &system_access("policy--v1"),
        )
        .unwrap();
    let elapsed = started.elapsed();
    let stats = index.storage_stats().unwrap();

    assert_eq!(page.total, 1_000);
    assert!(elapsed < Duration::from_secs(2));
    assert!(stats.disk_bytes > 0);
    assert!(stats.writer_memory_bytes <= 15_000_000);
    assert!(stats.max_candidates <= 10_000);
}

#[derive(Debug, Deserialize)]
struct RelevanceCorpus {
    metric: RelevanceMetric,
    queries: Vec<RelevanceQuery>,
}

#[derive(Debug, Deserialize)]
struct RelevanceMetric {
    minimum: f64,
}

#[derive(Debug, Deserialize)]
struct RelevanceQuery {
    text: String,
    mode: String,
    #[serde(default)]
    distance: u8,
    relevant_id: String,
}

#[test]
fn annotated_opencti_relevance_corpus_meets_the_mrr_gate() {
    let corpus: RelevanceCorpus = serde_json::from_str(include_str!(
        "../../../compatibility/opencti/7.260722.0/full-text-relevance.json"
    ))
    .unwrap();
    let index = ready_index("relevance-corpus");
    let reciprocal_rank_sum = corpus
        .queries
        .iter()
        .map(|annotated| {
            let mode = match annotated.mode.as_str() {
                "term" => FullTextMatchMode::Term,
                "phrase" => FullTextMatchMode::Phrase,
                "prefix" => FullTextMatchMode::Prefix,
                "fuzzy" => FullTextMatchMode::Fuzzy {
                    distance: annotated.distance,
                    prefix: false,
                },
                other => panic!("unsupported annotated mode {other}"),
            };
            let page = index
                .search(&query(&annotated.text, mode), &system_access("policy--v1"))
                .unwrap();
            page.hits
                .iter()
                .take(10)
                .position(|hit| hit.id == annotated.relevant_id)
                .map_or(0.0, |rank| 1.0 / (rank.saturating_add(1) as f64))
        })
        .sum::<f64>();
    let mrr = reciprocal_rank_sum / corpus.queries.len() as f64;
    assert!(
        mrr >= corpus.metric.minimum,
        "MRR@10 {mrr:.3} is below {:.3}",
        corpus.metric.minimum
    );
}

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
//! Integration contract for the immutable `Source` record (Epic 0029, WS-A
//! item 2, issue #148).
//!
//! A source is the stable origin identity behind observations and evidence:
//! URI or file identity, type, publisher, authority domain, acquisition time,
//! artifact hash, optional signature, optional parent source. It has no update
//! path: a changed artifact hash for the same identity creates a superseding
//! version and raises a content-drift validation issue. Legacy evidence records
//! are lifted into sources idempotently.
use graph_core::{
    EvidenceId, EvidenceInput, EvidenceRecordStore, EvidenceSourceType, GraphError, PropertyValue,
    Source, SourceId, SourceInput, SourceRegistrationOutcome, SourceStore, TemporalTimestamp,
    ValidationErrorSeverity, ValidationTarget,
};

const HASH_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const HASH_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn source_id(value: &str) -> SourceId {
    SourceId::new(value).expect("test source ID should be valid")
}

fn evidence_id(value: &str) -> EvidenceId {
    EvidenceId::new(value).expect("test evidence ID should be valid")
}

fn timestamp(value: &str) -> TemporalTimestamp {
    TemporalTimestamp::new(value).expect("test timestamp should be valid")
}

fn report_input(hash: &str) -> SourceInput {
    SourceInput::new(
        source_id("source--vendor-report-2026-08"),
        "https://vendor.example/reports/2026-08",
        EvidenceSourceType::Url,
    )
    .with_publisher("Vendor Threat Research")
    .with_authority_domain("cti.vendor-report")
    .with_acquired_at(timestamp("2026-08-30T10:00:00Z"))
    .with_artifact_sha256(hash)
}

//
// Verify that a source records every identity and acquisition field the
// product requirements name, starts at version 1, and is not marked legacy
// when created through the explicit input path.
#[test]
fn source_store_creates_first_version_with_all_fields() {
    let mut store = SourceStore::new();

    let registration = store
        .register_source(report_input(HASH_A).with_signature("sig:ed25519:abc"))
        .expect("source should be registered");
    assert_eq!(registration.outcome(), SourceRegistrationOutcome::Created);

    let source = store
        .current_source(&source_id("source--vendor-report-2026-08"))
        .expect("current version should exist");
    assert_eq!(source.id(), &source_id("source--vendor-report-2026-08"));
    assert_eq!(source.version(), 1);
    assert_eq!(
        source.version_id().as_str(),
        "source-version--source--vendor-report-2026-08--1"
    );
    assert_eq!(source.uri(), "https://vendor.example/reports/2026-08");
    assert_eq!(source.source_type(), EvidenceSourceType::Url);
    assert_eq!(source.publisher(), Some("Vendor Threat Research"));
    assert_eq!(source.authority_domain(), Some("cti.vendor-report"));
    assert_eq!(
        source.acquired_at().map(TemporalTimestamp::as_str),
        Some("2026-08-30T10:00:00Z")
    );
    assert_eq!(source.artifact_sha256(), Some(HASH_A));
    assert_eq!(source.signature(), Some("sig:ed25519:abc"));
    assert!(source.parent_source().is_none());
    assert!(source.supersedes().is_none());
    assert!(!source.derived_from_legacy());
    assert_eq!(store.len(), 1);
}

//
// Verify input validation: blank URI, malformed hash, blank optional strings,
// and a self-referencing parent are rejected before anything is stored.
#[test]
fn source_input_rejects_invalid_fields() {
    let mut store = SourceStore::new();

    let blank_uri = SourceInput::new(source_id("source--x"), "  ", EvidenceSourceType::Document);
    assert!(matches!(
        store.register_source(blank_uri),
        Err(GraphError::InvalidPropertyValue(message)) if message.contains("uri")
    ));

    let bad_hash = report_input("not-a-hash");
    assert!(matches!(
        store.register_source(bad_hash),
        Err(GraphError::InvalidPropertyValue(message)) if message.contains("artifact_sha256")
    ));

    let blank_publisher = report_input(HASH_A).with_publisher(" ");
    assert!(matches!(
        store.register_source(blank_publisher),
        Err(GraphError::InvalidPropertyValue(message)) if message.contains("publisher")
    ));

    let self_parent =
        report_input(HASH_A).with_parent_source(source_id("source--vendor-report-2026-08"));
    assert!(matches!(
        store.register_source(self_parent),
        Err(GraphError::InvalidPropertyValue(message)) if message.contains("parent")
    ));

    assert!(store.is_empty());
}

//
// Verify that a parent source must already be registered: provenance chains
// cannot point at sources the store has never seen.
#[test]
fn parent_source_must_exist() {
    let mut store = SourceStore::new();

    let orphan = SourceInput::new(
        source_id("source--syndicated-copy"),
        "https://mirror.example/copy",
        EvidenceSourceType::Url,
    )
    .with_parent_source(source_id("source--missing-original"));
    let error = store
        .register_source(orphan)
        .expect_err("unknown parent must be rejected");
    assert!(matches!(
        error,
        GraphError::SourceNotFound(id) if id == source_id("source--missing-original")
    ));

    store
        .register_source(report_input(HASH_A))
        .expect("original should register");
    let child = SourceInput::new(
        source_id("source--syndicated-copy"),
        "https://mirror.example/copy",
        EvidenceSourceType::Url,
    )
    .with_parent_source(source_id("source--vendor-report-2026-08"));
    store
        .register_source(child)
        .expect("child with known parent should register");
    let child = store
        .current_source(&source_id("source--syndicated-copy"))
        .expect("child should exist");
    assert_eq!(
        child.parent_source(),
        Some(&source_id("source--vendor-report-2026-08"))
    );
}

//
// Verify immutability and idempotence: registering the identical input again
// is a no-op that reports `Unchanged`, while changing any descriptive field
// without a hash change is a conflict, not a silent update.
#[test]
fn source_has_no_update_path() {
    let mut store = SourceStore::new();
    store
        .register_source(report_input(HASH_A))
        .expect("first registration should succeed");

    let again = store
        .register_source(report_input(HASH_A))
        .expect("identical registration should be accepted");
    assert_eq!(again.outcome(), SourceRegistrationOutcome::Unchanged);
    assert_eq!(store.len(), 1);

    let renamed = report_input(HASH_A).with_publisher("Someone Else");
    let error = store
        .register_source(renamed)
        .expect_err("metadata change without a new artifact is a conflict");
    assert!(matches!(
        error,
        GraphError::InvalidPropertyValue(message) if message.contains("conflicting source")
    ));
    assert_eq!(store.len(), 1);
    assert!(store.content_drift_issues().is_empty());
}

//
// Verify content-drift handling: a new artifact hash for the same identity
// creates a superseding version, keeps the previous version queryable, and
// raises a typed validation issue naming the source.
#[test]
fn changed_artifact_hash_supersedes_and_raises_drift_issue() {
    let mut store = SourceStore::new();
    store
        .register_source(report_input(HASH_A))
        .expect("first version should register");

    let registration = store
        .register_source(report_input(HASH_B))
        .expect("drifted artifact should create a new version");
    assert_eq!(
        registration.outcome(),
        SourceRegistrationOutcome::Superseded
    );
    assert_eq!(registration.version(), 2);

    let current = store
        .current_source(&source_id("source--vendor-report-2026-08"))
        .expect("current version should exist");
    assert_eq!(current.version(), 2);
    assert_eq!(current.artifact_sha256(), Some(HASH_B));
    assert_eq!(
        current.supersedes().map(|id| id.as_str()),
        Some("source-version--source--vendor-report-2026-08--1")
    );

    let versions = store.source_versions(&source_id("source--vendor-report-2026-08"));
    assert_eq!(versions.len(), 2);
    assert_eq!(versions[0].artifact_sha256(), Some(HASH_A));
    assert_eq!(versions[1].artifact_sha256(), Some(HASH_B));
    assert_eq!(store.len(), 2);

    let issues = store.content_drift_issues();
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].code(), "source.content_drift");
    assert_eq!(issues[0].severity(), ValidationErrorSeverity::Warning);
    assert_eq!(
        issues[0].target(),
        &ValidationTarget::source("source--vendor-report-2026-08")
    );
    assert!(issues[0].message().contains(HASH_A) && issues[0].message().contains(HASH_B));
}

//
// Verify the legacy lift: evidence records sharing a `source_ref` and hash
// lift to one source, the lift is idempotent, a differing hash creates a
// superseding version with a drift issue, and every lifted record now carries
// its `source_id`.
#[test]
fn evidence_records_lift_into_sources_idempotently() {
    let mut evidence = EvidenceRecordStore::new();
    evidence
        .create_evidence(
            EvidenceInput::new(
                evidence_id("evidence--1"),
                "source://report/2026-07-06",
                "first span",
            )
            .with_source_type(EvidenceSourceType::Document)
            .with_source_url("https://archive.example/report-2026-07-06.pdf")
            .with_content_sha256(HASH_A),
        )
        .expect("evidence 1 should be created");
    evidence
        .create_evidence(
            EvidenceInput::new(
                evidence_id("evidence--2"),
                "source://report/2026-07-06",
                "second span of the same document",
            )
            .with_source_type(EvidenceSourceType::Document)
            .with_source_url("https://archive.example/report-2026-07-06.pdf")
            .with_content_sha256(HASH_A),
        )
        .expect("evidence 2 should be created");
    evidence
        .create_evidence(EvidenceInput::new(
            evidence_id("evidence--3"),
            "source://feed/taxii/collection-9",
            "indicator without a hash or type",
        ))
        .expect("evidence 3 should be created");

    let mut sources = SourceStore::new();
    let first = evidence
        .lift_sources(&mut sources)
        .expect("lift should succeed");
    assert_eq!(first.len(), 3);
    assert_eq!(
        sources.len(),
        2,
        "two distinct source_refs lift to two sources"
    );

    let report = sources
        .current_source(&source_id("source://report/2026-07-06"))
        .expect("report source should exist");
    assert!(report.derived_from_legacy());
    assert_eq!(
        report.uri(),
        "https://archive.example/report-2026-07-06.pdf"
    );
    assert_eq!(report.source_type(), EvidenceSourceType::Document);
    assert_eq!(report.artifact_sha256(), Some(HASH_A));

    let feed = sources
        .current_source(&source_id("source://feed/taxii/collection-9"))
        .expect("feed source should exist");
    assert_eq!(feed.uri(), "source://feed/taxii/collection-9");
    assert_eq!(feed.source_type(), EvidenceSourceType::Other);
    assert!(feed.artifact_sha256().is_none());

    for id in ["evidence--1", "evidence--2"] {
        let record = evidence
            .evidence_by_id(&evidence_id(id))
            .expect("record should exist");
        assert_eq!(
            record.source_id(),
            Some(&source_id("source://report/2026-07-06"))
        );
    }

    let second = evidence
        .lift_sources(&mut sources)
        .expect("second lift should succeed");
    assert!(
        second
            .iter()
            .all(|registration| registration.outcome() == SourceRegistrationOutcome::Unchanged)
    );
    assert_eq!(sources.len(), 2);
    assert!(sources.content_drift_issues().is_empty());

    evidence
        .create_evidence(
            EvidenceInput::new(
                evidence_id("evidence--4"),
                "source://report/2026-07-06",
                "span from a re-fetched, changed document",
            )
            .with_source_type(EvidenceSourceType::Document)
            .with_content_sha256(HASH_B),
        )
        .expect("evidence 4 should be created");
    evidence
        .lift_sources(&mut sources)
        .expect("third lift should succeed");
    let report = sources
        .current_source(&source_id("source://report/2026-07-06"))
        .expect("report source should still exist");
    assert_eq!(report.version(), 2);
    assert_eq!(report.artifact_sha256(), Some(HASH_B));
    assert_eq!(sources.content_drift_issues().len(), 1);
}

//
// Verify that an evidence record can carry an explicit `source_id`, that the
// lift respects it instead of overriding it, and that records serialized
// before this change deserialize with `source_id = None` and keep serializing
// without the key.
#[test]
fn evidence_source_id_is_optional_and_serde_compatible() {
    let mut sources = SourceStore::new();
    sources
        .register_source(report_input(HASH_A))
        .expect("source should register");

    let mut evidence = EvidenceRecordStore::new();
    evidence
        .create_evidence(
            EvidenceInput::new(
                evidence_id("evidence--explicit"),
                "ref--explicit",
                "payload",
            )
            .with_source_id(source_id("source--vendor-report-2026-08")),
        )
        .expect("evidence with explicit source should be created");
    evidence
        .create_evidence(EvidenceInput::new(
            evidence_id("evidence--legacy"),
            "ref--legacy",
            "payload",
        ))
        .expect("legacy-shaped evidence should be created");

    let legacy_json = serde_json::to_value(
        evidence
            .evidence_by_id(&evidence_id("evidence--legacy"))
            .expect("legacy record should exist"),
    )
    .expect("record should serialize");
    assert!(legacy_json.get("source_id").is_none());

    let pre_change = serde_json::json!({
        "id": { "value": "evidence--pre-change" },
        "source_ref": "ref--pre-change",
        "payload": "payload",
        "source_type": null, "chunk_id": null, "offset_start": null, "offset_end": null,
        "source_url": null, "extraction_run_id": null, "extractor_id": null,
        "model_version": null, "observed_at": null, "language": null,
        "source_reliability": null, "information_credibility": null,
        "content_sha256": null, "locator": null
    });
    let restored: graph_core::EvidenceRecord =
        serde_json::from_value(pre_change).expect("pre-change payload should deserialize");
    assert!(restored.source_id().is_none());

    evidence
        .lift_sources(&mut sources)
        .expect("lift should succeed");
    let explicit = evidence
        .evidence_by_id(&evidence_id("evidence--explicit"))
        .expect("explicit record should exist");
    assert_eq!(
        explicit.source_id(),
        Some(&source_id("source--vendor-report-2026-08")),
        "an explicit source_id is never overridden by the lift"
    );
    assert!(
        sources
            .current_source(&source_id("ref--explicit"))
            .is_none(),
        "no legacy source is lifted for a record that already names its source"
    );
    assert!(sources.current_source(&source_id("ref--legacy")).is_some());
}

//
// Verify that a source round-trips through serde with every field intact and
// projects into additive, namespaced `source_*` properties.
#[test]
fn source_round_trips_and_projects_to_namespaced_properties() {
    let mut store = SourceStore::new();
    store
        .register_source(report_input(HASH_A))
        .expect("original should register");
    store
        .register_source(
            SourceInput::new(
                source_id("source--syndicated-copy"),
                "https://mirror.example/copy",
                EvidenceSourceType::Url,
            )
            .with_parent_source(source_id("source--vendor-report-2026-08"))
            .with_signature("sig:ed25519:def"),
        )
        .expect("copy should register");

    let copy = store
        .current_source(&source_id("source--syndicated-copy"))
        .expect("copy should exist")
        .clone();
    let json = serde_json::to_string(&copy).expect("source should serialize");
    let restored: Source = serde_json::from_str(&json).expect("source should deserialize");
    assert_eq!(restored, copy);

    let store_json = serde_json::to_string(&store).expect("store should serialize");
    let restored_store: SourceStore =
        serde_json::from_str(&store_json).expect("store should deserialize");
    assert_eq!(restored_store, store);

    let properties = copy.to_property_map();
    assert_eq!(
        properties.get("source_id"),
        Some(&PropertyValue::String("source--syndicated-copy".to_owned()))
    );
    assert_eq!(
        properties.get("source_version"),
        Some(&PropertyValue::Integer(1))
    );
    assert_eq!(
        properties.get("source_uri"),
        Some(&PropertyValue::String(
            "https://mirror.example/copy".to_owned()
        ))
    );
    assert_eq!(
        properties.get("source_type"),
        Some(&PropertyValue::String("url".to_owned()))
    );
    assert_eq!(
        properties.get("source_parent"),
        Some(&PropertyValue::String(
            "source--vendor-report-2026-08".to_owned()
        ))
    );
    assert_eq!(
        properties.get("source_signature"),
        Some(&PropertyValue::String("sig:ed25519:def".to_owned()))
    );
    assert_eq!(
        properties.get("source_derived_from_legacy"),
        Some(&PropertyValue::Bool(false))
    );
    assert!(!properties.contains_key("source_publisher"));
    assert!(!properties.contains_key("source_artifact_sha256"));
    assert!(properties.keys().all(|key| key.starts_with("source_")));
}

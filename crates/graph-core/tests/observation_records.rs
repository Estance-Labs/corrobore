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
//! Integration contract for the immutable `Observation` record (Epic 0029,
//! WS-A item 3, issue #149).
//!
//! An observation is the exact span, region, or structured record actually
//! observed in a source: it binds a verbatim payload and a selector to a
//! registered `Source`. It has no update path; a correction is a new
//! observation that supersedes the old one. Legacy evidence records lift into
//! observations idempotently once their source has been lifted.
use graph_core::{
    EvidenceAttachmentTarget, EvidenceId, EvidenceInput, EvidenceLocator, EvidenceRecordStore,
    EvidenceSourceType, GraphError, ImmutableRecordKind, Observation, ObservationId,
    ObservationInput, ObservationModality, ObservationStore, PropertyValue, SourceId, SourceInput,
    SourceStore, TemporalTimestamp,
};

const HASH_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const HASH_P: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

fn observation_id(value: &str) -> ObservationId {
    ObservationId::new(value).expect("test observation ID should be valid")
}

fn source_id(value: &str) -> SourceId {
    SourceId::new(value).expect("test source ID should be valid")
}

fn evidence_id(value: &str) -> EvidenceId {
    EvidenceId::new(value).expect("test evidence ID should be valid")
}

fn timestamp(value: &str) -> TemporalTimestamp {
    TemporalTimestamp::new(value).expect("test timestamp should be valid")
}

fn sources_with_report() -> SourceStore {
    let mut sources = SourceStore::new();
    sources
        .register_source(
            SourceInput::new(
                source_id("source--report"),
                "https://vendor.example/report.pdf",
                EvidenceSourceType::Document,
            )
            .with_artifact_sha256(HASH_A),
        )
        .expect("report source should register");
    sources
}

fn span_input(id: &str) -> ObservationInput {
    ObservationInput::new(
        observation_id(id),
        source_id("source--report"),
        "APT-K-47 operated the Winter Lantern campaign between January and June.",
        ObservationModality::Text,
    )
    .with_selector(EvidenceLocator::CharacterSpan {
        start: 1_204,
        end: 1_276,
    })
    .with_observed_at(timestamp("2026-08-30T10:05:00Z"))
    .with_payload_sha256(HASH_P)
}

//
// Verify that an observation records its source, selector, verbatim payload,
// modality, observation time, and payload hash, and starts as a current,
// non-legacy record.
#[test]
fn observation_store_creates_record_bound_to_source() {
    let sources = sources_with_report();
    let mut store = ObservationStore::new();

    let created = store
        .create_observation(span_input("observation--1"), &sources)
        .expect("observation should be created");
    assert_eq!(created, observation_id("observation--1"));

    let observation = store
        .observation_by_id(&observation_id("observation--1"))
        .expect("observation should exist");
    assert_eq!(observation.source_id(), &source_id("source--report"));
    assert_eq!(
        observation.selector(),
        Some(&EvidenceLocator::CharacterSpan {
            start: 1_204,
            end: 1_276,
        })
    );
    assert_eq!(
        observation.payload(),
        "APT-K-47 operated the Winter Lantern campaign between January and June."
    );
    assert_eq!(observation.modality(), ObservationModality::Text);
    assert_eq!(
        observation.observed_at().map(TemporalTimestamp::as_str),
        Some("2026-08-30T10:05:00Z")
    );
    assert_eq!(observation.payload_sha256(), Some(HASH_P));
    assert!(observation.supersedes().is_none());
    assert!(!observation.derived_from_legacy());
    assert!(store.is_current(&observation_id("observation--1")));
    assert_eq!(store.len(), 1);
}

//
// Verify that an observation cannot exist without a registered source, and
// that the source is checked against the store handed in, not a stale
// registry.
#[test]
fn observation_requires_registered_source() {
    let mut store = ObservationStore::new();

    let error = store
        .create_observation(span_input("observation--orphan"), &SourceStore::new())
        .expect_err("unknown source must be rejected");
    assert!(matches!(
        error,
        GraphError::SourceNotFound(id) if id == source_id("source--report")
    ));
    assert!(store.is_empty());
}

//
// Verify input validation: blank payload, malformed payload hash, and an
// invalid selector are rejected before anything is stored.
#[test]
fn observation_input_rejects_invalid_fields() {
    let sources = sources_with_report();
    let mut store = ObservationStore::new();

    let blank = ObservationInput::new(
        observation_id("observation--blank"),
        source_id("source--report"),
        "   ",
        ObservationModality::Text,
    );
    assert!(matches!(
        store.create_observation(blank, &sources),
        Err(GraphError::InvalidPropertyValue(message)) if message.contains("payload")
    ));

    let bad_hash = span_input("observation--hash").with_payload_sha256("nope");
    assert!(matches!(
        store.create_observation(bad_hash, &sources),
        Err(GraphError::InvalidPropertyValue(message)) if message.contains("payload_sha256")
    ));

    let empty_span = span_input("observation--span")
        .with_selector(EvidenceLocator::CharacterSpan { start: 10, end: 10 });
    assert!(matches!(
        store.create_observation(empty_span, &sources),
        Err(GraphError::InvalidPropertyValue(message)) if message.contains("locator")
    ));

    let blank_path = span_input("observation--path").with_selector(EvidenceLocator::RecordPath {
        path: " ".to_owned(),
    });
    assert!(matches!(
        store.create_observation(blank_path, &sources),
        Err(GraphError::InvalidPropertyValue(message)) if message.contains("locator")
    ));

    assert!(store.is_empty());
}

//
// Verify immutability: re-creating an identical observation is a no-op, a
// different payload under the same ID is a conflict, and the only correction
// path is a superseding observation that keeps the old one queryable.
#[test]
fn observation_has_no_update_path_and_supersedes_explicitly() {
    let sources = sources_with_report();
    let mut store = ObservationStore::new();
    store
        .create_observation(span_input("observation--1"), &sources)
        .expect("first creation should succeed");

    store
        .create_observation(span_input("observation--1"), &sources)
        .expect("identical re-creation is idempotent");
    assert_eq!(store.len(), 1);

    let changed = ObservationInput::new(
        observation_id("observation--1"),
        source_id("source--report"),
        "a different verbatim payload",
        ObservationModality::Text,
    );
    let error = store
        .create_observation(changed, &sources)
        .expect_err("changing an observation in place is a conflict");
    assert!(matches!(
        error,
        GraphError::ImmutableRecordConflict { kind: ImmutableRecordKind::Observation, id }
            if id == "observation--1"
    ));

    let corrected =
        span_input("observation--1-corrected").with_selector(EvidenceLocator::CharacterSpan {
            start: 1_204,
            end: 1_290,
        });
    let new_id = store
        .supersede_observation(&observation_id("observation--1"), corrected, &sources)
        .expect("correction should create a superseding observation");
    assert_eq!(new_id, observation_id("observation--1-corrected"));
    assert_eq!(store.len(), 2);

    let old = store
        .observation_by_id(&observation_id("observation--1"))
        .expect("superseded observation stays queryable");
    assert_eq!(
        old.selector(),
        Some(&EvidenceLocator::CharacterSpan {
            start: 1_204,
            end: 1_276,
        })
    );
    assert!(!store.is_current(&observation_id("observation--1")));
    assert_eq!(
        store.superseded_by(&observation_id("observation--1")),
        Some(&observation_id("observation--1-corrected"))
    );

    let new = store
        .observation_by_id(&new_id)
        .expect("new observation should exist");
    assert_eq!(new.supersedes(), Some(&observation_id("observation--1")));
    assert!(store.is_current(&new_id));

    let twice = store.supersede_observation(
        &observation_id("observation--1"),
        span_input("observation--1-again"),
        &sources,
    );
    assert!(matches!(twice, Err(GraphError::InvalidVersionState(_))));

    let missing = store.supersede_observation(
        &observation_id("observation--missing"),
        span_input("observation--x"),
        &sources,
    );
    assert!(matches!(
        missing,
        Err(GraphError::ObservationNotFound(id)) if id == observation_id("observation--missing")
    ));
}

//
// Verify that every locator variant, including the two added for
// observations, round-trips through the observation record and serde.
#[test]
fn every_selector_variant_round_trips() {
    let sources = sources_with_report();
    let mut store = ObservationStore::new();
    let selectors = [
        EvidenceLocator::Page { page: 3 },
        EvidenceLocator::Paragraph {
            page: Some(3),
            paragraph: 2,
        },
        EvidenceLocator::TableCell {
            page: None,
            table: 1,
            row: 4,
            column: 2,
        },
        EvidenceLocator::ByteRange { start: 0, end: 128 },
        EvidenceLocator::CharacterSpan { start: 5, end: 42 },
        EvidenceLocator::RecordPath {
            path: "/objects/3/pattern".to_owned(),
        },
    ];

    for (index, selector) in selectors.iter().enumerate() {
        let id = observation_id(&format!("observation--selector-{index}"));
        store
            .create_observation(
                ObservationInput::new(
                    id.clone(),
                    source_id("source--report"),
                    format!("payload {index}"),
                    ObservationModality::StructuredRecord,
                )
                .with_selector(selector.clone()),
                &sources,
            )
            .expect("observation with selector should be created");
        let stored = store
            .observation_by_id(&id)
            .expect("observation should exist");
        assert_eq!(stored.selector(), Some(selector));

        let json = serde_json::to_string(stored).expect("observation should serialize");
        let restored: Observation =
            serde_json::from_str(&json).expect("observation should deserialize");
        assert_eq!(&restored, stored);
    }

    let store_json = serde_json::to_string(&store).expect("store should serialize");
    let restored: ObservationStore =
        serde_json::from_str(&store_json).expect("store should deserialize");
    assert_eq!(restored, store);
}

//
// Verify the legacy lift: after sources are lifted, evidence records lift into
// observations bound to their source, keeping payload and offsets unchanged;
// records without any locator lift with no selector; the lift is idempotent
// and never overrides an explicit `observation_id`.
#[test]
fn evidence_records_lift_into_observations_idempotently() {
    let mut evidence = EvidenceRecordStore::new();
    evidence
        .create_evidence(
            EvidenceInput::new(
                evidence_id("evidence--offsets"),
                "source://report/2026-07-06",
                "span located by byte offsets",
            )
            .with_offsets(100, 128)
            .with_observed_at(timestamp("2026-07-06T08:00:00Z")),
        )
        .expect("evidence with offsets should be created");
    evidence
        .create_evidence(
            EvidenceInput::new(
                evidence_id("evidence--locator"),
                "source://report/2026-07-06",
                "span located by a page locator",
            )
            .with_locator(EvidenceLocator::Page { page: 7 }),
        )
        .expect("evidence with locator should be created");
    evidence
        .create_evidence(EvidenceInput::new(
            evidence_id("evidence--bare"),
            "source://feed/taxii/collection-9",
            "indicator with no selector at all",
        ))
        .expect("bare evidence should be created");

    let mut sources = SourceStore::new();
    let mut observations = ObservationStore::new();

    let too_early = evidence.lift_observations(&mut observations, &sources);
    assert!(
        matches!(too_early, Err(GraphError::InvalidVersionState(message)) if message.contains("source")),
        "observations cannot be lifted before sources are lifted"
    );

    evidence
        .lift_sources(&mut sources)
        .expect("sources should lift");
    let lifted = evidence
        .lift_observations(&mut observations, &sources)
        .expect("observations should lift");
    assert_eq!(lifted.len(), 3);
    assert_eq!(observations.len(), 3);

    let by_offsets = observations
        .observation_by_id(&observation_id("observation--evidence--offsets"))
        .expect("offset-located observation should exist");
    assert_eq!(
        by_offsets.source_id(),
        &source_id("source://report/2026-07-06")
    );
    assert_eq!(by_offsets.payload(), "span located by byte offsets");
    assert_eq!(
        by_offsets.selector(),
        Some(&EvidenceLocator::ByteRange {
            start: 100,
            end: 128,
        })
    );
    assert_eq!(
        by_offsets.observed_at().map(TemporalTimestamp::as_str),
        Some("2026-07-06T08:00:00Z")
    );
    assert_eq!(by_offsets.modality(), ObservationModality::Text);
    assert!(by_offsets.derived_from_legacy());

    let by_locator = observations
        .observation_by_id(&observation_id("observation--evidence--locator"))
        .expect("locator-located observation should exist");
    assert_eq!(
        by_locator.selector(),
        Some(&EvidenceLocator::Page { page: 7 })
    );

    let bare = observations
        .observation_by_id(&observation_id("observation--evidence--bare"))
        .expect("bare observation should exist");
    assert!(bare.selector().is_none());
    assert_eq!(
        bare.source_id(),
        &source_id("source://feed/taxii/collection-9")
    );

    for id in ["evidence--offsets", "evidence--locator", "evidence--bare"] {
        let record = evidence
            .evidence_by_id(&evidence_id(id))
            .expect("record should exist");
        assert_eq!(
            record.observation_id(),
            Some(&observation_id(&format!("observation--{id}")))
        );
    }

    let again = evidence
        .lift_observations(&mut observations, &sources)
        .expect("second lift should succeed");
    assert!(again.is_empty());
    assert_eq!(observations.len(), 3);
}

//
// Verify that an evidence record can name its observation explicitly, that the
// lift respects it, that pre-change payloads deserialize with
// `observation_id = None`, and that an attachment can target an observation
// only once the observation is registered as a target.
#[test]
fn evidence_observation_binding_and_attachment_target() {
    let sources = sources_with_report();
    let mut observations = ObservationStore::new();
    observations
        .create_observation(span_input("observation--1"), &sources)
        .expect("observation should be created");

    let mut evidence = EvidenceRecordStore::new();
    evidence
        .create_evidence(
            EvidenceInput::new(
                evidence_id("evidence--explicit"),
                "ref--explicit",
                "payload",
            )
            .with_source_id(source_id("source--report"))
            .with_observation_id(observation_id("observation--1")),
        )
        .expect("evidence with explicit observation should be created");

    let record = evidence
        .evidence_by_id(&evidence_id("evidence--explicit"))
        .expect("record should exist");
    let json = serde_json::to_value(record).expect("record should serialize");
    assert_eq!(
        json.get("observation_id"),
        Some(&serde_json::json!({ "value": "observation--1" }))
    );

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
    assert!(restored.observation_id().is_none());

    let lifted = evidence
        .lift_observations(&mut observations, &sources)
        .expect("lift should succeed");
    assert!(
        lifted.is_empty(),
        "an explicit observation_id is never overridden"
    );
    assert_eq!(observations.len(), 1);

    let unregistered = evidence.attach_evidence(
        evidence_id("evidence--explicit"),
        EvidenceAttachmentTarget::observation(observation_id("observation--1")),
    );
    assert!(matches!(
        unregistered,
        Err(GraphError::ObservationNotFound(id)) if id == observation_id("observation--1")
    ));

    evidence.register_observation_target(observation_id("observation--1"));
    evidence
        .attach_evidence(
            evidence_id("evidence--explicit"),
            EvidenceAttachmentTarget::observation(observation_id("observation--1")),
        )
        .expect("attachment to a registered observation should succeed");
    assert_eq!(evidence.attachments().len(), 1);
    assert_eq!(
        evidence.attachments()[0].target(),
        &EvidenceAttachmentTarget::observation(observation_id("observation--1"))
    );
}

//
// Verify the graph-facing projection: an observation renders as additive,
// namespaced `observation_*` properties, with optional fields omitted.
#[test]
fn observation_projects_to_namespaced_properties() {
    let sources = sources_with_report();
    let mut store = ObservationStore::new();
    store
        .create_observation(span_input("observation--1"), &sources)
        .expect("observation should be created");
    let observation = store
        .observation_by_id(&observation_id("observation--1"))
        .expect("observation should exist");

    let properties = observation.to_property_map();
    assert_eq!(
        properties.get("observation_id"),
        Some(&PropertyValue::String("observation--1".to_owned()))
    );
    assert_eq!(
        properties.get("observation_source"),
        Some(&PropertyValue::String("source--report".to_owned()))
    );
    assert_eq!(
        properties.get("observation_modality"),
        Some(&PropertyValue::String("text".to_owned()))
    );
    assert_eq!(
        properties.get("observation_selector"),
        Some(&PropertyValue::String(
            "character_span:1204-1276".to_owned()
        ))
    );
    assert_eq!(
        properties.get("observation_observed_at"),
        Some(&PropertyValue::String("2026-08-30T10:05:00Z".to_owned()))
    );
    assert_eq!(
        properties.get("observation_payload_sha256"),
        Some(&PropertyValue::String(HASH_P.to_owned()))
    );
    assert_eq!(
        properties.get("observation_derived_from_legacy"),
        Some(&PropertyValue::Bool(false))
    );
    assert!(!properties.contains_key("observation_supersedes"));
    assert!(
        !properties.contains_key("observation_payload"),
        "the verbatim payload is not duplicated into node properties"
    );
    assert!(properties.keys().all(|key| key.starts_with("observation_")));
}

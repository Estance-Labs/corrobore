// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//! Contract for evidence statistics on memory records and relationships (#310).
//!
//! `evidence_count`, `distinct_observation_days`, `first_observed_at`, and
//! `last_observed_at` are read-only derivations of the provenance the engine
//! already owns. An entry's effective observation time is its `observed_at`,
//! or the time the engine first appended it when the source gave none.
#![allow(clippy::unwrap_used)]

use chrono::{DateTime, Utc};
use corrobore_engine::{
    CorroboreEngine, MemoryContent, MemoryGetRequest, MemoryLifecycle, MemoryOperation,
    MemoryPermissions, MemoryRecord, MemoryRelationship, MemoryRequest, MemoryResponse,
    MemoryServiceContext, MemoryTarget, MemoryUpdateRequest, ProvenanceReference, RelateRequest,
    RememberRequest, UpdatePatch,
};

fn context() -> MemoryServiceContext {
    MemoryServiceContext::new(
        "workspace--evidence",
        "actor--evidence",
        None,
        "session--evidence",
        MemoryPermissions::all(),
        "request--evidence",
        "correlation--evidence",
    )
    .unwrap()
}

fn source(id: &str, observed_at: Option<&str>) -> ProvenanceReference {
    ProvenanceReference {
        source_id: id.to_owned(),
        locator: None,
        observed_at: observed_at.map(str::to_owned),
    }
}

fn remember(
    engine: &mut CorroboreEngine,
    key: &str,
    idempotency: &str,
    provenance: Vec<ProvenanceReference>,
) -> MemoryRecord {
    let request = MemoryRequest::new(MemoryOperation::Remember(RememberRequest {
        identity_key: Some(key.to_owned()),
        kind: "fact".to_owned(),
        schema_version: "1".to_owned(),
        content: MemoryContent::Text(format!("fact {key}")),
        provenance,
        confidence: None,
        valid_from: None,
        valid_until: None,
        expires_at: None,
        tags: vec![],
        durable: None,
    }))
    .with_idempotency_key(idempotency);
    match engine.execute_memory(&context(), &request).unwrap() {
        MemoryResponse::Remember { record, .. } => record,
        other => panic!("expected remember, got {other:?}"),
    }
}

fn utc(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .unwrap()
        .with_timezone(&Utc)
}

#[test]
fn statistics_count_distinct_sources_and_distinct_utc_days() {
    let mut engine = CorroboreEngine::strict_default();
    // Three conversations on two UTC days: 23:30 at UTC-02:00 is already the
    // next UTC day, and a repeated source counts once.
    remember(
        &mut engine,
        "fact:beer",
        "remember:1",
        vec![
            source("conversation--1", Some("2026-09-01T10:00:00Z")),
            source("conversation--2", Some("2026-09-01T23:30:00-02:00")),
        ],
    );
    let record = remember(
        &mut engine,
        "fact:beer",
        "remember:2",
        vec![
            source("conversation--3", Some("2026-09-02T08:00:00+02:00")),
            source("conversation--1", Some("2026-09-05T08:00:00Z")),
        ],
    );

    assert_eq!(record.provenance.len(), 4);
    assert_eq!(record.evidence_count, 3, "distinct source_id");
    // 2026-09-01 (conversation--1), 2026-09-02 (both shifted entries), and
    // 2026-09-05 (the repeated source on another day).
    assert_eq!(record.distinct_observation_days, 3);
    assert_eq!(
        utc(record.first_observed_at.as_deref().unwrap()),
        utc("2026-09-01T10:00:00Z")
    );
    assert_eq!(
        utc(record.last_observed_at.as_deref().unwrap()),
        utc("2026-09-05T08:00:00Z")
    );
    // Statistics are rendered in UTC.
    assert!(record.first_observed_at.as_deref().unwrap().ends_with('Z'));
}

#[test]
fn entries_without_observed_at_fall_back_to_their_recorded_time() {
    let mut engine = CorroboreEngine::strict_default();
    let before = Utc::now();
    let record = remember(
        &mut engine,
        "fact:undated",
        "remember:undated",
        vec![
            source("note--1", None),
            source("note--2", Some("2026-01-15T12:00:00Z")),
        ],
    );
    let after = Utc::now();
    assert_eq!(record.evidence_count, 2);
    assert_eq!(record.distinct_observation_days, 2);
    assert_eq!(
        utc(record.first_observed_at.as_deref().unwrap()),
        utc("2026-01-15T12:00:00Z")
    );
    let last = utc(record.last_observed_at.as_deref().unwrap());
    assert!(
        last >= before - chrono::Duration::seconds(1) && last <= after,
        "the undated entry is dated by the time the engine appended it"
    );

    // A record without provenance has no observation bounds.
    let empty = remember(&mut engine, "fact:empty", "remember:empty", vec![]);
    assert_eq!(empty.evidence_count, 0);
    assert_eq!(empty.distinct_observation_days, 0);
    assert!(empty.first_observed_at.is_none());
    assert!(empty.last_observed_at.is_none());
}

#[test]
fn update_relationships_and_get_expose_the_same_statistics() {
    let mut engine = CorroboreEngine::strict_default();
    let context = context();
    let alice = remember(
        &mut engine,
        "person:alice",
        "remember:alice",
        vec![source("conversation--1", Some("2026-09-01T10:00:00Z"))],
    );
    let bob = remember(
        &mut engine,
        "person:bob",
        "remember:bob",
        vec![source("conversation--1", Some("2026-09-01T10:00:00Z"))],
    );
    let updated = match engine
        .execute_memory(
            &context,
            &MemoryRequest::new(MemoryOperation::Update(MemoryUpdateRequest {
                target: MemoryTarget::Memory(alice.id.clone()),
                expected_version: None,
                patch: UpdatePatch {
                    add_provenance: vec![source("conversation--2", Some("2026-09-03T10:00:00Z"))],
                    ..UpdatePatch::default()
                },
            }))
            .with_idempotency_key("update:alice"),
        )
        .unwrap()
    {
        MemoryResponse::Update { record, .. } => record,
        other => panic!("expected update, got {other:?}"),
    };
    assert_eq!(updated.evidence_count, 2);
    assert_eq!(updated.distinct_observation_days, 2);

    let relate = |key: &str, provenance: Vec<ProvenanceReference>| {
        MemoryRequest::new(MemoryOperation::Relate(RelateRequest {
            identity_key: Some("alice-knows-bob".to_owned()),
            source_id: alice.id.clone(),
            target_id: bob.id.clone(),
            kind: "knows".to_owned(),
            properties: serde_json::json!({}),
            provenance,
            confidence: None,
            valid_from: None,
            valid_until: None,
            expires_at: None,
            lifecycle: MemoryLifecycle::Active,
        }))
        .with_idempotency_key(key)
    };
    engine
        .execute_memory(
            &context,
            &relate(
                "relate:1",
                vec![source("conversation--1", Some("2026-09-01T10:00:00Z"))],
            ),
        )
        .unwrap();
    let relationship: MemoryRelationship = match engine
        .execute_memory(
            &context,
            &relate(
                "relate:2",
                vec![
                    source("conversation--1", Some("2026-09-01T10:00:00Z")),
                    source("conversation--4", Some("2026-09-04T10:00:00Z")),
                ],
            ),
        )
        .unwrap()
    {
        MemoryResponse::Relate { relationship, .. } => relationship,
        other => panic!("expected relate, got {other:?}"),
    };
    assert_eq!(relationship.provenance.len(), 2);
    assert_eq!(relationship.evidence_count, 2);
    assert_eq!(relationship.distinct_observation_days, 2);
    assert_eq!(
        utc(relationship.last_observed_at.as_deref().unwrap()),
        utc("2026-09-04T10:00:00Z")
    );

    let lookup = match engine
        .execute_memory(
            &context,
            &MemoryRequest::new(MemoryOperation::Get(MemoryGetRequest {
                identity_keys: vec!["person:alice".to_owned()],
                relationship_identity_keys: vec!["alice-knows-bob".to_owned()],
                ..MemoryGetRequest::default()
            })),
        )
        .unwrap()
    {
        MemoryResponse::Get(result) => result,
        other => panic!("expected get, got {other:?}"),
    };
    assert_eq!(lookup.records[0].evidence_count, 2);
    assert_eq!(lookup.records[0].distinct_observation_days, 2);
    assert_eq!(lookup.relationships[0].evidence_count, 2);
}

#[test]
fn statistics_are_response_only_and_legacy_records_still_deserialize() {
    let legacy = serde_json::json!({
        "id": "node--1",
        "identity_key": null,
        "kind": "fact",
        "schema_version": "1",
        "content": {"format": "text", "value": "legacy"},
        "provenance": [{"source_id": "s", "locator": null, "observed_at": null}],
        "confidence": null,
        "valid_from": null,
        "valid_until": null,
        "recorded_at": "2026-07-26T00:00:00Z",
        "expires_at": null,
        "lifecycle": "active",
        "version": 1,
        "tags": []
    });
    let record: MemoryRecord = serde_json::from_value(legacy).unwrap();
    assert_eq!(record.evidence_count, 0);
    assert!(record.first_observed_at.is_none());

    // No request type accepts the derived fields.
    let rejected = serde_json::from_value::<MemoryRequest>(serde_json::json!({
        "contract_version": "v1",
        "idempotency_key": "k",
        "operation": "remember",
        "input": {
            "identity_key": null,
            "kind": "fact",
            "schema_version": "1",
            "content": {"format": "text", "value": "x"},
            "provenance": [],
            "confidence": null,
            "valid_from": null,
            "valid_until": null,
            "expires_at": null,
            "tags": [],
            "evidence_count": 5
        }
    }));
    assert!(rejected.is_err());
}

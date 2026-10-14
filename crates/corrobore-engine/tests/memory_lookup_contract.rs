// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//! Contract for the `get` memory operation (#309).
//!
//! A client that wrote a memory or relationship under an application identity
//! key must be able to read the current record back by that key, or by the
//! engine id, without mirroring engine ids locally. Lifecycle stays visible so
//! an expired or superseded record is distinguishable from an active one, and
//! everything outside the trusted workspace uses the not-found surface.
#![allow(clippy::unwrap_used)]

use corrobore_engine::{
    ConsolidateMode, ConsolidateRequest, CorroboreEngine, ForgetMode, ForgetRequest, MemoryContent,
    MemoryErrorCode, MemoryGetRequest, MemoryGetResult, MemoryLifecycle, MemoryLookupField,
    MemoryOperation, MemoryPermissions, MemoryRequest, MemoryResponse, MemoryServiceContext,
    ProvenanceReference, RelateRequest, RememberRequest,
};

fn context(workspace: &str, permissions: MemoryPermissions) -> MemoryServiceContext {
    MemoryServiceContext::new(
        workspace,
        "actor--lookup",
        None,
        "session--lookup",
        permissions,
        "request--lookup",
        "correlation--lookup",
    )
    .unwrap()
}

fn remember(engine: &mut CorroboreEngine, context: &MemoryServiceContext, key: &str) -> String {
    let request = MemoryRequest::new(MemoryOperation::Remember(RememberRequest {
        identity_key: Some(key.to_owned()),
        kind: "entity".to_owned(),
        schema_version: "1".to_owned(),
        content: MemoryContent::Text(format!("memory {key}")),
        provenance: vec![ProvenanceReference {
            source_id: format!("source--{key}"),
            locator: None,
            observed_at: Some("2026-07-26T10:00:00Z".to_owned()),
        }],
        confidence: Some(0.7),
        valid_from: None,
        valid_until: None,
        expires_at: None,
        tags: vec![],
        durable: None,
    }))
    .with_idempotency_key(format!("remember:{key}"));
    match engine.execute_memory(context, &request).unwrap() {
        MemoryResponse::Remember { record, .. } => record.id,
        other => panic!("expected remember, got {other:?}"),
    }
}

fn relate(
    engine: &mut CorroboreEngine,
    context: &MemoryServiceContext,
    key: &str,
    source: &str,
    target: &str,
) -> String {
    let request = MemoryRequest::new(MemoryOperation::Relate(RelateRequest {
        identity_key: Some(key.to_owned()),
        source_id: source.to_owned(),
        target_id: target.to_owned(),
        kind: "knows".to_owned(),
        properties: serde_json::json!({}),
        provenance: vec![ProvenanceReference {
            source_id: format!("source--{key}"),
            locator: None,
            observed_at: None,
        }],
        confidence: None,
        valid_from: None,
        valid_until: None,
        expires_at: None,
        lifecycle: MemoryLifecycle::Active,
    }))
    .with_idempotency_key(format!("relate:{key}"));
    match engine.execute_memory(context, &request).unwrap() {
        MemoryResponse::Relate { relationship, .. } => relationship.id,
        other => panic!("expected relate, got {other:?}"),
    }
}

fn get(
    engine: &mut CorroboreEngine,
    context: &MemoryServiceContext,
    input: MemoryGetRequest,
) -> MemoryGetResult {
    match engine
        .execute_memory(context, &MemoryRequest::new(MemoryOperation::Get(input)))
        .unwrap()
    {
        MemoryResponse::Get(result) => result,
        other => panic!("expected get, got {other:?}"),
    }
}

#[test]
fn get_resolves_memories_and_relationships_by_identity_key_and_id_in_request_order() {
    let mut engine = CorroboreEngine::strict_default();
    let context = context("workspace--lookup", MemoryPermissions::all());
    let alice = remember(&mut engine, &context, "person:alice");
    let bob = remember(&mut engine, &context, "person:bob");
    let knows = relate(&mut engine, &context, "alice-knows-bob", &alice, &bob);

    let result = get(
        &mut engine,
        &context,
        MemoryGetRequest {
            identity_keys: vec!["person:bob".to_owned(), "person:unknown".to_owned()],
            ids: vec![alice.clone(), bob.clone(), "node--absent".to_owned()],
            relationship_identity_keys: vec!["alice-knows-bob".to_owned()],
            relationship_ids: vec![knows.clone(), "rel--absent".to_owned()],
        },
    );

    // Identity keys resolve first, then ids; a record requested twice is
    // returned once, at its first position.
    let ids = result
        .records
        .iter()
        .map(|record| record.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, vec![bob.as_str(), alice.as_str()]);
    assert_eq!(
        result.records[0].identity_key.as_deref(),
        Some("person:bob")
    );
    assert!(result.records.iter().all(|record| record.recallable));
    assert!(
        result
            .records
            .iter()
            .all(|record| record.superseded_by.is_none())
    );
    assert_eq!(result.relationships.len(), 1);
    assert_eq!(result.relationships[0].id, knows);
    assert_eq!(
        result.relationships[0].identity_key.as_deref(),
        Some("alice-knows-bob")
    );
    assert!(result.relationships[0].recallable);

    let missing = result
        .missing
        .iter()
        .map(|miss| (miss.field, miss.value.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        missing,
        vec![
            (MemoryLookupField::IdentityKeys, "person:unknown"),
            (MemoryLookupField::Ids, "node--absent"),
            (MemoryLookupField::RelationshipIds, "rel--absent"),
        ]
    );
}

#[test]
fn get_keeps_expired_and_superseded_records_visible_with_their_lifecycle() {
    let mut engine = CorroboreEngine::strict_default();
    let context = context("workspace--lifecycle", MemoryPermissions::all());
    let expired = remember(&mut engine, &context, "fact:expired");
    let canonical = remember(&mut engine, &context, "entity:canonical");
    let duplicate = remember(&mut engine, &context, "entity:duplicate");
    let deleted = remember(&mut engine, &context, "fact:deleted");

    engine
        .execute_memory(
            &context,
            &MemoryRequest::new(MemoryOperation::Forget(ForgetRequest {
                memory_id: expired.clone(),
                mode: ForgetMode::Expire,
                expires_at: Some("2026-08-01T00:00:00Z".to_owned()),
                reason: "retention".to_owned(),
            }))
            .with_idempotency_key("forget:expired"),
        )
        .unwrap();
    engine
        .execute_memory(
            &context,
            &MemoryRequest::new(MemoryOperation::Forget(ForgetRequest {
                memory_id: deleted.clone(),
                mode: ForgetMode::ApplicationDelete,
                expires_at: None,
                reason: "user request".to_owned(),
            }))
            .with_idempotency_key("forget:deleted"),
        )
        .unwrap();
    let consolidate = |mode| {
        MemoryRequest::new(MemoryOperation::Consolidate(ConsolidateRequest {
            mode,
            memory_ids: vec![canonical.clone(), duplicate.clone()],
            canonical_id: Some(canonical.clone()),
            reason: "same entity".to_owned(),
            preserve_disagreements: true,
            authority_policy: None,
            revoked_source_ids: vec![],
        }))
    };
    let proposal_id = match engine
        .execute_memory(&context, &consolidate(ConsolidateMode::Propose))
        .unwrap()
    {
        MemoryResponse::Consolidate(result) => result.proposal_id,
        other => panic!("expected consolidation, got {other:?}"),
    };
    engine
        .execute_memory(
            &context,
            &consolidate(ConsolidateMode::ApplyApproved {
                proposal_id,
                approval_policy: "policy--test".to_owned(),
            })
            .with_idempotency_key("consolidate:apply"),
        )
        .unwrap();

    let result = get(
        &mut engine,
        &context,
        MemoryGetRequest {
            identity_keys: vec![
                "fact:expired".to_owned(),
                "entity:duplicate".to_owned(),
                "entity:canonical".to_owned(),
                "fact:deleted".to_owned(),
            ],
            ..MemoryGetRequest::default()
        },
    );
    let lifecycles = result
        .records
        .iter()
        .map(|record| {
            (
                record.id.as_str(),
                record.lifecycle,
                record.recallable,
                record.superseded_by.as_deref(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        lifecycles,
        vec![
            (expired.as_str(), MemoryLifecycle::Expired, false, None),
            (
                duplicate.as_str(),
                MemoryLifecycle::Superseded,
                false,
                Some(canonical.as_str())
            ),
            (canonical.as_str(), MemoryLifecycle::Active, true, None),
        ]
    );
    // Application deletion removes the record from ordinary reads.
    assert_eq!(result.missing.len(), 1);
    assert_eq!(result.missing[0].field, MemoryLookupField::IdentityKeys);
    assert_eq!(result.missing[0].value, "fact:deleted");
}

#[test]
fn get_uses_the_not_found_surface_for_other_workspaces() {
    let mut engine = CorroboreEngine::strict_default();
    let alpha = context("workspace--alpha", MemoryPermissions::all());
    let beta = context("workspace--beta", MemoryPermissions::all());
    let alpha_id = remember(&mut engine, &alpha, "person:shared-key");
    let other_id = remember(&mut engine, &alpha, "person:other");
    let rel_id = relate(&mut engine, &alpha, "rel:shared-key", &alpha_id, &other_id);

    let result = get(
        &mut engine,
        &beta,
        MemoryGetRequest {
            identity_keys: vec!["person:shared-key".to_owned()],
            ids: vec![alpha_id.clone()],
            relationship_identity_keys: vec!["rel:shared-key".to_owned()],
            relationship_ids: vec![rel_id.clone()],
        },
    );
    assert!(result.records.is_empty());
    assert!(result.relationships.is_empty());
    assert_eq!(result.missing.len(), 4);
}

#[test]
fn get_requires_read_permission_and_bounded_non_blank_values() {
    let mut engine = CorroboreEngine::strict_default();
    let write_only = context(
        "workspace--permissions",
        MemoryPermissions {
            read: false,
            ..MemoryPermissions::all()
        },
    );
    let denied = engine
        .execute_memory(
            &write_only,
            &MemoryRequest::new(MemoryOperation::Get(MemoryGetRequest {
                ids: vec!["node--1".to_owned()],
                ..MemoryGetRequest::default()
            })),
        )
        .unwrap_err();
    assert_eq!(denied.code, MemoryErrorCode::PermissionDenied);

    let reader = context("workspace--permissions", MemoryPermissions::read_only());
    // A read needs no idempotency key and is not a mutation.
    let empty = engine
        .execute_memory(
            &reader,
            &MemoryRequest::new(MemoryOperation::Get(MemoryGetRequest::default())),
        )
        .unwrap_err();
    assert_eq!(empty.code, MemoryErrorCode::InvalidRequest);

    let too_many = engine
        .execute_memory(
            &reader,
            &MemoryRequest::new(MemoryOperation::Get(MemoryGetRequest {
                identity_keys: (0..60).map(|index| format!("key-{index}")).collect(),
                relationship_ids: (0..41).map(|index| format!("rel-{index}")).collect(),
                ..MemoryGetRequest::default()
            })),
        )
        .unwrap_err();
    assert_eq!(too_many.code, MemoryErrorCode::InvalidRequest);

    let blank = engine
        .execute_memory(
            &reader,
            &MemoryRequest::new(MemoryOperation::Get(MemoryGetRequest {
                identity_keys: vec!["  ".to_owned()],
                ..MemoryGetRequest::default()
            })),
        )
        .unwrap_err();
    assert_eq!(blank.code, MemoryErrorCode::InvalidRequest);

    let hundred = engine
        .execute_memory(
            &reader,
            &MemoryRequest::new(MemoryOperation::Get(MemoryGetRequest {
                ids: (0..100).map(|index| format!("node--{index}")).collect(),
                ..MemoryGetRequest::default()
            })),
        )
        .expect("exactly 100 values stay within the bound");
    assert!(matches!(hundred, MemoryResponse::Get(result) if result.missing.len() == 100));
}

#[test]
fn get_wire_shape_defaults_omitted_fields_and_rejects_unknown_ones() {
    let request: MemoryRequest = serde_json::from_value(serde_json::json!({
        "contract_version": "v1",
        "operation": "get",
        "input": {"identity_keys": ["person:alice"]}
    }))
    .expect("omitted lookup arrays default to empty");
    assert!(matches!(
        request.operation,
        MemoryOperation::Get(MemoryGetRequest { ref identity_keys, ref ids, .. })
            if identity_keys == &vec!["person:alice".to_owned()] && ids.is_empty()
    ));
    // Serializing omits the empty arrays, so the canonical form stays minimal.
    assert_eq!(
        serde_json::to_value(&request).unwrap()["input"],
        serde_json::json!({"identity_keys": ["person:alice"]})
    );

    let unknown = serde_json::from_value::<MemoryRequest>(serde_json::json!({
        "contract_version": "v1",
        "operation": "get",
        "input": {"identity_keys": ["person:alice"], "workspace_id": "workspace--attacker"}
    }));
    assert!(unknown.is_err(), "unknown get fields are rejected");

    let miss = serde_json::to_value(corrobore_engine::MemoryGetMiss {
        field: MemoryLookupField::RelationshipIdentityKeys,
        value: "k".to_owned(),
    })
    .unwrap();
    assert_eq!(
        miss,
        serde_json::json!({"field": "relationship_identity_keys", "value": "k"})
    );
}

#[test]
fn records_written_before_lookup_fields_existed_still_deserialize() {
    // Durable idempotency receipts store full responses. A record serialized
    // before `superseded_by` and `recallable` existed must still replay.
    let legacy = serde_json::json!({
        "id": "node--1",
        "identity_key": "k",
        "kind": "entity",
        "schema_version": "1",
        "content": {"format": "text", "value": "legacy"},
        "provenance": [],
        "confidence": null,
        "valid_from": null,
        "valid_until": null,
        "recorded_at": "2026-07-26T00:00:00Z",
        "expires_at": null,
        "lifecycle": "active",
        "version": 1,
        "tags": []
    });
    let record: corrobore_engine::MemoryRecord = serde_json::from_value(legacy).unwrap();
    assert!(record.superseded_by.is_none());
    assert!(record.recallable);
}

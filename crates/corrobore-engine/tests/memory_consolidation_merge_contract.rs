// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//! Contract for consolidation that merges evidence (#312).
//!
//! With `merge_provenance: true`, an approved consolidation appends the
//! originals' provenance to the canonical memory, adds their heat, and
//! re-points their active relationships to the canonical memory, inside the
//! same commit. Without it, behaviour and proposal identities are unchanged.
#![allow(clippy::unwrap_used)]

use corrobore_engine::{
    ConsolidateMode, ConsolidateRequest, ConsolidationResult, CorroboreEngine, MemoryContent,
    MemoryErrorCode, MemoryGetRequest, MemoryGetResult, MemoryLifecycle, MemoryOperation,
    MemoryPermissions, MemoryRequest, MemoryResponse, MemoryServiceContext, ProvenanceReference,
    RelateRequest, RememberRequest,
};

fn context() -> MemoryServiceContext {
    MemoryServiceContext::new(
        "workspace--merge",
        "actor--merge",
        None,
        "session--merge",
        MemoryPermissions::all(),
        "request--merge",
        "correlation--merge",
    )
    .unwrap()
}

fn source(id: &str, observed_at: &str) -> ProvenanceReference {
    ProvenanceReference {
        source_id: id.to_owned(),
        locator: None,
        observed_at: Some(observed_at.to_owned()),
    }
}

fn remember(
    engine: &mut CorroboreEngine,
    key: &str,
    provenance: Vec<ProvenanceReference>,
    tags: &[&str],
    durable: bool,
) -> String {
    let request = MemoryRequest::new(MemoryOperation::Remember(RememberRequest {
        identity_key: Some(key.to_owned()),
        kind: "person".to_owned(),
        schema_version: "1".to_owned(),
        content: MemoryContent::Text(format!("person {key}")),
        provenance,
        confidence: Some(0.6),
        valid_from: None,
        valid_until: None,
        expires_at: None,
        tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
        durable: Some(durable),
    }))
    .with_idempotency_key(format!("remember:{key}"));
    match engine.execute_memory(&context(), &request).unwrap() {
        MemoryResponse::Remember { record, .. } => record.id,
        other => panic!("expected remember, got {other:?}"),
    }
}

fn relate(
    engine: &mut CorroboreEngine,
    key: Option<&str>,
    from: &str,
    to: &str,
    kind: &str,
    provenance: Vec<ProvenanceReference>,
) -> String {
    let request = MemoryRequest::new(MemoryOperation::Relate(RelateRequest {
        identity_key: key.map(str::to_owned),
        source_id: from.to_owned(),
        target_id: to.to_owned(),
        kind: kind.to_owned(),
        properties: serde_json::json!({"origin": key}),
        provenance,
        confidence: Some(0.7),
        valid_from: None,
        valid_until: None,
        expires_at: None,
        lifecycle: MemoryLifecycle::Active,
    }))
    .with_idempotency_key(format!("relate:{from}:{to}:{kind}:{key:?}"));
    match engine.execute_memory(&context(), &request).unwrap() {
        MemoryResponse::Relate { relationship, .. } => relationship.id,
        other => panic!("expected relate, got {other:?}"),
    }
}

fn consolidate_request(
    ids: &[&str],
    canonical: &str,
    merge: bool,
    mode: ConsolidateMode,
) -> MemoryRequest {
    MemoryRequest::new(MemoryOperation::Consolidate(ConsolidateRequest {
        mode,
        memory_ids: ids.iter().map(|id| (*id).to_owned()).collect(),
        canonical_id: Some(canonical.to_owned()),
        reason: "same person".to_owned(),
        preserve_disagreements: true,
        authority_policy: None,
        revoked_source_ids: vec![],
        merge_provenance: merge,
    }))
}

fn consolidated(response: MemoryResponse) -> ConsolidationResult {
    match response {
        MemoryResponse::Consolidate(result) => result,
        other => panic!("expected consolidation, got {other:?}"),
    }
}

fn apply(
    engine: &mut CorroboreEngine,
    ids: &[&str],
    canonical: &str,
    merge: bool,
) -> ConsolidationResult {
    let proposal = consolidated(
        engine
            .execute_memory(
                &context(),
                &consolidate_request(ids, canonical, merge, ConsolidateMode::Propose),
            )
            .unwrap(),
    );
    consolidated(
        engine
            .execute_memory(
                &context(),
                &consolidate_request(
                    ids,
                    canonical,
                    merge,
                    ConsolidateMode::ApplyApproved {
                        proposal_id: proposal.proposal_id,
                        approval_policy: "policy--test".to_owned(),
                    },
                )
                .with_idempotency_key(format!("consolidate:{merge}")),
            )
            .unwrap(),
    )
}

fn get(engine: &mut CorroboreEngine, input: MemoryGetRequest) -> MemoryGetResult {
    match engine
        .execute_memory(&context(), &MemoryRequest::new(MemoryOperation::Get(input)))
        .unwrap()
    {
        MemoryResponse::Get(result) => result,
        other => panic!("expected get, got {other:?}"),
    }
}

#[test]
fn without_the_option_consolidation_and_its_proposal_identity_are_unchanged() {
    let mut engine = CorroboreEngine::strict_default();
    let a = remember(
        &mut engine,
        "person:a",
        vec![source("c1", "2026-09-01T00:00:00Z")],
        &[],
        false,
    );
    let b = remember(
        &mut engine,
        "person:b",
        vec![source("c2", "2026-09-02T00:00:00Z")],
        &[],
        false,
    );
    // The serialized request omits the default flag, so request hashes and
    // proposal identities of existing clients do not move.
    let plain = consolidate_request(&[&a, &b], &a, false, ConsolidateMode::Propose);
    assert!(
        serde_json::to_value(&plain).unwrap()["input"]
            .get("merge_provenance")
            .is_none()
    );
    let plain_id = consolidated(engine.execute_memory(&context(), &plain).unwrap()).proposal_id;
    let merging = consolidated(
        engine
            .execute_memory(
                &context(),
                &consolidate_request(&[&a, &b], &a, true, ConsolidateMode::Propose),
            )
            .unwrap(),
    );
    assert_ne!(plain_id, merging.proposal_id, "merging is its own decision");
    // A proposal reports what would merge and writes nothing.
    assert_eq!(merging.merged_provenance_count, 1);
    let canonical = get(
        &mut engine,
        MemoryGetRequest {
            ids: vec![a.clone()],
            ..MemoryGetRequest::default()
        },
    );
    assert_eq!(canonical.records[0].provenance.len(), 1);

    // An approval for the plain consolidation never covers a merging one.
    let mismatch = engine
        .execute_memory(
            &context(),
            &consolidate_request(
                &[&a, &b],
                &a,
                true,
                ConsolidateMode::ApplyApproved {
                    proposal_id: plain_id,
                    approval_policy: "policy--test".to_owned(),
                },
            )
            .with_idempotency_key("consolidate:mismatch"),
        )
        .unwrap_err();
    assert_eq!(mismatch.code, MemoryErrorCode::PolicyApprovalRequired);

    let applied = apply(&mut engine, &[&a, &b], &a, false);
    assert!(applied.applied);
    assert_eq!(applied.merged_provenance_count, 0);
    assert!(applied.repointed_relationships.is_empty());
    let canonical = get(
        &mut engine,
        MemoryGetRequest {
            ids: vec![a.clone()],
            ..MemoryGetRequest::default()
        },
    );
    assert_eq!(canonical.records[0].provenance.len(), 1, "nothing merged");
}

#[test]
fn merging_appends_deduplicated_evidence_sums_heat_and_merges_tags_and_durability() {
    let mut engine = CorroboreEngine::strict_default();
    let a = remember(
        &mut engine,
        "person:a",
        vec![
            source("c1", "2026-09-01T00:00:00Z"),
            source("c2", "2026-09-02T00:00:00Z"),
        ],
        &["friend"],
        false,
    );
    let b = remember(
        &mut engine,
        "person:b",
        vec![
            source("c2", "2026-09-02T00:00:00Z"),
            source("c3", "2026-09-03T00:00:00Z"),
        ],
        &["colleague"],
        true,
    );
    let before = get(
        &mut engine,
        MemoryGetRequest {
            ids: vec![a.clone(), b.clone()],
            ..MemoryGetRequest::default()
        },
    );
    let heat_sum = before.records[0].heat + before.records[1].heat;

    let result = apply(&mut engine, &[&a, &b], &a, true);
    assert!(result.applied && result.disagreements_retained);
    assert_eq!(result.merged_provenance_count, 1, "c2 is already present");

    let after = get(
        &mut engine,
        MemoryGetRequest {
            ids: vec![a.clone(), b.clone()],
            ..MemoryGetRequest::default()
        },
    );
    let canonical = &after.records[0];
    let sources = canonical
        .provenance
        .iter()
        .map(|entry| entry.source_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(sources, vec!["c1", "c2", "c3"]);
    assert_eq!(canonical.evidence_count, 3);
    assert_eq!(canonical.distinct_observation_days, 3);
    assert!((canonical.heat - heat_sum).abs() < 1e-3);
    assert_eq!(canonical.tags, vec!["friend", "colleague"]);
    assert!(canonical.durable, "durable if any original is durable");
    assert_eq!(
        canonical.confidence,
        Some(0.6),
        "confidence is not modified"
    );
    assert_eq!(
        canonical.content,
        MemoryContent::Text("person person:a".to_owned())
    );
    let original = &after.records[1];
    assert_eq!(original.lifecycle, MemoryLifecycle::Superseded);
    assert_eq!(original.superseded_by.as_deref(), Some(a.as_str()));
    assert_eq!(
        original.provenance.len(),
        2,
        "originals keep their evidence"
    );
}

#[test]
fn merging_repoints_relationships_merges_equivalents_and_skips_self_loops() {
    let mut engine = CorroboreEngine::strict_default();
    let a = remember(&mut engine, "person:a", vec![], &[], false);
    let b = remember(&mut engine, "person:b", vec![], &[], false);
    let beer = remember(&mut engine, "entity:beer", vec![], &[], false);
    let city = remember(&mut engine, "entity:city", vec![], &[], false);

    // a and b both like beer: the two relationships become one.
    let a_likes = relate(
        &mut engine,
        Some("a-likes-beer"),
        &a,
        &beer,
        "likes",
        vec![source("c1", "2026-09-01T00:00:00Z")],
    );
    let b_likes = relate(
        &mut engine,
        Some("b-likes-beer"),
        &b,
        &beer,
        "likes",
        vec![source("c2", "2026-09-02T00:00:00Z")],
    );
    // Only b lives in the city, and the city points back at b: both move.
    let b_lives = relate(
        &mut engine,
        Some("b-lives-city"),
        &b,
        &city,
        "lives_in",
        vec![source("c3", "2026-09-03T00:00:00Z")],
    );
    let city_hosts = relate(
        &mut engine,
        None,
        &city,
        &b,
        "hosts",
        vec![source("c4", "2026-09-04T00:00:00Z")],
    );
    // a and b are related to each other: re-pointing would be a self-loop.
    let a_b = relate(
        &mut engine,
        Some("a-same-as-b"),
        &a,
        &b,
        "same_as",
        vec![source("c5", "2026-09-05T00:00:00Z")],
    );

    let result = apply(&mut engine, &[&a, &b], &a, true);
    let mut repointed = result
        .repointed_relationships
        .iter()
        .map(|entry| (entry.original_id.clone(), entry.replacement_id.clone()))
        .collect::<Vec<_>>();
    repointed.sort();
    assert_eq!(repointed.len(), 3, "{repointed:?}");
    let replacement_of = |original: &str| {
        repointed
            .iter()
            .find(|(id, _)| id == original)
            .and_then(|(_, replacement)| replacement.clone())
            .unwrap()
    };
    assert_eq!(replacement_of(&b_likes), a_likes, "merged into equivalent");
    let lives = replacement_of(&b_lives);
    let hosts = replacement_of(&city_hosts);

    let lookup = get(
        &mut engine,
        MemoryGetRequest {
            relationship_ids: vec![
                a_likes.clone(),
                b_likes.clone(),
                lives.clone(),
                b_lives.clone(),
                hosts.clone(),
                a_b.clone(),
            ],
            ..MemoryGetRequest::default()
        },
    );
    let by_id = |id: &str| {
        lookup
            .relationships
            .iter()
            .find(|relationship| relationship.id == id)
            .unwrap()
    };
    let merged = by_id(&a_likes);
    assert_eq!(merged.lifecycle, MemoryLifecycle::Active);
    assert_eq!(merged.evidence_count, 2);
    assert_eq!(by_id(&b_likes).lifecycle, MemoryLifecycle::Superseded);

    let moved = by_id(&lives);
    assert_eq!(
        (moved.source_id.as_str(), moved.target_id.as_str()),
        (a.as_str(), city.as_str())
    );
    assert_eq!(moved.kind, "lives_in");
    assert_eq!(moved.evidence_count, 1);
    assert_eq!(
        moved.identity_key.as_deref(),
        Some("b-lives-city"),
        "key moves"
    );
    let old = by_id(&b_lives);
    assert_eq!(old.lifecycle, MemoryLifecycle::Superseded);
    assert!(old.identity_key.is_none());
    let moved_back = by_id(&hosts);
    assert_eq!(
        (moved_back.source_id.as_str(), moved_back.target_id.as_str()),
        (city.as_str(), a.as_str())
    );
    // The self-loop candidate is left untouched.
    assert_eq!(by_id(&a_b).lifecycle, MemoryLifecycle::Active);

    // The moved identity key resolves to the replacement.
    let by_key = get(
        &mut engine,
        MemoryGetRequest {
            relationship_identity_keys: vec!["b-lives-city".to_owned()],
            ..MemoryGetRequest::default()
        },
    );
    assert_eq!(by_key.relationships[0].id, lives);
}

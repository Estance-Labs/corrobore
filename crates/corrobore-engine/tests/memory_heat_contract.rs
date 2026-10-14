// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//! Contract for reinforcement and temporal decay ("heat") on memories and
//! relationships, with opt-in cold expiry (#311).
//!
//! Heat follows the pheromone law with a wall-clock half-life: a stored value
//! decays lazily as `heat · 0.5^(Δt / half_life)`. Every newly appended
//! provenance entry deposits the evidence reward decayed from its observation
//! time, and `reinforce` deposits an explicit usage amount.
#![allow(clippy::unwrap_used)]

use chrono::{Duration, SecondsFormat, Utc};
use corrobore_engine::{
    CorroboreEngine, MemoryContent, MemoryErrorCode, MemoryGetRequest, MemoryHeatPolicy,
    MemoryLifecycle, MemoryLimits, MemoryOperation, MemoryPermissions, MemoryRecord, MemoryRequest,
    MemoryResponse, MemoryServiceContext, MemoryTarget, MemoryUpdateRequest, ProvenanceReference,
    RecallRequest, ReinforceRequest, RelateRequest, RememberRequest, UpdatePatch,
};

const TOLERANCE: f64 = 1e-3;

fn context(permissions: MemoryPermissions) -> MemoryServiceContext {
    MemoryServiceContext::new(
        "workspace--heat",
        "actor--heat",
        None,
        "session--heat",
        permissions,
        "request--heat",
        "correlation--heat",
    )
    .unwrap()
}

fn days_ago(days: i64) -> String {
    (Utc::now() - Duration::days(days)).to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn source(id: &str, observed_at: Option<String>) -> ProvenanceReference {
    ProvenanceReference {
        source_id: id.to_owned(),
        locator: None,
        observed_at,
    }
}

fn remember_request(
    key: &str,
    text: &str,
    idempotency: &str,
    provenance: Vec<ProvenanceReference>,
    durable: Option<bool>,
) -> MemoryRequest {
    MemoryRequest::new(MemoryOperation::Remember(RememberRequest {
        identity_key: Some(key.to_owned()),
        kind: "fact".to_owned(),
        schema_version: "1".to_owned(),
        content: MemoryContent::Text(text.to_owned()),
        provenance,
        confidence: None,
        valid_from: None,
        valid_until: None,
        expires_at: None,
        tags: vec![],
        durable,
    }))
    .with_idempotency_key(idempotency)
}

fn remember(engine: &mut CorroboreEngine, request: &MemoryRequest) -> MemoryRecord {
    match engine
        .execute_memory(&context(MemoryPermissions::all()), request)
        .unwrap()
    {
        MemoryResponse::Remember { record, .. } => record,
        other => panic!("expected remember, got {other:?}"),
    }
}

fn reinforce(
    engine: &mut CorroboreEngine,
    target: MemoryTarget,
    amount: f64,
    key: &str,
) -> MemoryResponse {
    engine
        .execute_memory(
            &context(MemoryPermissions::all()),
            &MemoryRequest::new(MemoryOperation::Reinforce(ReinforceRequest {
                target,
                amount,
                reason: Some("used in an answer".to_owned()),
            }))
            .with_idempotency_key(key),
        )
        .unwrap()
}

fn recall_ids(engine: &mut CorroboreEngine, objective: &str) -> Vec<String> {
    match engine
        .execute_memory(
            &context(MemoryPermissions::all()),
            &MemoryRequest::new(MemoryOperation::Recall(RecallRequest {
                objective: objective.to_owned(),
                seed_ids: vec![],
                limits: MemoryLimits::strict_default(),
                page_token: None,
            })),
        )
        .unwrap()
    {
        MemoryResponse::Recall(result) => result
            .items
            .into_iter()
            .map(|item| item.record.id)
            .collect(),
        other => panic!("expected recall, got {other:?}"),
    }
}

fn get_record(engine: &mut CorroboreEngine, id: &str) -> MemoryRecord {
    match engine
        .execute_memory(
            &context(MemoryPermissions::all()),
            &MemoryRequest::new(MemoryOperation::Get(MemoryGetRequest {
                ids: vec![id.to_owned()],
                ..MemoryGetRequest::default()
            })),
        )
        .unwrap()
    {
        MemoryResponse::Get(mut result) => result.records.remove(0),
        other => panic!("expected get, got {other:?}"),
    }
}

fn engine_with(policy: MemoryHeatPolicy) -> CorroboreEngine {
    CorroboreEngine::builder()
        .memory_heat_policy(policy)
        .build()
        .unwrap()
}

#[test]
fn heat_policy_defaults_and_validation() {
    let policy = MemoryHeatPolicy::default();
    assert!((policy.half_life_days() - 21.0).abs() < f64::EPSILON);
    assert!((policy.evidence_reward() - 1.0).abs() < f64::EPSILON);
    assert!(policy.cold_threshold().abs() < f64::EPSILON);
    // λ per day = 0.5^(1/21), validated through the pheromone decay type.
    assert!((policy.daily_decay().value() - 0.5_f64.powf(1.0 / 21.0)).abs() < 1e-12);

    for (half_life, reward, cold) in [
        (0.0, 1.0, 0.0),
        (-1.0, 1.0, 0.0),
        (f64::NAN, 1.0, 0.0),
        (f64::INFINITY, 1.0, 0.0),
        (21.0, -1.0, 0.0),
        (21.0, f64::NAN, 0.0),
        (21.0, 1.0, -0.1),
        (21.0, 1.0, f64::INFINITY),
    ] {
        assert!(
            MemoryHeatPolicy::new(half_life, reward, cold).is_err(),
            "({half_life}, {reward}, {cold}) must be rejected"
        );
    }
    assert!(MemoryHeatPolicy::new(7.0, 2.0, 0.5).is_ok());
}

#[test]
fn new_evidence_deposits_the_reward_decayed_from_its_observation_time() {
    let mut engine = CorroboreEngine::strict_default();
    let first = remember(
        &mut engine,
        &remember_request(
            "fact:old",
            "old fact",
            "remember:1",
            vec![source("conversation--1", Some(days_ago(21)))],
            None,
        ),
    );
    // One half-life ago: half the default reward of 1.0.
    assert!((first.heat - 0.5).abs() < TOLERANCE, "{}", first.heat);
    assert!(first.heat_updated_at.is_some());
    assert!(!first.durable);

    let second = remember(
        &mut engine,
        &remember_request(
            "fact:old",
            "old fact",
            "remember:2",
            vec![
                source("conversation--1", Some(days_ago(21))),
                source("conversation--2", Some(days_ago(42))),
                source("conversation--3", None),
            ],
            None,
        ),
    );
    // The repeated entry deposits nothing; a quarter for two half-lives; a
    // full reward for an undated entry, observed now.
    assert!((second.heat - 1.75).abs() < TOLERANCE, "{}", second.heat);

    // Re-sending only known evidence deposits nothing.
    let replayed = remember(
        &mut engine,
        &remember_request(
            "fact:old",
            "old fact",
            "remember:3",
            vec![source("conversation--2", Some(days_ago(42)))],
            None,
        ),
    );
    assert!(
        (replayed.heat - 1.75).abs() < TOLERANCE,
        "{}",
        replayed.heat
    );

    // A future observation never amplifies the reward.
    let future = remember(
        &mut engine,
        &remember_request(
            "fact:future",
            "future fact",
            "remember:future",
            vec![source("conversation--f", Some(days_ago(-30)))],
            None,
        ),
    );
    assert!((future.heat - 1.0).abs() < TOLERANCE, "{}", future.heat);
}

#[test]
fn replaying_history_in_any_order_yields_the_live_heat() {
    let history = [(3, "c3"), (40, "c40"), (10, "c10"), (70, "c70")];
    let expected = history
        .iter()
        .map(|(days, _)| 0.5_f64.powf(f64::from(*days) / 21.0))
        .sum::<f64>();

    for order in [[0, 1, 2, 3], [3, 1, 2, 0]] {
        let mut engine = CorroboreEngine::strict_default();
        let mut heat = 0.0;
        for (step, index) in order.iter().enumerate() {
            let (days, id) = history[*index];
            heat = remember(
                &mut engine,
                &remember_request(
                    "fact:replayed",
                    "replayed fact",
                    &format!("replay:{step}"),
                    vec![source(id, Some(days_ago(i64::from(days))))],
                    None,
                ),
            )
            .heat;
        }
        assert!((heat - expected).abs() < TOLERANCE, "{heat} != {expected}");
    }
}

#[test]
fn reinforce_deposits_usage_on_memories_and_relationships() {
    let mut engine = CorroboreEngine::strict_default();
    let alice = remember(
        &mut engine,
        &remember_request(
            "person:alice",
            "alice",
            "remember:alice",
            vec![source("c1", None)],
            None,
        ),
    );
    let bob = remember(
        &mut engine,
        &remember_request("person:bob", "bob", "remember:bob", vec![], None),
    );
    assert!(bob.heat.abs() < TOLERANCE, "no evidence, no heat");

    let reinforced = match reinforce(
        &mut engine,
        MemoryTarget::Memory(alice.id.clone()),
        2.0,
        "reinforce:alice",
    ) {
        MemoryResponse::Reinforce { record, receipt } => {
            assert_eq!(receipt.committed_id, alice.id);
            assert_eq!(receipt.committed_version, alice.version + 1);
            assert!(!receipt.replayed);
            record
        }
        other => panic!("expected reinforce, got {other:?}"),
    };
    assert!((reinforced.heat - 3.0).abs() < TOLERANCE);
    assert_eq!(reinforced.version, alice.version + 1);

    // Idempotent replay returns the original response without depositing.
    match reinforce(
        &mut engine,
        MemoryTarget::Memory(alice.id.clone()),
        2.0,
        "reinforce:alice",
    ) {
        MemoryResponse::Reinforce { receipt, .. } => assert!(receipt.replayed),
        other => panic!("expected reinforce, got {other:?}"),
    }
    assert!((get_record(&mut engine, &alice.id).heat - 3.0).abs() < TOLERANCE);

    // A penalty lowers heat and never below zero.
    match reinforce(
        &mut engine,
        MemoryTarget::Memory(alice.id.clone()),
        -10.0,
        "penalize:alice",
    ) {
        MemoryResponse::Reinforce { record, .. } => assert!(record.heat.abs() < f64::EPSILON),
        other => panic!("expected reinforce, got {other:?}"),
    }

    let relationship = match engine
        .execute_memory(
            &context(MemoryPermissions::all()),
            &MemoryRequest::new(MemoryOperation::Relate(RelateRequest {
                identity_key: Some("alice-knows-bob".to_owned()),
                source_id: alice.id.clone(),
                target_id: bob.id.clone(),
                kind: "knows".to_owned(),
                properties: serde_json::json!({}),
                provenance: vec![source("c1", Some(days_ago(21)))],
                confidence: None,
                valid_from: None,
                valid_until: None,
                expires_at: None,
                lifecycle: MemoryLifecycle::Active,
            }))
            .with_idempotency_key("relate:alice-bob"),
        )
        .unwrap()
    {
        MemoryResponse::Relate { relationship, .. } => relationship,
        other => panic!("expected relate, got {other:?}"),
    };
    assert!((relationship.heat - 0.5).abs() < TOLERANCE);
    match reinforce(
        &mut engine,
        MemoryTarget::Relationship(relationship.id.clone()),
        1.0,
        "reinforce:relationship",
    ) {
        MemoryResponse::ReinforceRelationship { relationship, .. } => {
            assert!((relationship.heat - 1.5).abs() < TOLERANCE);
            assert!(relationship.heat_updated_at.is_some());
        }
        other => panic!("expected relationship reinforce, got {other:?}"),
    }
}

#[test]
fn reinforce_is_a_validated_permissioned_mutation() {
    let mut engine = CorroboreEngine::strict_default();
    let alice = remember(
        &mut engine,
        &remember_request("person:alice", "alice", "remember:alice", vec![], None),
    );
    let request = |amount: f64, target: MemoryTarget| {
        MemoryRequest::new(MemoryOperation::Reinforce(ReinforceRequest {
            target,
            amount,
            reason: None,
        }))
    };
    let target = MemoryTarget::Memory(alice.id.clone());

    let missing_key = engine
        .execute_memory(
            &context(MemoryPermissions::all()),
            &request(1.0, target.clone()),
        )
        .unwrap_err();
    assert_eq!(missing_key.code, MemoryErrorCode::IdempotencyKeyRequired);

    let read_only = engine
        .execute_memory(
            &context(MemoryPermissions::read_only()),
            &request(1.0, target.clone()).with_idempotency_key("k"),
        )
        .unwrap_err();
    assert_eq!(read_only.code, MemoryErrorCode::PermissionDenied);

    for amount in [0.0, f64::NAN, f64::INFINITY, 1_000.5, -1_000.5] {
        let error = engine
            .execute_memory(
                &context(MemoryPermissions::all()),
                &request(amount, target.clone()).with_idempotency_key(format!("bad:{amount}")),
            )
            .unwrap_err();
        assert_eq!(error.code, MemoryErrorCode::InvalidRequest, "{amount}");
    }
    let recall_target = engine
        .execute_memory(
            &context(MemoryPermissions::all()),
            &request(1.0, MemoryTarget::Recall("recall--1".to_owned()))
                .with_idempotency_key("bad:target"),
        )
        .unwrap_err();
    assert_eq!(recall_target.code, MemoryErrorCode::InvalidRequest);
    let unknown = engine
        .execute_memory(
            &context(MemoryPermissions::all()),
            &request(1.0, MemoryTarget::Memory("node--missing".to_owned()))
                .with_idempotency_key("bad:missing"),
        )
        .unwrap_err();
    assert_eq!(unknown.code, MemoryErrorCode::NotFound);

    // Wire shape: reason is optional and omitted when absent.
    let wire: MemoryRequest = serde_json::from_value(serde_json::json!({
        "contract_version": "v1",
        "idempotency_key": "k",
        "operation": "reinforce",
        "input": {"target": {"kind": "memory", "id": "node--1"}, "amount": 1.5}
    }))
    .unwrap();
    assert_eq!(
        serde_json::to_value(&wire).unwrap()["input"],
        serde_json::json!({"target": {"kind": "memory", "id": "node--1"}, "amount": 1.5})
    );
}

#[test]
fn cold_expiry_hides_cold_non_durable_memories_from_recall_only() {
    let policy = MemoryHeatPolicy::new(21.0, 1.0, 0.3).unwrap();
    let mut engine = engine_with(policy);
    let cold = remember(
        &mut engine,
        &remember_request(
            "fact:cold",
            "lager beer",
            "remember:cold",
            vec![source("c-old", Some(days_ago(42)))],
            None,
        ),
    );
    let warm = remember(
        &mut engine,
        &remember_request(
            "fact:warm",
            "stout beer",
            "remember:warm",
            vec![source("c-new", Some(days_ago(1)))],
            None,
        ),
    );
    let pinned = remember(
        &mut engine,
        &remember_request(
            "fact:pinned",
            "porter beer",
            "remember:pinned",
            vec![source("c-pinned", Some(days_ago(400)))],
            Some(true),
        ),
    );
    assert!(cold.heat < 0.3 && !cold.recallable);
    assert!(warm.recallable);
    assert!(pinned.durable && pinned.recallable);

    let ids = recall_ids(&mut engine, "beer");
    assert!(ids.contains(&warm.id));
    assert!(ids.contains(&pinned.id));
    assert!(
        !ids.contains(&cold.id),
        "a cold memory is treated as expired"
    );

    // `get` still returns the cold memory with its lifecycle untouched.
    let looked_up = get_record(&mut engine, &cold.id);
    assert_eq!(looked_up.lifecycle, MemoryLifecycle::Active);
    assert!(!looked_up.recallable);

    // Marking it durable exempts it from cold expiry.
    let durable = match engine
        .execute_memory(
            &context(MemoryPermissions::all()),
            &MemoryRequest::new(MemoryOperation::Update(MemoryUpdateRequest {
                target: MemoryTarget::Memory(cold.id.clone()),
                expected_version: None,
                patch: UpdatePatch {
                    durable: Some(true),
                    ..UpdatePatch::default()
                },
            }))
            .with_idempotency_key("update:durable"),
        )
        .unwrap()
    {
        MemoryResponse::Update { record, .. } => record,
        other => panic!("expected update, got {other:?}"),
    };
    assert!(durable.durable && durable.recallable);
    assert!(recall_ids(&mut engine, "beer").contains(&cold.id));

    // Clearing durability makes it cold again; a reinforcement warms it up.
    engine
        .execute_memory(
            &context(MemoryPermissions::all()),
            &MemoryRequest::new(MemoryOperation::Update(MemoryUpdateRequest {
                target: MemoryTarget::Memory(cold.id.clone()),
                expected_version: None,
                patch: UpdatePatch {
                    durable: Some(false),
                    ..UpdatePatch::default()
                },
            }))
            .with_idempotency_key("update:not-durable"),
        )
        .unwrap();
    assert!(!recall_ids(&mut engine, "beer").contains(&cold.id));
    reinforce(
        &mut engine,
        MemoryTarget::Memory(cold.id.clone()),
        1.0,
        "reinforce:cold",
    );
    assert!(recall_ids(&mut engine, "beer").contains(&cold.id));

    // A remember upsert without `durable` keeps the stored flag.
    let kept = remember(
        &mut engine,
        &remember_request(
            "fact:pinned",
            "porter beer",
            "remember:pinned:again",
            vec![],
            None,
        ),
    );
    assert!(kept.durable);
}

#[test]
fn the_default_policy_never_cold_expires() {
    let mut engine = CorroboreEngine::strict_default();
    let ancient = remember(
        &mut engine,
        &remember_request(
            "fact:ancient",
            "ancient beer",
            "remember:ancient",
            vec![source("c", Some(days_ago(3_650)))],
            None,
        ),
    );
    assert!(ancient.recallable);
    assert_eq!(recall_ids(&mut engine, "beer"), vec![ancient.id]);
}

#[test]
fn heat_breaks_ties_between_equal_lexical_matches_only() {
    let mut engine = CorroboreEngine::strict_default();
    // Engine ids are allocated in creation order, so id order is
    // stale, fresh, used; heat order is used, fresh, stale.
    let stale = remember(
        &mut engine,
        &remember_request(
            "fact:stale",
            "coffee",
            "remember:stale",
            vec![source("a", Some(days_ago(60)))],
            None,
        ),
    );
    let fresh = remember(
        &mut engine,
        &remember_request(
            "fact:fresh",
            "coffee",
            "remember:fresh",
            vec![source("b", Some(days_ago(1)))],
            None,
        ),
    );
    let used = remember(
        &mut engine,
        &remember_request(
            "fact:used",
            "coffee",
            "remember:used",
            vec![source("c", Some(days_ago(60)))],
            None,
        ),
    );
    reinforce(
        &mut engine,
        MemoryTarget::Memory(used.id.clone()),
        5.0,
        "reinforce:used",
    );
    // A stronger lexical match still wins over heat.
    let exact = remember(
        &mut engine,
        &remember_request(
            "fact:exact",
            "coffee arabica",
            "remember:exact",
            vec![source("d", Some(days_ago(300)))],
            None,
        ),
    );
    assert_eq!(
        recall_ids(&mut engine, "coffee arabica"),
        vec![exact.id, used.id, fresh.id, stale.id]
    );
}

// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//! A fused memory keeps its origins and cannot launder authority.
//!
//! Consolidation aggregates without erasing where an item came from, remembering
//! something repeatedly never makes it more authoritative than its strongest
//! justified source, and revoking one source recomputes the fusion while every
//! other observation stays.
#![allow(clippy::unwrap_used)]

use corrobore_engine::{
    ConsolidateMode, ConsolidateRequest, CorroboreEngine, EngineMutationContext, FusionInput,
    MemoryAuthorityPolicyRef, MemoryContent, MemoryOperation, MemoryPermissions, MemoryRequest,
    MemoryResponse, MemoryServiceContext, ProvenanceReference, RememberRequest, SourceAuthorityCap,
    fuse_lineage,
};
use graph_core::{Confidence, SourceAuthority, SourceAuthorityPolicy, SourceId};

const POLICY: &str = "memory-authority-v1";
const DOMAIN: &str = "operations";
const PREDICATE: &str = "measurement";
const STRONG: &str = "source--registry";
const WEAK: &str = "source--forum";

fn context() -> MemoryServiceContext {
    MemoryServiceContext::new(
        "workspace--fusion",
        "actor--fusion",
        None,
        "session--fusion",
        MemoryPermissions::all(),
        "request--fusion",
        "correlation--fusion",
    )
    .unwrap()
}

fn weight(value: f64) -> Confidence {
    Confidence::new(value).unwrap()
}

fn policy() -> SourceAuthorityPolicy {
    SourceAuthorityPolicy::new(
        POLICY,
        vec![
            SourceAuthority::new(
                SourceId::new(STRONG).unwrap(),
                DOMAIN,
                PREDICATE,
                weight(0.9),
                POLICY,
            )
            .unwrap(),
            SourceAuthority::new(
                SourceId::new(WEAK).unwrap(),
                DOMAIN,
                PREDICATE,
                weight(0.2),
                POLICY,
            )
            .unwrap(),
        ],
    )
    .unwrap()
}

fn origin(memory_id: &str, source_id: &str, confidence: f64) -> FusionInput {
    FusionInput {
        memory_id: memory_id.to_owned(),
        provenance: vec![ProvenanceReference {
            source_id: source_id.to_owned(),
            locator: None,
            observed_at: None,
        }],
        confidence: Some(confidence),
    }
}

fn cap(policy: &SourceAuthorityPolicy) -> SourceAuthorityCap<'_> {
    SourceAuthorityCap::new(policy, DOMAIN, PREDICATE)
}

fn remember(identity_key: &str, source_id: &str, confidence: f64) -> MemoryRequest {
    MemoryRequest::new(MemoryOperation::Remember(RememberRequest {
        identity_key: Some(identity_key.to_owned()),
        kind: "observation".to_owned(),
        schema_version: "1".to_owned(),
        content: MemoryContent::Text(format!("reading from {source_id}")),
        provenance: vec![ProvenanceReference {
            source_id: source_id.to_owned(),
            locator: None,
            observed_at: Some("2026-09-07T00:00:00Z".to_owned()),
        }],
        confidence: Some(confidence),
        valid_from: None,
        valid_until: None,
        expires_at: None,
        tags: vec![],
    }))
    .with_idempotency_key(format!("remember:{identity_key}"))
}

fn remembered(engine: &mut CorroboreEngine, request: &MemoryRequest) -> String {
    match engine.execute_memory(&context(), request).unwrap() {
        MemoryResponse::Remember { record, .. } => record.id,
        other => panic!("expected remember, got {other:?}"),
    }
}

fn consolidated(engine: &mut CorroboreEngine, request: &MemoryRequest) -> MemoryResponse {
    engine.execute_memory(&context(), request).unwrap()
}

fn consolidate(
    ids: &[String],
    canonical: &str,
    mode: ConsolidateMode,
    revoked: Vec<String>,
    idempotency_key: &str,
) -> MemoryRequest {
    MemoryRequest::new(MemoryOperation::Consolidate(ConsolidateRequest {
        mode,
        memory_ids: ids.to_vec(),
        canonical_id: Some(canonical.to_owned()),
        reason: "duplicate reading".to_owned(),
        preserve_disagreements: true,
        authority_policy: Some(MemoryAuthorityPolicyRef {
            version: POLICY.to_owned(),
            authority_domain: DOMAIN.to_owned(),
            predicate_class: PREDICATE.to_owned(),
        }),
        revoked_source_ids: revoked,
    }))
    .with_idempotency_key(idempotency_key.to_owned())
}

fn lineage(response: &MemoryResponse) -> corrobore_engine::FusionLineage {
    match response {
        MemoryResponse::Consolidate(result) => result.lineage.clone(),
        other => panic!("expected consolidate, got {other:?}"),
    }
}

fn engine_with_policy() -> CorroboreEngine {
    let mut engine = CorroboreEngine::strict_default();
    engine
        .mutate_graph_atomically(
            EngineMutationContext::new("workspace--fusion", "session--fusion", "budget--fusion"),
            |graph| {
                graph
                    .epistemic_stores_mut()
                    .verdicts
                    .register_source_authority_policy(policy())
            },
        )
        .unwrap();
    engine
}

//
// Every fused memory enumerates the atomic origins that created it, including
// the sources behind each one: consolidation aggregates without erasing where
// the interpretation came from.
#[test]
fn a_fused_memory_enumerates_the_atomic_observations_that_created_it() {
    let policy = policy();
    let fused = fuse_lineage(
        "memory--canonical",
        &[
            origin("memory--strong", STRONG, 0.8),
            origin("memory--weak", WEAK, 0.8),
        ],
        Some(&cap(&policy)),
        &[],
    );

    assert_eq!(fused.canonical_id(), "memory--canonical");
    assert_eq!(
        fused
            .origins()
            .iter()
            .map(|origin| origin.memory_id())
            .collect::<Vec<_>>(),
        ["memory--strong", "memory--weak"]
    );
    assert_eq!(fused.origins()[0].source_ids(), [STRONG]);
    assert_eq!(fused.active_origins().len(), 2);
    assert!(fused.origins().iter().all(|origin| !origin.is_revoked()));
    assert_eq!(fused.policy_version(), Some(POLICY));
}

//
// The provenance-laundering guardrail: remembering the same weak observation
// again and again never raises its authority, because the fused authority is a
// maximum over origins and never a function of how many there are.
#[test]
fn repeated_consolidation_of_a_weak_observation_never_raises_its_authority() {
    let policy = policy();
    let once = fuse_lineage(
        "memory--canonical",
        &[origin("memory--weak-1", WEAK, 1.0)],
        Some(&cap(&policy)),
        &[],
    );
    let many = fuse_lineage(
        "memory--canonical",
        &(1..=5)
            .map(|index| origin(&format!("memory--weak-{index}"), WEAK, 1.0))
            .collect::<Vec<_>>(),
        Some(&cap(&policy)),
        &[],
    );

    assert_eq!(once.authority(), Some(0.2));
    assert_eq!(many.authority(), once.authority());
    assert_eq!(many.origins().len(), 5, "every repetition stays enumerated");
    assert_eq!(many.authority_source_id(), Some(WEAK));
}

//
// A memory cannot claim more authority than its source is granted, whatever
// confidence the application asserted, and an unbound source yields no
// justified authority at all rather than a default.
#[test]
fn a_fused_memory_never_exceeds_the_strongest_justified_source_authority() {
    let policy = policy();
    let capped = fuse_lineage(
        "memory--canonical",
        &[origin("memory--weak", WEAK, 0.99)],
        Some(&cap(&policy)),
        &[],
    );
    assert_eq!(capped.authority(), Some(0.2));

    let mixed = fuse_lineage(
        "memory--canonical",
        &[
            origin("memory--weak", WEAK, 0.99),
            origin("memory--strong", STRONG, 0.5),
        ],
        Some(&cap(&policy)),
        &[],
    );
    assert_eq!(
        mixed.authority(),
        Some(0.5),
        "the strongest justified origin is capped by its own confidence"
    );
    assert_eq!(mixed.authority_source_id(), Some(STRONG));

    let unbound = fuse_lineage(
        "memory--canonical",
        &[origin("memory--unbound", "source--unknown", 1.0)],
        Some(&cap(&policy)),
        &[],
    );
    assert_eq!(unbound.authority(), None);
    assert_eq!(unbound.origins()[0].justified_authority(), None);

    let unpoliced = fuse_lineage(
        "memory--canonical",
        &[origin("memory--strong", STRONG, 1.0)],
        None,
        &[],
    );
    assert_eq!(
        unpoliced.authority(),
        None,
        "authority without a policy is unjustified, not assumed"
    );
    assert_eq!(unpoliced.origins().len(), 1);
}

//
// Revoking a source recomputes the interpretation and keeps every other
// observation: the revoked origin stays enumerated and stops counting.
#[test]
fn revoking_one_source_recomputes_the_fusion_without_deleting_the_others() {
    let policy = policy();
    let originals = [
        origin("memory--strong", STRONG, 0.9),
        origin("memory--weak", WEAK, 0.9),
    ];
    let before = fuse_lineage("memory--canonical", &originals, Some(&cap(&policy)), &[]);
    let after = fuse_lineage(
        "memory--canonical",
        &originals,
        Some(&cap(&policy)),
        &[STRONG.to_owned()],
    );

    assert_eq!(before.authority(), Some(0.9));
    assert_eq!(after.authority(), Some(0.2));
    assert_eq!(after.origins().len(), 2, "nothing is deleted");
    assert_eq!(after.active_origins().len(), 1);
    assert!(after.origins()[0].is_revoked());
    assert!(!after.origins()[1].is_revoked());
    assert_eq!(after.revoked_source_ids(), [STRONG]);
    assert_eq!(after.authority_source_id(), Some(WEAK));

    let all_revoked = fuse_lineage(
        "memory--canonical",
        &originals,
        Some(&cap(&policy)),
        &[STRONG.to_owned(), WEAK.to_owned()],
    );
    assert_eq!(all_revoked.authority(), None);
    assert_eq!(all_revoked.origins().len(), 2);
}

//
// The operation carries the lineage: a proposal shows what the fusion would be
// without writing anything, and an approved apply retains it on the canonical
// memory while every original survives.
#[test]
fn consolidation_carries_the_lineage_and_retains_it_on_the_canonical_memory() {
    let mut engine = engine_with_policy();
    let strong = remembered(&mut engine, &remember("strong", STRONG, 0.9));
    let weak = remembered(&mut engine, &remember("weak", WEAK, 0.9));
    let ids = vec![strong.clone(), weak.clone()];

    let proposed = consolidated(
        &mut engine,
        &consolidate(&ids, &strong, ConsolidateMode::Propose, vec![], "propose"),
    );
    let proposal_id = match &proposed {
        MemoryResponse::Consolidate(result) => {
            assert!(!result.applied);
            result.proposal_id.clone()
        }
        other => panic!("expected consolidate, got {other:?}"),
    };
    let proposal_lineage = lineage(&proposed);
    assert_eq!(proposal_lineage.authority(), Some(0.9));
    assert_eq!(proposal_lineage.origins().len(), 2);

    let applied = consolidated(
        &mut engine,
        &consolidate(
            &ids,
            &strong,
            ConsolidateMode::ApplyApproved {
                proposal_id,
                approval_policy: "consolidation-approval-v1".to_owned(),
            },
            vec![],
            "apply",
        ),
    );
    let applied_lineage = lineage(&applied);
    assert_eq!(applied_lineage.authority(), Some(0.9));
    assert_eq!(
        applied_lineage
            .origins()
            .iter()
            .map(|origin| origin.memory_id().to_owned())
            .collect::<Vec<_>>(),
        {
            let mut expected = ids.clone();
            expected.sort();
            expected
        }
    );

    let canonical = engine
        .graph()
        .get_node(&graph_core::NodeId::new(&strong).unwrap())
        .unwrap()
        .unwrap();
    let retained = canonical
        .property("corrobore.memory.fusion_origins")
        .expect("the canonical memory retains its back-pointers");
    assert!(
        serde_json::to_value(retained)
            .unwrap()
            .to_string()
            .contains(&weak)
    );
    assert!(
        canonical
            .property("corrobore.memory.fused_authority")
            .is_some()
    );
    // The superseded original is retained, not deleted.
    assert!(
        engine
            .graph()
            .get_node(&graph_core::NodeId::new(&weak).unwrap())
            .unwrap()
            .is_some()
    );
}

//
// A revocation is its own governed decision: it needs its own approval, and
// applying it rewrites the retained interpretation without touching the
// observations behind it.
#[test]
fn a_revocation_is_a_separate_approved_consolidation_that_keeps_every_original() {
    let mut engine = engine_with_policy();
    let strong = remembered(&mut engine, &remember("strong", STRONG, 0.9));
    let weak = remembered(&mut engine, &remember("weak", WEAK, 0.9));
    let ids = vec![strong.clone(), weak.clone()];
    let approve = |proposal_id: String, revoked: Vec<String>, key: &str| {
        consolidate(
            &ids,
            &strong,
            ConsolidateMode::ApplyApproved {
                proposal_id,
                approval_policy: "consolidation-approval-v1".to_owned(),
            },
            revoked,
            key,
        )
    };
    let proposal = |revoked: Vec<String>, key: &str| {
        consolidate(&ids, &strong, ConsolidateMode::Propose, revoked, key)
    };

    let first = match consolidated(&mut engine, &proposal(vec![], "propose-live")) {
        MemoryResponse::Consolidate(result) => result.proposal_id,
        other => panic!("expected consolidate, got {other:?}"),
    };
    consolidated(&mut engine, &approve(first.clone(), vec![], "apply-live"));

    // The earlier approval cannot silently cover a different set of live sources.
    let stale = engine
        .execute_memory(
            &context(),
            &approve(first, vec![STRONG.to_owned()], "apply-stale"),
        )
        .expect_err("a revocation needs its own approval");
    assert_eq!(
        stale.code,
        corrobore_engine::MemoryErrorCode::PolicyApprovalRequired
    );

    let revoked_proposal = match consolidated(
        &mut engine,
        &proposal(vec![STRONG.to_owned()], "propose-revoked"),
    ) {
        MemoryResponse::Consolidate(result) => result.proposal_id,
        other => panic!("expected consolidate, got {other:?}"),
    };
    let applied = consolidated(
        &mut engine,
        &approve(revoked_proposal, vec![STRONG.to_owned()], "apply-revoked"),
    );
    let recomputed = lineage(&applied);

    assert_eq!(recomputed.authority(), Some(0.2));
    assert_eq!(recomputed.active_origins().len(), 1);
    assert_eq!(recomputed.origins().len(), 2);
    for id in [&strong, &weak] {
        assert!(
            engine
                .graph()
                .get_node(&graph_core::NodeId::new(id).unwrap())
                .unwrap()
                .is_some(),
            "{id} must survive a revocation"
        );
    }
}

//
// The v1 contract stays compatible: a payload without the fusion fields still
// deserializes, and one that does not use them serializes unchanged.
#[test]
fn the_consolidation_contract_stays_additive() {
    let legacy = serde_json::json!({
        "mode": {"mode": "propose"},
        "memory_ids": ["memory--alpha", "memory--beta"],
        "canonical_id": null,
        "reason": "duplicate identity",
        "preserve_disagreements": true
    });
    let request: ConsolidateRequest = serde_json::from_value(legacy.clone()).unwrap();

    assert!(request.authority_policy.is_none());
    assert!(request.revoked_source_ids.is_empty());
    assert_eq!(serde_json::to_value(&request).unwrap(), legacy);
}

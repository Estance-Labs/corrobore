// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
#![allow(clippy::unwrap_used)]

use std::sync::{Arc, Mutex};

use corrobore_engine::{
    ConsolidateMode, ConsolidateRequest, CorroboreEngine, EnginePersistence, ForgetMode,
    ForgetRequest, MemoryContent, MemoryContractVersion, MemoryErrorCode, MemoryLifecycle,
    MemoryLimits, MemoryOperation, MemoryPermissions, MemoryRequest, MemoryResponse,
    MemoryServiceContext, MemoryTarget, MemoryUpdateRequest, ProvenanceReference, RecallOutcome,
    RecallRequest, RelateRequest, RememberRequest, TraceRequest, UpdatePatch,
};
use graph_core::Graph;

fn context(workspace: &str, permissions: MemoryPermissions) -> MemoryServiceContext {
    MemoryServiceContext::new(
        workspace,
        "actor--contract",
        Some("agent--contract".to_owned()),
        "session--contract",
        permissions,
        "request--contract",
        "correlation--contract",
    )
    .expect("trusted context should be valid")
}

fn remember(identity_key: &str, text: &str) -> MemoryRequest {
    MemoryRequest::new(MemoryOperation::Remember(RememberRequest {
        identity_key: Some(identity_key.to_owned()),
        kind: "observation".to_owned(),
        schema_version: "1".to_owned(),
        content: MemoryContent::Text(text.to_owned()),
        provenance: vec![ProvenanceReference {
            source_id: "source--contract".to_owned(),
            locator: Some("urn:contract:1".to_owned()),
            observed_at: Some("2026-07-26T00:00:00Z".to_owned()),
        }],
        confidence: Some(0.8),
        valid_from: Some("2026-07-26T00:00:00Z".to_owned()),
        valid_until: None,
        expires_at: None,
        tags: vec!["contract".to_owned()],
    }))
    .with_idempotency_key(format!("remember:{identity_key}"))
}

fn remembered(response: MemoryResponse) -> (String, u64, bool) {
    match response {
        MemoryResponse::Remember { record, receipt } => {
            (record.id, record.version, receipt.replayed)
        }
        other => panic!("expected remember response, got {other:?}"),
    }
}

#[test]
fn public_contract_is_versioned_domain_neutral_and_contains_no_cypher_field() {
    let operations = vec![
        remember("alpha", "alpha memory"),
        MemoryRequest::new(MemoryOperation::Relate(RelateRequest {
            identity_key: Some("alpha-beta".to_owned()),
            source_id: "memory--alpha".to_owned(),
            target_id: "memory--beta".to_owned(),
            kind: "supports".to_owned(),
            properties: serde_json::json!({"weight": 2}),
            provenance: vec![],
            confidence: Some(0.7),
            valid_from: None,
            valid_until: None,
            expires_at: None,
            lifecycle: MemoryLifecycle::Active,
        })),
        MemoryRequest::new(MemoryOperation::Recall(RecallRequest {
            objective: "find alpha".to_owned(),
            seed_ids: vec![],
            limits: MemoryLimits::strict_default(),
            page_token: None,
        })),
        MemoryRequest::new(MemoryOperation::Update(MemoryUpdateRequest {
            target: MemoryTarget::Memory("memory--alpha".to_owned()),
            expected_version: Some(1),
            patch: UpdatePatch::default(),
        })),
        MemoryRequest::new(MemoryOperation::Forget(ForgetRequest {
            memory_id: "memory--alpha".to_owned(),
            mode: ForgetMode::Tombstone,
            expires_at: None,
            reason: "application request".to_owned(),
        })),
        MemoryRequest::new(MemoryOperation::Consolidate(ConsolidateRequest {
            mode: ConsolidateMode::Propose,
            memory_ids: vec!["memory--alpha".to_owned(), "memory--beta".to_owned()],
            canonical_id: None,
            reason: "duplicate identity".to_owned(),
            preserve_disagreements: true,
        })),
        MemoryRequest::new(MemoryOperation::Trace(TraceRequest {
            target: MemoryTarget::Memory("memory--alpha".to_owned()),
        })),
    ];

    for request in operations {
        assert_eq!(request.contract_version, MemoryContractVersion::V1);
        let value = serde_json::to_value(request).expect("contract should serialize");
        let object = value.as_object().expect("request should be an object");
        assert!(!object.contains_key("workspace_id"));
        assert!(!object.contains_key("actor_id"));
        assert!(!object.contains_key("permissions"));
        assert!(!value.to_string().to_ascii_lowercase().contains("cypher"));
    }

    let bounded_outcomes = [
        RecallOutcome::SupernodeBlocked,
        RecallOutcome::CostBudgetExhausted,
        RecallOutcome::PayloadBudgetExhausted,
        RecallOutcome::Timeout,
        RecallOutcome::SemanticProviderUnavailable,
        RecallOutcome::PartialPageIn,
        RecallOutcome::Cancelled,
        RecallOutcome::Overloaded,
    ];
    assert_eq!(
        serde_json::to_value(bounded_outcomes).unwrap(),
        serde_json::json!([
            "supernode_blocked",
            "cost_budget_exhausted",
            "payload_budget_exhausted",
            "timeout",
            "semantic_provider_unavailable",
            "partial_page_in",
            "cancelled",
            "overloaded"
        ])
    );

    let corpus: serde_json::Value = serde_json::from_str(include_str!(
        "../../../compatibility/memory/v1/conformance.json"
    ))
    .expect("shared memory conformance corpus should be valid JSON");
    let corpus_operations = corpus["operations"]
        .as_array()
        .expect("corpus operations should be an array")
        .iter()
        .map(|entry| entry["operation"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        corpus_operations,
        std::collections::BTreeSet::from([
            "remember",
            "relate",
            "recall",
            "update",
            "forget",
            "consolidate",
            "trace",
        ])
    );
}

#[test]
fn domain_free_journey_is_idempotent_bounded_explainable_and_workspace_isolated() {
    let mut engine = CorroboreEngine::strict_default();
    let alpha_context = context("workspace--alpha", MemoryPermissions::all());
    let beta_context = context("workspace--beta", MemoryPermissions::all());

    let (alpha_id, version, replayed) = remembered(
        engine
            .execute_memory(&alpha_context, &remember("alpha", "alpha signal"))
            .expect("remember should succeed"),
    );
    assert_eq!(version, 1);
    assert!(!replayed);
    let (replayed_id, replayed_version, replayed) = remembered(
        engine
            .execute_memory(&alpha_context, &remember("alpha", "alpha signal"))
            .expect("same idempotent request should replay"),
    );
    assert_eq!(
        (replayed_id, replayed_version, replayed),
        (alpha_id.clone(), 1, true)
    );

    let conflict = engine
        .execute_memory(&alpha_context, &remember("alpha", "changed payload"))
        .expect_err("an idempotency key cannot be reused with another payload");
    assert_eq!(conflict.code, MemoryErrorCode::IdempotencyConflict);

    let (beta_id, _, _) = remembered(
        engine
            .execute_memory(&alpha_context, &remember("beta", "beta evidence"))
            .expect("second memory should be stored"),
    );
    let relation = engine
        .execute_memory(
            &alpha_context,
            &MemoryRequest::new(MemoryOperation::Relate(RelateRequest {
                identity_key: Some("alpha-supports-beta".to_owned()),
                source_id: alpha_id.clone(),
                target_id: beta_id.clone(),
                kind: "supports".to_owned(),
                properties: serde_json::json!({"strength": "direct"}),
                provenance: vec![ProvenanceReference {
                    source_id: "source--relation".to_owned(),
                    locator: None,
                    observed_at: None,
                }],
                confidence: Some(0.9),
                valid_from: None,
                valid_until: None,
                expires_at: None,
                lifecycle: MemoryLifecycle::Active,
            }))
            .with_idempotency_key("relate:alpha-beta"),
        )
        .expect("relate should succeed");
    let relation_id = match relation {
        MemoryResponse::Relate {
            relationship,
            receipt,
        } => {
            assert_eq!(relationship.version, 1);
            assert_eq!(receipt.committed_version, 1);
            relationship.id
        }
        other => panic!("expected relate response, got {other:?}"),
    };

    let recall = engine
        .execute_memory(
            &alpha_context,
            &MemoryRequest::new(MemoryOperation::Recall(RecallRequest {
                objective: "alpha evidence".to_owned(),
                seed_ids: vec![alpha_id.clone()],
                limits: MemoryLimits {
                    max_items: 2,
                    max_depth: 1,
                    max_payload_bytes: 8_192,
                    max_cost: 8,
                    timeout_ms: 1_000,
                    supernode_threshold: 4,
                },
                page_token: None,
            })),
        )
        .expect("bounded recall should succeed");
    let recall_id = match recall {
        MemoryResponse::Recall(result) => {
            assert_eq!(result.items.len(), 2);
            assert!(
                result
                    .items
                    .iter()
                    .all(|item| !item.selection_reasons.is_empty())
            );
            assert_eq!(result.relationships.len(), 1);
            assert!(result.usage.items <= 2);
            assert!(result.usage.depth <= 1);
            assert!(result.completeness.complete || result.completeness.truncated);
            result.recall_id
        }
        other => panic!("expected recall response, got {other:?}"),
    };

    let update = engine
        .execute_memory(
            &alpha_context,
            &MemoryRequest::new(MemoryOperation::Update(MemoryUpdateRequest {
                target: MemoryTarget::Memory(alpha_id.clone()),
                expected_version: Some(1),
                patch: UpdatePatch {
                    confidence: Some(0.95),
                    add_provenance: vec![ProvenanceReference {
                        source_id: "source--update".to_owned(),
                        locator: None,
                        observed_at: None,
                    }],
                    lifecycle: Some(MemoryLifecycle::Active),
                    ..UpdatePatch::default()
                },
            }))
            .with_idempotency_key("update:alpha:2"),
        )
        .expect("auditable update should succeed");
    match update {
        MemoryResponse::Update { record, receipt } => {
            assert_eq!(record.version, 2);
            assert_eq!(record.provenance.len(), 2);
            assert_eq!(receipt.committed_version, 2);
        }
        other => panic!("expected update response, got {other:?}"),
    }

    let trace = engine
        .execute_memory(
            &alpha_context,
            &MemoryRequest::new(MemoryOperation::Trace(TraceRequest {
                target: MemoryTarget::Recall(recall_id),
            })),
        )
        .expect("trace should explain recall selection");
    match trace {
        MemoryResponse::Trace(trace) => {
            assert!(
                trace
                    .paths
                    .iter()
                    .any(|path| path.relationship_ids.contains(&relation_id))
            );
            assert!(trace.versions.iter().any(|version| version.version == 2));
            assert_eq!(trace.actor_id, "actor--contract");
            assert_eq!(trace.session_id, "session--contract");
            assert!(!trace.policy_decisions.is_empty());
        }
        other => panic!("expected trace response, got {other:?}"),
    }

    let hidden = engine
        .execute_memory(
            &beta_context,
            &MemoryRequest::new(MemoryOperation::Trace(TraceRequest {
                target: MemoryTarget::Memory(alpha_id.clone()),
            })),
        )
        .expect_err("another workspace must not observe identifiers or traces");
    assert_eq!(hidden.code, MemoryErrorCode::NotFound);

    engine
        .execute_memory(
            &alpha_context,
            &MemoryRequest::new(MemoryOperation::Forget(ForgetRequest {
                memory_id: alpha_id.clone(),
                mode: ForgetMode::ApplicationDelete,
                expires_at: None,
                reason: "user deleted application memory".to_owned(),
            }))
            .with_idempotency_key("forget:alpha"),
        )
        .expect("application forgetting should tombstone ordinary retrieval");

    let after_forget = engine
        .execute_memory(
            &alpha_context,
            &MemoryRequest::new(MemoryOperation::Recall(RecallRequest {
                objective: "alpha".to_owned(),
                seed_ids: vec![alpha_id],
                limits: MemoryLimits::strict_default(),
                page_token: None,
            })),
        )
        .expect("recall should remain bounded after forgetting");
    match after_forget {
        MemoryResponse::Recall(result) => assert!(
            result
                .items
                .iter()
                .all(|item| item.record.identity_key.as_deref() != Some("alpha"))
        ),
        other => panic!("expected recall response, got {other:?}"),
    }
}

#[test]
fn permissions_are_independent_and_budget_failures_are_typed() {
    let mut engine = CorroboreEngine::strict_default();
    let read_only = context("workspace--permissions", MemoryPermissions::read_only());
    let denied = engine
        .execute_memory(&read_only, &remember("denied", "not written"))
        .expect_err("write permission should be independent from read");
    assert_eq!(denied.code, MemoryErrorCode::PermissionDenied);

    let invalid_budget = engine
        .execute_memory(
            &read_only,
            &MemoryRequest::new(MemoryOperation::Recall(RecallRequest {
                objective: "bounded".to_owned(),
                seed_ids: vec![],
                limits: MemoryLimits {
                    max_items: 0,
                    ..MemoryLimits::strict_default()
                },
                page_token: None,
            })),
        )
        .expect_err("zero item budget should fail before execution");
    assert_eq!(invalid_budget.code, MemoryErrorCode::InvalidBudget);

    let no_trace = context(
        "workspace--permissions",
        MemoryPermissions {
            trace: false,
            ..MemoryPermissions::read_only()
        },
    );
    let denied = engine
        .execute_memory(
            &no_trace,
            &MemoryRequest::new(MemoryOperation::Trace(TraceRequest {
                target: MemoryTarget::Memory("memory--unknown".to_owned()),
            })),
        )
        .expect_err("trace requires its own permission");
    assert_eq!(denied.code, MemoryErrorCode::PermissionDenied);
}

#[test]
fn consolidation_proposal_and_approved_apply_preserve_originals_and_disagreements() {
    let mut engine = CorroboreEngine::strict_default();
    let context = context("workspace--consolidation", MemoryPermissions::all());
    let (first, _, _) = remembered(
        engine
            .execute_memory(&context, &remember("duplicate-a", "fact A"))
            .expect("first memory should be stored"),
    );
    let (second, _, _) = remembered(
        engine
            .execute_memory(&context, &remember("duplicate-b", "fact B disagrees"))
            .expect("second memory should be stored"),
    );

    let proposal = engine
        .execute_memory(
            &context,
            &MemoryRequest::new(MemoryOperation::Consolidate(ConsolidateRequest {
                mode: ConsolidateMode::Propose,
                memory_ids: vec![first.clone(), second.clone()],
                canonical_id: Some(first.clone()),
                reason: "same application identity".to_owned(),
                preserve_disagreements: true,
            })),
        )
        .expect("proposal mode should not destroy evidence");
    let proposal_id = match proposal {
        MemoryResponse::Consolidate(result) => {
            assert!(!result.applied);
            assert_eq!(result.originals_retained.len(), 2);
            assert!(result.disagreements_retained);
            result.proposal_id
        }
        other => panic!("expected consolidation response, got {other:?}"),
    };

    let applied = engine
        .execute_memory(
            &context,
            &MemoryRequest::new(MemoryOperation::Consolidate(ConsolidateRequest {
                mode: ConsolidateMode::ApplyApproved {
                    proposal_id,
                    approval_policy: "policy--human-approved".to_owned(),
                },
                memory_ids: vec![first.clone(), second.clone()],
                canonical_id: Some(first.clone()),
                reason: "approved identity merge".to_owned(),
                preserve_disagreements: true,
            }))
            .with_idempotency_key("consolidate:approved"),
        )
        .expect("approved consolidation should apply non-destructively");
    match applied {
        MemoryResponse::Consolidate(result) => {
            assert!(result.applied);
            assert_eq!(result.originals_retained.len(), 2);
            assert!(result.disagreements_retained);
        }
        other => panic!("expected consolidation response, got {other:?}"),
    }

    for memory_id in [first, second] {
        let trace = engine
            .execute_memory(
                &context,
                &MemoryRequest::new(MemoryOperation::Trace(TraceRequest {
                    target: MemoryTarget::Memory(memory_id),
                })),
            )
            .expect("original should remain traceable");
        assert!(matches!(trace, MemoryResponse::Trace(_)));
    }
}

#[derive(Clone, Debug, Default)]
struct SharedSnapshot(Arc<Mutex<Option<Graph>>>);

impl EnginePersistence for SharedSnapshot {
    fn load_graph(&self) -> Result<Graph, String> {
        Ok(self
            .0
            .lock()
            .map_err(|_| "snapshot lock poisoned")?
            .clone()
            .unwrap_or_default())
    }

    fn persist_graph(&mut self, graph: &Graph) -> Result<(), String> {
        *self.0.lock().map_err(|_| "snapshot lock poisoned")? = Some(graph.clone());
        Ok(())
    }
}

#[test]
fn accepted_mutations_cross_the_durability_gate_and_survive_restart() {
    let persistence = SharedSnapshot::default();
    let context = context("workspace--restart", MemoryPermissions::all());
    let memory_id = {
        let mut engine = CorroboreEngine::builder()
            .persistence(Box::new(persistence.clone()))
            .build()
            .expect("persistent engine should open");
        let (memory_id, _, _) = remembered(
            engine
                .execute_memory(&context, &remember("restart", "durable memory"))
                .expect("remember should cross persistence boundary"),
        );
        memory_id
    };

    let mut reopened = CorroboreEngine::builder()
        .persistence(Box::new(persistence))
        .build()
        .expect("persistent engine should reopen");
    let trace = reopened
        .execute_memory(
            &context,
            &MemoryRequest::new(MemoryOperation::Trace(TraceRequest {
                target: MemoryTarget::Memory(memory_id),
            })),
        )
        .expect("durable memory should be traceable after restart");
    assert!(matches!(trace, MemoryResponse::Trace(_)));

    let cypher = reopened
        .read("MATCH (n) RETURN n")
        .expect("advanced Cypher remains a compatible separately callable interface");
    assert!(matches!(
        cypher.status,
        corrobore_engine::CypherResponseStatus::Success
    ));
}

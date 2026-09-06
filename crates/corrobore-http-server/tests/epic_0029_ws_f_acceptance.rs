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
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode},
};
use corrobore_http_server::{AppState, ServerConfig, build_router};
use serde_json::{Value, json};
use std::collections::HashMap;
use tower::ServiceExt;
fn state(path: &std::path::Path) -> AppState {
    let config = ServerConfig::from_map(&HashMap::from([
        ("CORROBORE_HTTP_AUTH_TOKEN".into(), "token-123".into()),
        (
            "CORROBORE_HTTP_SESSION_STORE_DIR".into(),
            path.join("sessions").display().to_string(),
        ),
        ("CORROBORE_STORAGE_MODE".into(), "persistent".into()),
        (
            "CORROBORE_STORAGE_DIR".into(),
            path.join("storage").display().to_string(),
        ),
    ]))
    .expect("config");
    AppState::new(config).expect("state")
}
fn unique_path() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("candidate-http-{}", uuid::Uuid::new_v4()))
}
async fn send(app: &Router, method: Method, path: &str, value: Value) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", "Bearer token-123")
                .header("content-type", "application/json")
                .body(Body::from(value.to_string()))
                .expect("request"),
        )
        .await
        .expect("response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (
        status,
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned())),
    )
}

use graph_core::*;
#[path = "../../graph-core/tests/support/ingestion_quality.rs"]
mod fixtures;
fn seeded() -> (Graph, ClaimId) {
    let mut graph = fixtures::seeded();
    let id = ClaimId::new("claim--audit").expect("id");
    let obs = ObservationId::new("observation--left-0").expect("observation");
    let stores = graph.epistemic_stores_mut();
    stores.claims.register_observation(obs.clone());
    stores
        .claims
        .create_asserted_claim(ClaimInput::new(
            id.clone(),
            ClaimStatement::new("The source identifies the organization").expect("statement"),
            ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new("audit", None)),
        ))
        .expect("claim");
    stores
        .claims
        .attach_link(
            ClaimLink::new(
                ClaimLinkSource::Observation(obs),
                id.clone(),
                ClaimLinkKind::Supports,
            )
            .with_independence_cluster("registry"),
        )
        .expect("link");
    let obs = ObservationId::new("observation--right-0").expect("observation");
    stores.claims.register_observation(obs.clone());
    stores
        .claims
        .attach_link(
            ClaimLink::new(
                ClaimLinkSource::Observation(obs),
                id.clone(),
                ClaimLinkKind::Refutes,
            )
            .with_independence_cluster("registry"),
        )
        .expect("refutation");
    graph
        .link_claim_audit_record(
            &id,
            ClaimAuditReference::Candidate(CandidateId::new("repair-0").expect("candidate")),
        )
        .expect("lineage");
    graph
        .link_claim_audit_record(
            &id,
            ClaimAuditReference::Reconciliation(
                ReconciliationRecordId::new("decision-0").expect("decision"),
            ),
        )
        .expect("reconciliation lineage");
    (graph, id)
}

fn stamp() -> BitemporalStamp {
    BitemporalStamp::new(
        TemporalTimestamp::new("2026-09-06T12:00:00Z").expect("valid WS-F acceptance fixture"),
        TemporalTimestamp::new("2026-09-06T12:00:01Z").expect("valid WS-F acceptance fixture"),
    )
    .expect("valid WS-F acceptance fixture")
}

#[tokio::test]
async fn four_questions_human_reversals_and_offline_audit_share_one_stored_contract() {
    let (mut graph, root) = seeded();
    let peer = ClaimId::new("unchecked-dependency").expect("valid WS-F acceptance fixture");
    let stores = graph.epistemic_stores_mut();
    stores
        .claims
        .create_asserted_claim(ClaimInput::new(
            peer.clone(),
            ClaimStatement::new("Unchecked dependency").expect("valid WS-F acceptance fixture"),
            ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new("peer", None)),
        ))
        .expect("valid WS-F acceptance fixture");
    stores
        .claims
        .attach_link(ClaimLink::new(
            ClaimLinkSource::Claim(peer),
            root.clone(),
            ClaimLinkKind::DependsOn,
        ))
        .expect("valid WS-F acceptance fixture");
    for (id, deterministic, result) in [
        ("mechanical", true, VerificationResult::Pass),
        ("semantic", false, VerificationResult::Pass),
        ("failed", true, VerificationResult::Fail),
    ] {
        stores
            .verifications
            .append(VerificationRecord::new(
                VerificationRecordId::new(id).expect("valid WS-F acceptance fixture"),
                id,
                "1.0",
                deterministic,
                VerificationInputs::for_claim(root.clone()).with_observation(
                    ObservationId::new("observation--left-0")
                        .expect("valid WS-F acceptance fixture"),
                ),
                result,
                stamp(),
            ))
            .expect("valid WS-F acceptance fixture");
    }
    let evidence = EvidenceRecordStore::new();
    let inputs = ResolutionInputs::new(
        &stores.verifications,
        &evidence,
        &stores.observations,
        &stores.sources,
    );
    resolve_claim_verdict(
        &mut stores.claims,
        &mut stores.verdicts,
        &inputs,
        &root,
        stamp(),
        "ws-a-minimal-v1",
    )
    .expect("valid WS-F acceptance fixture");
    let expected = graph
        .claim_audit_path(&root)
        .expect("valid WS-F acceptance fixture");
    let before =
        serde_json::to_value(graph.persistence_snapshot()).expect("valid WS-F acceptance fixture");
    let path = unique_path();
    let app_state = state(&path);
    app_state
        .engine
        .lock()
        .expect("valid WS-F acceptance fixture")
        .mutate_graph_atomically(
            corrobore_engine::EngineMutationContext::new("acceptance", "seed", "seed"),
            |target| {
                *target = graph;
                Ok(())
            },
        )
        .expect("valid WS-F acceptance fixture");
    let app = build_router(app_state.clone());
    let endpoint = format!("/v1/claims/{}/audit", root.as_str());
    let (status, audit) = send(&app, Method::GET, &endpoint, json!(null)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(audit, expected);
    for field in [
        "observations",
        "contradictions",
        "state_transitions",
        "unverified_steps",
        "candidates",
        "reconciliations",
    ] {
        assert!(
            !audit[field]
                .as_array()
                .expect("valid WS-F acceptance fixture")
                .is_empty(),
            "missing answer: {field}"
        );
    }
    assert_eq!(audit["observations"][0]["payload"], "A");
    assert!(audit["explanation"]["dimensions"].is_object());
    assert!(
        !audit["explanation"]["clusters"]
            .as_array()
            .expect("valid WS-F acceptance fixture")
            .is_empty()
    );
    assert!(
        audit["link_membership"]
            .as_array()
            .expect("valid WS-F acceptance fixture")
            .iter()
            .any(|member| !member["stored_cluster_ids"]
                .as_array()
                .expect("valid WS-F acceptance fixture")
                .is_empty())
    );
    let classes: std::collections::BTreeSet<_> = audit["coverage"]
        .as_array()
        .expect("valid WS-F acceptance fixture")
        .iter()
        .flat_map(|group| {
            group["entries"]
                .as_array()
                .expect("valid WS-F acceptance fixture")
        })
        .map(|entry| {
            entry["class"]
                .as_str()
                .expect("valid WS-F acceptance fixture")
        })
        .collect();
    assert_eq!(
        classes,
        [
            "mechanically_checked",
            "semantically_judged",
            "unchecked",
            "failing"
        ]
        .into_iter()
        .collect()
    );
    for group in audit["coverage"]
        .as_array()
        .expect("valid WS-F acceptance fixture")
    {
        for entry in group["entries"]
            .as_array()
            .expect("valid WS-F acceptance fixture")
        {
            if entry["class"] != "unchecked" {
                assert!(entry["verifier_id"].is_string());
                assert_eq!(entry["verifier_version"], "1.0");
            }
        }
    }
    assert_eq!(
        before,
        serde_json::to_value(
            app_state
                .engine
                .lock()
                .expect("valid WS-F acceptance fixture")
                .graph()
                .persistence_snapshot()
        )
        .expect("valid WS-F acceptance fixture"),
        "reading must never resolve or verify"
    );
    let decision_endpoint = format!("/v1/claims/{}/decisions", root.as_str());
    for (id, action) in [
        (
            "note",
            json!({"kind":"annotation","text":"Reviewed original words"}),
        ),
        (
            "override",
            json!({"kind":"override","judgment":"Needs review","rationale":"Conflicting sources"}),
        ),
        (
            "reverse",
            json!({"kind":"reversal","decision_id":"override","rationale":"Withdrawn after source review"}),
        ),
    ] {
        let payload =
            json!({"id":id,"actor":"analyst","recorded_at":"2026-09-06T13:00:00Z","action":action});
        let (status, receipt) = send(&app, Method::POST, &decision_endpoint, payload.clone()).await;
        assert_eq!(status, StatusCode::OK, "{receipt}");
        assert_eq!(receipt["decision_id"], id);
        assert_eq!(
            send(&app, Method::POST, &decision_endpoint, payload)
                .await
                .1,
            receipt,
            "idempotent retry"
        );
    }
    let after = send(&app, Method::GET, &endpoint, json!(null)).await.1;
    assert_eq!(
        after["analyst_decisions"]
            .as_array()
            .expect("valid WS-F acceptance fixture")
            .len(),
        3
    );
    for (field, value) in audit.as_object().expect("valid WS-F acceptance fixture") {
        if field != "analyst_decisions" {
            assert_eq!(&after[field], value, "human write changed {field}");
        }
    }
    {
        let engine = app_state
            .engine
            .lock()
            .expect("valid WS-F acceptance fixture");
        let archive = engine
            .graph()
            .export_claim_audit_archive(std::slice::from_ref(&root))
            .expect("valid WS-F acceptance fixture");
        let restored =
            Graph::from_claim_audit_archive(&archive).expect("valid WS-F acceptance fixture");
        assert_eq!(
            restored
                .claim_audit_path(&root)
                .expect("valid WS-F acceptance fixture"),
            after
        );
        let memory = engine
            .graph()
            .export_memory_json()
            .expect("valid WS-F acceptance fixture");
        assert_eq!(
            Graph::from_memory_json(&memory)
                .expect("valid WS-F acceptance fixture")
                .claim_audit_path(&root)
                .expect("valid WS-F acceptance fixture"),
            after
        );
    }
    drop(app);
    drop(app_state);
    let restarted = build_router(state(&path));
    assert_eq!(
        send(&restarted, Method::GET, &endpoint, json!(null))
            .await
            .1,
        after
    );
    drop(restarted);
    std::fs::remove_dir_all(path).expect("valid WS-F acceptance fixture");
}

#[test]
fn openapi_exposes_every_canonical_coverage_class_and_a_read_only_audit_route() {
    let spec = include_str!("../../../docs/api/openapi.yaml");
    for class in [
        VerificationCoverageClass::MechanicallyChecked,
        VerificationCoverageClass::SemanticallyJudged,
        VerificationCoverageClass::Unchecked,
        VerificationCoverageClass::Failing,
    ] {
        let token = serde_json::to_value(class).expect("valid WS-F acceptance fixture");
        let schema = spec
            .split("    VerificationCoverageEntry:\n")
            .nth(1)
            .expect("typed coverage schema")
            .split("    VerificationCoverage:\n")
            .next()
            .expect("valid WS-F acceptance fixture");
        assert!(schema.contains(token.as_str().expect("valid WS-F acceptance fixture")));
    }
    let route = spec
        .split("  /v1/claims/{id}/audit:\n")
        .nth(1)
        .expect("valid WS-F acceptance fixture")
        .split("\n  /v1/")
        .next()
        .expect("valid WS-F acceptance fixture");
    assert!(route.contains("    get:"));
    assert!(!route.contains("    post:"));
    assert!(route.contains("#/components/schemas/ClaimAuditPath"));
    assert!(route.contains("'401'"));
}

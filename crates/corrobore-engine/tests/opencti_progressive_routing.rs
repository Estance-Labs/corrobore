// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

use std::{collections::BTreeSet, fs};

use corrobore_engine::{
    AccessContext, ContractVersion, GetByIdRequest, GraphReadPolicy, KnowledgeDataOperation,
    KnowledgeDataRequest, NeighborsRequest, OpenCtiReadRoutingRuntime, OperationKind,
    ProviderTarget, QueryClass, ReadRoutingGates, ReadRoutingMetadata, ReadRoutingMode,
    ReadRoutingPolicy, ReadRoutingRule, ReadRoutingThresholds, RequestContext, RollbackReason,
    RoutingBlockReason, RoutingDecisionReason, RoutingSignal, RoutingWindow,
};

fn request(operation: KnowledgeDataOperation, correlation_id: &str) -> KnowledgeDataRequest {
    KnowledgeDataRequest {
        contract_version: ContractVersion::CURRENT,
        context: RequestContext {
            request_id: format!("request--{correlation_id}"),
            correlation_id: correlation_id.to_owned(),
            access: AccessContext {
                subject_id: "user--synthetic".to_owned(),
                organization_ids: vec!["organization--alpha".to_owned()],
                tenant_id: Some("tenant--alpha".to_owned()),
                ..AccessContext::default()
            },
            ..RequestContext::default()
        },
        operation,
    }
}

fn point_read(correlation_id: &str) -> KnowledgeDataRequest {
    request(
        KnowledgeDataOperation::GetById(GetByIdRequest {
            id: "indicator--visible".to_owned(),
        }),
        correlation_id,
    )
}

fn graph_read(correlation_id: &str) -> KnowledgeDataRequest {
    request(
        KnowledgeDataOperation::Neighbors(NeighborsRequest {
            id: "indicator--visible".to_owned(),
            incoming: true,
            outgoing: true,
            policy: GraphReadPolicy::default(),
        }),
        correlation_id,
    )
}

fn metadata() -> ReadRoutingMetadata {
    ReadRoutingMetadata {
        environment: "production".to_owned(),
        entity_type: Some("indicator".to_owned()),
        user_cohort: Some("analyst-canary".to_owned()),
        feature_flags: BTreeSet::from(["corrobore-reads".to_owned()]),
        session_id: Some("session--stable".to_owned()),
        index_generation: Some("generation--7".to_owned()),
    }
}

fn healthy_gates() -> ReadRoutingGates {
    ReadRoutingGates {
        synchronization_ready: true,
        reference_fresh: true,
        corrobore_available: true,
        corruption_detected: false,
        security_divergence: false,
        parity_breach: false,
        error_rate_basis_points: 25,
        latency_p95_ms: 80,
    }
}

fn canary_policy() -> ReadRoutingPolicy {
    ReadRoutingPolicy {
        policy_version: "routing-v7".to_owned(),
        mode: ReadRoutingMode::Canary,
        default_percentage_basis_points: 0,
        rules: vec![ReadRoutingRule {
            environment: Some("production".to_owned()),
            operation: Some(OperationKind::GetById),
            query_class: Some(QueryClass::PointRead),
            entity_type: Some("indicator".to_owned()),
            organization_id: Some("organization--alpha".to_owned()),
            tenant_id: Some("tenant--alpha".to_owned()),
            user_cohort: Some("analyst-canary".to_owned()),
            required_feature_flag: Some("corrobore-reads".to_owned()),
            percentage_basis_points: 10_000,
        }],
        thresholds: ReadRoutingThresholds {
            max_error_rate_basis_points: 100,
            max_latency_p95_ms: 120,
            minimum_soak_requests: 1_000,
        },
    }
}

#[test]
fn every_routing_dimension_is_conjunctive_deterministic_and_explainable() {
    let mut runtime = OpenCtiReadRoutingRuntime::new(canary_policy()).expect("valid policy");
    let selected = runtime
        .decide(
            &point_read("correlation--selected"),
            &metadata(),
            &healthy_gates(),
            100,
        )
        .expect("canary request should route");
    assert_eq!(selected.primary, ProviderTarget::Corrobore);
    assert_eq!(selected.shadow, Some(ProviderTarget::Reference));
    assert_eq!(
        selected.reason,
        RoutingDecisionReason::MatchedRule { index: 0 }
    );

    let repeated = runtime
        .decide(
            &point_read("correlation--selected"),
            &metadata(),
            &healthy_gates(),
            101,
        )
        .expect("retry should route identically");
    assert_eq!(selected.primary, repeated.primary);

    let mut without_flag = metadata();
    without_flag.feature_flags.clear();
    without_flag.session_id = Some("session--excluded".to_owned());
    let excluded = runtime
        .decide(
            &point_read("correlation--excluded"),
            &without_flag,
            &healthy_gates(),
            102,
        )
        .expect("non-canary traffic should use the reference");
    assert_eq!(excluded.primary, ProviderTarget::Reference);
    assert_eq!(excluded.reason, RoutingDecisionReason::CanaryNotSelected);

    for (dimension, mismatched) in [
        (
            "environment",
            ReadRoutingMetadata {
                environment: "staging".to_owned(),
                session_id: Some("session--environment".to_owned()),
                ..metadata()
            },
        ),
        (
            "entity",
            ReadRoutingMetadata {
                entity_type: Some("malware".to_owned()),
                session_id: Some("session--entity".to_owned()),
                ..metadata()
            },
        ),
        (
            "cohort",
            ReadRoutingMetadata {
                user_cohort: Some("control".to_owned()),
                session_id: Some("session--cohort".to_owned()),
                ..metadata()
            },
        ),
    ] {
        assert_eq!(
            runtime
                .decide(
                    &point_read(&format!("correlation--{dimension}")),
                    &mismatched,
                    &healthy_gates(),
                    103,
                )
                .expect("selector mismatch should remain safe")
                .primary,
            ProviderTarget::Reference,
            "{dimension} selector"
        );
    }

    let mut wrong_organization = point_read("correlation--organization");
    wrong_organization.context.access.organization_ids = vec!["organization--beta".to_owned()];
    assert_eq!(
        runtime
            .decide(
                &wrong_organization,
                &ReadRoutingMetadata {
                    session_id: Some("session--organization".to_owned()),
                    ..metadata()
                },
                &healthy_gates(),
                104,
            )
            .expect("organization mismatch should remain safe")
            .primary,
        ProviderTarget::Reference
    );

    let mut wrong_tenant = point_read("correlation--tenant");
    wrong_tenant.context.access.tenant_id = Some("tenant--beta".to_owned());
    assert_eq!(
        runtime
            .decide(
                &wrong_tenant,
                &ReadRoutingMetadata {
                    session_id: Some("session--tenant".to_owned()),
                    ..metadata()
                },
                &healthy_gates(),
                105,
            )
            .expect("tenant mismatch should remain safe")
            .primary,
        ProviderTarget::Reference
    );

    assert_eq!(
        runtime
            .decide(
                &graph_read("correlation--operation-query-class"),
                &ReadRoutingMetadata {
                    session_id: Some("session--operation-query-class".to_owned()),
                    ..metadata()
                },
                &healthy_gates(),
                106,
            )
            .expect("operation and query mismatch should remain safe")
            .primary,
        ProviderTarget::Reference
    );

    let explanation = runtime
        .explain("correlation--selected")
        .expect("decision should be auditable");
    let serialized = serde_json::to_string(explanation).expect("audit should serialize");
    assert_eq!(explanation.policy_version, "routing-v7");
    assert_eq!(explanation.primary, ProviderTarget::Corrobore);
    assert!(!serialized.contains("user--synthetic"));
    assert!(!serialized.contains("organization--alpha"));
    assert!(!serialized.contains("tenant--alpha"));
}

#[test]
fn graph_and_primary_modes_only_route_their_supported_surface() {
    let mut graph_policy = canary_policy();
    graph_policy.mode = ReadRoutingMode::GraphReads;
    graph_policy.rules.clear();
    let mut graph_runtime = OpenCtiReadRoutingRuntime::new(graph_policy).expect("valid policy");
    assert_eq!(
        graph_runtime
            .decide(
                &graph_read("correlation--graph"),
                &metadata(),
                &healthy_gates(),
                1
            )
            .expect("graph read should route")
            .primary,
        ProviderTarget::Corrobore
    );
    assert_eq!(
        graph_runtime
            .decide(
                &point_read("correlation--point"),
                &ReadRoutingMetadata {
                    session_id: Some("session--point".to_owned()),
                    ..metadata()
                },
                &healthy_gates(),
                2
            )
            .expect("non-graph read should remain on reference")
            .primary,
        ProviderTarget::Reference
    );

    let mut primary_policy = canary_policy();
    primary_policy.mode = ReadRoutingMode::PrimaryReads;
    primary_policy.rules.clear();
    let mut primary_runtime = OpenCtiReadRoutingRuntime::new(primary_policy).expect("valid policy");
    assert_eq!(
        primary_runtime
            .decide(
                &point_read("correlation--primary"),
                &metadata(),
                &healthy_gates(),
                3
            )
            .expect("supported read should route")
            .primary,
        ProviderTarget::Corrobore
    );
}

#[test]
fn session_and_pagination_are_bound_to_provider_and_index_generation() {
    let mut runtime = OpenCtiReadRoutingRuntime::new(canary_policy()).expect("valid policy");
    let first = runtime
        .decide(
            &point_read("correlation--page-1"),
            &metadata(),
            &healthy_gates(),
            1,
        )
        .expect("first page should bind the session");
    assert_eq!(first.primary, ProviderTarget::Corrobore);

    let mut changed_generation = metadata();
    changed_generation.index_generation = Some("generation--8".to_owned());
    let error = runtime
        .decide(
            &point_read("correlation--page-2"),
            &changed_generation,
            &healthy_gates(),
            2,
        )
        .expect_err("generation changes must fail explicitly");
    assert_eq!(
        error.reason,
        RoutingBlockReason::IncompatibleSessionGeneration
    );
}

#[test]
fn security_health_parity_error_and_latency_gates_trigger_safe_rollback() {
    let gate_cases = [
        ("security", RollbackReason::SecurityDivergence),
        ("corruption", RollbackReason::Corruption),
        ("unavailable", RollbackReason::Unavailability),
        ("parity", RollbackReason::ParityBreach),
        ("errors", RollbackReason::ErrorRate),
        ("latency", RollbackReason::ExcessiveLatency),
    ];

    for (case, expected) in gate_cases {
        let mut gates = healthy_gates();
        match case {
            "security" => gates.security_divergence = true,
            "corruption" => gates.corruption_detected = true,
            "unavailable" => gates.corrobore_available = false,
            "parity" => gates.parity_breach = true,
            "errors" => gates.error_rate_basis_points = 101,
            "latency" => gates.latency_p95_ms = 121,
            _ => unreachable!(),
        }
        let mut runtime = OpenCtiReadRoutingRuntime::new(canary_policy()).expect("valid policy");
        let decision = runtime
            .decide(
                &point_read(&format!("correlation--{case}")),
                &metadata(),
                &gates,
                10,
            )
            .expect("fresh reference should make rollback immediately safe");
        assert_eq!(decision.primary, ProviderTarget::Reference);
        assert_eq!(
            decision.reason,
            RoutingDecisionReason::AutomaticRollback(expected)
        );
    }

    let mut gates = healthy_gates();
    gates.security_divergence = true;
    gates.reference_fresh = false;
    let mut runtime = OpenCtiReadRoutingRuntime::new(canary_policy()).expect("valid policy");
    let blocked = runtime
        .decide(&point_read("correlation--unsafe"), &metadata(), &gates, 20)
        .expect_err("rollback may not serve a stale reference");
    assert_eq!(blocked.reason, RoutingBlockReason::ReferenceNotFresh);
}

#[test]
fn runtime_signal_opens_the_circuit_and_preserves_the_rollback_reason() {
    let mut runtime = OpenCtiReadRoutingRuntime::new(canary_policy()).expect("valid policy");
    runtime
        .record_signal(RoutingSignal::SecurityDivergence, 100)
        .expect("rollback state should persist");
    let decision = runtime
        .decide(
            &point_read("correlation--after-signal"),
            &metadata(),
            &healthy_gates(),
            101,
        )
        .expect("fresh reference should receive traffic after rollback");
    assert_eq!(decision.primary, ProviderTarget::Reference);
    assert_eq!(
        decision.reason,
        RoutingDecisionReason::AutomaticRollback(RollbackReason::SecurityDivergence)
    );
    assert_eq!(
        runtime.rollback_reason(),
        Some(RollbackReason::SecurityDivergence)
    );
}

#[test]
fn canary_and_full_read_soak_require_slo_parity_and_zero_leakage() {
    let thresholds = canary_policy().thresholds;
    let healthy = RoutingWindow {
        requests: 10_000,
        errors: 20,
        latency_p95_ms: 90,
        parity_breaches: 0,
        security_divergences: 0,
    };
    assert!(healthy.promotion_ready(&thresholds));

    let leaked = RoutingWindow {
        security_divergences: 1,
        ..healthy
    };
    assert!(!leaked.promotion_ready(&thresholds));
}

#[test]
fn sticky_bindings_audits_and_circuit_state_survive_restart() {
    let root = std::env::temp_dir().join(format!(
        "corrobore-routing-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    fs::create_dir_all(&root).expect("temporary root should exist");
    let state_path = root.join("opencti-read-routing.json");
    let mut runtime =
        OpenCtiReadRoutingRuntime::open(Some(state_path.clone()), canary_policy(), 10)
            .expect("routing state should open");
    runtime
        .decide(
            &point_read("correlation--durable"),
            &metadata(),
            &healthy_gates(),
            10,
        )
        .expect("decision should persist");
    runtime
        .record_signal(RoutingSignal::OperatorRollback, 11)
        .expect("circuit should persist");

    let reopened = OpenCtiReadRoutingRuntime::open(Some(state_path), canary_policy(), 10)
        .expect("routing state should recover");
    assert_eq!(
        reopened.rollback_reason(),
        Some(RollbackReason::OperatorRequested)
    );
    assert_eq!(reopened.audits(10).len(), 1);
    assert_eq!(
        reopened
            .explain("correlation--durable")
            .expect("audit should recover")
            .primary,
        ProviderTarget::Corrobore
    );
    fs::remove_dir_all(root).expect("temporary root should be removable");
}

// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

use std::{
    fs,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use corrobore_engine::{
    AccessContext, ContractVersion, GetByIdRequest, KnowledgeDataOperation, KnowledgeDataOutcome,
    KnowledgeDataRequest, KnowledgeDataResponse, KnowledgeDataResponseEnvelope, ProviderDescriptor,
    ProviderExecution, QueryClass, RequestContext, ShadowComparisonGate, ShadowRequestMetadata,
    ShadowSamplingPolicy, compare_shadow_read,
};
use corrobore_http_server::opencti_shadow::{
    OpenCtiShadowRuntime, ShadowAdmission, ShadowCompletion, ShadowShedReason, dispatch_shadowed,
};
use uuid::Uuid;

#[tokio::test]
async fn slow_failed_or_shed_shadow_work_never_changes_the_reference_response() {
    let completions = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&completions);
    let started = Instant::now();

    let response = dispatch_shadowed(
        async {
            tokio::time::sleep(Duration::from_millis(10)).await;
            "reference-response".to_owned()
        },
        Some(async {
            tokio::time::sleep(Duration::from_millis(250)).await;
            Err::<String, _>("shadow failed")
        }),
        Duration::from_millis(30),
        move |reference, completion| {
            recorded
                .lock()
                .expect("completion lock")
                .push((reference, completion));
        },
    )
    .await;

    assert_eq!(response, "reference-response");
    assert!(
        started.elapsed() < Duration::from_millis(100),
        "reference latency must not wait for the shadow deadline"
    );
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert_eq!(
        completions.lock().expect("completion lock").as_slice(),
        &[("reference-response".to_owned(), ShadowCompletion::TimedOut)]
    );

    let shed = dispatch_shadowed(
        async { 42_u64 },
        None::<std::future::Ready<Result<u64, &'static str>>>,
        Duration::from_millis(30),
        |_reference, completion| assert_eq!(completion, ShadowCompletion::Shed),
    )
    .await;
    assert_eq!(shed, 42);
}

fn request(correlation_id: &str) -> KnowledgeDataRequest {
    KnowledgeDataRequest {
        contract_version: ContractVersion::CURRENT,
        context: RequestContext {
            request_id: format!("request--{correlation_id}"),
            correlation_id: correlation_id.to_owned(),
            access: AccessContext {
                subject_id: "user--shadow-runtime".to_owned(),
                organization_ids: vec!["organization--alpha".to_owned()],
                tenant_id: Some("tenant--alpha".to_owned()),
                ..AccessContext::default()
            },
            ..RequestContext::default()
        },
        operation: KnowledgeDataOperation::GetById(GetByIdRequest {
            id: "indicator--synthetic".to_owned(),
        }),
    }
}

fn execution(provider: &str, correlation_id: &str, latency_ms: u64) -> ProviderExecution {
    ProviderExecution {
        provider: ProviderDescriptor {
            name: provider.to_owned(),
            version: "test-version".to_owned(),
            release: "issue-43".to_owned(),
        },
        latency_ms,
        envelope: KnowledgeDataResponseEnvelope {
            contract_version: ContractVersion::CURRENT,
            correlation_id: correlation_id.to_owned(),
            outcome: KnowledgeDataOutcome::Success {
                response: KnowledgeDataResponse::Record(None),
            },
        },
    }
}

#[test]
fn runtime_enforces_sync_sampling_and_concurrency_gates() {
    let policy = ShadowSamplingPolicy {
        default_percentage_basis_points: 10_000,
        rules: Vec::new(),
    };
    let runtime =
        OpenCtiShadowRuntime::open(None, policy, Vec::new(), 1, 10).expect("runtime should open");
    let request = request("correlation--admission");
    let metadata = ShadowRequestMetadata {
        environment: "production".to_owned(),
        entity_type: Some("indicator".to_owned()),
        user_cohort: None,
    };

    assert!(matches!(
        runtime.admit(&request, &metadata, false),
        ShadowAdmission::Shed(ShadowShedReason::SynchronizationGate)
    ));
    let first = runtime.admit(&request, &metadata, true);
    let ShadowAdmission::Accepted(permit) = first else {
        panic!("first sampled execution should be accepted");
    };
    assert!(matches!(
        runtime.admit(&request, &metadata, true),
        ShadowAdmission::Shed(ShadowShedReason::ConcurrencyLimit)
    ));
    drop(permit);
    assert!(matches!(
        runtime.admit(&request, &metadata, true),
        ShadowAdmission::Accepted(_)
    ));
}

#[test]
fn divergence_reports_are_durable_bounded_and_queryable() {
    let root = std::env::temp_dir().join(format!("corrobore-shadow-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).expect("temporary root should exist");
    let state_path = root.join("runtime/opencti-shadow-reports.json");
    let policy = ShadowSamplingPolicy {
        default_percentage_basis_points: 10_000,
        rules: Vec::new(),
    };
    let request = request("correlation--durable-report");
    let mut divergent_shadow = execution("corrobore", "correlation--durable-report", 9);
    divergent_shadow.envelope.outcome = KnowledgeDataOutcome::Success {
        response: KnowledgeDataResponse::Count(corrobore_engine::CountResult { count: 2 }),
    };
    let mut reference = execution("opensearch", "correlation--durable-report", 12);
    reference.envelope.outcome = KnowledgeDataOutcome::Success {
        response: KnowledgeDataResponse::Count(corrobore_engine::CountResult { count: 1 }),
    };
    let report = compare_shadow_read(&request, reference, divergent_shadow, &[], 1);
    assert_eq!(report.gate, ShadowComparisonGate::Blocked);

    let mut runtime =
        OpenCtiShadowRuntime::open(Some(state_path.clone()), policy.clone(), Vec::new(), 2, 1)
            .expect("runtime should open");
    runtime
        .record(report.clone())
        .expect("report should persist");
    assert_eq!(
        runtime.reports(Some(QueryClass::PointRead), Some("issue-43"), 10),
        vec![report.clone()]
    );

    let reopened = OpenCtiShadowRuntime::open(Some(state_path), policy, Vec::new(), 2, 1)
        .expect("runtime should recover");
    assert_eq!(
        reopened.reports(Some(QueryClass::PointRead), Some("issue-43"), 10),
        vec![report]
    );
    assert_eq!(reopened.metrics().series()[0].comparisons, 1);

    fs::remove_dir_all(root).expect("temporary root should be removable");
}

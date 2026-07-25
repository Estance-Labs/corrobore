// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use corrobore_engine::{
    AccessContext, AggregateRequest, AggregationResult, ContractVersion, DivergenceBaseline,
    DivergenceSeverity, GetByIdRequest, GraphResult, KnowledgeDataError, KnowledgeDataErrorCode,
    KnowledgeDataOperation, KnowledgeDataOutcome, KnowledgeDataRequest, KnowledgeDataResponse,
    KnowledgeDataResponseEnvelope, KnowledgeRecord, ListRequest, OperationKind, ProviderDescriptor,
    ProviderExecution, QueryClass, RecordPage, RequestContext, ShadowComparisonGate, ShadowMetrics,
    ShadowRequestMetadata, ShadowSamplingPolicy, ShadowSamplingRule, compare_shadow_read,
};
use serde_json::{Value, json};

fn request(operation: KnowledgeDataOperation, correlation_id: &str) -> KnowledgeDataRequest {
    KnowledgeDataRequest {
        contract_version: ContractVersion::CURRENT,
        context: RequestContext {
            request_id: format!("request--{correlation_id}"),
            correlation_id: correlation_id.to_owned(),
            access: AccessContext {
                subject_id: "user--synthetic".to_owned(),
                organization_ids: vec!["organization--alpha".to_owned()],
                marking_ids: vec!["marking--amber".to_owned()],
                tenant_id: Some("tenant--alpha".to_owned()),
                roles: vec!["analyst".to_owned()],
                ..AccessContext::default()
            },
            ..RequestContext::default()
        },
        operation,
    }
}

fn execution(
    provider: &str,
    version: &str,
    release: &str,
    latency_ms: u64,
    correlation_id: &str,
    response: KnowledgeDataResponse,
) -> ProviderExecution {
    ProviderExecution {
        provider: ProviderDescriptor {
            name: provider.to_owned(),
            version: version.to_owned(),
            release: release.to_owned(),
        },
        latency_ms,
        envelope: KnowledgeDataResponseEnvelope {
            contract_version: ContractVersion::CURRENT,
            correlation_id: correlation_id.to_owned(),
            outcome: KnowledgeDataOutcome::Success { response },
        },
    }
}

fn record(id: &str, kind: &str, body: Value) -> KnowledgeRecord {
    KnowledgeRecord {
        id: id.to_owned(),
        kind: kind.to_owned(),
        revision: 7,
        body,
    }
}

#[test]
fn provider_specific_record_representations_normalize_to_the_same_result() {
    let request = request(
        KnowledgeDataOperation::GetById(GetByIdRequest {
            id: "indicator--visible".to_owned(),
        }),
        "correlation--normalization",
    );
    let reference = execution(
        "opensearch",
        "2.19.2",
        "opencti-7.260722.0",
        12,
        "correlation--normalization",
        KnowledgeDataResponse::Record(Some(record(
            "indicator--visible",
            "Indicator",
            json!({
                "standard_id": "indicator--visible",
                "entity_type": "Indicator",
                "name": "Synthetic indicator",
                "labels": ["beta", "alpha"],
                "_score": 1.0
            }),
        ))),
    );
    let shadow = execution(
        "corrobore",
        "0.1.0",
        "corrobore-issue-43",
        8,
        "correlation--normalization",
        KnowledgeDataResponse::Record(Some(record(
            "indicator--visible",
            "indicator",
            json!({
                "id": "indicator--visible",
                "type": "indicator",
                "labels": ["alpha", "beta"],
                "name": "Synthetic indicator"
            }),
        ))),
    );

    let report = compare_shadow_read(&request, reference, shadow, &[], 1_785_000_000_000);

    assert!(report.equivalent);
    assert_eq!(report.gate, ShadowComparisonGate::Pass);
    assert_eq!(report.query_class, QueryClass::PointRead);
    assert_eq!(report.correlation_id, "correlation--normalization");
    assert_eq!(report.reference_latency_ms, 12);
    assert_eq!(report.shadow_latency_ms, Some(8));
}

#[test]
fn comparison_reports_all_dimensions_and_redacts_inaccessible_values() {
    let request = request(
        KnowledgeDataOperation::GetById(GetByIdRequest {
            id: "indicator--visible".to_owned(),
        }),
        "correlation--security",
    );
    let reference = execution(
        "opensearch",
        "2.19.2",
        "opencti-7.260722.0",
        15,
        "correlation--security",
        KnowledgeDataResponse::Records(RecordPage {
            records: vec![
                record(
                    "indicator--visible",
                    "indicator",
                    json!({"name": "Allowed value", "score": 42}),
                ),
                record(
                    "indicator--missing",
                    "indicator",
                    json!({"name": "Missing from shadow"}),
                ),
            ],
            next_token: Some("reference-token".to_owned()),
        }),
    );
    let shadow = execution(
        "corrobore",
        "0.1.0",
        "corrobore-issue-43",
        23,
        "correlation--security",
        KnowledgeDataResponse::Records(RecordPage {
            records: vec![
                record(
                    "indicator--unauthorized-secret",
                    "indicator",
                    json!({"name": "TOP SECRET VALUE"}),
                ),
                record(
                    "indicator--visible",
                    "indicator",
                    json!({"name": "Changed sensitive value", "score": 43}),
                ),
            ],
            next_token: None,
        }),
    );

    let report = compare_shadow_read(&request, reference, shadow, &[], 1_785_000_000_000);
    let serialized = serde_json::to_string(&report).expect("report should serialize");

    assert!(!report.equivalent);
    assert_eq!(report.gate, ShadowComparisonGate::Blocked);
    assert_eq!(report.severity, DivergenceSeverity::Blocking);
    assert!(!report.missing_ids.is_empty());
    assert!(!report.unexpected_ids.is_empty());
    assert!(!report.property_differences.is_empty());
    assert!(!report.ordering_differences.is_empty());
    assert!(!report.security_differences.is_empty());
    assert!(!report.cursor_differences.is_empty());
    assert!(report.aggregation_differences.is_empty());
    assert!(!serialized.contains("indicator--unauthorized-secret"));
    assert!(!serialized.contains("TOP SECRET VALUE"));
    assert!(!serialized.contains("Changed sensitive value"));
}

#[test]
fn sampling_rules_cover_every_required_dimension_deterministically() {
    let request = request(
        KnowledgeDataOperation::GetById(GetByIdRequest {
            id: "indicator--visible".to_owned(),
        }),
        "correlation--sampled",
    );
    let metadata = ShadowRequestMetadata {
        environment: "production".to_owned(),
        entity_type: Some("indicator".to_owned()),
        user_cohort: Some("analyst-canary".to_owned()),
    };
    let policy = ShadowSamplingPolicy {
        default_percentage_basis_points: 0,
        rules: vec![ShadowSamplingRule {
            environment: Some("production".to_owned()),
            operation: Some(OperationKind::GetById),
            query_class: Some(QueryClass::PointRead),
            entity_type: Some("indicator".to_owned()),
            organization_id: Some("organization--alpha".to_owned()),
            tenant_id: Some("tenant--alpha".to_owned()),
            user_cohort: Some("analyst-canary".to_owned()),
            percentage_basis_points: 10_000,
        }],
    };

    assert!(policy.should_sample(&request, &metadata));
    assert!(policy.should_sample(&request, &metadata));
    assert!(!policy.should_sample(
        &request,
        &ShadowRequestMetadata {
            environment: "staging".to_owned(),
            ..metadata
        }
    ));
}

#[test]
fn known_functional_divergences_require_an_owner_and_unexpired_baseline() {
    let request = request(
        KnowledgeDataOperation::GetById(GetByIdRequest {
            id: "indicator--visible".to_owned(),
        }),
        "correlation--baseline",
    );
    let reference = execution(
        "opensearch",
        "2.19.2",
        "opencti-7.260722.0",
        12,
        "correlation--baseline",
        KnowledgeDataResponse::Record(Some(record(
            "indicator--visible",
            "indicator",
            json!({"name": "reference"}),
        ))),
    );
    let shadow = execution(
        "corrobore",
        "0.1.0",
        "corrobore-issue-43",
        8,
        "correlation--baseline",
        KnowledgeDataResponse::Record(Some(record(
            "indicator--visible",
            "indicator",
            json!({"name": "shadow"}),
        ))),
    );
    let initial = compare_shadow_read(
        &request,
        reference.clone(),
        shadow.clone(),
        &[],
        1_785_000_000_000,
    );
    let baseline = DivergenceBaseline {
        id: "baseline--known-name-mapping".to_owned(),
        query_class: QueryClass::PointRead,
        fingerprint: initial.divergence_fingerprint.clone(),
        owner: "team-opencti-adapter".to_owned(),
        expires_at_unix_ms: 1_786_000_000_000,
    };

    let accepted = compare_shadow_read(
        &request,
        reference.clone(),
        shadow.clone(),
        std::slice::from_ref(&baseline),
        1_785_000_000_000,
    );
    assert_eq!(accepted.gate, ShadowComparisonGate::BaselineAccepted);
    assert_eq!(accepted.severity, DivergenceSeverity::Warning);
    assert_eq!(
        accepted.baseline.as_ref().map(|value| value.owner.as_str()),
        Some("team-opencti-adapter")
    );

    let expired = compare_shadow_read(
        &request,
        reference,
        shadow,
        &[DivergenceBaseline {
            expires_at_unix_ms: 1_784_999_999_999,
            ..baseline
        }],
        1_785_000_000_000,
    );
    assert_eq!(expired.gate, ShadowComparisonGate::Blocked);
}

#[test]
fn metrics_are_bounded_to_query_class_and_release_labels() {
    let request = request(
        KnowledgeDataOperation::GetById(GetByIdRequest {
            id: "indicator--visible".to_owned(),
        }),
        "correlation--metrics",
    );
    let report = compare_shadow_read(
        &request,
        execution(
            "opensearch",
            "2.19.2",
            "opencti-7.260722.0",
            12,
            "correlation--metrics",
            KnowledgeDataResponse::Record(None),
        ),
        execution(
            "corrobore",
            "0.1.0",
            "corrobore-issue-43",
            9,
            "correlation--metrics",
            KnowledgeDataResponse::Record(None),
        ),
        &[],
        1_785_000_000_000,
    );
    let mut metrics = ShadowMetrics::default();
    metrics.record(&report);
    let series = metrics.series();

    assert_eq!(series.len(), 1);
    assert_eq!(series[0].query_class, QueryClass::PointRead);
    assert_eq!(series[0].release, "corrobore-issue-43");
    assert_eq!(series[0].comparisons, 1);
    assert_eq!(series[0].equivalent, 1);
    assert_eq!(series[0].reference_latency_buckets.last(), Some(&1));
    assert_eq!(series[0].shadow_latency_buckets.last(), Some(&1));

    let labels = BTreeMap::from([
        ("query_class", series[0].query_class.as_str()),
        ("release", series[0].release.as_str()),
    ]);
    assert_eq!(labels.len(), 2);
}

#[test]
fn aggregation_relationship_and_latency_dimensions_apply_deterministic_gates() {
    let aggregation_request = request(
        KnowledgeDataOperation::Aggregate(AggregateRequest {
            plan: json!({"field": "entity_type"}),
        }),
        "correlation--aggregation",
    );
    let aggregation = compare_shadow_read(
        &aggregation_request,
        execution(
            "opensearch",
            "2.19.2",
            "opencti-7.260722.0",
            10,
            "correlation--aggregation",
            KnowledgeDataResponse::Aggregation(AggregationResult {
                buckets: vec![json!({"key": "indicator", "count": 2})],
            }),
        ),
        execution(
            "corrobore",
            "0.1.0",
            "corrobore-issue-43",
            15,
            "correlation--aggregation",
            KnowledgeDataResponse::Aggregation(AggregationResult {
                buckets: vec![json!({"key": "indicator", "count": 3})],
            }),
        ),
        &[],
        1,
    );
    assert_eq!(aggregation.query_class, QueryClass::Aggregation);
    assert!(!aggregation.aggregation_differences.is_empty());
    assert_eq!(aggregation.gate, ShadowComparisonGate::Blocked);

    let graph_request = request(
        KnowledgeDataOperation::Subgraph(Default::default()),
        "correlation--graph",
    );
    let graph = compare_shadow_read(
        &graph_request,
        execution(
            "opensearch",
            "2.19.2",
            "opencti-7.260722.0",
            10,
            "correlation--graph",
            KnowledgeDataResponse::Graph(GraphResult {
                records: vec![record("indicator--one", "indicator", json!({}))],
                relationships: vec![json!({
                    "id": "relationship--one",
                    "source_ref": "indicator--one",
                    "target_ref": "malware--one",
                    "relationship_type": "indicates"
                })],
            }),
        ),
        execution(
            "corrobore",
            "0.1.0",
            "corrobore-issue-43",
            15,
            "correlation--graph",
            KnowledgeDataResponse::Graph(GraphResult {
                records: vec![record("indicator--one", "indicator", json!({}))],
                relationships: vec![],
            }),
        ),
        &[],
        1,
    );
    assert_eq!(graph.query_class, QueryClass::Graph);
    assert!(!graph.relationship_differences.is_empty());

    let latency_request = request(
        KnowledgeDataOperation::GetById(GetByIdRequest {
            id: "indicator--one".to_owned(),
        }),
        "correlation--latency",
    );
    let latency = compare_shadow_read(
        &latency_request,
        execution(
            "opensearch",
            "2.19.2",
            "opencti-7.260722.0",
            10,
            "correlation--latency",
            KnowledgeDataResponse::Record(None),
        ),
        execution(
            "corrobore",
            "0.1.0",
            "corrobore-issue-43",
            500,
            "correlation--latency",
            KnowledgeDataResponse::Record(None),
        ),
        &[],
        1,
    );
    assert!(latency.equivalent);
    assert_eq!(latency.gate, ShadowComparisonGate::Blocked);
    assert_eq!(
        latency.performance_differences,
        vec!["shadow_latency_budget"]
    );
}

#[test]
fn opencti_compatibility_corpus_produces_repeatable_end_to_end_reports() {
    let corpus: Value = serde_json::from_str(include_str!(
        "../../../compatibility/opencti/7.260722.0/parity-corpus.json"
    ))
    .expect("compatibility corpus should parse");
    let records = corpus["fixtures"]
        .as_array()
        .expect("fixtures should be an array")
        .iter()
        .map(|fixture| {
            let id = fixture["id"].as_str().expect("fixture id");
            let kind = fixture["type"].as_str().expect("fixture type");
            record(id, kind, fixture.clone())
        })
        .collect::<Vec<_>>();
    let request = request(
        KnowledgeDataOperation::List(ListRequest {
            kinds: Vec::new(),
            limit: 1_000,
        }),
        "correlation--compatibility-corpus",
    );
    let reference_response = KnowledgeDataResponse::Records(RecordPage {
        records: records
            .iter()
            .cloned()
            .map(|mut record| {
                record.body["_score"] = json!(1.0);
                record
            })
            .collect(),
        next_token: None,
    });
    let shadow_response = KnowledgeDataResponse::Records(RecordPage {
        records,
        next_token: None,
    });
    let compare = || {
        compare_shadow_read(
            &request,
            execution(
                "opensearch",
                "2.19.2",
                "opencti-7.260722.0",
                20,
                "correlation--compatibility-corpus",
                reference_response.clone(),
            ),
            execution(
                "corrobore",
                "0.1.0",
                "corrobore-issue-43",
                18,
                "correlation--compatibility-corpus",
                shadow_response.clone(),
            ),
            &[],
            1,
        )
    };

    let first = compare();
    let second = compare();
    assert!(first.equivalent);
    assert_eq!(first, second);
    assert_eq!(first.gate, ShadowComparisonGate::Pass);
}

#[test]
fn authorization_and_access_policy_divergences_are_blocking_and_not_baselinable() {
    let request = request(
        KnowledgeDataOperation::GetById(GetByIdRequest {
            id: "indicator--visible".to_owned(),
        }),
        "correlation--authorization",
    );
    let reference = execution(
        "opensearch",
        "2.19.2",
        "opencti-7.260722.0",
        10,
        "correlation--authorization",
        KnowledgeDataResponse::Record(Some(record(
            "indicator--visible",
            "indicator",
            json!({"object_marking_refs": ["marking--amber"]}),
        ))),
    );
    let shadow = ProviderExecution {
        provider: ProviderDescriptor {
            name: "corrobore".to_owned(),
            version: "0.1.0".to_owned(),
            release: "corrobore-issue-43".to_owned(),
        },
        latency_ms: 9,
        envelope: KnowledgeDataResponseEnvelope {
            contract_version: ContractVersion::CURRENT,
            correlation_id: "correlation--authorization".to_owned(),
            outcome: KnowledgeDataOutcome::Failure {
                error: KnowledgeDataError {
                    code: KnowledgeDataErrorCode::Unauthorized,
                    message: "secret provider authorization detail".to_owned(),
                    retryable: false,
                },
            },
        },
    };
    let initial = compare_shadow_read(&request, reference.clone(), shadow.clone(), &[], 1);
    assert_eq!(initial.gate, ShadowComparisonGate::Blocked);
    assert!(!initial.security_differences.is_empty());
    assert!(
        !serde_json::to_string(&initial)
            .expect("report should serialize")
            .contains("secret provider authorization detail")
    );
    let baselined = compare_shadow_read(
        &request,
        reference,
        shadow,
        &[DivergenceBaseline {
            id: "baseline--must-not-apply".to_owned(),
            query_class: QueryClass::PointRead,
            fingerprint: initial.divergence_fingerprint,
            owner: "security-team".to_owned(),
            expires_at_unix_ms: 10,
        }],
        1,
    );
    assert_eq!(baselined.gate, ShadowComparisonGate::Blocked);
    assert!(baselined.baseline.is_none());

    let access_policy = compare_shadow_read(
        &request,
        execution(
            "opensearch",
            "2.19.2",
            "opencti-7.260722.0",
            10,
            "correlation--authorization",
            KnowledgeDataResponse::Record(Some(record(
                "indicator--visible",
                "indicator",
                json!({"object_marking_refs": ["marking--amber"]}),
            ))),
        ),
        execution(
            "corrobore",
            "0.1.0",
            "corrobore-issue-43",
            9,
            "correlation--authorization",
            KnowledgeDataResponse::Record(Some(record(
                "indicator--visible",
                "indicator",
                json!({"object_marking_refs": []}),
            ))),
        ),
        &[],
        1,
    );
    assert_eq!(
        access_policy.security_differences,
        vec!["access_policy_mismatch"]
    );
    assert_eq!(access_policy.gate, ShadowComparisonGate::Blocked);
}

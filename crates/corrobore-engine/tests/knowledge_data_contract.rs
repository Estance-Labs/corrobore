// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use corrobore_engine::{
    AccessContext, ConsistencyLevel, ContractVersion, CountRequest, GetByIdRequest, HealthRequest,
    InitializeRequest, KnowledgeDataErrorCode, KnowledgeDataOperation, KnowledgeDataRequest,
    ListRequest, OperationKind, ProviderCapability, ProviderCapabilityStatus, ProviderRouteConfig,
    RequestContext,
};
use serde_json::json;

fn request_context() -> RequestContext {
    RequestContext {
        request_id: "request--contract-1".to_owned(),
        correlation_id: "correlation--contract-1".to_owned(),
        idempotency_key: Some("idempotency--contract-1".to_owned()),
        deadline_unix_ms: Some(4_102_444_800_000),
        cancellation_id: Some("cancel--contract-1".to_owned()),
        access: AccessContext {
            subject_id: "identity--analyst".to_owned(),
            organization_ids: vec!["identity--example-org".to_owned()],
            marking_ids: vec!["marking-definition--tlp-green".to_owned()],
            tenant_id: Some("tenant--example".to_owned()),
            roles: vec!["analyst".to_owned()],
            attributes: BTreeMap::from([("scope".to_owned(), "investigation".to_owned())]),
        },
        consistency: ConsistencyLevel::ReadYourWrites,
    }
}

#[test]
fn typed_request_preserves_boundary_context_without_transport_fields() {
    let request = KnowledgeDataRequest {
        contract_version: ContractVersion::CURRENT,
        context: request_context(),
        operation: KnowledgeDataOperation::GetById(GetByIdRequest {
            id: "node--1".to_owned(),
        }),
    };

    let value = serde_json::to_value(&request).expect("request should serialize");
    assert_eq!(value["contract_version"], json!({"major": 1, "minor": 0}));
    assert_eq!(value["context"]["request_id"], "request--contract-1");
    assert_eq!(
        value["context"]["correlation_id"],
        "correlation--contract-1"
    );
    assert_eq!(
        value["context"]["access"]["organization_ids"][0],
        "identity--example-org"
    );
    assert_eq!(value["context"]["consistency"], "read_your_writes");
    assert!(
        value.get("headers").is_none()
            && value.get("http_status").is_none()
            && value.get("grpc_metadata").is_none()
    );
}

#[test]
fn public_operation_contract_covers_every_required_operation_kind() {
    let operations = [
        KnowledgeDataOperation::Initialize(InitializeRequest {
            client_contract_version: ContractVersion::CURRENT,
            required_capabilities: vec![OperationKind::Health],
        }),
        KnowledgeDataOperation::Health(HealthRequest { verbose: true }),
        KnowledgeDataOperation::Migrate(Default::default()),
        KnowledgeDataOperation::GetById(GetByIdRequest {
            id: "node--1".to_owned(),
        }),
        KnowledgeDataOperation::List(ListRequest::default()),
        KnowledgeDataOperation::Paginate(Default::default()),
        KnowledgeDataOperation::Search(Default::default()),
        KnowledgeDataOperation::Count(CountRequest::default()),
        KnowledgeDataOperation::Aggregate(Default::default()),
        KnowledgeDataOperation::Neighbors(Default::default()),
        KnowledgeDataOperation::Traverse(Default::default()),
        KnowledgeDataOperation::Subgraph(Default::default()),
        KnowledgeDataOperation::Create(Default::default()),
        KnowledgeDataOperation::Update(Default::default()),
        KnowledgeDataOperation::Delete(Default::default()),
        KnowledgeDataOperation::Bulk(Default::default()),
        KnowledgeDataOperation::Merge(Default::default()),
        KnowledgeDataOperation::Snapshot(Default::default()),
        KnowledgeDataOperation::Restore(Default::default()),
        KnowledgeDataOperation::RebuildIndexes(Default::default()),
    ];

    let actual: Vec<OperationKind> = operations
        .iter()
        .map(KnowledgeDataOperation::kind)
        .collect();
    assert_eq!(actual, OperationKind::ALL);

    let serialized = serde_json::to_string(&operations).expect("operations should serialize");
    for forbidden in ["elasticsearch", "opensearch"] {
        assert!(
            !serialized.to_ascii_lowercase().contains(forbidden),
            "public operation contract must stay backend-neutral"
        );
    }
}

#[test]
fn contract_version_negotiation_is_major_strict_and_minor_backward_compatible() {
    assert!(ContractVersion::CURRENT.accepts(ContractVersion::new(1, 0)));
    assert!(!ContractVersion::CURRENT.accepts(ContractVersion::new(1, 1)));
    assert!(!ContractVersion::CURRENT.accepts(ContractVersion::new(2, 0)));
}

#[test]
fn stable_error_codes_round_trip_as_documented_wire_values() {
    let expected = [
        (KnowledgeDataErrorCode::InvalidRequest, "INVALID_REQUEST"),
        (
            KnowledgeDataErrorCode::IncompatibleContractVersion,
            "INCOMPATIBLE_CONTRACT_VERSION",
        ),
        (
            KnowledgeDataErrorCode::UnsupportedCapability,
            "UNSUPPORTED_CAPABILITY",
        ),
        (
            KnowledgeDataErrorCode::InvalidPaginationToken,
            "INVALID_PAGINATION_TOKEN",
        ),
        (
            KnowledgeDataErrorCode::IncompatiblePaginationToken,
            "INCOMPATIBLE_PAGINATION_TOKEN",
        ),
        (
            KnowledgeDataErrorCode::DeadlineExceeded,
            "DEADLINE_EXCEEDED",
        ),
        (KnowledgeDataErrorCode::Cancelled, "CANCELLED"),
    ];

    for (code, wire) in expected {
        assert_eq!(code.as_str(), wire);
        assert_eq!(
            serde_json::to_value(code).expect("error code should serialize"),
            wire
        );
    }
}

#[test]
fn provider_capability_status_distinguishes_supported_and_explicitly_unsupported() {
    assert!(ProviderCapabilityStatus::Supported.is_supported());
    assert!(
        !ProviderCapabilityStatus::Unsupported {
            reason: "scheduled for a downstream issue".to_owned(),
        }
        .is_supported()
    );
}

#[test]
fn capability_deprecation_is_enforced_by_contract_version() {
    let capability = ProviderCapability {
        operation: OperationKind::Health,
        status: ProviderCapabilityStatus::Supported,
        since: ContractVersion::new(1, 0),
        deprecated_after: Some(ContractVersion::new(1, 2)),
    };

    assert!(capability.is_available_to(ContractVersion::new(1, 0)));
    assert!(capability.is_available_to(ContractVersion::new(1, 1)));
    assert!(!capability.is_available_to(ContractVersion::new(1, 2)));
    assert!(!capability.is_available_to(ContractVersion::new(2, 0)));
}

#[test]
fn provider_route_is_configuration_only_and_backend_neutral() {
    let embedded: ProviderRouteConfig =
        serde_json::from_value(json!({"provider": "embedded_corrobore"}))
            .expect("embedded provider route should parse");
    assert_eq!(embedded, ProviderRouteConfig::EmbeddedCorrobore);

    let reference: ProviderRouteConfig = serde_json::from_value(json!({
        "provider": "remote_reference",
        "endpoint": "https://reference.example.com/v1/knowledge-data"
    }))
    .expect("reference provider route should parse");
    assert!(matches!(
        reference,
        ProviderRouteConfig::RemoteReference { endpoint }
            if endpoint == "https://reference.example.com/v1/knowledge-data"
    ));
}

// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

use std::{ffi::c_void, mem};

use domain_provider_abi::{
    CORROBORE_DOMAIN_PROVIDER_ABI_MAJOR_V1, CORROBORE_DOMAIN_PROVIDER_ABI_MINOR_V1,
    CORROBORE_DOMAIN_PROVIDER_ENTRYPOINT_V1, CapabilityDeclaration, ClaimVerifyAsOf,
    ClaimVerifyRequestPayload, ClaimVerifyResponsePayload, ClaimVerifyResult, DomainName,
    DomainProviderApiV1, DomainProviderBuffer, DomainProviderSlice, InvokeRequest, InvokeResponse,
    IssueSeverity, ProviderIssue, ProviderMetadata, ProviderResponseStatus, SCHEMA_V1,
    STATUS_INVALID_ARGUMENT, STATUS_OK,
};
use serde_json::json;

#[test]
fn abi_v1_contract_has_stable_version_and_entrypoint() {
    assert_eq!(CORROBORE_DOMAIN_PROVIDER_ABI_MAJOR_V1, 1);
    assert_eq!(CORROBORE_DOMAIN_PROVIDER_ABI_MINOR_V1, 2);
    assert_eq!(
        CORROBORE_DOMAIN_PROVIDER_ENTRYPOINT_V1,
        b"corrobore_domain_provider_get_api_v1\0"
    );
    assert_eq!(STATUS_OK, 0);
    assert_ne!(STATUS_INVALID_ARGUMENT, STATUS_OK);
}

#[test]
fn abi_v1_contract_uses_explicit_pointer_length_buffers() {
    let borrowed = DomainProviderSlice {
        ptr: std::ptr::null(),
        len: 0,
    };
    let owned = DomainProviderBuffer {
        ptr: std::ptr::null_mut(),
        len: 0,
    };

    assert!(borrowed.ptr.is_null());
    assert_eq!(borrowed.len, 0);
    assert!(owned.ptr.is_null());
    assert_eq!(owned.len, 0);
    assert_eq!(
        mem::size_of::<DomainProviderSlice>(),
        mem::size_of::<*const u8>() + mem::size_of::<usize>()
    );
    assert_eq!(
        mem::size_of::<DomainProviderBuffer>(),
        mem::size_of::<*mut u8>() + mem::size_of::<usize>()
    );
}

#[test]
fn abi_v1_function_table_is_prefix_versioned_and_complete() {
    let api = DomainProviderApiV1 {
        abi_major: CORROBORE_DOMAIN_PROVIDER_ABI_MAJOR_V1,
        abi_minor: CORROBORE_DOMAIN_PROVIDER_ABI_MINOR_V1,
        struct_size: mem::size_of::<DomainProviderApiV1>(),
        metadata: None,
        create: None,
        invoke: None,
        health: None,
        destroy: None,
        free_buffer: None,
    };

    assert_eq!(api.abi_major, 1);
    assert_eq!(api.struct_size, mem::size_of::<DomainProviderApiV1>());
    assert!(api.metadata.is_none());
    assert!(api.create.is_none());
    assert!(api.invoke.is_none());
    assert!(api.health.is_none());
    assert!(api.destroy.is_none());
    assert!(api.free_buffer.is_none());

    let _: Option<unsafe extern "C" fn(DomainProviderSlice, *mut DomainProviderBuffer) -> i32> =
        api.metadata;
    let _: Option<unsafe extern "C" fn(DomainProviderSlice, *mut *mut c_void) -> i32> = api.create;
}

#[test]
fn provider_metadata_contract_declares_identity_capabilities_and_limits() {
    let metadata: ProviderMetadata = serde_json::from_value(json!({
        "schema_version": "1",
        "provider_id": "fr.estance.corrobore.domain.cti",
        "provider_version": "0.1.0",
        "domain": "cti",
        "thread_safe": true,
        "max_concurrency": 16,
        "max_request_bytes": 1048576,
        "max_response_bytes": 1048576,
        "capabilities": [{"name": "node.validate", "version": "1"}]
    }))
    .expect("metadata should deserialize");

    assert_eq!(metadata.schema_version, SCHEMA_V1);
    assert_eq!(metadata.domain, DomainName::Cti);
    assert_eq!(metadata.max_concurrency, 16);
    assert_eq!(
        metadata.capabilities,
        vec![CapabilityDeclaration {
            name: "node.validate".to_owned(),
            version: "1".to_owned(),
            deterministic: None,
        }]
    );
}

#[test]
fn claim_verify_capability_declares_determinism_without_breaking_legacy_metadata() {
    let legacy: ProviderMetadata = serde_json::from_value(json!({
        "schema_version": "1",
        "provider_id": "fr.estance.corrobore.domain.medical",
        "provider_version": "0.1.0",
        "domain": "medical",
        "thread_safe": true,
        "max_concurrency": 4,
        "max_request_bytes": 1048576,
        "max_response_bytes": 1048576,
        "capabilities": [{"name": "node.validate", "version": "1"}]
    }))
    .expect("legacy provider metadata should remain valid");
    assert_eq!(legacy.capabilities[0].deterministic, None);

    let verifier: CapabilityDeclaration = serde_json::from_value(json!({
        "name": "claim.verify",
        "version": "1",
        "deterministic": true
    }))
    .expect("claim.verify should accept a determinism declaration");
    assert_eq!(verifier.deterministic, Some(true));

    let error = serde_json::from_value::<CapabilityDeclaration>(json!({
        "name": "claim.verify",
        "version": "1",
        "deterministic": true,
        "precedence_exempt": true
    }))
    .expect_err("providers cannot declare themselves exempt from host precedence");
    assert!(error.to_string().contains("unknown field"));
}

#[test]
fn claim_verify_payload_contract_round_trips_a_governed_verification_run() {
    let request = ClaimVerifyRequestPayload {
        claim: json!({"id": "claim--1", "statement": "example"}),
        links: vec![json!({"kind": "Supports"})],
        observations: vec![json!({"id": "observation--1", "payload": "example"})],
        sources: vec![json!({"id": "source--1", "uri": "https://example.test"})],
        evidence_records: vec![],
        as_of: ClaimVerifyAsOf {
            valid_time: "2026-09-06T00:00:00Z".to_owned(),
            system_time: "2026-09-06T00:01:00Z".to_owned(),
        },
    };
    let encoded = serde_json::to_value(&request).expect("request should serialize");
    let decoded: ClaimVerifyRequestPayload =
        serde_json::from_value(encoded).expect("request should deserialize");
    assert_eq!(decoded, request);

    let response = ClaimVerifyResponsePayload {
        result: ClaimVerifyResult::Fail,
        rationale: Some("domain rule rejected the claim".to_owned()),
        limits: vec!["fixture rule".to_owned()],
        evidence_consumed: vec!["observation:observation--1".to_owned()],
    };
    let encoded = serde_json::to_value(&response).expect("response should serialize");
    let decoded: ClaimVerifyResponsePayload =
        serde_json::from_value(encoded).expect("response should deserialize");
    assert_eq!(decoded, response);
}

#[test]
fn invocation_contract_round_trips_correlated_domain_validation() {
    let request = InvokeRequest {
        schema_version: SCHEMA_V1.to_owned(),
        request_id: "request-123".to_owned(),
        domain: DomainName::Fimi,
        operation: "node.validate".to_owned(),
        workspace_id: Some("workspace--demo".to_owned()),
        snapshot_id: None,
        payload: json!({"labels": ["Narrative"], "properties": {"claim": "example"}}),
    };

    let encoded = serde_json::to_vec(&request).expect("request should serialize");
    let decoded: InvokeRequest =
        serde_json::from_slice(&encoded).expect("request should deserialize");
    assert_eq!(decoded, request);

    let response = InvokeResponse {
        schema_version: SCHEMA_V1.to_owned(),
        request_id: request.request_id.clone(),
        status: ProviderResponseStatus::Rejected,
        issues: vec![ProviderIssue {
            code: "FIMI_CLAIM_REQUIRED".to_owned(),
            message: "narrative requires an attributable claim".to_owned(),
            field: Some("claim".to_owned()),
            severity: IssueSeverity::Error,
            node_id: Some("node--narrative".to_owned()),
        }],
        diagnostics: None,
        payload: None,
    };

    let encoded = serde_json::to_vec(&response).expect("response should serialize");
    let decoded: InvokeResponse =
        serde_json::from_slice(&encoded).expect("response should deserialize");
    assert_eq!(decoded, response);
    assert_eq!(decoded.request_id, request.request_id);
}

#[test]
fn domain_contract_rejects_unknown_domain_names() {
    let error = serde_json::from_value::<ProviderMetadata>(json!({
        "schema_version": "1",
        "provider_id": "com.example.unknown",
        "provider_version": "1.0.0",
        "domain": "unknown",
        "thread_safe": false,
        "max_concurrency": 1,
        "max_request_bytes": 1024,
        "max_response_bytes": 1024,
        "capabilities": []
    }))
    .expect_err("unknown domains must fail closed");

    assert!(error.to_string().contains("unknown variant"));
}

#[test]
fn canonical_c_header_exposes_the_complete_v1_contract() {
    let header = include_str!("../include/corrobore_domain_provider.h");

    assert!(header.contains("corrobore_domain_provider_get_api_v1"));
    assert!(header.contains("corrobore_domain_provider_metadata_v1_fn"));
    assert!(header.contains("corrobore_domain_provider_create_v1_fn"));
    assert!(header.contains("corrobore_domain_provider_invoke_v1_fn"));
    assert!(header.contains("corrobore_domain_provider_health_v1_fn"));
    assert!(header.contains("corrobore_domain_provider_destroy_v1_fn"));
    assert!(header.contains("corrobore_domain_provider_free_buffer_v1_fn"));
    assert!(header.contains("uint16_t abi_major"));
    assert!(header.contains("CORROBORE_DOMAIN_PROVIDER_ABI_MINOR_V1 UINT16_C(2)"));
    assert!(header.contains("claim.verify/1"));
    assert!(header.contains("deterministic"));
    assert!(header.contains("evidence_consumed"));
    assert!(header.contains("size_t struct_size"));
    assert!(!header.contains("std::string"));
    assert!(!header.contains("Vec<"));
}

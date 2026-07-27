// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
#![allow(clippy::unwrap_used)]

//! Contract for the RESEARCH provider over the domain provider ABI v1.

use std::ffi::c_void;

use domain_provider_abi::{
    CORROBORE_DOMAIN_PROVIDER_ABI_MAJOR_V1, CORROBORE_DOMAIN_PROVIDER_ABI_MINOR_V1,
    DomainProviderBuffer, DomainProviderSlice, ProviderMetadata, ProviderResponseStatus, SCHEMA_V1,
    STATUS_INVALID_REQUEST, STATUS_OK, STATUS_UNSUPPORTED_CAPABILITY,
};
use domain_research::research_provider_api_v1;
use serde_json::Value;

fn read_output(output: &DomainProviderBuffer) -> Vec<u8> {
    assert!(
        !output.ptr.is_null(),
        "provider output pointer must not be null"
    );
    // SAFETY: the provider gives ownership of this allocation to the host until
    // free_buffer is called. We only borrow bytes here.
    unsafe { std::slice::from_raw_parts(output.ptr, output.len).to_vec() }
}

fn create_handle() -> *mut c_void {
    // SAFETY: the provider accessor has no preconditions.
    let api = unsafe { &*research_provider_api_v1() };
    let create_fn = api.create.expect("create callback must be present");
    let mut handle: *mut c_void = std::ptr::null_mut();
    let payload = serde_json::json!({"schema_version":"1","domain":"research"}).to_string();
    let bytes = payload.as_bytes();
    // SAFETY: create receives a valid borrowed payload and output handle pointer.
    let status = unsafe {
        create_fn(
            DomainProviderSlice {
                ptr: bytes.as_ptr(),
                len: bytes.len(),
            },
            &mut handle,
        )
    };
    assert_eq!(status, STATUS_OK);
    assert!(!handle.is_null());
    handle
}

fn invoke(handle: *mut c_void, request: &Value) -> (i32, Option<Value>) {
    // SAFETY: the provider accessor has no preconditions.
    let api = unsafe { &*research_provider_api_v1() };
    let invoke_fn = api.invoke.expect("invoke callback must be present");
    let free_fn = api
        .free_buffer
        .expect("free_buffer callback must be present");

    let payload = request.to_string();
    let bytes = payload.as_bytes();
    let mut output = DomainProviderBuffer {
        ptr: std::ptr::null_mut(),
        len: 0,
    };
    // SAFETY: handle came from create and payload/output pointers are valid.
    let status = unsafe {
        invoke_fn(
            handle,
            DomainProviderSlice {
                ptr: bytes.as_ptr(),
                len: bytes.len(),
            },
            &mut output,
        )
    };

    let value = if status == STATUS_OK {
        let out = read_output(&output);
        // SAFETY: output ownership returns to the provider free callback.
        unsafe { free_fn(output) };
        Some(serde_json::from_slice(&out).expect("response must be valid JSON"))
    } else {
        None
    };
    (status, value)
}

fn destroy(handle: *mut c_void) {
    // SAFETY: the provider accessor has no preconditions.
    let api = unsafe { &*research_provider_api_v1() };
    let destroy_fn = api.destroy.expect("destroy callback must be present");
    // SAFETY: handle ownership returns to the provider destroy callback.
    unsafe { destroy_fn(handle) };
}

fn issue_codes(response: &Value) -> Vec<String> {
    response["issues"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|issue| issue["code"].as_str())
        .map(str::to_owned)
        .collect()
}

#[test]
fn provider_entrypoint_exposes_complete_abi_v1_table() {
    // SAFETY: the provider accessor has no preconditions.
    let api = unsafe { &*research_provider_api_v1() };
    assert_eq!(api.abi_major, CORROBORE_DOMAIN_PROVIDER_ABI_MAJOR_V1);
    assert_eq!(api.abi_minor, CORROBORE_DOMAIN_PROVIDER_ABI_MINOR_V1);
    assert!(api.metadata.is_some());
    assert!(api.create.is_some());
    assert!(api.invoke.is_some());
    assert!(api.health.is_some());
    assert!(api.destroy.is_some());
    assert!(api.free_buffer.is_some());
}

#[test]
fn provider_metadata_declares_research_domain_and_capability() {
    // SAFETY: the provider accessor has no preconditions.
    let api = unsafe { &*research_provider_api_v1() };
    let metadata_fn = api.metadata.expect("metadata callback must be present");
    let free_fn = api
        .free_buffer
        .expect("free_buffer callback must be present");

    let mut output = DomainProviderBuffer {
        ptr: std::ptr::null_mut(),
        len: 0,
    };
    // SAFETY: null host context per v1 contract and a valid output pointer.
    let status = unsafe {
        metadata_fn(
            DomainProviderSlice {
                ptr: std::ptr::null(),
                len: 0,
            },
            &mut output,
        )
    };
    assert_eq!(status, STATUS_OK);
    let bytes = read_output(&output);
    // SAFETY: output ownership returns to the provider free callback.
    unsafe { free_fn(output) };

    let metadata: ProviderMetadata =
        serde_json::from_slice(&bytes).expect("metadata must be valid JSON");
    assert_eq!(metadata.domain.as_str(), "research");
    assert_eq!(metadata.provider_id, "fr.estance.corrobore.domain.research");
    assert!(
        metadata
            .capabilities
            .iter()
            .any(|capability| capability.name == "node.validate" && capability.version == SCHEMA_V1)
    );
}

#[test]
fn provider_accepts_an_attributed_claim() {
    let handle = create_handle();
    let request = serde_json::json!({
        "schema_version":"1",
        "request_id":"research-accept",
        "domain":"research",
        "operation":"node.validate",
        "workspace_id":"workspace--contract",
        "snapshot_id":null,
        "payload":{
            "labels":["Claim"],
            "asserting_work":"publication--1",
            "credited_actor":"person--1",
            "evidence_refs":["evidence--1"],
            "intended_status":"validated"
        }
    });
    let (status, response) = invoke(handle, &request);
    assert_eq!(status, STATUS_OK);
    let response = response.unwrap();
    assert_eq!(response["request_id"], "research-accept");
    assert_eq!(
        response["status"],
        serde_json::to_value(ProviderResponseStatus::Accepted).unwrap()
    );
    destroy(handle);
}

#[test]
fn provider_rejects_a_claim_without_attribution() {
    let handle = create_handle();
    let request = serde_json::json!({
        "schema_version":"1",
        "request_id":"research-reject",
        "domain":"research",
        "operation":"node.validate",
        "workspace_id":"workspace--contract",
        "snapshot_id":null,
        "payload":{"labels":["Claim"],"asserting_work":"publication--1"}
    });
    let (status, response) = invoke(handle, &request);
    assert_eq!(status, STATUS_OK);
    let response = response.unwrap();
    assert_eq!(
        response["status"],
        serde_json::to_value(ProviderResponseStatus::Rejected).unwrap()
    );
    assert!(issue_codes(&response).contains(&"RESEARCH_CLAIM_ATTRIBUTION_REQUIRED".to_owned()));
    destroy(handle);
}

#[test]
fn provider_rejects_retracted_support_without_an_override() {
    let handle = create_handle();
    let request = serde_json::json!({
        "schema_version":"1",
        "request_id":"research-retracted",
        "domain":"research",
        "operation":"node.validate",
        "workspace_id":"workspace--contract",
        "snapshot_id":null,
        "payload":{
            "labels":["Claim"],
            "asserting_work":"publication--1",
            "credited_actor":"person--1",
            "evidence_refs":["evidence--1"],
            "supporting_works":[{"work_id":"publication--retracted","retracted":true}],
            "intended_status":"validated"
        }
    });
    let (status, response) = invoke(handle, &request);
    assert_eq!(status, STATUS_OK);
    let response = response.unwrap();
    assert!(
        issue_codes(&response).contains(&"RESEARCH_RETRACTED_SUPPORT_REQUIRES_OVERRIDE".to_owned())
    );
    destroy(handle);
}

#[test]
fn provider_rejects_an_unknown_node_label_rather_than_defaulting() {
    let handle = create_handle();
    let request = serde_json::json!({
        "schema_version":"1",
        "request_id":"research-unknown-label",
        "domain":"research",
        "operation":"node.validate",
        "workspace_id":"workspace--contract",
        "snapshot_id":null,
        "payload":{"labels":["Author"]}
    });
    let (status, response) = invoke(handle, &request);
    assert_eq!(status, STATUS_OK);
    let response = response.unwrap();
    assert_eq!(
        response["status"],
        serde_json::to_value(ProviderResponseStatus::Rejected).unwrap()
    );
    assert!(issue_codes(&response).contains(&"RESEARCH_NODE_TYPE_REQUIRED".to_owned()));
    destroy(handle);
}

#[test]
fn provider_fails_closed_on_domain_mismatch_and_unknown_operation() {
    let handle = create_handle();

    let wrong_domain = serde_json::json!({
        "schema_version":"1",
        "request_id":"research-wrong-domain",
        "domain":"medical",
        "operation":"node.validate",
        "workspace_id":"workspace--contract",
        "snapshot_id":null,
        "payload":{"labels":["Study"]}
    });
    assert_eq!(invoke(handle, &wrong_domain).0, STATUS_INVALID_REQUEST);

    let unknown_operation = serde_json::json!({
        "schema_version":"1",
        "request_id":"research-unknown-op",
        "domain":"research",
        "operation":"node.rank",
        "workspace_id":"workspace--contract",
        "snapshot_id":null,
        "payload":{"labels":["Study"]}
    });
    assert_eq!(
        invoke(handle, &unknown_operation).0,
        STATUS_UNSUPPORTED_CAPABILITY
    );

    destroy(handle);
}

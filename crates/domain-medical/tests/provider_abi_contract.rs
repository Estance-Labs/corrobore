// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
#![allow(clippy::unwrap_used)]

//! Contract for the MEDICAL provider over the domain provider ABI v1.

use std::ffi::c_void;

use domain_medical::corrobore_domain_provider_get_api_v1;
use domain_provider_abi::{
    CORROBORE_DOMAIN_PROVIDER_ABI_MAJOR_V1, CORROBORE_DOMAIN_PROVIDER_ABI_MINOR_V1,
    DomainProviderBuffer, DomainProviderSlice, ProviderMetadata, ProviderResponseStatus, SCHEMA_V1,
    STATUS_INVALID_REQUEST, STATUS_OK,
};
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
    // SAFETY: the provider entrypoint has no preconditions.
    let api = unsafe { &*corrobore_domain_provider_get_api_v1() };
    let create_fn = api.create.expect("create callback must be present");
    let mut handle: *mut c_void = std::ptr::null_mut();
    let payload = serde_json::json!({"schema_version":"1","domain":"medical"}).to_string();
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
    // SAFETY: the provider entrypoint has no preconditions.
    let api = unsafe { &*corrobore_domain_provider_get_api_v1() };
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
    // SAFETY: the provider entrypoint has no preconditions.
    let api = unsafe { &*corrobore_domain_provider_get_api_v1() };
    let destroy_fn = api.destroy.expect("destroy callback must be present");
    // SAFETY: handle ownership returns to the provider destroy callback.
    unsafe { destroy_fn(handle) };
}

#[test]
fn provider_entrypoint_exposes_complete_abi_v1_table() {
    // SAFETY: the provider entrypoint has no preconditions.
    let api = unsafe { &*corrobore_domain_provider_get_api_v1() };
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
fn provider_metadata_declares_medical_domain_and_capability() {
    // SAFETY: the provider entrypoint has no preconditions.
    let api = unsafe { &*corrobore_domain_provider_get_api_v1() };
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
    assert_eq!(metadata.domain.as_str(), "medical");
    assert_eq!(metadata.provider_id, "fr.estance.corrobore.domain.medical");
    assert!(
        metadata
            .capabilities
            .iter()
            .any(|capability| capability.name == "node.validate" && capability.version == SCHEMA_V1)
    );
}

#[test]
fn provider_accepts_a_valid_effect_estimate() {
    let handle = create_handle();
    let request = serde_json::json!({
        "schema_version":"1",
        "request_id":"medical-accept",
        "domain":"medical",
        "operation":"node.validate",
        "workspace_id":"workspace--contract",
        "snapshot_id":null,
        "payload":{
            "labels":["EffectEstimate"],
            "evidence_refs":["evidence--1"],
            "study_refs":["study--a"],
            "intended_status":"validated",
            "effect_estimate":{"measure":"ratio","point":0.82,"interval":[0.70,0.96]}
        }
    });
    let (status, response) = invoke(handle, &request);
    assert_eq!(status, STATUS_OK);
    let response = response.unwrap();
    assert_eq!(response["request_id"], "medical-accept");
    assert_eq!(
        response["status"],
        serde_json::to_value(ProviderResponseStatus::Accepted).unwrap()
    );
    destroy(handle);
}

#[test]
fn provider_rejects_participant_level_content_without_attestation() {
    let handle = create_handle();
    let request = serde_json::json!({
        "schema_version":"1",
        "request_id":"medical-reject",
        "domain":"medical",
        "operation":"node.validate",
        "workspace_id":"workspace--contract",
        "snapshot_id":null,
        "payload":{
            "labels":["Population"],
            "contains_participant_level":true
        }
    });
    let (status, response) = invoke(handle, &request);
    assert_eq!(status, STATUS_OK);
    let response = response.unwrap();
    assert_eq!(
        response["status"],
        serde_json::to_value(ProviderResponseStatus::Rejected).unwrap()
    );
    let codes: Vec<&str> = response["issues"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|issue| issue["code"].as_str())
        .collect();
    assert!(codes.contains(&"MEDICAL_DEIDENTIFICATION_REQUIRED"));
    destroy(handle);
}

#[test]
fn provider_fails_closed_on_domain_mismatch() {
    let handle = create_handle();
    // A request addressed to another domain must not be silently validated.
    let request = serde_json::json!({
        "schema_version":"1",
        "request_id":"medical-wrong-domain",
        "domain":"cti",
        "operation":"node.validate",
        "workspace_id":"workspace--contract",
        "snapshot_id":null,
        "payload":{"labels":["Study"]}
    });
    let (status, _) = invoke(handle, &request);
    assert_eq!(status, STATUS_INVALID_REQUEST);
    destroy(handle);
}

// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! RESEARCH domain provider over the stable Corrobore domain provider ABI v1.
//!
//! Exposes the `node.validate` capability with `domain: research`. Behaviour is
//! fail-closed: malformed input, a domain mismatch, or an unsupported operation
//! yields an explicit status rather than a silent success.

use std::ffi::c_void;

use domain_common::DomainValidationSeverity;
use domain_provider_abi::{
    CORROBORE_DOMAIN_PROVIDER_ABI_MAJOR_V1, CORROBORE_DOMAIN_PROVIDER_ABI_MINOR_V1,
    CapabilityDeclaration, DomainName, DomainProviderApiV1, DomainProviderBuffer,
    DomainProviderSlice, InvokeRequest, InvokeResponse, IssueSeverity, ProviderIssue,
    ProviderMetadata, ProviderResponseStatus, SCHEMA_V1, STATUS_INVALID_ARGUMENT,
    STATUS_INVALID_REQUEST, STATUS_OK, STATUS_PROVIDER_ERROR, STATUS_UNSUPPORTED_CAPABILITY,
};
use serde_json::Value;

use crate::{
    ReplicationAttemptRecord, ReplicationOutcome, ReproducibilityArtifacts, ResearchNodeRecord,
    ResearchNodeType, RetractionOverride, SupportingWork, validate_research_node,
};

struct ResearchProviderHandle;

const PROVIDER_ID: &str = "fr.estance.corrobore.domain.research";
const PROVIDER_VERSION: &str = env!("CARGO_PKG_VERSION");
const OPERATION_NODE_VALIDATE: &str = "node.validate";
const MAX_PAYLOAD_BYTES: usize = 1_048_576;

fn write_json<T: serde::Serialize>(value: &T, out: *mut DomainProviderBuffer) -> i32 {
    if out.is_null() {
        return STATUS_INVALID_ARGUMENT;
    }

    let bytes = match serde_json::to_vec(value) {
        Ok(encoded) => encoded,
        Err(_) => return STATUS_PROVIDER_ERROR,
    };

    let owned = bytes.into_boxed_slice();
    let len = owned.len();
    let ptr = Box::into_raw(owned) as *mut u8;

    // SAFETY: caller provides a valid output pointer by ABI contract.
    unsafe {
        (*out).ptr = ptr;
        (*out).len = len;
    }

    STATUS_OK
}

fn read_json_slice(input: DomainProviderSlice) -> Result<&'static [u8], i32> {
    if input.ptr.is_null() {
        return Err(STATUS_INVALID_ARGUMENT);
    }
    if input.len > MAX_PAYLOAD_BYTES {
        return Err(STATUS_INVALID_REQUEST);
    }

    // SAFETY: provider contract guarantees input bytes remain alive during call.
    let bytes = unsafe { std::slice::from_raw_parts(input.ptr, input.len) };
    Ok(bytes)
}

unsafe extern "C" fn provider_metadata(
    _host_context_json: DomainProviderSlice,
    output_json: *mut DomainProviderBuffer,
) -> i32 {
    let metadata = ProviderMetadata {
        schema_version: SCHEMA_V1.to_owned(),
        provider_id: PROVIDER_ID.to_owned(),
        provider_version: PROVIDER_VERSION.to_owned(),
        domain: DomainName::Research,
        thread_safe: true,
        max_concurrency: 1,
        max_request_bytes: MAX_PAYLOAD_BYTES,
        max_response_bytes: MAX_PAYLOAD_BYTES,
        capabilities: vec![CapabilityDeclaration {
            name: OPERATION_NODE_VALIDATE.to_owned(),
            version: SCHEMA_V1.to_owned(),
        }],
    };

    write_json(&metadata, output_json)
}

unsafe extern "C" fn provider_create(
    _config_json: DomainProviderSlice,
    provider_handle: *mut *mut c_void,
) -> i32 {
    if provider_handle.is_null() {
        return STATUS_INVALID_ARGUMENT;
    }

    let handle = Box::new(ResearchProviderHandle);
    // SAFETY: output handle pointer is validated non-null above.
    unsafe {
        *provider_handle = Box::into_raw(handle) as *mut c_void;
    }
    STATUS_OK
}

fn string_list(payload: &Value, key: &str) -> Vec<String> {
    payload
        .get(key)
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn replication_outcome_from_str(value: &str) -> Option<ReplicationOutcome> {
    let outcome = match value {
        "successful" => ReplicationOutcome::Successful,
        "failed" => ReplicationOutcome::Failed,
        "partially_successful" => ReplicationOutcome::PartiallySuccessful,
        "inconclusive" => ReplicationOutcome::Inconclusive,
        _ => return None,
    };
    Some(outcome)
}

fn supporting_work_from_value(value: &Value) -> Option<SupportingWork> {
    let work_id = value.get("work_id").and_then(Value::as_str)?;
    let mut work = SupportingWork::new(work_id);
    if value
        .get("retracted")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        work = work.retracted();
    }
    if let Some(raw) = value.get("retraction_override") {
        let justification = raw
            .get("justification")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let recorded_by = raw
            .get("recorded_by")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        work = work.with_override(RetractionOverride {
            justification,
            recorded_by,
        });
    }
    Some(work)
}

fn replication_attempt_from_value(value: &Value) -> Option<ReplicationAttemptRecord> {
    let outcome = value
        .get("outcome")
        .and_then(Value::as_str)
        .and_then(replication_outcome_from_str)?;
    Some(ReplicationAttemptRecord {
        target_work: value
            .get("target_work")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        reporting_work: value
            .get("reporting_work")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        outcome,
    })
}

fn to_research_record(payload: &Value) -> ResearchNodeRecord {
    // An unknown or absent label leaves the node type unset, which validation
    // rejects rather than silently defaulting to a permissive type.
    let node_type = payload
        .get("labels")
        .and_then(Value::as_array)
        .and_then(|labels| labels.first())
        .and_then(Value::as_str)
        .and_then(ResearchNodeType::from_label);

    let mut record = ResearchNodeRecord {
        node_type,
        ..ResearchNodeRecord::default()
    };

    if let Some(external_id) = payload.get("external_id").and_then(Value::as_str) {
        record = record.with_external_id(external_id);
    }
    record.evidence_refs = string_list(payload, "evidence_refs");
    record.result_refs = string_list(payload, "result_refs");
    record.source_refs = string_list(payload, "source_refs");
    record.conflict_refs = string_list(payload, "conflict_refs");
    record.supersedes = string_list(payload, "supersedes");

    if let Some(confidence) = payload.get("confidence").and_then(Value::as_f64)
        && let Ok(confidence) = graph_core::Confidence::new(confidence)
    {
        record = record.with_confidence(confidence);
    }
    if let (Some(work), Some(actor)) = (
        payload.get("asserting_work").and_then(Value::as_str),
        payload.get("credited_actor").and_then(Value::as_str),
    ) {
        record = record.with_attribution(work, actor);
    } else {
        // Preserve a partial attribution so validation can reject it.
        record.asserting_work = payload
            .get("asserting_work")
            .and_then(Value::as_str)
            .map(str::to_owned);
        record.credited_actor = payload
            .get("credited_actor")
            .and_then(Value::as_str)
            .map(str::to_owned);
    }

    if let Some(works) = payload.get("supporting_works").and_then(Value::as_array) {
        record.supporting_works = works
            .iter()
            .filter_map(supporting_work_from_value)
            .collect();
    }
    if let Some(attempts) = payload
        .get("replication_attempts")
        .and_then(Value::as_array)
    {
        record.replication_attempts = attempts
            .iter()
            .filter_map(replication_attempt_from_value)
            .collect();
    }

    record.reproducibility = ReproducibilityArtifacts {
        dataset_refs: string_list(payload, "dataset_refs"),
        code_refs: string_list(payload, "code_refs"),
        method_ref: payload
            .get("method_ref")
            .and_then(Value::as_str)
            .map(str::to_owned),
    };

    if payload
        .get("retracted")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        record = record.retracted();
    }
    if payload.get("intended_status").and_then(Value::as_str) == Some("validated") {
        record = record.intended_validated();
    }

    record
}

unsafe extern "C" fn provider_invoke(
    provider_handle: *mut c_void,
    request_json: DomainProviderSlice,
    response_json: *mut DomainProviderBuffer,
) -> i32 {
    if provider_handle.is_null() || response_json.is_null() {
        return STATUS_INVALID_ARGUMENT;
    }

    let request = match read_json_slice(request_json)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<InvokeRequest>(bytes).ok())
    {
        Some(request) => request,
        None => return STATUS_INVALID_REQUEST,
    };

    if request.schema_version != SCHEMA_V1 || request.domain != DomainName::Research {
        return STATUS_INVALID_REQUEST;
    }
    if request.operation != OPERATION_NODE_VALIDATE {
        return STATUS_UNSUPPORTED_CAPABILITY;
    }

    let record = to_research_record(&request.payload);
    let validation = validate_research_node(&record);

    let status = if validation
        .issues()
        .iter()
        .any(|issue| matches!(issue.severity, DomainValidationSeverity::Error))
    {
        ProviderResponseStatus::Rejected
    } else {
        ProviderResponseStatus::Accepted
    };

    let issues: Vec<ProviderIssue> = validation
        .issues()
        .iter()
        .map(|issue| ProviderIssue {
            code: issue.code.clone(),
            message: issue.message.clone(),
            field: issue.field.clone(),
            severity: match issue.severity {
                DomainValidationSeverity::Error => IssueSeverity::Error,
                DomainValidationSeverity::Warning => IssueSeverity::Warning,
            },
            node_id: None,
        })
        .collect();

    let response = InvokeResponse {
        schema_version: SCHEMA_V1.to_owned(),
        request_id: request.request_id,
        status,
        issues,
        diagnostics: None,
    };

    write_json(&response, response_json)
}

unsafe extern "C" fn provider_health(
    provider_handle: *mut c_void,
    health_json: *mut DomainProviderBuffer,
) -> i32 {
    if provider_handle.is_null() || health_json.is_null() {
        return STATUS_INVALID_ARGUMENT;
    }

    write_json(
        &serde_json::json!({
            "schema_version": SCHEMA_V1,
            "status": "ready"
        }),
        health_json,
    )
}

unsafe extern "C" fn provider_destroy(provider_handle: *mut c_void) {
    if provider_handle.is_null() {
        return;
    }

    // SAFETY: handle was created by Box::into_raw in provider_create.
    unsafe {
        let _ = Box::from_raw(provider_handle as *mut ResearchProviderHandle);
    }
}

unsafe extern "C" fn provider_free_buffer(buffer: DomainProviderBuffer) {
    if buffer.ptr.is_null() || buffer.len == 0 {
        return;
    }

    // SAFETY: pointer and len were created from a Box<[u8]> in write_json.
    unsafe {
        let slice_ptr = std::ptr::slice_from_raw_parts_mut(buffer.ptr, buffer.len);
        let _ = Box::from_raw(slice_ptr);
    }
}

static PROVIDER_API: DomainProviderApiV1 = DomainProviderApiV1 {
    abi_major: CORROBORE_DOMAIN_PROVIDER_ABI_MAJOR_V1,
    abi_minor: CORROBORE_DOMAIN_PROVIDER_ABI_MINOR_V1,
    struct_size: std::mem::size_of::<DomainProviderApiV1>(),
    metadata: Some(provider_metadata),
    create: Some(provider_create),
    invoke: Some(provider_invoke),
    health: Some(provider_health),
    destroy: Some(provider_destroy),
    free_buffer: Some(provider_free_buffer),
};

/// Returns this pack's ABI v1 function table.
///
/// This is the accessor Rust callers must use. It is uniquely named per pack,
/// so a binary linking several packs as `rlib`s always reaches the intended
/// provider.
#[must_use]
pub fn research_provider_api_v1() -> *const DomainProviderApiV1 {
    &PROVIDER_API
}

/// Returns the stable ABI v1 function table consumed by the Corrobore host.
///
/// This is the `dlopen`/`dlsym` entry point named by
/// `CORROBORE_DOMAIN_PROVIDER_ENTRYPOINT_V1`, and every pack exports it under
/// the same symbol by design: the host resolves it per loaded library.
///
/// Rust callers must not use it. Because the symbol name is shared, linking two
/// packs as `rlib`s into one binary lets the linker resolve every call to a
/// single definition, silently returning another pack's table. Call
/// [`research_provider_api_v1`] instead, which has no such ambiguity.
#[unsafe(no_mangle)]
pub extern "C" fn corrobore_domain_provider_get_api_v1() -> *const DomainProviderApiV1 {
    &PROVIDER_API
}

// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! MEDICAL domain provider over the stable Corrobore domain provider ABI v1.
//!
//! Exposes the `node.validate` capability with `domain: medical`. Behaviour is
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
    DeidentificationAttestation, DeidentificationMethod, EffectEstimate, EffectMeasure,
    MedicalNodeRecord, MedicalNodeType, MedicalValidationPolicy, ObservationWindow, StudyDesign,
    validate_medical_node,
};

struct MedicalProviderHandle;

const PROVIDER_ID: &str = "fr.estance.corrobore.domain.medical";
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
        domain: DomainName::Medical,
        thread_safe: true,
        max_concurrency: 1,
        max_request_bytes: MAX_PAYLOAD_BYTES,
        max_response_bytes: MAX_PAYLOAD_BYTES,
        capabilities: vec![CapabilityDeclaration {
            name: OPERATION_NODE_VALIDATE.to_owned(),
            version: SCHEMA_V1.to_owned(),
            deterministic: None,
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

    let handle = Box::new(MedicalProviderHandle);
    // SAFETY: output handle pointer is validated non-null above.
    unsafe {
        *provider_handle = Box::into_raw(handle) as *mut c_void;
    }
    STATUS_OK
}

fn study_design_from_str(value: &str) -> Option<StudyDesign> {
    let design = match value {
        "SystematicReview" => StudyDesign::SystematicReview,
        "RandomizedControlledTrial" => StudyDesign::RandomizedControlledTrial,
        "CohortStudy" => StudyDesign::CohortStudy,
        "CaseControlStudy" => StudyDesign::CaseControlStudy,
        "CaseSeries" => StudyDesign::CaseSeries,
        "CaseReport" => StudyDesign::CaseReport,
        "ExpertOpinion" => StudyDesign::ExpertOpinion,
        _ => return None,
    };
    Some(design)
}

fn effect_estimate_from_value(value: &Value) -> Option<EffectEstimate> {
    let measure = match value.get("measure").and_then(Value::as_str)? {
        "ratio" => EffectMeasure::Ratio,
        "difference" => EffectMeasure::Difference,
        _ => return None,
    };
    let point = value.get("point").and_then(Value::as_f64)?;
    let interval = match value.get("interval") {
        Some(Value::Array(bounds)) if bounds.len() == 2 => {
            let low = bounds[0].as_f64()?;
            let high = bounds[1].as_f64()?;
            Some((low, high))
        }
        _ => None,
    };
    Some(EffectEstimate {
        measure,
        point,
        interval,
    })
}

fn deidentification_from_value(value: &Value) -> Option<DeidentificationAttestation> {
    let method = match value.get("method").and_then(Value::as_str)? {
        "SafeHarbor" => DeidentificationMethod::SafeHarbor,
        "ExpertDetermination" => DeidentificationMethod::ExpertDetermination,
        "Aggregate" => DeidentificationMethod::Aggregate,
        "SyntheticData" => DeidentificationMethod::SyntheticData,
        _ => return None,
    };
    let attested_by = value
        .get("attested_by")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    Some(DeidentificationAttestation {
        method,
        attested_by,
    })
}

fn to_medical_record(payload: &Value) -> MedicalNodeRecord {
    let node_type = payload
        .get("labels")
        .and_then(Value::as_array)
        .and_then(|labels| labels.first())
        .and_then(Value::as_str)
        .and_then(MedicalNodeType::from_label)
        .unwrap_or(MedicalNodeType::Evidence);

    let mut record = MedicalNodeRecord::new(node_type);

    if let Some(external_id) = payload.get("external_id").and_then(Value::as_str) {
        record = record.with_external_id(external_id);
    }
    if let Some(evidence_refs) = payload.get("evidence_refs").and_then(Value::as_array) {
        for reference in evidence_refs.iter().filter_map(Value::as_str) {
            record = record.with_evidence_ref(reference);
        }
    }
    if let Some(confidence) = payload.get("confidence").and_then(Value::as_f64)
        && let Ok(confidence) = graph_core::Confidence::new(confidence)
    {
        record = record.with_confidence(confidence);
    }
    if let Some(design) = payload
        .get("study_design")
        .and_then(Value::as_str)
        .and_then(study_design_from_str)
    {
        record = record.with_study_design(design);
    }
    if let Some(study_refs) = payload.get("study_refs").and_then(Value::as_array) {
        for reference in study_refs.iter().filter_map(Value::as_str) {
            record = record.with_study_ref(reference);
        }
    }
    if let Some(estimate) = payload
        .get("effect_estimate")
        .and_then(effect_estimate_from_value)
    {
        record = record.with_effect_estimate(estimate);
    }
    if let Some(window) = payload.get("observation_window")
        && let (Some(start), Some(end)) = (
            window.get("start").and_then(Value::as_str),
            window.get("end").and_then(Value::as_str),
        )
    {
        record = record.with_observation_window(ObservationWindow {
            start: start.to_owned(),
            end: end.to_owned(),
        });
    }
    if payload
        .get("contains_participant_level")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        let attestation = payload
            .get("deidentification")
            .and_then(deidentification_from_value);
        record = record.with_participant_level(attestation);
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

    if request.schema_version != SCHEMA_V1 || request.domain != DomainName::Medical {
        return STATUS_INVALID_REQUEST;
    }
    if request.operation != OPERATION_NODE_VALIDATE {
        return STATUS_UNSUPPORTED_CAPABILITY;
    }

    let policy = MedicalValidationPolicy::strict_default();
    let record = to_medical_record(&request.payload);
    let validation = validate_medical_node(&record, &policy);

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
        payload: None,
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
        let _ = Box::from_raw(provider_handle as *mut MedicalProviderHandle);
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
pub fn medical_provider_api_v1() -> *const DomainProviderApiV1 {
    &PROVIDER_API
}

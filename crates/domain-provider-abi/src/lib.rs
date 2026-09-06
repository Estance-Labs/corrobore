// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! Stable C ABI shared by the Corrobore runtime and enterprise domain providers.
//!
//! The boundary deliberately exposes only fixed-layout scalars, opaque handles,
//! and pointer-length byte buffers. Rust-owned dynamic types must never cross it.

use std::{collections::BTreeMap, ffi::c_void};

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const CORROBORE_DOMAIN_PROVIDER_ABI_MAJOR_V1: u16 = 1;
// Minor 2 adds the optional `claim.verify/1` capability, its JSON payloads,
// and the optional determinism declaration. The C function table is unchanged;
// hosts at minor 2 keep accepting providers built against supported minor 1.
pub const CORROBORE_DOMAIN_PROVIDER_ABI_MINOR_V1: u16 = 2;
/// Oldest ABI v1 minor still accepted by this host.
pub const CORROBORE_DOMAIN_PROVIDER_ABI_MIN_SUPPORTED_MINOR_V1: u16 = 1;
pub const CORROBORE_DOMAIN_PROVIDER_ENTRYPOINT_V1: &[u8] =
    b"corrobore_domain_provider_get_api_v1\0";
pub const SCHEMA_V1: &str = "1";

pub const STATUS_OK: i32 = 0;
pub const STATUS_INVALID_ARGUMENT: i32 = 1;
pub const STATUS_INVALID_REQUEST: i32 = 2;
pub const STATUS_UNSUPPORTED_CAPABILITY: i32 = 3;
pub const STATUS_PROVIDER_ERROR: i32 = 4;
pub const STATUS_RESPONSE_TOO_LARGE: i32 = 5;

/// Borrowed bytes owned by the caller and valid only for the duration of a call.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct DomainProviderSlice {
    pub ptr: *const u8,
    pub len: usize,
}

/// Bytes allocated by a provider and released only through its `free_buffer`.
#[repr(C)]
#[derive(Debug)]
pub struct DomainProviderBuffer {
    pub ptr: *mut u8,
    pub len: usize,
}

pub type MetadataFn = unsafe extern "C" fn(DomainProviderSlice, *mut DomainProviderBuffer) -> i32;
pub type CreateFn = unsafe extern "C" fn(DomainProviderSlice, *mut *mut c_void) -> i32;
pub type InvokeFn =
    unsafe extern "C" fn(*mut c_void, DomainProviderSlice, *mut DomainProviderBuffer) -> i32;
pub type HealthFn = unsafe extern "C" fn(*mut c_void, *mut DomainProviderBuffer) -> i32;
pub type DestroyFn = unsafe extern "C" fn(*mut c_void);
pub type FreeBufferFn = unsafe extern "C" fn(DomainProviderBuffer);

/// Prefix-versioned v1 function table returned by the provider entry point.
///
/// The host validates `abi_major`, the minimum supported `abi_minor`, and
/// `struct_size` before reading any function pointer. Every function is required
/// for a provider accepted by the production loader.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct DomainProviderApiV1 {
    pub abi_major: u16,
    pub abi_minor: u16,
    pub struct_size: usize,
    pub metadata: Option<MetadataFn>,
    pub create: Option<CreateFn>,
    pub invoke: Option<InvokeFn>,
    pub health: Option<HealthFn>,
    pub destroy: Option<DestroyFn>,
    pub free_buffer: Option<FreeBufferFn>,
}

pub type GetDomainProviderApiV1Fn = unsafe extern "C" fn() -> *const DomainProviderApiV1;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DomainName {
    Cti,
    Fimi,
    Crisis,
    /// Clinical and biomedical evidence pack (open source, MIT).
    Medical,
    /// Academic and scientific research pack (open source, MIT).
    Research,
}

impl DomainName {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cti => "cti",
            Self::Fimi => "fimi",
            Self::Crisis => "crisis",
            Self::Medical => "medical",
            Self::Research => "research",
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityDeclaration {
    pub name: String,
    pub version: String,
    /// Whether this capability is mechanically decidable. Omission preserves
    /// compatibility with pre-1.2 providers and defaults host adapters to an
    /// advisory, non-deterministic verifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deterministic: Option<bool>,
}

/// Bitemporal point at which a provider evaluates a claim.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimVerifyAsOf {
    pub valid_time: String,
    pub system_time: String,
}

/// Governed records supplied to the additive `claim.verify/1` capability.
///
/// Records remain JSON values at the cross-repository boundary so domain packs
/// can consume additive graph fields without changing the binary function
/// table or depending on Rust-owned core types.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimVerifyRequestPayload {
    pub claim: Value,
    pub links: Vec<Value>,
    pub observations: Vec<Value>,
    pub sources: Vec<Value>,
    pub evidence_records: Vec<Value>,
    pub as_of: ClaimVerifyAsOf,
}

/// Result vocabulary returned by `claim.verify/1`.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ClaimVerifyResult {
    Pass,
    Fail,
    Inconclusive,
}

/// Capability payload mapped by the host to a governed verification outcome.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimVerifyResponsePayload {
    pub result: ClaimVerifyResult,
    pub rationale: Option<String>,
    pub limits: Vec<String>,
    pub evidence_consumed: Vec<String>,
}

/// Provider identity and operational limits returned before instance creation.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderMetadata {
    pub schema_version: String,
    pub provider_id: String,
    pub provider_version: String,
    pub domain: DomainName,
    pub thread_safe: bool,
    pub max_concurrency: u32,
    pub max_request_bytes: usize,
    pub max_response_bytes: usize,
    pub capabilities: Vec<CapabilityDeclaration>,
}

/// Capability invocation envelope. Capability-specific data remains JSON so
/// adding operations does not alter the binary function table.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InvokeRequest {
    pub schema_version: String,
    pub request_id: String,
    pub domain: DomainName,
    pub operation: String,
    pub workspace_id: Option<String>,
    pub snapshot_id: Option<String>,
    pub payload: Value,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderResponseStatus {
    Accepted,
    Rejected,
    Failed,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum IssueSeverity {
    Error,
    Warning,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderIssue {
    pub code: String,
    pub message: String,
    pub field: Option<String>,
    pub severity: IssueSeverity,
    pub node_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InvokeResponse {
    pub schema_version: String,
    pub request_id: String,
    pub status: ProviderResponseStatus,
    pub issues: Vec<ProviderIssue>,
    pub diagnostics: Option<BTreeMap<String, Value>>,
    /// Capability-specific response. Absent on ABI 1.0 and 1.1 providers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
}

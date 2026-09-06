// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

use std::{
    collections::HashSet,
    ffi::c_void,
    fs,
    path::{Path, PathBuf},
    ptr,
    sync::{Arc, Mutex},
};

use domain_provider_abi::{
    CORROBORE_DOMAIN_PROVIDER_ABI_MAJOR_V1, CORROBORE_DOMAIN_PROVIDER_ABI_MIN_SUPPORTED_MINOR_V1,
    CORROBORE_DOMAIN_PROVIDER_ABI_MINOR_V1, CORROBORE_DOMAIN_PROVIDER_ENTRYPOINT_V1,
    CapabilityDeclaration, ClaimVerifyAsOf, ClaimVerifyRequestPayload, ClaimVerifyResponsePayload,
    ClaimVerifyResult, DomainName, DomainProviderApiV1, DomainProviderBuffer, DomainProviderSlice,
    GetDomainProviderApiV1Fn, InvokeRequest, InvokeResponse, ProviderMetadata,
    ProviderResponseStatus, SCHEMA_V1, STATUS_OK,
};
use libloading::Library;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use super::manifest::DomainProviderManifest;
use graph_core::{
    GraphError, VerificationOutcome, VerificationRequest, Verifier, VerifierCostClass,
    VerifierRegistry, VerifierSpec,
};

const CLAIM_VERIFY_CAPABILITY: &str = "claim.verify";
const CLAIM_VERIFY_CAPABILITY_VERSION: &str = "1";

#[derive(Debug, Error)]
pub(crate) enum DomainProviderRegistryError {
    #[error("{0}")]
    Initialization(String),
    #[error("{0}")]
    Invocation(String),
}

pub(crate) struct DomainProviderRegistry {
    providers: Vec<LoadedDomainProvider>,
}

struct LoadedDomainProvider {
    status: DomainProviderStatus,
    instance: Mutex<ProviderInstance>,
    max_request_bytes: usize,
    max_response_bytes: usize,
}

#[derive(Clone, Serialize)]
pub(crate) struct DomainProviderStatus {
    pub provider_id: String,
    pub provider_version: String,
    pub domain: DomainName,
    pub capabilities: Vec<CapabilityDeclaration>,
    pub ready: bool,
}

impl DomainProviderStatus {
    pub(crate) fn has_capability(&self, name: &str, version: &str) -> bool {
        self.capabilities
            .iter()
            .any(|capability| capability.name == name && capability.version == version)
    }
}

struct ProviderInstance {
    _library: Library,
    api: DomainProviderApiV1,
    handle: *mut c_void,
}

// SAFETY: provider calls are always serialized through the containing Mutex.
// The loaded provider is trusted native code and declares its own concurrency
// posture in metadata; v1 deliberately serializes even thread-safe providers.
unsafe impl Send for ProviderInstance {}

impl Drop for ProviderInstance {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            if let Some(destroy) = self.api.destroy {
                // SAFETY: the handle was returned by this API table's create
                // function and is destroyed before the backing Library drops.
                unsafe { destroy(self.handle) };
            }
            self.handle = ptr::null_mut();
        }
    }
}

impl DomainProviderRegistry {
    pub(crate) fn initialize(
        provider_dir: &Path,
        manifest_file: &Path,
    ) -> Result<Self, DomainProviderRegistryError> {
        // Read and validate the deployment manifest, canonicalize every path,
        // verify hashes, negotiate each provider ABI, and run provider health
        // checks before the HTTP server accepts traffic.
        let provider_root = fs::canonicalize(provider_dir).map_err(|error| {
            DomainProviderRegistryError::Initialization(format!(
                "failed to canonicalize provider directory '{}': {error}",
                provider_dir.display()
            ))
        })?;
        let manifest_json = fs::read_to_string(manifest_file).map_err(|error| {
            DomainProviderRegistryError::Initialization(format!(
                "failed to read provider manifest '{}': {error}",
                manifest_file.display()
            ))
        })?;
        let manifest = DomainProviderManifest::from_json(&manifest_json).map_err(|error| {
            DomainProviderRegistryError::Initialization(format!(
                "invalid provider manifest '{}': {error}",
                manifest_file.display()
            ))
        })?;

        let mut providers = Vec::with_capacity(manifest.providers().len());
        for entry in manifest.providers() {
            let candidate = provider_root.join(&entry.library);
            let library_path = match fs::canonicalize(&candidate) {
                Ok(path) => path,
                Err(error) if !entry.required && error.kind() == std::io::ErrorKind::NotFound => {
                    continue;
                }
                Err(error) => {
                    return Err(DomainProviderRegistryError::Initialization(format!(
                        "failed to canonicalize {} provider library '{}': {error}",
                        entry.domain.as_str(),
                        candidate.display()
                    )));
                }
            };
            if !library_path.starts_with(&provider_root) {
                return Err(DomainProviderRegistryError::Initialization(format!(
                    "{} provider library resolves outside trusted provider directory",
                    entry.domain.as_str()
                )));
            }

            let bytes = fs::read(&library_path).map_err(|error| {
                DomainProviderRegistryError::Initialization(format!(
                    "failed to read {} provider library: {error}",
                    entry.domain.as_str()
                ))
            })?;
            let actual_hash = Sha256::digest(bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            if actual_hash != entry.sha256 {
                return Err(DomainProviderRegistryError::Initialization(format!(
                    "SHA-256 mismatch for {} provider library",
                    entry.domain.as_str()
                )));
            }

            providers.push(load_provider(
                entry.domain,
                library_path,
                &entry.capabilities,
            )?);
        }

        Ok(Self { providers })
    }

    pub(crate) fn provider_count(&self) -> usize {
        self.providers.len()
    }

    pub(crate) fn ready_count(&self) -> usize {
        self.providers
            .iter()
            .filter(|provider| provider.status.ready)
            .count()
    }

    pub(crate) fn statuses(&self) -> Vec<DomainProviderStatus> {
        self.providers
            .iter()
            .map(|provider| provider.status.clone())
            .collect()
    }

    pub(crate) fn status(&self, domain: DomainName) -> Option<&DomainProviderStatus> {
        self.providers
            .iter()
            .find(|provider| provider.status.domain == domain)
            .map(|provider| &provider.status)
    }

    pub(crate) fn invoke(
        &self,
        request: InvokeRequest,
    ) -> Result<InvokeResponse, DomainProviderRegistryError> {
        if request.schema_version != SCHEMA_V1 {
            return Err(invocation_error("unsupported invocation schema"));
        }
        if !is_supported_capability(&request.operation, SCHEMA_V1) {
            return Err(invocation_error(format!(
                "host does not support provider capability {}/{}",
                request.operation, SCHEMA_V1
            )));
        }
        let provider = self
            .providers
            .iter()
            .find(|provider| provider.status.domain == request.domain)
            .ok_or_else(|| {
                invocation_error(format!(
                    "no ready provider for domain {}",
                    request.domain.as_str()
                ))
            })?;
        if !provider
            .status
            .has_capability(&request.operation, SCHEMA_V1)
        {
            return Err(invocation_error(format!(
                "provider does not declare capability {}/{}",
                request.operation, SCHEMA_V1
            )));
        }
        let encoded = serde_json::to_vec(&request)
            .map_err(|error| invocation_error(format!("failed to encode request: {error}")))?;
        if encoded.len() > provider.max_request_bytes {
            return Err(invocation_error("provider request exceeds declared limit"));
        }

        let instance = provider
            .instance
            .lock()
            .map_err(|_| invocation_error("provider instance lock poisoned"))?;
        let invoke = instance
            .api
            .invoke
            .ok_or_else(|| invocation_error("provider invoke function unavailable"))?;
        let mut output = DomainProviderBuffer {
            ptr: ptr::null_mut(),
            len: 0,
        };
        // SAFETY: the provider handle belongs to this API table, the request
        // bytes remain alive for the call, and output is released below.
        let status = unsafe {
            invoke(
                instance.handle,
                DomainProviderSlice {
                    ptr: encoded.as_ptr(),
                    len: encoded.len(),
                },
                &mut output,
            )
        };
        let response_bytes = copy_and_release_invocation_output(
            &instance.api,
            output,
            status,
            provider.max_response_bytes,
            request.domain.as_str(),
        )?;
        let response: InvokeResponse =
            serde_json::from_slice(&response_bytes).map_err(|error| {
                invocation_error(format!("invalid provider response JSON: {error}"))
            })?;
        if response.schema_version != SCHEMA_V1 || response.request_id != request.request_id {
            return Err(invocation_error(
                "provider response schema or request_id mismatch",
            ));
        }
        Ok(response)
    }

    /// Register one host-owned verifier adapter for every provider declaring
    /// `claim.verify/1`. Providers without the capability are intentionally
    /// skipped so ABI 1.1 packs continue to load unchanged.
    pub(crate) fn register_claim_verifiers(
        self: &Arc<Self>,
        registry: &mut VerifierRegistry,
    ) -> Result<usize, GraphError> {
        let mut registered = 0;
        for provider in &self.providers {
            let Some(capability) = provider.status.capabilities.iter().find(|capability| {
                capability.name == CLAIM_VERIFY_CAPABILITY
                    && capability.version == CLAIM_VERIFY_CAPABILITY_VERSION
            }) else {
                continue;
            };
            registry.register(VerifierSpec::new(Box::new(DomainProviderVerifier {
                providers: Arc::clone(self),
                domain: provider.status.domain,
                id: format!("{}.claim.verify", provider.status.provider_id),
                version: provider.status.provider_version.clone(),
                deterministic: capability.deterministic.unwrap_or(false),
            })))?;
            registered += 1;
        }
        Ok(registered)
    }
}

/// Host-side adapter that preserves registry-owned provenance and delegates
/// only the outcome calculation to a native domain provider.
struct DomainProviderVerifier {
    providers: Arc<DomainProviderRegistry>,
    domain: DomainName,
    id: String,
    version: String,
    deterministic: bool,
}

impl Verifier for DomainProviderVerifier {
    fn id(&self) -> &str {
        &self.id
    }

    fn version(&self) -> &str {
        &self.version
    }

    fn deterministic(&self) -> bool {
        self.deterministic
    }

    fn cost_class(&self) -> VerifierCostClass {
        VerifierCostClass::High
    }

    fn verify(&self, request: &VerificationRequest<'_>) -> Result<VerificationOutcome, GraphError> {
        let payload = ClaimVerifyRequestPayload {
            claim: self.to_value(request.claim(), "claim")?,
            links: self.to_values(request.links(), "links")?,
            observations: self.to_values(request.observations(), "observations")?,
            sources: self.to_values(request.sources(), "sources")?,
            evidence_records: self.to_values(request.evidence_records(), "evidence_records")?,
            as_of: ClaimVerifyAsOf {
                valid_time: request.as_of().valid_time().as_str().to_owned(),
                system_time: request.as_of().system_time().as_str().to_owned(),
            },
        };
        let request_id = format!(
            "claim-verify--{}--{}",
            request.claim().id().as_str(),
            request.as_of().system_time().as_str()
        );
        let response = self
            .providers
            .invoke(InvokeRequest {
                schema_version: SCHEMA_V1.to_owned(),
                request_id,
                domain: self.domain,
                operation: CLAIM_VERIFY_CAPABILITY.to_owned(),
                workspace_id: request
                    .claim()
                    .workspace_id()
                    .map(|workspace| workspace.as_str().to_owned()),
                snapshot_id: None,
                payload: serde_json::to_value(payload).map_err(|error| {
                    self.execution_error(format!("cannot encode request payload: {error}"))
                })?,
            })
            .map_err(|error| self.execution_error(error.to_string()))?;
        if response.status != ProviderResponseStatus::Accepted {
            return Err(self.execution_error(format!(
                "provider returned invocation status {:?}",
                response.status
            )));
        }
        let payload = response
            .payload
            .ok_or_else(|| self.execution_error("provider response payload is missing"))?;
        let response: ClaimVerifyResponsePayload =
            serde_json::from_value(payload).map_err(|error| {
                self.execution_error(format!("invalid claim.verify response payload: {error}"))
            })?;

        let result = match response.result {
            ClaimVerifyResult::Pass => graph_core::VerificationResult::Pass,
            ClaimVerifyResult::Fail => graph_core::VerificationResult::Fail,
            ClaimVerifyResult::Inconclusive => graph_core::VerificationResult::Inconclusive,
        };
        let mut outcome = VerificationOutcome::new(result);
        if let Some(rationale) = response.rationale {
            outcome = outcome.with_rationale(rationale);
        }
        for limit in response.limits {
            outcome = outcome.with_limit(limit);
        }
        for evidence in response.evidence_consumed {
            outcome = outcome.with_evidence_consumed(evidence);
        }
        Ok(outcome)
    }
}

impl DomainProviderVerifier {
    fn to_value<T: Serialize + ?Sized>(
        &self,
        value: &T,
        field: &str,
    ) -> Result<serde_json::Value, GraphError> {
        serde_json::to_value(value)
            .map_err(|error| self.execution_error(format!("cannot encode {field}: {error}")))
    }

    fn to_values<T: Serialize>(
        &self,
        values: &[&T],
        field: &str,
    ) -> Result<Vec<serde_json::Value>, GraphError> {
        values
            .iter()
            .map(|value| self.to_value(*value, field))
            .collect()
    }

    fn execution_error(&self, reason: impl Into<String>) -> GraphError {
        GraphError::VerifierExecutionFailed {
            id: self.id.clone(),
            version: self.version.clone(),
            reason: reason.into(),
        }
    }
}

const MAX_METADATA_BYTES: usize = 64 * 1024;
const MAX_HEALTH_BYTES: usize = 16 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderHealth {
    schema_version: String,
    status: String,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct DomainProviderApiHeader {
    abi_major: u16,
    abi_minor: u16,
    struct_size: usize,
}

fn load_provider(
    expected_domain: DomainName,
    library_path: PathBuf,
    required_capabilities: &[CapabilityDeclaration],
) -> Result<LoadedDomainProvider, DomainProviderRegistryError> {
    let domain = expected_domain.as_str();
    // SAFETY: the path has been canonicalized, confined to the trusted root,
    // and hash-verified against the deployment manifest.
    let library = unsafe { Library::new(&library_path) }.map_err(|error| {
        initialization_error(format!("failed to load {domain} provider library: {error}"))
    })?;
    // SAFETY: the symbol name is the fixed v1 contract entrypoint. Copying the
    // function pointer detaches its borrow while `library` remains owned below.
    let get_api =
        unsafe { library.get::<GetDomainProviderApiV1Fn>(CORROBORE_DOMAIN_PROVIDER_ENTRYPOINT_V1) }
            .map_err(|error| {
                initialization_error(format!(
                    "missing v1 entrypoint for {domain} provider: {error}"
                ))
            })?;
    let get_api = *get_api;
    // SAFETY: the entrypoint takes no arguments and returns an immutable table
    // pointer whose lifetime is required to match the loaded library.
    let api_ptr = unsafe { get_api() };
    if api_ptr.is_null() {
        return Err(initialization_error(format!(
            "{domain} provider returned a null v1 API table"
        )));
    }
    // SAFETY: every v1 entrypoint must return at least the fixed ABI header.
    // No function pointer is read until `struct_size` is validated below.
    let header = unsafe { *api_ptr.cast::<DomainProviderApiHeader>() };
    if header.abi_major != CORROBORE_DOMAIN_PROVIDER_ABI_MAJOR_V1 {
        return Err(initialization_error(format!(
            "incompatible ABI major for {domain} provider: host {}, provider {}",
            CORROBORE_DOMAIN_PROVIDER_ABI_MAJOR_V1, header.abi_major
        )));
    }
    if !abi_minor_is_compatible(header.abi_minor, CORROBORE_DOMAIN_PROVIDER_ABI_MINOR_V1) {
        return Err(initialization_error(format!(
            "incompatible ABI minor for {domain} provider: host requires {}, provider {}",
            CORROBORE_DOMAIN_PROVIDER_ABI_MINOR_V1, header.abi_minor
        )));
    }
    if header.struct_size < std::mem::size_of::<DomainProviderApiV1>() {
        return Err(initialization_error(format!(
            "v1 API table is too small for {domain} provider"
        )));
    }
    // SAFETY: the provider-declared table size now covers the complete host v1
    // table, and the pointer is aligned according to the entrypoint contract.
    let api = unsafe { *api_ptr };

    let metadata_fn = api
        .metadata
        .ok_or_else(|| initialization_error(format!("metadata function missing for {domain}")))?;
    let create_fn = api
        .create
        .ok_or_else(|| initialization_error(format!("create function missing for {domain}")))?;
    api.invoke
        .ok_or_else(|| initialization_error(format!("invoke function missing for {domain}")))?;
    let health_fn = api
        .health
        .ok_or_else(|| initialization_error(format!("health function missing for {domain}")))?;
    api.destroy
        .ok_or_else(|| initialization_error(format!("destroy function missing for {domain}")))?;
    api.free_buffer.ok_or_else(|| {
        initialization_error(format!("free_buffer function missing for {domain}"))
    })?;

    let metadata_bytes = call_metadata(&api, metadata_fn, MAX_METADATA_BYTES, domain)?;
    let metadata: ProviderMetadata = serde_json::from_slice(&metadata_bytes).map_err(|error| {
        initialization_error(format!(
            "invalid metadata JSON from {domain} provider: {error}"
        ))
    })?;
    validate_metadata(&metadata, expected_domain, required_capabilities)?;

    let create_config = serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_V1,
        "domain": domain,
    }))
    .map_err(|error| initialization_error(format!("failed to encode provider config: {error}")))?;
    let mut handle = ptr::null_mut();
    // SAFETY: create receives a borrowed byte slice valid for the call and a
    // valid output pointer. A successful provider must return a non-null handle.
    let create_status = unsafe {
        create_fn(
            DomainProviderSlice {
                ptr: create_config.as_ptr(),
                len: create_config.len(),
            },
            &mut handle,
        )
    };
    if create_status != STATUS_OK || handle.is_null() {
        return Err(initialization_error(format!(
            "{domain} provider create failed with status {create_status}"
        )));
    }

    let health_bytes = match call_health(&api, health_fn, handle, MAX_HEALTH_BYTES, domain) {
        Ok(bytes) => bytes,
        Err(error) => {
            if let Some(destroy) = api.destroy {
                // SAFETY: create returned this handle and health failed before
                // ownership was transferred into ProviderInstance.
                unsafe { destroy(handle) };
            }
            return Err(error);
        }
    };
    let health: ProviderHealth = serde_json::from_slice(&health_bytes).map_err(|error| {
        initialization_error(format!(
            "invalid health JSON from {domain} provider: {error}"
        ))
    })?;
    if health.schema_version != SCHEMA_V1 || health.status != "ready" {
        if let Some(destroy) = api.destroy {
            // SAFETY: create returned this handle and validation failed before
            // ownership was transferred into ProviderInstance.
            unsafe { destroy(handle) };
        }
        return Err(initialization_error(format!(
            "{domain} provider health check did not report ready"
        )));
    }

    Ok(LoadedDomainProvider {
        status: DomainProviderStatus {
            provider_id: metadata.provider_id,
            provider_version: metadata.provider_version,
            domain: metadata.domain,
            capabilities: metadata.capabilities,
            ready: true,
        },
        instance: Mutex::new(ProviderInstance {
            _library: library,
            api,
            handle,
        }),
        max_request_bytes: metadata.max_request_bytes,
        max_response_bytes: metadata.max_response_bytes,
    })
}

fn abi_minor_is_compatible(provider_minor: u16, required_minor: u16) -> bool {
    (CORROBORE_DOMAIN_PROVIDER_ABI_MIN_SUPPORTED_MINOR_V1..=required_minor)
        .contains(&provider_minor)
}

fn is_supported_capability(name: &str, version: &str) -> bool {
    matches!(
        (name, version),
        ("node.validate", SCHEMA_V1) | (CLAIM_VERIFY_CAPABILITY, CLAIM_VERIFY_CAPABILITY_VERSION)
    )
}

fn validate_metadata(
    metadata: &ProviderMetadata,
    expected_domain: DomainName,
    required_capabilities: &[CapabilityDeclaration],
) -> Result<(), DomainProviderRegistryError> {
    let domain = expected_domain.as_str();
    if metadata.schema_version != SCHEMA_V1 {
        return Err(initialization_error(format!(
            "unsupported metadata schema from {domain} provider"
        )));
    }
    if metadata.domain != expected_domain {
        return Err(initialization_error(format!(
            "provider domain mismatch: expected {domain}, got {}",
            metadata.domain.as_str()
        )));
    }
    if metadata.provider_id.trim().is_empty()
        || metadata.provider_version.trim().is_empty()
        || metadata.max_concurrency == 0
        || metadata.max_request_bytes == 0
        || metadata.max_response_bytes == 0
    {
        return Err(initialization_error(format!(
            "invalid operational metadata from {domain} provider"
        )));
    }
    let mut capabilities = HashSet::new();
    for capability in &metadata.capabilities {
        if !capabilities.insert((capability.name.as_str(), capability.version.as_str())) {
            return Err(initialization_error(format!(
                "{domain} provider declares duplicate capability {}/{}",
                capability.name, capability.version
            )));
        }
    }
    for required in required_capabilities {
        if !metadata.capabilities.iter().any(|available| {
            available.name == required.name && available.version == required.version
        }) {
            return Err(initialization_error(format!(
                "{domain} provider is missing required capability {}/{}",
                required.name, required.version
            )));
        }
    }
    Ok(())
}

fn call_metadata(
    api: &DomainProviderApiV1,
    function: domain_provider_abi::MetadataFn,
    max_bytes: usize,
    domain: &str,
) -> Result<Vec<u8>, DomainProviderRegistryError> {
    let mut output = DomainProviderBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    // SAFETY: arguments satisfy the v1 metadata contract and output is released
    // through the matching provider allocator before returning.
    let status = unsafe {
        function(
            DomainProviderSlice {
                ptr: ptr::null(),
                len: 0,
            },
            &mut output,
        )
    };
    copy_and_release_output(api, output, status, max_bytes, domain, "metadata")
}

fn call_health(
    api: &DomainProviderApiV1,
    function: domain_provider_abi::HealthFn,
    handle: *mut c_void,
    max_bytes: usize,
    domain: &str,
) -> Result<Vec<u8>, DomainProviderRegistryError> {
    let mut output = DomainProviderBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    // SAFETY: handle was created by this API table and output is released
    // through the matching provider allocator before returning.
    let status = unsafe { function(handle, &mut output) };
    copy_and_release_output(api, output, status, max_bytes, domain, "health")
}

fn copy_and_release_output(
    api: &DomainProviderApiV1,
    output: DomainProviderBuffer,
    status: i32,
    max_bytes: usize,
    domain: &str,
    operation: &str,
) -> Result<Vec<u8>, DomainProviderRegistryError> {
    let free_buffer = api.free_buffer.ok_or_else(|| {
        initialization_error(format!("free_buffer function missing for {domain}"))
    })?;
    if status != STATUS_OK {
        if !output.ptr.is_null() {
            // SAFETY: any non-null output belongs to this provider allocator.
            unsafe { free_buffer(output) };
        }
        return Err(initialization_error(format!(
            "{domain} provider {operation} failed with status {status}"
        )));
    }
    if output.ptr.is_null() || output.len == 0 || output.len > max_bytes {
        if !output.ptr.is_null() {
            // SAFETY: any non-null output belongs to this provider allocator.
            unsafe { free_buffer(output) };
        }
        return Err(initialization_error(format!(
            "invalid {operation} buffer from {domain} provider"
        )));
    }
    // SAFETY: provider returned a non-null buffer of bounded length valid until
    // its matching free_buffer call. Bytes are copied before release.
    let bytes = unsafe { std::slice::from_raw_parts(output.ptr, output.len) }.to_vec();
    // SAFETY: output was allocated by this provider and has not been freed yet.
    unsafe { free_buffer(output) };
    Ok(bytes)
}

fn initialization_error(message: String) -> DomainProviderRegistryError {
    DomainProviderRegistryError::Initialization(message)
}

fn invocation_error(message: impl Into<String>) -> DomainProviderRegistryError {
    DomainProviderRegistryError::Invocation(message.into())
}

fn copy_and_release_invocation_output(
    api: &DomainProviderApiV1,
    output: DomainProviderBuffer,
    status: i32,
    max_bytes: usize,
    domain: &str,
) -> Result<Vec<u8>, DomainProviderRegistryError> {
    let free_buffer = api
        .free_buffer
        .ok_or_else(|| invocation_error("provider free_buffer function unavailable"))?;
    if status != STATUS_OK {
        if !output.ptr.is_null() {
            // SAFETY: any non-null output belongs to this provider allocator.
            unsafe { free_buffer(output) };
        }
        return Err(invocation_error(format!(
            "{domain} provider invocation failed with status {status}"
        )));
    }
    if output.ptr.is_null() || output.len == 0 || output.len > max_bytes {
        if !output.ptr.is_null() {
            // SAFETY: any non-null output belongs to this provider allocator.
            unsafe { free_buffer(output) };
        }
        return Err(invocation_error(format!(
            "invalid invocation buffer from {domain} provider"
        )));
    }
    // SAFETY: the provider returned a bounded buffer valid until free_buffer.
    let bytes = unsafe { std::slice::from_raw_parts(output.ptr, output.len) }.to_vec();
    // SAFETY: output belongs to this provider and has not yet been released.
    unsafe { free_buffer(output) };
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use std::{fs, process::Command, sync::Arc};

    use graph_core::{
        BitemporalStamp, ClaimAnalyticalTarget, ClaimId, ClaimInput, ClaimLink, ClaimLinkKind,
        ClaimLinkSource, ClaimStatement, ClaimStatus, ClaimStore, ClaimTarget, Confidence,
        EvidenceRecordStore, EvidenceSourceType, ObservationId, ObservationInput,
        ObservationModality, ObservationStore, ResolutionInputs, SourceId, SourceInput,
        SourceStore, TemporalTimestamp, VerdictState, VerdictStore, VerificationContext,
        VerificationRecordStore, VerificationResult, VerifierRegistry, resolve_claim_verdict,
    };
    use sha2::{Digest, Sha256};
    use uuid::Uuid;

    use super::{DomainProviderRegistry, abi_minor_is_compatible, validate_metadata};
    use domain_provider_abi::{
        CORROBORE_DOMAIN_PROVIDER_ABI_MINOR_V1, CapabilityDeclaration, DomainName, InvokeRequest,
        ProviderMetadata, ProviderResponseStatus, SCHEMA_V1,
    };
    use serde_json::json;

    #[test]
    fn registry_contract_loads_negotiates_and_health_checks_real_cdylib() {
        let fixture = compile_provider_fixture(1);
        let registry = DomainProviderRegistry::initialize(&fixture.root, &fixture.manifest)
            .expect("compatible healthy provider should load");

        assert_eq!(registry.provider_count(), 1);
        let status = registry
            .status(DomainName::Cti)
            .expect("cti provider status should exist");
        assert_eq!(status.provider_id, "fr.estance.corrobore.domain.cti");
        assert_eq!(status.provider_version, "0.1.0-test");
        assert!(status.ready);
        assert!(status.has_capability("node.validate", "1"));

        let response = registry
            .invoke(InvokeRequest {
                schema_version: SCHEMA_V1.to_owned(),
                request_id: "fixture".to_owned(),
                domain: DomainName::Cti,
                operation: "node.validate".to_owned(),
                workspace_id: Some("workspace--test".to_owned()),
                snapshot_id: None,
                payload: json!({"labels": ["ThreatActor"]}),
            })
            .expect("declared provider capability should execute");
        assert_eq!(response.request_id, "fixture");
        assert_eq!(response.status, ProviderResponseStatus::Accepted);
    }

    #[test]
    fn registry_contract_rejects_incompatible_abi_major() {
        let fixture = compile_provider_fixture(2);
        let error = match DomainProviderRegistry::initialize(&fixture.root, &fixture.manifest) {
            Ok(_) => panic!("incompatible ABI major must fail startup"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("incompatible ABI major"));
    }

    #[test]
    fn registry_contract_rejects_null_api_table() {
        let fixture = compile_provider_fixture_with(1, |source| {
            source.replace(
                "fn corrobore_domain_provider_get_api_v1() -> *const Api { &API }",
                "fn corrobore_domain_provider_get_api_v1() -> *const Api { std::ptr::null() }",
            )
        });
        let error = initialize_fixture(&fixture, "null API table must fail startup");

        assert!(error.to_string().contains("null v1 API table"));
    }

    #[test]
    fn registry_contract_rejects_missing_mandatory_function() {
        let fixture = compile_provider_fixture_with(1, |source| {
            source.replace("invoke: Some(invoke)", "invoke: None")
        });
        let error = initialize_fixture(&fixture, "missing invoke must fail startup");

        assert!(error.to_string().contains("invoke function missing"));
    }

    #[test]
    fn registry_contract_rejects_provider_metadata_domain_mismatch() {
        let fixture = compile_provider_fixture_with(1, |source| {
            source.replace("\"domain\":\"cti\"", "\"domain\":\"fimi\"")
        });
        let error = initialize_fixture(&fixture, "wrong metadata domain must fail startup");

        assert!(error.to_string().contains("provider domain mismatch"));
    }

    #[test]
    fn registry_contract_rejects_unhealthy_provider() {
        let fixture = compile_provider_fixture_with(1, |source| {
            source.replace("\"status\":\"ready\"", "\"status\":\"degraded\"")
        });
        let error = initialize_fixture(&fixture, "unhealthy provider must fail startup");

        assert!(error.to_string().contains("did not report ready"));
    }

    #[test]
    fn registry_contract_rejects_invocation_output_over_declared_limit() {
        let fixture = compile_provider_fixture_with(1, |source| {
            source.replace("\"max_response_bytes\":1048576", "\"max_response_bytes\":8")
        });
        let registry = DomainProviderRegistry::initialize(&fixture.root, &fixture.manifest)
            .expect("provider should load before oversized invocation output");

        let error = registry
            .invoke(InvokeRequest {
                schema_version: SCHEMA_V1.to_owned(),
                request_id: "fixture".to_owned(),
                domain: DomainName::Cti,
                operation: "node.validate".to_owned(),
                workspace_id: None,
                snapshot_id: None,
                payload: json!({}),
            })
            .expect_err("oversized invocation output must fail closed");

        assert!(error.to_string().contains("invalid invocation buffer"));
    }

    #[test]
    fn registry_contract_accepts_equal_or_newer_abi_minor_only() {
        assert!(abi_minor_is_compatible(1, 2));
        assert!(abi_minor_is_compatible(2, 2));
        assert!(!abi_minor_is_compatible(0, 2));
        assert!(!abi_minor_is_compatible(3, 2));
    }

    #[test]
    fn registry_contract_rejects_response_request_id_mismatch() {
        let fixture = compile_provider_fixture(1);
        let registry = DomainProviderRegistry::initialize(&fixture.root, &fixture.manifest)
            .expect("compatible healthy provider should load");

        let error = registry
            .invoke(InvokeRequest {
                schema_version: SCHEMA_V1.to_owned(),
                request_id: "different-request".to_owned(),
                domain: DomainName::Cti,
                operation: "node.validate".to_owned(),
                workspace_id: None,
                snapshot_id: None,
                payload: json!({}),
            })
            .expect_err("provider response must preserve request correlation");

        assert!(error.to_string().contains("request_id mismatch"));
    }

    #[test]
    fn registry_contract_rejects_wrong_domain_and_missing_capability_metadata() {
        let metadata = ProviderMetadata {
            schema_version: SCHEMA_V1.to_owned(),
            provider_id: "fr.estance.corrobore.domain.fimi".to_owned(),
            provider_version: "1.0.0".to_owned(),
            domain: DomainName::Fimi,
            thread_safe: true,
            max_concurrency: 1,
            max_request_bytes: 1024,
            max_response_bytes: 1024,
            capabilities: vec![],
        };
        let required = [CapabilityDeclaration {
            name: "node.validate".to_owned(),
            version: SCHEMA_V1.to_owned(),
            deterministic: None,
        }];

        let wrong_domain = validate_metadata(&metadata, DomainName::Cti, &required)
            .expect_err("provider domain must match its manifest entry");
        assert!(
            wrong_domain
                .to_string()
                .contains("provider domain mismatch")
        );

        let missing_capability = validate_metadata(&metadata, DomainName::Fimi, &required)
            .expect_err("manifest capabilities must be declared by provider metadata");
        assert!(
            missing_capability
                .to_string()
                .contains("missing required capability")
        );
    }

    #[test]
    fn registry_contract_rejects_unknown_provider_capabilities() {
        let fixture = compile_provider_fixture_with(1, |source| {
            source.replace("node.validate", "claim.unknown")
        });
        let manifest = fs::read_to_string(&fixture.manifest).expect("manifest should be readable");
        fs::write(
            &fixture.manifest,
            manifest.replace("node.validate", "claim.unknown"),
        )
        .expect("unknown capability manifest should be written");
        let registry = DomainProviderRegistry::initialize(&fixture.root, &fixture.manifest)
            .expect("unknown declarations may load for forward-compatible status reporting");

        let error = registry
            .invoke(InvokeRequest {
                schema_version: SCHEMA_V1.to_owned(),
                request_id: "unknown-capability".to_owned(),
                domain: DomainName::Cti,
                operation: "claim.unknown".to_owned(),
                workspace_id: None,
                snapshot_id: None,
                payload: json!({}),
            })
            .expect_err("unknown capability dispatch must fail closed");
        assert!(error.to_string().contains("host does not support"));
    }

    #[test]
    fn provider_without_claim_verify_loads_and_registers_no_verifier() {
        let fixture = compile_provider_fixture_with_abi_minor(1, 1, |source| source);
        let providers = Arc::new(
            DomainProviderRegistry::initialize(&fixture.root, &fixture.manifest)
                .expect("legacy provider should load unchanged"),
        );
        let mut verifiers = VerifierRegistry::new();

        let registered = providers
            .register_claim_verifiers(&mut verifiers)
            .expect("absence of claim.verify must not be an error");

        assert_eq!(registered, 0);
        assert!(verifiers.is_empty());
    }

    #[test]
    fn claim_verify_defaults_to_advisory_when_determinism_is_absent() {
        let fixture = compile_claim_verifier_fixture(None, "pass");
        let providers = Arc::new(
            DomainProviderRegistry::initialize(&fixture.root, &fixture.manifest)
                .expect("claim verifier provider should load"),
        );
        let mut verifiers = VerifierRegistry::new();
        assert_eq!(
            providers
                .register_claim_verifiers(&mut verifiers)
                .expect("claim verifier should register"),
            1
        );

        let mut fixture = ClaimFixture::new();
        let record_id = fixture
            .run(&verifiers)
            .expect("provider verifier should run");
        let record = fixture
            .verifications
            .record_by_id(&record_id)
            .expect("verification record should persist");
        assert!(!record.deterministic());
        assert_eq!(record.result(), VerificationResult::Pass);
        assert_eq!(
            record.rationale(),
            Some("fixture domain rule accepted the claim")
        );
    }

    #[test]
    fn deterministic_provider_failure_blocks_a_trusted_verdict() {
        let fixture = compile_claim_verifier_fixture(Some(true), "fail");
        let providers = Arc::new(
            DomainProviderRegistry::initialize(&fixture.root, &fixture.manifest)
                .expect("deterministic claim verifier provider should load"),
        );
        let mut verifiers = VerifierRegistry::new();
        providers
            .register_claim_verifiers(&mut verifiers)
            .expect("claim verifier should register");

        let mut fixture = ClaimFixture::new();
        fixture
            .run(&verifiers)
            .expect("provider verifier should produce a failure record");
        let outcome = resolve_claim_verdict(
            &mut fixture.claims,
            &mut fixture.verdicts,
            &ResolutionInputs::new(
                &fixture.verifications,
                &fixture.evidence,
                &fixture.observations,
                &fixture.sources,
            ),
            &fixture.claim,
            stamp("2026-09-06T00:02:00Z"),
            "deterministic-first-v1",
        )
        .expect("verdict should resolve");

        assert_eq!(
            outcome.state(),
            VerdictState::Mixed,
            "the reachable supporting observation remains visible beside the authoritative failure"
        );
        assert_ne!(outcome.state(), VerdictState::Supported);
        assert_ne!(
            fixture
                .claims
                .claim_by_id(&fixture.claim)
                .expect("claim should remain available")
                .status(),
            ClaimStatus::Validated
        );
    }

    struct ClaimFixture {
        claim: ClaimId,
        claims: ClaimStore,
        observations: ObservationStore,
        sources: SourceStore,
        evidence: EvidenceRecordStore,
        verifications: VerificationRecordStore,
        verdicts: VerdictStore,
    }

    impl ClaimFixture {
        fn new() -> Self {
            let source = SourceId::new("source--provider-fixture").expect("source id");
            let observation =
                ObservationId::new("observation--provider-fixture").expect("observation id");
            let claim = ClaimId::new("claim--provider-fixture").expect("claim id");
            let mut sources = SourceStore::new();
            sources
                .register_source(SourceInput::new(
                    source.clone(),
                    "https://example.test/provider-fixture",
                    EvidenceSourceType::Document,
                ))
                .expect("source should register");
            let mut observations = ObservationStore::new();
            observations
                .create_observation(
                    ObservationInput::new(
                        observation.clone(),
                        source,
                        "A domain-specific assertion.",
                        ObservationModality::Text,
                    ),
                    &sources,
                )
                .expect("observation should register");
            let mut claims = ClaimStore::new();
            claims.register_observation(observation.clone());
            claims
                .create_asserted_claim(
                    ClaimInput::new(
                        claim.clone(),
                        ClaimStatement::new("A domain-specific assertion.")
                            .expect("claim statement"),
                        ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new(
                            "provider-fixture",
                            None,
                        )),
                    )
                    .with_confidence(Confidence::new(0.99).expect("confidence")),
                )
                .expect("claim should register");
            claims
                .attach_link(ClaimLink::new(
                    ClaimLinkSource::Observation(observation),
                    claim.clone(),
                    ClaimLinkKind::Supports,
                ))
                .expect("link should attach");
            Self {
                claim,
                claims,
                observations,
                sources,
                evidence: EvidenceRecordStore::new(),
                verifications: VerificationRecordStore::new(),
                verdicts: VerdictStore::new(),
            }
        }

        fn run(
            &mut self,
            registry: &VerifierRegistry,
        ) -> Result<graph_core::VerificationRecordId, graph_core::GraphError> {
            let context = VerificationContext::new(
                &self.claims,
                &self.observations,
                &self.sources,
                &self.evidence,
            );
            registry.run(
                "fr.estance.corrobore.domain.cti.claim.verify",
                "0.1.0-test",
                &self.claim,
                &context,
                &mut self.verifications,
                stamp("2026-09-06T00:01:00Z"),
            )
        }
    }

    fn stamp(transaction: &str) -> BitemporalStamp {
        BitemporalStamp::new(
            TemporalTimestamp::new("2026-09-06T00:00:00Z").expect("valid time"),
            TemporalTimestamp::new(transaction).expect("transaction time"),
        )
        .expect("stamp")
    }

    fn compile_claim_verifier_fixture(
        deterministic: Option<bool>,
        result: &str,
    ) -> ProviderFixture {
        let declaration = deterministic.map_or_else(
            || r#"{"name":"claim.verify","version":"1"}"#.to_owned(),
            |value| format!(r#"{{"name":"claim.verify","version":"1","deterministic":{value}}}"#),
        );
        let rationale = if result == "fail" {
            "fixture domain rule rejected the claim"
        } else {
            "fixture domain rule accepted the claim"
        };
        let payload = format!(
            r#""payload":{{"result":"{result}","rationale":"{rationale}","limits":["fixture domain rule"],"evidence_consumed":["observation:observation--provider-fixture"]}}"#
        );
        let fixture = compile_provider_fixture_with(1, |source| {
            source
                .replace(r#"{"name":"node.validate","version":"1"}"#, &declaration)
                .replace(
                    r#""request_id":"fixture""#,
                    r#""request_id":"claim-verify--claim--provider-fixture--2026-09-06T00:01:00Z""#,
                )
                .replace(
                    r#""diagnostics":null"#,
                    &format!(r#""diagnostics":null,{payload}"#),
                )
        });
        let manifest = fs::read_to_string(&fixture.manifest).expect("manifest should be readable");
        fs::write(
            &fixture.manifest,
            manifest.replace(r#"{"name":"node.validate","version":"1"}"#, &declaration),
        )
        .expect("claim verifier manifest should be written");
        fixture
    }

    struct ProviderFixture {
        root: std::path::PathBuf,
        manifest: std::path::PathBuf,
    }

    fn compile_provider_fixture(abi_major: u16) -> ProviderFixture {
        compile_provider_fixture_with(abi_major, |source| source)
    }

    fn compile_provider_fixture_with(
        abi_major: u16,
        transform: impl FnOnce(String) -> String,
    ) -> ProviderFixture {
        compile_provider_fixture_with_abi_minor(
            abi_major,
            CORROBORE_DOMAIN_PROVIDER_ABI_MINOR_V1,
            transform,
        )
    }

    fn compile_provider_fixture_with_abi_minor(
        abi_major: u16,
        abi_minor: u16,
        transform: impl FnOnce(String) -> String,
    ) -> ProviderFixture {
        let root = std::env::temp_dir().join(format!("corrobore-provider-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).expect("fixture root should be created");
        let source = root.join("provider.rs");
        let library_name = if cfg!(target_os = "macos") {
            "libdomain_cti.dylib"
        } else if cfg!(target_os = "windows") {
            "domain_cti.dll"
        } else {
            "libdomain_cti.so"
        };
        let library = root.join(library_name);
        fs::write(&source, transform(provider_source(abi_major, abi_minor)))
            .expect("fixture source should be written");
        let output = Command::new("rustc")
            .args(["--edition=2021", "--crate-type", "cdylib"])
            .arg(&source)
            .arg("-o")
            .arg(&library)
            .output()
            .expect("rustc should compile provider fixture");
        assert!(
            output.status.success(),
            "provider fixture compilation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let hash = Sha256::digest(fs::read(&library).expect("fixture library should be readable"))
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let manifest = root.join("providers.json");
        fs::write(
            &manifest,
            format!(
                r#"{{"schema_version":"1","providers":[{{"domain":"cti","library":"{library_name}","sha256":"{hash}","required":true,"capabilities":[{{"name":"node.validate","version":"1"}}]}}]}}"#
            ),
        )
        .expect("fixture manifest should be written");

        ProviderFixture { root, manifest }
    }

    fn initialize_fixture(
        fixture: &ProviderFixture,
        expectation: &str,
    ) -> super::DomainProviderRegistryError {
        match DomainProviderRegistry::initialize(&fixture.root, &fixture.manifest) {
            Ok(_) => panic!("{expectation}"),
            Err(error) => error,
        }
    }

    /// Builds a fake provider.
    ///
    /// `abi_minor` tracks the host constant rather than a literal, so bumping
    /// the ABI minor does not silently invalidate every fixture built here.
    fn provider_source(abi_major: u16, abi_minor: u16) -> String {
        format!(
            r###"
use std::ffi::c_void;

#[repr(C)] #[derive(Clone, Copy)] struct Slice {{ ptr: *const u8, len: usize }}
#[repr(C)] struct Buffer {{ ptr: *mut u8, len: usize }}
type MetadataFn = unsafe extern "C" fn(Slice, *mut Buffer) -> i32;
type CreateFn = unsafe extern "C" fn(Slice, *mut *mut c_void) -> i32;
type InvokeFn = unsafe extern "C" fn(*mut c_void, Slice, *mut Buffer) -> i32;
type HealthFn = unsafe extern "C" fn(*mut c_void, *mut Buffer) -> i32;
type DestroyFn = unsafe extern "C" fn(*mut c_void);
type FreeBufferFn = unsafe extern "C" fn(Buffer);
#[repr(C)] struct Api {{
    abi_major: u16, abi_minor: u16, struct_size: usize,
    metadata: Option<MetadataFn>, create: Option<CreateFn>, invoke: Option<InvokeFn>,
    health: Option<HealthFn>, destroy: Option<DestroyFn>, free_buffer: Option<FreeBufferFn>,
}}
fn output(value: &str, out: *mut Buffer) -> i32 {{
    if out.is_null() {{ return 1; }}
    let bytes = value.as_bytes().to_vec().into_boxed_slice();
    let len = bytes.len();
    let ptr = Box::into_raw(bytes) as *mut u8;
    unsafe {{ *out = Buffer {{ ptr, len }}; }}
    0
}}
unsafe extern "C" fn metadata(_: Slice, out: *mut Buffer) -> i32 {{
    output(r#"{{"schema_version":"1","provider_id":"fr.estance.corrobore.domain.cti","provider_version":"0.1.0-test","domain":"cti","thread_safe":true,"max_concurrency":4,"max_request_bytes":1048576,"max_response_bytes":1048576,"capabilities":[{{"name":"node.validate","version":"1"}}]}}"#, out)
}}
unsafe extern "C" fn create(_: Slice, out: *mut *mut c_void) -> i32 {{
    if out.is_null() {{ return 1; }}
    unsafe {{ *out = Box::into_raw(Box::new(1_u8)).cast(); }}
    0
}}
unsafe extern "C" fn invoke(_: *mut c_void, _: Slice, out: *mut Buffer) -> i32 {{
    output(r#"{{"schema_version":"1","request_id":"fixture","status":"accepted","issues":[],"diagnostics":null}}"#, out)
}}
unsafe extern "C" fn health(_: *mut c_void, out: *mut Buffer) -> i32 {{
    output(r#"{{"schema_version":"1","status":"ready"}}"#, out)
}}
unsafe extern "C" fn destroy(handle: *mut c_void) {{
    if !handle.is_null() {{ unsafe {{ drop(Box::from_raw(handle.cast::<u8>())); }} }}
}}
unsafe extern "C" fn free_buffer(buffer: Buffer) {{
    if !buffer.ptr.is_null() {{
        unsafe {{ drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(buffer.ptr, buffer.len))); }}
    }}
}}
static API: Api = Api {{
    abi_major: {abi_major}, abi_minor: {abi_minor}, struct_size: std::mem::size_of::<Api>(),
    metadata: Some(metadata), create: Some(create), invoke: Some(invoke),
    health: Some(health), destroy: Some(destroy), free_buffer: Some(free_buffer),
}};
#[no_mangle] pub extern "C" fn corrobore_domain_provider_get_api_v1() -> *const Api {{ &API }}
"###
        )
    }
}

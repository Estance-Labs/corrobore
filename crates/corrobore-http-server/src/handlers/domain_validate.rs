// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

use std::time::Duration;

use axum::{
    Json,
    extract::{Path, State},
};
use domain_provider_abi::{DomainName, InvokeRequest, InvokeResponse, SCHEMA_V1};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{app::AppState, error::ApiError};

#[cfg(feature = "enterprise-cti")]
const CTI_COMPILED: bool = true;
#[cfg(not(feature = "enterprise-cti"))]
const CTI_COMPILED: bool = false;
#[cfg(feature = "enterprise-fimi")]
const FIMI_COMPILED: bool = true;
#[cfg(not(feature = "enterprise-fimi"))]
const FIMI_COMPILED: bool = false;
#[cfg(feature = "enterprise-crisis")]
const CRISIS_COMPILED: bool = true;
#[cfg(not(feature = "enterprise-crisis"))]
const CRISIS_COMPILED: bool = false;

#[derive(Debug, Deserialize)]
pub struct DomainValidationRequest {
    pub request_id: Option<String>,
    pub workspace_id: Option<String>,
    pub snapshot_id: Option<String>,
    pub payload: Value,
}

#[derive(Debug, Serialize)]
pub struct DomainValidationResponse {
    pub ok: bool,
    pub result: InvokeResponse,
}

pub async fn validate_domain(
    State(state): State<AppState>,
    Path(domain): Path<String>,
    Json(payload): Json<DomainValidationRequest>,
) -> Result<Json<DomainValidationResponse>, ApiError> {
    let (domain, compiled) = parse_domain(&domain)?;
    if !compiled {
        return Err(ApiError::forbidden(
            "FEATURE_NOT_AVAILABLE",
            format!(
                "domain '{}' requires its enterprise build feature",
                domain.as_str()
            ),
        ));
    }
    if !state.config.is_module_licensed(domain.as_str()) {
        return Err(ApiError::forbidden(
            "LICENSE_MODULE_MISSING",
            format!(
                "domain '{}' requires a valid enterprise license claim",
                domain.as_str()
            ),
        ));
    }
    let registry = state.domain_providers.clone().ok_or_else(|| {
        ApiError::service_unavailable(
            "DOMAIN_PROVIDER_NOT_READY",
            format!("domain '{}' provider is not configured", domain.as_str()),
        )
    })?;
    let status = registry.status(domain).ok_or_else(|| {
        ApiError::service_unavailable(
            "DOMAIN_PROVIDER_NOT_READY",
            format!("domain '{}' provider is not loaded", domain.as_str()),
        )
    })?;
    if !status.ready || !status.has_capability("node.validate", SCHEMA_V1) {
        return Err(ApiError::service_unavailable(
            "DOMAIN_PROVIDER_CAPABILITY_MISSING",
            format!(
                "domain '{}' provider does not expose node.validate/{}",
                domain.as_str(),
                SCHEMA_V1
            ),
        ));
    }

    let request = InvokeRequest {
        schema_version: SCHEMA_V1.to_owned(),
        request_id: payload
            .request_id
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        domain,
        operation: "node.validate".to_owned(),
        workspace_id: payload.workspace_id,
        snapshot_id: payload.snapshot_id,
        payload: payload.payload,
    };
    let timeout = Duration::from_millis(state.config.request_timeout_ms);
    let response = tokio::time::timeout(
        timeout,
        tokio::task::spawn_blocking(move || registry.invoke(request)),
    )
    .await
    .map_err(|_| ApiError::timeout("REQUEST_TIMEOUT", "domain provider invocation timeout"))?
    .map_err(|error| ApiError::internal("TASK_JOIN_FAILED", error.to_string()))?
    .map_err(|error| ApiError::bad_gateway("DOMAIN_PROVIDER_ERROR", error.to_string()))?;

    Ok(Json(DomainValidationResponse {
        ok: true,
        result: response,
    }))
}

fn parse_domain(value: &str) -> Result<(DomainName, bool), ApiError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "cti" => Ok((DomainName::Cti, CTI_COMPILED)),
        "fimi" => Ok((DomainName::Fimi, FIMI_COMPILED)),
        "crisis" => Ok((DomainName::Crisis, CRISIS_COMPILED)),
        _ => Err(ApiError::bad_request(
            "INVALID_DOMAIN",
            "unknown domain; use cti, fimi, or crisis",
        )),
    }
}

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

/// Accepted values for the `domain` path segment, in error-message order.
const ACCEPTED_DOMAINS: &str = "cti, fimi, crisis, medical, or research";

/// How a domain is gated before its request reaches the provider registry.
enum DomainGating {
    /// Enterprise domain: requires both its build feature and a signed license
    /// claim naming the module.
    Enterprise {
        /// Whether the enterprise feature was compiled into this build.
        compiled: bool,
    },
    /// Open-source domain: part of the MIT runtime, gated by neither a build
    /// feature nor a license claim.
    OpenSource,
}

/// Maps a domain to its gating.
///
/// The match is exhaustive on purpose. Adding a `DomainName` variant breaks
/// this function rather than letting the new domain inherit whichever branch
/// happens to be last, so the enterprise-versus-open-source decision is always
/// made explicitly.
const fn gating_for(domain: DomainName) -> DomainGating {
    match domain {
        DomainName::Cti => DomainGating::Enterprise {
            compiled: CTI_COMPILED,
        },
        DomainName::Fimi => DomainGating::Enterprise {
            compiled: FIMI_COMPILED,
        },
        DomainName::Crisis => DomainGating::Enterprise {
            compiled: CRISIS_COMPILED,
        },
        // The MEDICAL and RESEARCH packs ship under MIT as part of the
        // open-source runtime. Requiring an enterprise license to call them
        // would make an open-source pack unusable without a commercial
        // agreement.
        DomainName::Medical | DomainName::Research => DomainGating::OpenSource,
    }
}

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
    let domain = parse_domain(&domain)?;
    if let DomainGating::Enterprise { compiled } = gating_for(domain) {
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

fn parse_domain(value: &str) -> Result<DomainName, ApiError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "cti" => Ok(DomainName::Cti),
        "fimi" => Ok(DomainName::Fimi),
        "crisis" => Ok(DomainName::Crisis),
        "medical" => Ok(DomainName::Medical),
        "research" => Ok(DomainName::Research),
        _ => Err(ApiError::bad_request(
            "INVALID_DOMAIN",
            format!("unknown domain; use {ACCEPTED_DOMAINS}"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_accepted_domain_parses_to_its_variant() {
        for (value, expected) in [
            ("cti", DomainName::Cti),
            ("fimi", DomainName::Fimi),
            ("crisis", DomainName::Crisis),
            ("medical", DomainName::Medical),
            ("research", DomainName::Research),
        ] {
            let parsed = parse_domain(value).expect("accepted domain should parse");
            assert_eq!(parsed, expected);
            // Casing and surrounding whitespace are normalized.
            assert_eq!(
                parse_domain(&format!("  {} ", value.to_ascii_uppercase()))
                    .expect("normalized input should parse"),
                expected
            );
        }

        assert!(parse_domain("astrology").is_err());
    }

    #[test]
    fn open_source_domains_carry_no_enterprise_gating() {
        for domain in [DomainName::Medical, DomainName::Research] {
            assert!(
                matches!(gating_for(domain), DomainGating::OpenSource),
                "{} must not be enterprise-gated",
                domain.as_str()
            );
        }
    }

    #[test]
    fn enterprise_domains_keep_their_feature_gating() {
        for (domain, compiled) in [
            (DomainName::Cti, CTI_COMPILED),
            (DomainName::Fimi, FIMI_COMPILED),
            (DomainName::Crisis, CRISIS_COMPILED),
        ] {
            match gating_for(domain) {
                DomainGating::Enterprise { compiled: actual } => assert_eq!(
                    actual,
                    compiled,
                    "{} must report its build feature",
                    domain.as_str()
                ),
                DomainGating::OpenSource => {
                    panic!("{} must stay enterprise-gated", domain.as_str())
                }
            }
        }
    }
}

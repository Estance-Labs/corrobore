// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

use axum::{Json, extract::State, http::HeaderMap};
use serde::Serialize;

use crate::{app::AppState, domain_providers::registry::DomainProviderStatus, error::ApiError};

#[derive(Serialize)]
pub struct DomainProviderStatusResponse {
    ok: bool,
    result: DomainProviderStatusResult,
}

#[derive(Serialize)]
struct DomainProviderStatusResult {
    providers: Vec<DomainProviderStatus>,
}

pub async fn admin_domain_provider_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<DomainProviderStatusResponse>, ApiError> {
    crate::auth::require_admin_or_policy(
        &state,
        &headers,
        crate::policy::PolicyRequest::graph_wide(crate::policy::PolicyAction::Admin),
    )
    .await?;
    let providers = state
        .domain_providers
        .as_deref()
        .map(|registry| registry.statuses())
        .unwrap_or_default();
    Ok(Json(DomainProviderStatusResponse {
        ok: true,
        result: DomainProviderStatusResult { providers },
    }))
}

// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

use axum::{Json, extract::State, http::HeaderMap};
use serde::Serialize;

use crate::{
    app::AppState, auth::require_admin_auth, enterprise::registry::DomainProviderStatus,
    error::ApiError,
};

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
    require_admin_auth(&state, &headers)?;
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

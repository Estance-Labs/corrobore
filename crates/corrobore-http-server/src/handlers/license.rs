// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.
use axum::{Json, extract::State, http::HeaderMap};
use serde::Serialize;

use crate::{app::AppState, auth::require_admin_auth, error::ApiError};

#[derive(Debug, Serialize)]
pub struct LicenseStatusResponse {
    pub ok: bool,
    pub result: LicenseStatusResult,
}

#[derive(Debug, Serialize)]
pub struct LicenseStatusResult {
    pub source: String,
    pub client_uuid: Option<String>,
    pub client_email: Option<String>,
    pub modules: Vec<String>,
}

pub async fn license_status(State(state): State<AppState>) -> Json<LicenseStatusResponse> {
    Json(LicenseStatusResponse {
        ok: true,
        result: license_status_result(&state),
    })
}

pub async fn admin_license_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<LicenseStatusResponse>, ApiError> {
    require_admin_auth(&state, &headers)?;

    Ok(Json(LicenseStatusResponse {
        ok: true,
        result: license_status_result(&state),
    }))
}

fn license_status_result(state: &AppState) -> LicenseStatusResult {
    let source = if state.config.license_client_uuid.is_some()
        && state.config.license_client_email.is_some()
    {
        "signed_pem"
    } else {
        "none"
    };

    LicenseStatusResult {
        source: source.to_owned(),
        client_uuid: state.config.license_client_uuid.clone(),
        client_email: state.config.license_client_email.clone(),
        modules: state.config.licensed_modules.clone(),
    }
}

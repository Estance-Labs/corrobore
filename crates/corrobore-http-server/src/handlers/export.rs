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
use std::time::Duration;

use axum::{
    Json,
    extract::{Query, State},
};
use export_stix::export_stix_subset_bundle;
use graph_core::{
    ExportMetadata, ExportMode, ExportProfile, TransactionId, build_deterministic_export_plan,
};
use serde::Deserialize;
use serde_json::Value;

use crate::{app::AppState, error::ApiError};

#[derive(Debug, Deserialize)]
pub struct ExportQuery {
    pub snapshot_id: Option<String>,
    pub transaction_id: Option<String>,
    pub exporter_version: Option<String>,
    pub mode: Option<String>,
    pub profile: Option<String>,
}

pub async fn export_stix(
    State(state): State<AppState>,
    Query(query): Query<ExportQuery>,
) -> Result<Json<Value>, ApiError> {
    let timeout = Duration::from_millis(state.config.request_timeout_ms);
    let gateway = state.gateway.clone();

    let bundle = tokio::time::timeout(
        timeout,
        tokio::task::spawn_blocking(move || {
            let gateway = gateway.lock().map_err(|_| {
                ApiError::internal("STATE_LOCK_FAILED", "cypher gateway lock poisoned")
            })?;

            let snapshot_id = query
                .snapshot_id
                .clone()
                .unwrap_or_else(|| "snapshot--current".to_owned());

            let transaction_id = TransactionId::new(
                query
                    .transaction_id
                    .clone()
                    .unwrap_or_else(|| "transaction--http-export".to_owned()),
            )
            .map_err(|error| ApiError::bad_request("INVALID_TRANSACTION_ID", error.to_string()))?;

            let exporter_version = query
                .exporter_version
                .clone()
                .unwrap_or_else(|| "corrobore-http-server-v0".to_owned());

            let mode = parse_mode(query.mode.as_deref())?;
            let profile = parse_profile(query.profile.as_deref())?;

            let metadata = ExportMetadata::new(
                snapshot_id,
                transaction_id,
                exporter_version,
                profile,
                mode,
                None,
            )
            .map_err(|error| ApiError::bad_request("INVALID_EXPORT_METADATA", error.to_string()))?;

            let plan = build_deterministic_export_plan(gateway.graph(), metadata, &[])
                .map_err(|error| ApiError::bad_request("EXPORT_PLAN_FAILED", error.to_string()))?;

            Ok::<_, ApiError>(export_stix_subset_bundle(gateway.graph(), &plan))
        }),
    )
    .await
    .map_err(|_| ApiError::timeout("REQUEST_TIMEOUT", "export timeout"))?
    .map_err(|error| ApiError::internal("TASK_JOIN_FAILED", error.to_string()))??;

    let json = serde_json::to_value(bundle)
        .map_err(|error| ApiError::internal("SERIALIZATION_FAILED", error.to_string()))?;

    Ok(Json(json))
}

fn parse_mode(value: Option<&str>) -> Result<ExportMode, ApiError> {
    match value.map(|raw| raw.trim().to_ascii_lowercase()) {
        Some(mode) if mode == "strict" => Ok(ExportMode::Strict),
        Some(mode) if mode == "permissive" => Ok(ExportMode::Permissive),
        Some(mode) => Err(ApiError::bad_request(
            "INVALID_EXPORT_MODE",
            format!("unsupported export mode: {mode}"),
        )),
        None => Ok(ExportMode::Strict),
    }
}

fn parse_profile(value: Option<&str>) -> Result<ExportProfile, ApiError> {
    match value.map(|raw| raw.trim().to_ascii_lowercase()) {
        Some(profile) if profile == "stix-mvp" => Ok(ExportProfile::StixMvp),
        Some(profile) => Err(ApiError::bad_request(
            "INVALID_EXPORT_PROFILE",
            format!("unsupported export profile: {profile}"),
        )),
        None => Ok(ExportProfile::StixMvp),
    }
}

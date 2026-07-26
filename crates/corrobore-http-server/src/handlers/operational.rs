// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! Stable, unauthenticated operational contracts for orchestrators and probes.

use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

use crate::{LifecycleState, app::AppState, durability::collect_durability_snapshot};

#[derive(Serialize)]
struct LivenessResponse {
    status: &'static str,
    live: bool,
    service: &'static str,
    lifecycle_state: &'static str,
}

#[derive(Serialize)]
struct ReadinessChecks {
    engine_initialized: bool,
    storage_recovered: bool,
    accepting_requests: bool,
}

#[derive(Serialize)]
struct ReadinessResponse {
    status: &'static str,
    ready: bool,
    service: &'static str,
    lifecycle_state: &'static str,
    checks: ReadinessChecks,
}

#[derive(Serialize)]
struct StorageCompatibility {
    supported_versions: [&'static str; 1],
    supported_record_formats: [&'static str; 1],
    active_storage_version: Option<&'static str>,
    active_record_format: Option<&'static str>,
}

#[derive(Serialize)]
struct VersionResponse {
    service: &'static str,
    version: &'static str,
    commit: &'static str,
    build_target: String,
    storage_compatibility: StorageCompatibility,
    opencti_mode: &'static str,
}

/// Report only whether the HTTP event loop can serve a response.
pub async fn liveness(State(state): State<AppState>) -> Response {
    Json(LivenessResponse {
        status: "live",
        live: true,
        service: "corrobore-http-server",
        lifecycle_state: state.lifecycle.state().as_str(),
    })
    .into_response()
}

/// Report whether initialization and storage recovery are complete and the
/// lifecycle is accepting work.
pub async fn readiness(State(state): State<AppState>) -> Response {
    let durability = collect_durability_snapshot(&state);
    let storage_recovered = matches!(durability.recovery.outcome, "ephemeral" | "recovered");
    let accepting_requests = state.lifecycle.state() == LifecycleState::Ready;
    let ready = storage_recovered && accepting_requests;
    (
        if ready {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        Json(ReadinessResponse {
            status: if ready { "ready" } else { "not_ready" },
            ready,
            service: "corrobore-http-server",
            lifecycle_state: state.lifecycle.state().as_str(),
            checks: ReadinessChecks {
                engine_initialized: true,
                storage_recovered,
                accepting_requests,
            },
        }),
    )
        .into_response()
}

/// Expose reproducible build identity and storage-format compatibility without
/// runtime configuration or secrets.
pub async fn version(State(state): State<AppState>) -> Response {
    let durability = collect_durability_snapshot(&state);
    Json(VersionResponse {
        service: "corrobore-http-server",
        version: env!("CARGO_PKG_VERSION"),
        commit: option_env!("CORROBORE_BUILD_REVISION").unwrap_or("unknown"),
        build_target: option_env!("CORROBORE_BUILD_TARGET")
            .map(str::to_owned)
            .unwrap_or_else(|| format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS)),
        storage_compatibility: StorageCompatibility {
            supported_versions: ["V1"],
            supported_record_formats: ["JsonLinesV1"],
            active_storage_version: durability.storage_version,
            active_record_format: durability.record_format,
        },
        opencti_mode: if state.config.opencti_elastic_free {
            "elastic_free"
        } else {
            "reversible_reference"
        },
    })
    .into_response()
}

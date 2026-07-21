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
use axum::{Json, extract::State};
use serde::Serialize;

use crate::app::AppState;
use crate::durability::{DurabilityObservabilitySnapshot, collect_durability_snapshot};

#[derive(Serialize)]
struct SessionTtlMetrics {
    total_expired_sessions: u64,
    expired_last_5m_sessions: u64,
}

#[derive(Serialize)]
struct DomainProviderHealth {
    configured: usize,
    ready: usize,
}

#[derive(Serialize)]
pub struct HealthResponse {
    status: &'static str,
    service: &'static str,
    version: &'static str,
    storage_mode: &'static str,
    uptime_ms: u128,
    session_ttl_metrics: SessionTtlMetrics,
    domain_providers: DomainProviderHealth,
    durability: DurabilityObservabilitySnapshot,
}

pub async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    let session_ttl_metrics = state
        .sessions
        .lock()
        .ok()
        .and_then(|sessions| sessions.expiration_metrics().ok())
        .map(|metrics| SessionTtlMetrics {
            total_expired_sessions: metrics.total_expired_sessions,
            expired_last_5m_sessions: metrics.expired_last_5m_sessions,
        })
        .unwrap_or(SessionTtlMetrics {
            total_expired_sessions: 0,
            expired_last_5m_sessions: 0,
        });

    Json(HealthResponse {
        status: "ok",
        service: "corrobore-http-server",
        version: env!("CARGO_PKG_VERSION"),
        storage_mode: state.config.storage_mode.as_str(),
        uptime_ms: state.started_at.elapsed().as_millis(),
        session_ttl_metrics,
        domain_providers: state
            .domain_providers
            .as_deref()
            .map(|registry| DomainProviderHealth {
                configured: registry.provider_count(),
                ready: registry.ready_count(),
            })
            .unwrap_or(DomainProviderHealth {
                configured: 0,
                ready: 0,
            }),
        durability: collect_durability_snapshot(&state),
    })
}

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
//! Issue #239 (§7 observability): a Prometheus `/metrics` endpoint.
//!
//! Intent: complete the observability story that `tracing` already started by
//! exposing the server's runtime counters in the Prometheus text exposition
//! format (version `0.0.4`). The endpoint is deliberately scrape-friendly:
//! it is served unauthenticated alongside `/health` so a Prometheus server can
//! poll it without a bearer token, and it reuses the exact session-expiration
//! metrics already surfaced by the health handler.
//!
//! Implementation direction: the exposition is rendered by hand (no Prometheus
//! client dependency) to keep the dependency/audit surface minimal. Each metric
//! carries the mandatory `# HELP` / `# TYPE` header lines and a single sample.

use axum::{extract::State, http::header, response::IntoResponse};

use crate::app::AppState;
use crate::durability::collect_durability_snapshot;

/// The Prometheus text exposition content type, including the format version.
const PROMETHEUS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// Render the current server metrics in the Prometheus text exposition format.
///
/// Exposes:
/// - `corrobore_build_info{version="…"}` — a constant `1` gauge that pins the build
///   version as a label so dashboards can group by release.
/// - `corrobore_uptime_seconds` — process uptime derived from `started_at`.
/// - `corrobore_sessions_expired_total` — cumulative idle-session expirations.
/// - `corrobore_sessions_expired_last_5m` — expirations observed in the last 5 min.
pub async fn metrics(State(state): State<AppState>) -> impl IntoResponse {
    let uptime_seconds = state.started_at.elapsed().as_secs_f64();
    let durability = collect_durability_snapshot(&state);

    // Reuse the same source as the `/health` handler; fall back to zeros when
    // the session runtime lock or metrics computation is momentarily
    // unavailable so scraping never fails.
    let (expired_total, expired_last_5m) = state
        .sessions
        .lock()
        .ok()
        .and_then(|sessions| sessions.expiration_metrics().ok())
        .map(|metrics| {
            (
                metrics.total_expired_sessions,
                metrics.expired_last_5m_sessions,
            )
        })
        .unwrap_or((0, 0));

    let version = env!("CARGO_PKG_VERSION");
    let (providers_configured, providers_ready) = state
        .domain_providers
        .as_deref()
        .map(|registry| (registry.provider_count(), registry.ready_count()))
        .unwrap_or((0, 0));

    let body = format!(
        concat!(
            "# HELP corrobore_build_info Build information for the running server, versioned via labels.\n",
            "# TYPE corrobore_build_info gauge\n",
            "corrobore_build_info{{version=\"{version}\"}} 1\n",
            "# HELP corrobore_uptime_seconds Time in seconds since the server started.\n",
            "# TYPE corrobore_uptime_seconds gauge\n",
            "corrobore_uptime_seconds {uptime_seconds}\n",
            "# HELP corrobore_sessions_expired_total Total number of sessions expired due to idle TTL.\n",
            "# TYPE corrobore_sessions_expired_total counter\n",
            "corrobore_sessions_expired_total {expired_total}\n",
            "# HELP corrobore_sessions_expired_last_5m Sessions expired due to idle TTL in the last 5 minutes.\n",
            "# TYPE corrobore_sessions_expired_last_5m gauge\n",
            "corrobore_sessions_expired_last_5m {expired_last_5m}\n",
            "# HELP corrobore_storage_mode Active storage mode for the runtime process.\n",
            "# TYPE corrobore_storage_mode gauge\n",
            "corrobore_storage_mode{{mode=\"{storage_mode}\"}} 1\n",
            "# HELP corrobore_storage_wal_bytes Current transaction WAL size in bytes.\n",
            "# TYPE corrobore_storage_wal_bytes gauge\n",
            "corrobore_storage_wal_bytes {wal_bytes}\n",
            "# HELP corrobore_storage_wal_lag_sequences WAL lag in sequence numbers behind latest checkpoint.\n",
            "# TYPE corrobore_storage_wal_lag_sequences gauge\n",
            "corrobore_storage_wal_lag_sequences {wal_lag_sequences}\n",
            "# HELP corrobore_storage_checkpoint_age_seconds Seconds since latest checkpoint update (-1 when unavailable).\n",
            "# TYPE corrobore_storage_checkpoint_age_seconds gauge\n",
            "corrobore_storage_checkpoint_age_seconds {checkpoint_age_seconds}\n",
            "# HELP corrobore_storage_compaction_backlog_bytes Pending compaction backlog represented by sealed segment bytes.\n",
            "# TYPE corrobore_storage_compaction_backlog_bytes gauge\n",
            "corrobore_storage_compaction_backlog_bytes {compaction_backlog_bytes}\n",
            "# HELP corrobore_storage_recovery_warning_count Number of warnings emitted during storage recovery.\n",
            "# TYPE corrobore_storage_recovery_warning_count gauge\n",
            "corrobore_storage_recovery_warning_count {recovery_warning_count}\n",
            "# HELP corrobore_domain_providers_configured Native domain providers loaded from the deployment manifest.\n",
            "# TYPE corrobore_domain_providers_configured gauge\n",
            "corrobore_domain_providers_configured {providers_configured}\n",
            "# HELP corrobore_domain_providers_ready Native domain providers that passed startup health checks.\n",
            "# TYPE corrobore_domain_providers_ready gauge\n",
            "corrobore_domain_providers_ready {providers_ready}\n",
        ),
        version = version,
        uptime_seconds = uptime_seconds,
        expired_total = expired_total,
        expired_last_5m = expired_last_5m,
        storage_mode = state.config.storage_mode.as_str(),
        wal_bytes = durability.wal_bytes,
        wal_lag_sequences = durability.wal_lag_sequences,
        checkpoint_age_seconds = durability
            .checkpoint_age_seconds
            .map_or(-1.0, |value| value as f64),
        compaction_backlog_bytes = durability.compaction_backlog_bytes,
        recovery_warning_count = durability.recovery.warning_count,
        providers_configured = providers_configured,
        providers_ready = providers_ready,
    );

    ([(header::CONTENT_TYPE, PROMETHEUS_CONTENT_TYPE)], body)
}

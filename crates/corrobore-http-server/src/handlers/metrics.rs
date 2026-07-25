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
use corrobore_engine::SHADOW_LATENCY_BUCKETS_MS;

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
    let lifecycle_state = state.lifecycle.state().as_str();
    let ready = u8::from(state.lifecycle.state() == crate::LifecycleState::Ready);
    let active_requests = state.lifecycle.active_requests();
    let shutdown_started = state.lifecycle.shutdown_started();
    let shutdown_failures = state.lifecycle.shutdown_failures();
    let (providers_configured, providers_ready) = state
        .domain_providers
        .as_deref()
        .map(|registry| (registry.provider_count(), registry.ready_count()))
        .unwrap_or((0, 0));

    let mut body = format!(
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
            "# HELP corrobore_storage_page_ins_total Canonical graph payload page-ins since startup.\n",
            "# TYPE corrobore_storage_page_ins_total counter\n",
            "corrobore_storage_page_ins_total {page_ins}\n",
            "# HELP corrobore_storage_cache_hits_total Canonical graph payload cache hits since startup.\n",
            "# TYPE corrobore_storage_cache_hits_total counter\n",
            "corrobore_storage_cache_hits_total {cache_hits}\n",
            "# HELP corrobore_storage_resident_records Current hot, warm, and cold working-set record counts.\n",
            "# TYPE corrobore_storage_resident_records gauge\n",
            "corrobore_storage_resident_records{{tier=\"hot\",kind=\"node\"}} {resident_hot_nodes}\n",
            "corrobore_storage_resident_records{{tier=\"hot\",kind=\"relationship\"}} {resident_hot_relationships}\n",
            "corrobore_storage_resident_records{{tier=\"warm\",kind=\"adjacency\"}} {resident_warm_adjacency_entries}\n",
            "corrobore_storage_resident_records{{tier=\"cold\",kind=\"node\"}} {resident_cold_nodes}\n",
            "corrobore_storage_resident_records{{tier=\"cold\",kind=\"relationship\"}} {resident_cold_relationships}\n",
            "# HELP corrobore_storage_index_entries Current canonical and derived index entry counts.\n",
            "# TYPE corrobore_storage_index_entries gauge\n",
            "corrobore_storage_index_entries{{index=\"node\"}} {node_index_entries}\n",
            "corrobore_storage_index_entries{{index=\"relationship\"}} {relationship_index_entries}\n",
            "corrobore_storage_index_entries{{index=\"label\"}} {label_index_entries}\n",
            "corrobore_storage_index_entries{{index=\"relationship_type\"}} {relationship_type_index_entries}\n",
            "# HELP corrobore_storage_recovery_outcome Current storage recovery outcome as a labeled gauge.\n",
            "# TYPE corrobore_storage_recovery_outcome gauge\n",
            "corrobore_storage_recovery_outcome{{outcome=\"{recovery_outcome}\"}} 1\n",
            "# HELP corrobore_storage_replayed_transactions Transactions replayed after the selected recovery checkpoint.\n",
            "# TYPE corrobore_storage_replayed_transactions gauge\n",
            "corrobore_storage_replayed_transactions {replayed_transaction_count}\n",
            "# HELP corrobore_domain_providers_configured Native domain providers loaded from the deployment manifest.\n",
            "# TYPE corrobore_domain_providers_configured gauge\n",
            "corrobore_domain_providers_configured {providers_configured}\n",
            "# HELP corrobore_domain_providers_ready Native domain providers that passed startup health checks.\n",
            "# TYPE corrobore_domain_providers_ready gauge\n",
            "corrobore_domain_providers_ready {providers_ready}\n",
            "# HELP corrobore_lifecycle_state Current lifecycle state represented as a labeled one-hot gauge.\n",
            "# TYPE corrobore_lifecycle_state gauge\n",
            "corrobore_lifecycle_state{{state=\"{lifecycle_state}\"}} 1\n",
            "# HELP corrobore_ready Whether the server currently accepts application requests.\n",
            "# TYPE corrobore_ready gauge\n",
            "corrobore_ready {ready}\n",
            "# HELP corrobore_active_requests Application requests currently in flight.\n",
            "# TYPE corrobore_active_requests gauge\n",
            "corrobore_active_requests {active_requests}\n",
            "# HELP corrobore_shutdown_started_total Graceful shutdown sequences started.\n",
            "# TYPE corrobore_shutdown_started_total counter\n",
            "corrobore_shutdown_started_total {shutdown_started}\n",
            "# HELP corrobore_shutdown_failures_total Shutdown sequences that timed out or failed.\n",
            "# TYPE corrobore_shutdown_failures_total counter\n",
            "corrobore_shutdown_failures_total {shutdown_failures}\n",
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
        page_ins = durability.page_ins,
        cache_hits = durability.cache_hits,
        resident_hot_nodes = durability.resident_hot_nodes,
        resident_hot_relationships = durability.resident_hot_relationships,
        resident_warm_adjacency_entries = durability.resident_warm_adjacency_entries,
        resident_cold_nodes = durability.resident_cold_nodes,
        resident_cold_relationships = durability.resident_cold_relationships,
        node_index_entries = durability.node_index_entries,
        relationship_index_entries = durability.relationship_index_entries,
        label_index_entries = durability.label_index_entries,
        relationship_type_index_entries = durability.relationship_type_index_entries,
        recovery_outcome = durability.recovery.outcome,
        replayed_transaction_count = durability.recovery.replayed_transaction_count,
        providers_configured = providers_configured,
        providers_ready = providers_ready,
        lifecycle_state = lifecycle_state,
        ready = ready,
        active_requests = active_requests,
        shutdown_started = shutdown_started,
        shutdown_failures = shutdown_failures,
    );
    let sync = state
        .opencti_sync
        .lock()
        .map(|runtime| runtime.status())
        .unwrap_or_default();
    body.push_str(&format!(
        concat!(
            "# HELP corrobore_opencti_sync_lag Source sequences behind the latest observed high-water mark.\n",
            "# TYPE corrobore_opencti_sync_lag gauge\n",
            "corrobore_opencti_sync_lag {lag}\n",
            "# HELP corrobore_opencti_sync_queue_depth Retryable OpenCTI operations awaiting contiguous replay.\n",
            "# TYPE corrobore_opencti_sync_queue_depth gauge\n",
            "corrobore_opencti_sync_queue_depth {queue_depth}\n",
            "# HELP corrobore_opencti_sync_retries_total Cumulative retryable OpenCTI operations.\n",
            "# TYPE corrobore_opencti_sync_retries_total counter\n",
            "corrobore_opencti_sync_retries_total {retry_count}\n",
            "# HELP corrobore_opencti_sync_rejected_total Cumulative permanently rejected OpenCTI operations.\n",
            "# TYPE corrobore_opencti_sync_rejected_total counter\n",
            "corrobore_opencti_sync_rejected_total {rejected}\n",
            "# HELP corrobore_opencti_sync_checkpoint Last acknowledged OpenCTI source sequence.\n",
            "# TYPE corrobore_opencti_sync_checkpoint gauge\n",
            "corrobore_opencti_sync_checkpoint {checkpoint}\n",
            "# HELP corrobore_opencti_sync_shadow_reads Whether divergence validation permits shadow reads.\n",
            "# TYPE corrobore_opencti_sync_shadow_reads gauge\n",
            "corrobore_opencti_sync_shadow_reads {shadow_reads}\n",
        ),
        lag = sync.lag,
        queue_depth = sync.queue_depth,
        retry_count = sync.retry_count,
        rejected = sync.rejected_operations,
        checkpoint = sync.last_acknowledged_sequence,
        shadow_reads = u8::from(sync.shadow_reads_enabled),
    ));
    let shadow_series = state
        .opencti_shadow
        .lock()
        .map(|runtime| runtime.metrics().series())
        .unwrap_or_default();
    body.push_str(
        "# HELP corrobore_opencti_shadow_comparisons_total OpenCTI shadow comparisons by bounded query class and release.\n\
# TYPE corrobore_opencti_shadow_comparisons_total counter\n\
# HELP corrobore_opencti_shadow_equivalent_total Equivalent OpenCTI shadow comparisons.\n\
# TYPE corrobore_opencti_shadow_equivalent_total counter\n\
# HELP corrobore_opencti_shadow_security_blocking_total Blocking security divergences.\n\
# TYPE corrobore_opencti_shadow_security_blocking_total counter\n\
# HELP corrobore_opencti_shadow_latency_ms Shadow-read provider latency distribution in milliseconds.\n\
# TYPE corrobore_opencti_shadow_latency_ms histogram\n",
    );
    for series in shadow_series {
        let query_class = series.query_class.as_str();
        let release = prometheus_label(&series.release);
        body.push_str(&format!(
            "corrobore_opencti_shadow_comparisons_total{{query_class=\"{query_class}\",release=\"{release}\"}} {}\n\
corrobore_opencti_shadow_equivalent_total{{query_class=\"{query_class}\",release=\"{release}\"}} {}\n\
corrobore_opencti_shadow_security_blocking_total{{query_class=\"{query_class}\",release=\"{release}\"}} {}\n",
            series.comparisons, series.equivalent, series.security_blocking,
        ));
        for (provider, buckets) in [
            ("reference", series.reference_latency_buckets),
            ("shadow", series.shadow_latency_buckets),
        ] {
            for (upper_bound, count) in SHADOW_LATENCY_BUCKETS_MS.iter().zip(buckets) {
                let upper_bound = if *upper_bound == u64::MAX {
                    "+Inf".to_owned()
                } else {
                    upper_bound.to_string()
                };
                body.push_str(&format!(
                    "corrobore_opencti_shadow_latency_ms_bucket{{query_class=\"{query_class}\",release=\"{release}\",provider=\"{provider}\",le=\"{upper_bound}\"}} {count}\n"
                ));
            }
        }
    }

    ([(header::CONTENT_TYPE, PROMETHEUS_CONTENT_TYPE)], body)
}

fn prometheus_label(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

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

use std::{collections::BTreeMap, path::Path, time::SystemTime};

use axum::{extract::State, http::header, response::IntoResponse};
use corrobore_engine::{CoreReadQueryClass, SHADOW_LATENCY_BUCKETS_MS};
use opencti_file_search::{FileJobMetrics, FileJobStore};

use crate::app::AppState;
use crate::durability::collect_durability_snapshot;
use crate::opencti_write::WriteAuthority;

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
            "corrobore_storage_index_entries{{index=\"identifier\"}} {identifier_index_entries}\n",
            "corrobore_storage_index_entries{{index=\"property\"}} {property_index_entries}\n",
            "corrobore_storage_index_entries{{index=\"temporal\"}} {temporal_index_entries}\n",
            "corrobore_storage_index_entries{{index=\"node_access\"}} {node_access_index_entries}\n",
            "corrobore_storage_index_entries{{index=\"relationship_access\"}} {relationship_access_index_entries}\n",
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
        identifier_index_entries = durability.identifier_index_entries,
        property_index_entries = durability.property_index_entries,
        temporal_index_entries = durability.temporal_index_entries,
        node_access_index_entries = durability.node_access_index_entries,
        relationship_access_index_entries = durability.relationship_access_index_entries,
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
    let import_metrics = state
        .stix_import_metrics
        .lock()
        .map(|metrics| metrics.clone())
        .unwrap_or_default();
    body.push_str(&format!(
        concat!(
            "# HELP corrobore_stix_import_records_total Cumulative STIX import records by bounded outcome.\n",
            "# TYPE corrobore_stix_import_records_total counter\n",
            "corrobore_stix_import_records_total{{outcome=\"requested\"}} {requested}\n",
            "corrobore_stix_import_records_total{{outcome=\"created\"}} {created}\n",
            "corrobore_stix_import_records_total{{outcome=\"updated\"}} {updated}\n",
            "corrobore_stix_import_records_total{{outcome=\"duplicate\"}} {duplicate}\n",
            "corrobore_stix_import_records_total{{outcome=\"rejected\"}} {rejected}\n",
            "corrobore_stix_import_records_total{{outcome=\"unresolved_reference\"}} {unresolved_reference}\n",
            "corrobore_stix_import_records_total{{outcome=\"failed\"}} {failed}\n",
        ),
        requested = import_metrics.requested,
        created = import_metrics.created,
        updated = import_metrics.updated,
        duplicate = import_metrics.duplicate,
        rejected = import_metrics.rejected,
        unresolved_reference = import_metrics.unresolved_reference,
        failed = import_metrics.failed,
    ));
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
    let file_metrics = file_job_metrics(&state);
    body.push_str(&format!(
        concat!(
            "# HELP corrobore_opencti_file_queue_depth File extraction jobs pending execution.\n",
            "# TYPE corrobore_opencti_file_queue_depth gauge\n",
            "corrobore_opencti_file_queue_depth {queue_depth}\n",
            "# HELP corrobore_opencti_file_failures_total Safe file extraction failures.\n",
            "# TYPE corrobore_opencti_file_failures_total counter\n",
            "corrobore_opencti_file_failures_total {failures}\n",
            "# HELP corrobore_opencti_file_retries_total File extraction retries scheduled.\n",
            "# TYPE corrobore_opencti_file_retries_total counter\n",
            "corrobore_opencti_file_retries_total {retries}\n",
            "# HELP corrobore_opencti_file_quarantines_total File extraction jobs quarantined.\n",
            "# TYPE corrobore_opencti_file_quarantines_total counter\n",
            "corrobore_opencti_file_quarantines_total {quarantines}\n",
            "# HELP corrobore_opencti_file_extracted_bytes_total Searchable file bytes extracted.\n",
            "# TYPE corrobore_opencti_file_extracted_bytes_total counter\n",
            "corrobore_opencti_file_extracted_bytes_total {extracted_bytes}\n",
            "# HELP corrobore_opencti_file_processing_latency_ms Latest file extraction latency.\n",
            "# TYPE corrobore_opencti_file_processing_latency_ms gauge\n",
            "corrobore_opencti_file_processing_latency_ms {processing_latency_ms}\n",
            "# HELP corrobore_opencti_file_index_lag_ms Age of the oldest pending file extraction.\n",
            "# TYPE corrobore_opencti_file_index_lag_ms gauge\n",
            "corrobore_opencti_file_index_lag_ms {index_lag_ms}\n",
        ),
        queue_depth = file_metrics.queue_depth,
        failures = file_metrics.failures,
        retries = file_metrics.retries,
        quarantines = file_metrics.quarantines,
        extracted_bytes = file_metrics.extracted_bytes,
        processing_latency_ms = file_metrics.last_processing_latency_ms,
        index_lag_ms = file_metrics.index_lag_ms,
    ));
    let shadow_series = state
        .opencti_shadow
        .lock()
        .map(|runtime| runtime.metrics().series())
        .unwrap_or_default();
    body.push_str(
        "# HELP corrobore_opencti_core_reads_total Completed fundamental OpenCTI reads by bounded query class.\n\
# TYPE corrobore_opencti_core_reads_total counter\n\
# HELP corrobore_opencti_core_read_latency_ms Approximate fundamental-read latency percentiles in milliseconds.\n\
# TYPE corrobore_opencti_core_read_latency_ms gauge\n\
# HELP corrobore_opencti_core_read_records_examined_total Records evaluated by exact predicates.\n\
# TYPE corrobore_opencti_core_read_records_examined_total counter\n\
# HELP corrobore_opencti_core_read_page_ins_total Persistent payload page-ins by query class.\n\
# TYPE corrobore_opencti_core_read_page_ins_total counter\n\
# HELP corrobore_opencti_core_read_cache_hits_total Persistent payload cache hits by query class.\n\
# TYPE corrobore_opencti_core_read_cache_hits_total counter\n",
    );
    if let Ok(engine) = state.engine.lock() {
        for query_class in [
            CoreReadQueryClass::PointRead,
            CoreReadQueryClass::List,
            CoreReadQueryClass::Pagination,
            CoreReadQueryClass::Count,
            CoreReadQueryClass::Neighbors,
            CoreReadQueryClass::Traverse,
            CoreReadQueryClass::Subgraph,
        ] {
            let Some(series) = engine.core_read_metrics().series(query_class) else {
                continue;
            };
            let query_class = query_class.as_str();
            body.push_str(&format!(
                "corrobore_opencti_core_reads_total{{query_class=\"{query_class}\"}} {}\n\
corrobore_opencti_core_read_latency_ms{{query_class=\"{query_class}\",quantile=\"0.50\"}} {}\n\
corrobore_opencti_core_read_latency_ms{{query_class=\"{query_class}\",quantile=\"0.95\"}} {}\n\
corrobore_opencti_core_read_latency_ms{{query_class=\"{query_class}\",quantile=\"0.99\"}} {}\n\
corrobore_opencti_core_read_records_examined_total{{query_class=\"{query_class}\"}} {}\n\
corrobore_opencti_core_read_page_ins_total{{query_class=\"{query_class}\"}} {}\n\
corrobore_opencti_core_read_cache_hits_total{{query_class=\"{query_class}\"}} {}\n",
                series.requests,
                series.p50_latency_ms,
                series.p95_latency_ms,
                series.p99_latency_ms,
                series.records_examined,
                series.page_ins,
                series.cache_hits,
            ));
        }
    }
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
    let (routing_audits, rollback_reason) = state
        .opencti_routing
        .lock()
        .map(|runtime| (runtime.audits(10_000), runtime.rollback_reason()))
        .unwrap_or_default();
    let mut routing_counts = BTreeMap::<(String, String), u64>::new();
    for event in routing_audits {
        let provider = match event.primary {
            corrobore_engine::ProviderTarget::Reference => "reference",
            corrobore_engine::ProviderTarget::Corrobore => "corrobore",
        };
        *routing_counts
            .entry((event.query_class.as_str().to_owned(), provider.to_owned()))
            .or_default() += 1;
    }
    body.push_str(
        "# HELP corrobore_opencti_routing_decisions_total Durable visible-provider decisions by bounded query class and provider.\n\
# TYPE corrobore_opencti_routing_decisions_total counter\n\
# HELP corrobore_opencti_routing_rollback_active Whether the automatic or operator circuit breaker is open.\n\
# TYPE corrobore_opencti_routing_rollback_active gauge\n",
    );
    for ((query_class, provider), count) in routing_counts {
        body.push_str(&format!(
            "corrobore_opencti_routing_decisions_total{{query_class=\"{query_class}\",provider=\"{provider}\"}} {count}\n"
        ));
    }
    body.push_str(&format!(
        "corrobore_opencti_routing_rollback_active {}\n",
        u8::from(rollback_reason.is_some())
    ));
    let write_status = state
        .opencti_write
        .lock()
        .map(|runtime| runtime.status())
        .unwrap_or_default();
    body.push_str(
        "# HELP corrobore_opencti_write_operations_total Transactional OpenCTI mutation items by bounded outcome.\n\
# TYPE corrobore_opencti_write_operations_total counter\n\
# HELP corrobore_opencti_write_idempotent_replays_total Transactions served from durable WAL receipts.\n\
# TYPE corrobore_opencti_write_idempotent_replays_total counter\n\
# HELP corrobore_opencti_write_reconciliation_pending Partial dual writes awaiting replay or quarantine.\n\
# TYPE corrobore_opencti_write_reconciliation_pending gauge\n\
# HELP corrobore_opencti_write_reconciliation_quarantined Partial dual writes requiring operator action.\n\
# TYPE corrobore_opencti_write_reconciliation_quarantined gauge\n\
# HELP corrobore_opencti_projection_outbox_depth Accepted canonical writes awaiting verified reference projection.\n\
# TYPE corrobore_opencti_projection_outbox_depth gauge\n\
# HELP corrobore_opencti_projection_lag Accepted sequences not yet verified on the reference.\n\
# TYPE corrobore_opencti_projection_lag gauge\n\
# HELP corrobore_opencti_projection_retries_total Retryable reference projection failures.\n\
# TYPE corrobore_opencti_projection_retries_total counter\n\
# HELP corrobore_opencti_projection_quarantined Divergent reference projections requiring operator action.\n\
# TYPE corrobore_opencti_projection_quarantined gauge\n\
# HELP corrobore_opencti_projection_reconstruction_total Lossless reference reconstruction plans generated.\n\
# TYPE corrobore_opencti_projection_reconstruction_total counter\n\
# HELP corrobore_opencti_write_authority Exclusive OpenCTI write authority as a one-hot labeled gauge.\n\
# TYPE corrobore_opencti_write_authority gauge\n",
    );
    let authority = match write_status.write_authority {
        WriteAuthority::CorroborePrimary => "corrobore_primary",
        WriteAuthority::WritesSuspended => "writes_suspended",
        WriteAuthority::ReferencePrimary => "reference_primary",
    };
    body.push_str(&format!(
        "corrobore_opencti_write_operations_total{{outcome=\"applied\"}} {}\n\
corrobore_opencti_write_operations_total{{outcome=\"failed\"}} {}\n\
corrobore_opencti_write_idempotent_replays_total {}\n\
corrobore_opencti_write_reconciliation_pending {}\n\
corrobore_opencti_write_reconciliation_quarantined {}\n\
corrobore_opencti_projection_outbox_depth {}\n\
corrobore_opencti_projection_lag {}\n\
corrobore_opencti_projection_retries_total {}\n\
corrobore_opencti_projection_quarantined {}\n\
corrobore_opencti_projection_reconstruction_total {}\n\
corrobore_opencti_write_authority{{authority=\"{}\"}} 1\n",
        write_status.applied_operations,
        write_status.failed_operations,
        write_status.idempotent_replays,
        write_status.pending_reconciliation,
        write_status.quarantined_reconciliation,
        write_status.projection_outbox_depth,
        write_status.projection_lag,
        write_status.projection_retries,
        write_status.projection_quarantined,
        write_status.reconstruction_runs,
        authority,
    ));
    let reconciliation_status = state
        .opencti_reconciliation
        .lock()
        .map(|runtime| runtime.status())
        .unwrap_or_default();
    body.push_str(
        "# HELP corrobore_opencti_reconciliation_reports Bounded retained reconciliation reports.\n\
# TYPE corrobore_opencti_reconciliation_reports gauge\n\
# HELP corrobore_opencti_reconciliation_quarantined Commands requiring operator policy.\n\
# TYPE corrobore_opencti_reconciliation_quarantined gauge\n\
# HELP corrobore_opencti_reconciliation_parity_verified Commands with verified post-repair parity.\n\
# TYPE corrobore_opencti_reconciliation_parity_verified gauge\n",
    );
    body.push_str(&format!(
        "corrobore_opencti_reconciliation_reports {}\n\
corrobore_opencti_reconciliation_quarantined {}\n\
corrobore_opencti_reconciliation_parity_verified {}\n",
        reconciliation_status.retained_reports,
        reconciliation_status.quarantined_commands,
        reconciliation_status.parity_verified_commands,
    ));
    let database = state
        .database_operations
        .lock()
        .map(|metrics| metrics.clone())
        .unwrap_or_default();
    body.push_str(&format!(
        concat!(
            "# HELP corrobore_database_snapshots_total Completed coherent database snapshots.\n",
            "# TYPE corrobore_database_snapshots_total counter\n",
            "corrobore_database_snapshots_total {}\n",
            "# HELP corrobore_database_snapshot_failures_total Failed database snapshot attempts.\n",
            "# TYPE corrobore_database_snapshot_failures_total counter\n",
            "corrobore_database_snapshot_failures_total {}\n",
            "# HELP corrobore_database_snapshot_bytes Bytes in the latest coherent snapshot.\n",
            "# TYPE corrobore_database_snapshot_bytes gauge\n",
            "corrobore_database_snapshot_bytes {}\n",
            "# HELP corrobore_database_snapshot_duration_ms Duration of the latest snapshot.\n",
            "# TYPE corrobore_database_snapshot_duration_ms gauge\n",
            "corrobore_database_snapshot_duration_ms {}\n",
            "# HELP corrobore_database_rebuilds_total Completed full derived-index rebuilds.\n",
            "# TYPE corrobore_database_rebuilds_total counter\n",
            "corrobore_database_rebuilds_total {}\n",
            "# HELP corrobore_database_rebuild_failures_total Failed derived-index rebuild attempts.\n",
            "# TYPE corrobore_database_rebuild_failures_total counter\n",
            "corrobore_database_rebuild_failures_total {}\n",
            "# HELP corrobore_database_rebuild_duration_ms Duration of the latest rebuild.\n",
            "# TYPE corrobore_database_rebuild_duration_ms gauge\n",
            "corrobore_database_rebuild_duration_ms {}\n",
            "# HELP corrobore_database_rebuild_records_scanned Canonical records scanned by the latest rebuild.\n",
            "# TYPE corrobore_database_rebuild_records_scanned gauge\n",
            "corrobore_database_rebuild_records_scanned {}\n",
        ),
        database.snapshots_completed,
        database.snapshot_failures,
        database.snapshot_bytes,
        database.snapshot_duration_ms,
        database.rebuilds_completed,
        database.rebuild_failures,
        database.rebuild_duration_ms,
        database.rebuild_records_scanned,
    ));

    ([(header::CONTENT_TYPE, PROMETHEUS_CONTENT_TYPE)], body)
}

fn file_job_metrics(state: &AppState) -> FileJobMetrics {
    let Some(storage_dir) = state.config.storage_dir.as_deref() else {
        return FileJobMetrics::default();
    };
    let metadata_dir = Path::new(storage_dir).join("file-content").join("metadata");
    if !metadata_dir.join("file-jobs.json").is_file() {
        return FileJobMetrics::default();
    }
    let now_ms = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
        .unwrap_or(0);
    FileJobStore::open(metadata_dir, 3, 60_000)
        .map(|store| store.metrics(now_ms))
        .unwrap_or_default()
}

fn prometheus_label(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

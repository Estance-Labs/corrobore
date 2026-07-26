// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! Authenticated online snapshot and derived-index operation boundary.

use std::{path::PathBuf, time::Instant};

use graph_storage::{
    IndexRebuildReport, SnapshotReport, SnapshotRequest, create_consistent_snapshot,
    rebuild_derived_indexes,
};
use serde::{Deserialize, Serialize};

use crate::{RuntimeStoreProvider, app::AppState};

/// Online local snapshot request. S3/MinIO exports consume the same artifact
/// through the `SnapshotArtifactStore` provider boundary.
#[derive(Clone, Debug, Deserialize)]
pub struct CreateSnapshotCommand {
    /// New local artifact directory.
    pub destination: PathBuf,
    /// Optional encryption key-provider identity; secret key material is never accepted.
    #[serde(default)]
    pub encryption_key_id: Option<String>,
    /// Optional provider retention/lifecycle hook.
    #[serde(default)]
    pub retention_hook: Option<String>,
}

/// Bounded database-operation counters exposed through status and Prometheus.
#[derive(Clone, Debug, Default, Serialize)]
pub struct DatabaseOperationMetrics {
    /// Completed snapshots.
    pub snapshots_completed: u64,
    /// Failed snapshot attempts.
    pub snapshot_failures: u64,
    /// Bytes in the latest completed snapshot.
    pub snapshot_bytes: u64,
    /// Milliseconds spent in the latest snapshot.
    pub snapshot_duration_ms: u64,
    /// Completed index rebuilds.
    pub rebuilds_completed: u64,
    /// Failed index rebuild attempts.
    pub rebuild_failures: u64,
    /// Milliseconds spent in the latest rebuild.
    pub rebuild_duration_ms: u64,
    /// Records scanned by the latest rebuild.
    pub rebuild_records_scanned: u64,
}

/// Acquire the canonical write barrier and create a coherent online snapshot.
pub fn create_online_snapshot(
    state: &AppState,
    command: CreateSnapshotCommand,
) -> Result<SnapshotReport, String> {
    let RuntimeStoreProvider::Persistent(runtime) = &state.runtime_store else {
        return Err("database operations require persistent storage".to_owned());
    };
    let started = Instant::now();
    let result = runtime
        .canonical_store
        .lock()
        .map_err(|_| "canonical store lock is poisoned".to_owned())
        .and_then(|store| {
            create_consistent_snapshot(
                store.root(),
                command.destination,
                SnapshotRequest {
                    created_at: chrono::Utc::now().to_rfc3339(),
                    encryption_key_id: command.encryption_key_id,
                    retention_hook: command.retention_hook,
                },
            )
            .map_err(|error| error.to_string())
        });
    if let Ok(mut metrics) = state.database_operations.lock() {
        metrics.snapshot_duration_ms = started.elapsed().as_millis() as u64;
        match &result {
            Ok(report) => {
                metrics.snapshots_completed = metrics.snapshots_completed.saturating_add(1);
                metrics.snapshot_bytes = report.total_bytes;
            }
            Err(_) => metrics.snapshot_failures = metrics.snapshot_failures.saturating_add(1),
        }
    }
    result
}

/// Acquire the canonical write barrier and rebuild every derived projection.
pub fn rebuild_online_indexes(state: &AppState) -> Result<IndexRebuildReport, String> {
    let RuntimeStoreProvider::Persistent(runtime) = &state.runtime_store else {
        return Err("database operations require persistent storage".to_owned());
    };
    let started = Instant::now();
    let result = runtime
        .canonical_store
        .lock()
        .map_err(|_| "canonical store lock is poisoned".to_owned())
        .and_then(|mut store| {
            rebuild_derived_indexes(&mut store, None).map_err(|error| error.to_string())
        });
    if let Ok(mut metrics) = state.database_operations.lock() {
        metrics.rebuild_duration_ms = started.elapsed().as_millis() as u64;
        match &result {
            Ok(report) => {
                metrics.rebuilds_completed = metrics.rebuilds_completed.saturating_add(1);
                metrics.rebuild_records_scanned = report.records_scanned;
            }
            Err(_) => metrics.rebuild_failures = metrics.rebuild_failures.saturating_add(1),
        }
    }
    result
}

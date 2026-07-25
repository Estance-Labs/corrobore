// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! Durable server integration for OpenCTI snapshot synchronization.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use graph_storage::{CanonicalEngineStore, CanonicalProjectionRequest, DurableTransactionId};
use opencti_adapter::{
    BulkLimits, DivergenceStatus, GraphDigest, OpenCtiSyncBatch, OpenCtiSynchronizer,
    SyncBatchResult, SyncCheckpoint, SyncPhase, SyncValidationReport,
};
use serde::{Deserialize, Serialize};

const STATE_SCHEMA_VERSION: u32 = 1;

/// Snapshot of synchronization progress exposed to health and metrics.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct OpenCtiSyncStatus {
    /// Whether a durable checkpoint has been established.
    pub initialized: bool,
    /// Current lifecycle phase.
    pub phase: Option<SyncPhase>,
    /// Highest contiguous acknowledged source sequence.
    pub last_acknowledged_sequence: u64,
    /// Latest observed source high-water mark.
    pub high_water_mark: u64,
    /// Source lag measured in sequences.
    pub lag: u64,
    /// Retryable queue depth.
    pub queue_depth: u64,
    /// Cumulative retryable results.
    pub retry_count: u64,
    /// Cumulative permanent rejections.
    pub rejected_operations: u64,
    /// Cumulative quarantined operations.
    pub quarantined_operations: u64,
    /// Last parity status.
    pub divergence: Option<DivergenceStatus>,
    /// Stable names of the currently mismatched parity dimensions.
    pub divergence_dimensions: Vec<String>,
    /// Whether parity currently permits shadow reads.
    pub shadow_reads_enabled: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PersistedSyncState {
    schema_version: u32,
    checkpoint: SyncCheckpoint,
    validation: Option<SyncValidationReport>,
}

/// Result persisted and returned after one canonical batch commit.
#[derive(Clone, Debug, Serialize)]
pub struct DurableSyncBatchResult {
    /// Per-operation batch result.
    pub batch: SyncBatchResult,
    /// Durable checkpoint after the commit.
    pub checkpoint: SyncCheckpoint,
    /// Optional parity validation report.
    pub validation: Option<SyncValidationReport>,
}

/// In-process synchronization coordinator backed by an fsynced checkpoint.
#[derive(Clone, Debug)]
pub struct OpenCtiSyncRuntime {
    synchronizer: OpenCtiSynchronizer,
    state_path: Option<PathBuf>,
    checkpoint: Option<SyncCheckpoint>,
    validation: Option<SyncValidationReport>,
}

impl OpenCtiSyncRuntime {
    /// Restore a durable checkpoint before server readiness.
    pub fn open(state_path: Option<PathBuf>, limits: BulkLimits) -> Result<Self, String> {
        let persisted = state_path
            .as_deref()
            .filter(|path| path.is_file())
            .map(read_state)
            .transpose()?;
        if persisted
            .as_ref()
            .is_some_and(|state| state.schema_version != STATE_SCHEMA_VERSION)
        {
            return Err("unsupported OpenCTI synchronization state version".to_owned());
        }
        Ok(Self {
            synchronizer: OpenCtiSynchronizer::new(limits),
            state_path,
            checkpoint: persisted.as_ref().map(|state| state.checkpoint.clone()),
            validation: persisted.and_then(|state| state.validation),
        })
    }

    /// Apply a batch to a cloned graph, commit one WAL transaction, then fsync
    /// the source checkpoint. A crash between graph commit and checkpoint write
    /// replays as duplicates on restart.
    pub fn apply(
        &mut self,
        store: &mut CanonicalEngineStore,
        batch: OpenCtiSyncBatch,
        expected: Option<&GraphDigest>,
    ) -> Result<DurableSyncBatchResult, String> {
        let mut checkpoint = self.checkpoint.clone().unwrap_or_else(|| {
            SyncCheckpoint::new(batch.source_id.clone(), batch.snapshot_id.clone())
        });
        let previous = store
            .load_projection(CanonicalProjectionRequest::all())
            .map_err(|error| error.to_string())?;
        let mut current = previous.clone();
        let result = self
            .synchronizer
            .apply_batch(&mut current, &mut checkpoint, batch)
            .map_err(|error| error.to_string())?;
        let transaction_id = DurableTransactionId::new(result.transaction_id.clone())
            .map_err(|error| error.to_string())?;
        store
            .commit_transition(&previous, &current, transaction_id, None)
            .map_err(|error| error.to_string())?;

        let mut validation = expected
            .map(|expected| self.synchronizer.validate(&current, expected, true))
            .transpose()
            .map_err(|error| error.to_string())?;
        if let Some(report) = &mut validation
            && (checkpoint.queue_depth != 0
                || checkpoint.last_acknowledged_sequence < checkpoint.high_water_mark)
        {
            report.divergence = DivergenceStatus::Diverged;
            report.shadow_reads_enabled = false;
            if !report
                .differences
                .iter()
                .any(|difference| difference == "source_progress")
            {
                report.differences.push("source_progress".to_owned());
            }
        }
        let validation_still_current = result
            .operations
            .iter()
            .all(|operation| operation.status == opencti_adapter::OperationStatus::Duplicate);
        if validation
            .as_ref()
            .is_some_and(|report| report.shadow_reads_enabled)
            && checkpoint.queue_depth == 0
            && checkpoint.last_acknowledged_sequence >= checkpoint.high_water_mark
        {
            checkpoint.phase = SyncPhase::SteadyState;
        }
        let persisted = PersistedSyncState {
            schema_version: STATE_SCHEMA_VERSION,
            checkpoint: checkpoint.clone(),
            validation: validation.clone().or_else(|| {
                validation_still_current
                    .then(|| self.validation.clone())
                    .flatten()
            }),
        };
        if let Some(path) = &self.state_path {
            write_state(path, &persisted)?;
        }
        self.checkpoint = Some(checkpoint.clone());
        self.validation = persisted.validation.clone();
        Ok(DurableSyncBatchResult {
            batch: result,
            checkpoint,
            validation,
        })
    }

    /// Current bounded observability snapshot.
    pub fn status(&self) -> OpenCtiSyncStatus {
        let Some(checkpoint) = &self.checkpoint else {
            return OpenCtiSyncStatus::default();
        };
        OpenCtiSyncStatus {
            initialized: true,
            phase: Some(checkpoint.phase),
            last_acknowledged_sequence: checkpoint.last_acknowledged_sequence,
            high_water_mark: checkpoint.high_water_mark,
            lag: checkpoint
                .high_water_mark
                .saturating_sub(checkpoint.last_acknowledged_sequence),
            queue_depth: checkpoint.queue_depth,
            retry_count: checkpoint.retry_count,
            rejected_operations: checkpoint.rejected_operations,
            quarantined_operations: checkpoint.quarantined_operations,
            divergence: self.validation.as_ref().map(|report| report.divergence),
            divergence_dimensions: self
                .validation
                .as_ref()
                .map(|report| report.differences.clone())
                .unwrap_or_default(),
            shadow_reads_enabled: self
                .validation
                .as_ref()
                .is_some_and(|report| report.shadow_reads_enabled),
        }
    }
}

fn read_state(path: &Path) -> Result<PersistedSyncState, String> {
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    serde_json::from_slice(&bytes).map_err(|error| error.to_string())
}

fn write_state(path: &Path, state: &PersistedSyncState) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "synchronization state path has no parent".to_owned())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temporary = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(state).map_err(|error| error.to_string())?;
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&temporary)
        .map_err(|error| error.to_string())?;
    file.write_all(&bytes).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    fs::rename(&temporary, path).map_err(|error| error.to_string())?;
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| error.to_string())
}

/// Map synchronization errors to stable transport categories.
pub fn is_client_sync_error(error: &str) -> bool {
    error.starts_with("invalid synchronization input:") || error.starts_with("bulk limit exceeded")
}

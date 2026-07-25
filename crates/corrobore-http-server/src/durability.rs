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
use std::{
    fs,
    io::{BufRead, BufReader},
    path::Path,
    time::{Duration, SystemTime},
};

use graph_storage::{
    AtomicPersistentRecoveryPath, DurableWalEntry, DurableWalEntryKind, GraphStoreRecoveryReport,
    RecordFormat, StorageVersion,
};
use serde::Serialize;

use crate::{
    app::{AppState, RuntimeStoreProvider},
    config::StorageMode,
};

#[derive(Clone, Debug, Serialize)]
pub struct DurabilityControlsSnapshot {
    pub require_fsync: bool,
    pub strict_recovery: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct DurabilityRecoverySnapshot {
    pub outcome: &'static str,
    pub manifest_validated: bool,
    pub required_components_validated: bool,
    pub catalog_recovered: bool,
    pub adjacency_storage_recovered: bool,
    pub warning_count: usize,
    pub derived_state_rebuilt: bool,
    pub replayed_transaction_count: usize,
    pub recovery_path: Option<&'static str>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DurabilityObservabilitySnapshot {
    pub controls: DurabilityControlsSnapshot,
    pub storage_version: Option<&'static str>,
    pub record_format: Option<&'static str>,
    pub wal_bytes: u64,
    pub wal_lag_sequences: u64,
    pub checkpoint_sequence: Option<u64>,
    pub checkpoint_age_seconds: Option<u64>,
    pub compaction_backlog_bytes: u64,
    pub page_ins: u64,
    pub cache_hits: u64,
    pub resident_hot_nodes: u64,
    pub resident_hot_relationships: u64,
    pub resident_warm_adjacency_entries: u64,
    pub resident_cold_nodes: u64,
    pub resident_cold_relationships: u64,
    pub node_index_entries: u64,
    pub relationship_index_entries: u64,
    pub label_index_entries: u64,
    pub relationship_type_index_entries: u64,
    pub identifier_index_entries: u64,
    pub property_index_entries: u64,
    pub temporal_index_entries: u64,
    pub recovery: DurabilityRecoverySnapshot,
}

pub fn collect_durability_snapshot(state: &AppState) -> DurabilityObservabilitySnapshot {
    let controls = DurabilityControlsSnapshot {
        require_fsync: state.config.storage_require_fsync,
        strict_recovery: state.config.storage_strict_recovery,
    };

    match &state.runtime_store {
        RuntimeStoreProvider::Ephemeral => DurabilityObservabilitySnapshot {
            controls,
            storage_version: None,
            record_format: None,
            wal_bytes: 0,
            wal_lag_sequences: 0,
            checkpoint_sequence: None,
            checkpoint_age_seconds: None,
            compaction_backlog_bytes: 0,
            page_ins: 0,
            cache_hits: 0,
            resident_hot_nodes: 0,
            resident_hot_relationships: 0,
            resident_warm_adjacency_entries: 0,
            resident_cold_nodes: 0,
            resident_cold_relationships: 0,
            node_index_entries: 0,
            relationship_index_entries: 0,
            label_index_entries: 0,
            relationship_type_index_entries: 0,
            identifier_index_entries: 0,
            property_index_entries: 0,
            temporal_index_entries: 0,
            recovery: durability_recovery_snapshot(state.config.storage_mode, None),
        },
        RuntimeStoreProvider::Persistent(runtime) => {
            let transaction_root = runtime.root_path.join("transactions");
            let wal_path = transaction_root.join("transaction_wal.log");
            let checkpoints_dir = transaction_root.join("checkpoints");
            let segments_dir = transaction_root.join("segments");

            let wal_bytes = fs::metadata(&wal_path)
                .map(|metadata| metadata.len())
                .unwrap_or(0);
            let latest_commit_sequence = latest_committed_wal_sequence(&wal_path);
            let latest_checkpoint = latest_checkpoint_diagnostic(&checkpoints_dir);
            let wal_lag_sequences = latest_commit_sequence
                .zip(latest_checkpoint.sequence)
                .map(|(commit, checkpoint)| commit.saturating_sub(checkpoint))
                .unwrap_or(0);
            let store_stats = runtime
                .canonical_store
                .lock()
                .ok()
                .map(|store| store.stats().clone())
                .unwrap_or_default();

            let mut recovery = durability_recovery_snapshot(
                state.config.storage_mode,
                Some(&runtime.recovery_report),
            );
            if let Ok(store) = runtime.canonical_store.lock() {
                recovery.derived_state_rebuilt = store.startup_report().derived_indexes_rebuilt;
                recovery.replayed_transaction_count =
                    store.startup_report().replayed_transaction_count;
                recovery.recovery_path = store
                    .startup_report()
                    .recovery_path
                    .map(recovery_path_label);
            }

            DurabilityObservabilitySnapshot {
                controls,
                storage_version: Some(storage_version_label(&runtime.manifest.storage_version)),
                record_format: Some(record_format_label(&runtime.manifest.record_format)),
                wal_bytes,
                wal_lag_sequences,
                checkpoint_sequence: latest_checkpoint.sequence,
                checkpoint_age_seconds: latest_checkpoint.age_seconds,
                compaction_backlog_bytes: directory_size(&segments_dir),
                page_ins: store_stats.page_ins,
                cache_hits: store_stats.cache_hits,
                resident_hot_nodes: store_stats.resident_hot_nodes,
                resident_hot_relationships: store_stats.resident_hot_relationships,
                resident_warm_adjacency_entries: store_stats.resident_warm_adjacency_entries,
                resident_cold_nodes: store_stats.resident_cold_nodes,
                resident_cold_relationships: store_stats.resident_cold_relationships,
                node_index_entries: store_stats.node_index_entries,
                relationship_index_entries: store_stats.relationship_index_entries,
                label_index_entries: store_stats.label_index_entries,
                relationship_type_index_entries: store_stats.relationship_type_index_entries,
                identifier_index_entries: store_stats.identifier_index_entries,
                property_index_entries: store_stats.property_index_entries,
                temporal_index_entries: store_stats.temporal_index_entries,
                recovery,
            }
        }
    }
}

fn storage_version_label(version: &StorageVersion) -> &'static str {
    match version {
        StorageVersion::V1 => "V1",
        StorageVersion::Unsupported(_) => "unsupported",
    }
}

fn record_format_label(format: &RecordFormat) -> &'static str {
    match format {
        RecordFormat::JsonLinesV1 => "JsonLinesV1",
        RecordFormat::Unsupported(_) => "unsupported",
    }
}

fn durability_recovery_snapshot(
    mode: StorageMode,
    report: Option<&GraphStoreRecoveryReport>,
) -> DurabilityRecoverySnapshot {
    match report {
        Some(report) => DurabilityRecoverySnapshot {
            outcome: "recovered",
            manifest_validated: report.manifest_validated,
            required_components_validated: report.required_components_validated,
            catalog_recovered: report.catalog_recovered,
            adjacency_storage_recovered: report.adjacency_storage_recovered,
            warning_count: report.warnings.len(),
            derived_state_rebuilt: report.catalog_rebuild_report.is_some(),
            replayed_transaction_count: 0,
            recovery_path: None,
        },
        None => DurabilityRecoverySnapshot {
            outcome: if matches!(mode, StorageMode::Ephemeral) {
                "ephemeral"
            } else {
                "unavailable"
            },
            manifest_validated: false,
            required_components_validated: false,
            catalog_recovered: false,
            adjacency_storage_recovered: false,
            warning_count: 0,
            derived_state_rebuilt: false,
            replayed_transaction_count: 0,
            recovery_path: None,
        },
    }
}

fn recovery_path_label(path: AtomicPersistentRecoveryPath) -> &'static str {
    match path {
        AtomicPersistentRecoveryPath::CheckpointAndBoundedReplay => "checkpoint_and_bounded_replay",
        AtomicPersistentRecoveryPath::FullReplay => "full_replay",
    }
}

fn latest_committed_wal_sequence(path: &Path) -> Option<u64> {
    let file = fs::File::open(path).ok()?;
    let reader = BufReader::new(file);
    let mut latest = None;
    for line in reader.lines() {
        let Ok(line) = line else {
            continue;
        };
        let Ok(entry) = serde_json::from_str::<DurableWalEntry>(&line) else {
            continue;
        };
        if entry.kind == DurableWalEntryKind::Commit {
            latest = Some(entry.sequence_number.0);
        }
    }
    latest
}

struct CheckpointDiagnostic {
    sequence: Option<u64>,
    age_seconds: Option<u64>,
}

fn latest_checkpoint_diagnostic(directory: &Path) -> CheckpointDiagnostic {
    let mut latest = None::<(u64, Option<SystemTime>)>;
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(_) => {
            return CheckpointDiagnostic {
                sequence: None,
                age_seconds: None,
            };
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if !file_name.starts_with("checkpoint-") || !file_name.ends_with(".json") {
            continue;
        }
        let sequence = file_name
            .trim_start_matches("checkpoint-")
            .trim_end_matches(".json")
            .parse::<u64>()
            .ok();
        let Some(sequence) = sequence else {
            continue;
        };
        let modified = fs::metadata(&path)
            .ok()
            .and_then(|metadata| metadata.modified().ok());
        if latest
            .map(|(latest_sequence, _)| sequence > latest_sequence)
            .unwrap_or(true)
        {
            latest = Some((sequence, modified));
        }
    }

    let age_seconds = latest
        .and_then(|(_, modified)| modified)
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .unwrap_or(Duration::from_secs(0))
        .as_secs();

    CheckpointDiagnostic {
        sequence: latest.map(|(sequence, _)| sequence),
        age_seconds: latest.map(|_| age_seconds),
    }
}

fn directory_size(path: &Path) -> u64 {
    let mut total = 0u64;
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(_) => return 0,
    };
    for entry in entries.flatten() {
        let entry_path = entry.path();
        if entry_path.is_dir() {
            total = total.saturating_add(directory_size(&entry_path));
            continue;
        }
        total = total.saturating_add(
            fs::metadata(&entry_path)
                .map(|metadata| metadata.len())
                .unwrap_or(0),
        );
    }
    total
}

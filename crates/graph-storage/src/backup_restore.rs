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
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{
    AtomicPersistentRuntimeState, DurableWalEntry, GraphCatalog, GraphStorageError,
    GraphStorageResult, JsonLinesRecordCodec, RecordChecksum, StorageRef, StorageRoot,
    StorageSegment, WalSequenceNumber, calculate_encoded_record_checksum,
    classify_transaction_replay_status, open_storage_root,
    recover_atomic_persistent_runtime_state_with_report, snapshot_persisted_adjacency_records,
    validate_durable_wal_entry, validate_encoded_record_checksum,
};

const BACKUP_MANIFEST_FILE_NAME: &str = "backup_manifest.json";

/// Backup result for atomic persistent storage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AtomicPersistentBackupOutcome {
    /// Checkpoint sequence bound to this backup.
    pub checkpoint_sequence_number: WalSequenceNumber,
    /// Number of copied files.
    pub file_count: usize,
    /// Total copied bytes.
    pub total_bytes: u64,
    /// Backup root path.
    pub backup_root_path: PathBuf,
}

/// Backup integrity validation diagnostics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AtomicPersistentBackupValidationReport {
    /// Checkpoint sequence expected by the backup manifest.
    pub checkpoint_sequence_number: WalSequenceNumber,
    /// Number of validated files.
    pub file_count: usize,
    /// Total validated bytes.
    pub total_bytes: u64,
    /// Number of WAL transactions validated as replay-safe.
    pub replay_safe_transaction_count: usize,
}

/// Restore result for atomic persistent storage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AtomicPersistentRestoreOutcome {
    /// Reopened restored root.
    pub restored_root: StorageRoot,
    /// Restored file count.
    pub file_count: usize,
    /// Restored total bytes.
    pub total_bytes: u64,
    /// Checkpoint sequence restored.
    pub checkpoint_sequence_number: WalSequenceNumber,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct BackupManifestRecord {
    checkpoint_sequence_number: WalSequenceNumber,
    files: Vec<BackupFileRecord>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct BackupFileRecord {
    relative_path: String,
    byte_length: u64,
    checksum: RecordChecksum,
}

/// Create a consistent backup from a persistent root and its latest committed checkpoint.
pub fn create_atomic_persistent_backup(
    source_root: &StorageRoot,
    backup_root_path: impl Into<PathBuf>,
) -> GraphStorageResult<AtomicPersistentBackupOutcome> {
    if !source_root.path().is_dir() {
        return Err(GraphStorageError::StorageRootNotFound {
            path: source_root.path().to_path_buf(),
        });
    }

    let recovery = recover_atomic_persistent_runtime_state_with_report(source_root)?;
    let checkpoint_sequence_number =
        recovery.report.checkpoint_sequence_number.ok_or_else(|| {
            GraphStorageError::OperationFailed {
                operation: "create_atomic_persistent_backup",
                message: "backup requires at least one committed checkpoint".to_owned(),
            }
        })?;

    let backup_root_path = backup_root_path.into();
    if backup_root_path.exists() {
        return Err(GraphStorageError::StorageRootAlreadyExists {
            path: backup_root_path,
        });
    }

    copy_directory_recursive(source_root.path(), &backup_root_path, &HashSet::new())?;
    let files = collect_backup_files(&backup_root_path, true)?;
    let manifest = BackupManifestRecord {
        checkpoint_sequence_number,
        files,
    };
    write_json_file(
        &backup_root_path.join(BACKUP_MANIFEST_FILE_NAME),
        &manifest,
        "create_atomic_persistent_backup",
    )?;

    let report = validate_atomic_persistent_backup(&backup_root_path)?;
    Ok(AtomicPersistentBackupOutcome {
        checkpoint_sequence_number: report.checkpoint_sequence_number,
        file_count: report.file_count,
        total_bytes: report.total_bytes,
        backup_root_path,
    })
}

/// Validate a backup manifest, checksums, transaction continuity, and recovery consistency.
pub fn validate_atomic_persistent_backup(
    backup_root_path: impl Into<PathBuf>,
) -> GraphStorageResult<AtomicPersistentBackupValidationReport> {
    let backup_root_path = backup_root_path.into();
    let backup_root = open_storage_root(backup_root_path.clone())?;
    let backup_manifest_path = backup_root_path.join(BACKUP_MANIFEST_FILE_NAME);
    let manifest = read_json_file::<BackupManifestRecord>(
        &backup_manifest_path,
        "validate_atomic_persistent_backup",
    )?;
    let expected_files = manifest_files_by_relative_path(&manifest);
    let actual_files = collect_backup_files(&backup_root_path, false)?;
    let actual_files_by_path: BTreeMap<String, BackupFileRecord> = actual_files
        .into_iter()
        .map(|record| (record.relative_path.clone(), record))
        .collect();

    if expected_files.len() != actual_files_by_path.len() {
        return Err(GraphStorageError::OperationFailed {
            operation: "validate_atomic_persistent_backup",
            message: format!(
                "backup file count mismatch: expected {}, found {}",
                expected_files.len(),
                actual_files_by_path.len()
            ),
        });
    }

    for (relative_path, expected) in &expected_files {
        let Some(actual) = actual_files_by_path.get(relative_path) else {
            return Err(GraphStorageError::OperationFailed {
                operation: "validate_atomic_persistent_backup",
                message: format!("backup file missing: {relative_path}"),
            });
        };
        if expected.byte_length != actual.byte_length {
            return Err(GraphStorageError::OperationFailed {
                operation: "validate_atomic_persistent_backup",
                message: format!(
                    "backup file length mismatch for {}: expected {}, found {}",
                    relative_path, expected.byte_length, actual.byte_length
                ),
            });
        }
        if expected.checksum != actual.checksum {
            return Err(GraphStorageError::ChecksumMismatch {
                expected: expected.checksum.clone(),
                actual: actual.checksum.clone(),
            });
        }
    }

    let recovery = recover_atomic_persistent_runtime_state_with_report(&backup_root)?;
    let recovered_checkpoint = recovery.report.checkpoint_sequence_number.ok_or_else(|| {
        GraphStorageError::OperationFailed {
            operation: "validate_atomic_persistent_backup",
            message: "backup recovery did not find a checkpoint baseline".to_owned(),
        }
    })?;
    if recovered_checkpoint != manifest.checkpoint_sequence_number {
        return Err(GraphStorageError::OperationFailed {
            operation: "validate_atomic_persistent_backup",
            message: format!(
                "checkpoint mismatch: manifest {}, recovery {}",
                manifest.checkpoint_sequence_number.0, recovered_checkpoint.0
            ),
        });
    }

    let replay_safe_transaction_count = validate_transaction_continuity(
        &backup_root_path
            .join("transactions")
            .join("transaction_wal.log"),
    )?;
    validate_catalog_index_and_adjacency_consistency(&recovery.state)?;
    validate_catalog_payload_checksums(&backup_root, &recovery.state.catalog)?;

    let total_bytes = manifest.files.iter().map(|record| record.byte_length).sum();
    Ok(AtomicPersistentBackupValidationReport {
        checkpoint_sequence_number: manifest.checkpoint_sequence_number,
        file_count: manifest.files.len(),
        total_bytes,
        replay_safe_transaction_count,
    })
}

/// Restore a validated backup into an empty target root and reopen it.
pub fn restore_atomic_persistent_backup(
    backup_root_path: impl Into<PathBuf>,
    target_root_path: impl Into<PathBuf>,
) -> GraphStorageResult<AtomicPersistentRestoreOutcome> {
    let backup_root_path = backup_root_path.into();
    let validation = validate_atomic_persistent_backup(backup_root_path.clone())?;
    let target_root_path = target_root_path.into();

    ensure_target_root_is_empty(&target_root_path)?;
    let mut excluded = HashSet::new();
    excluded.insert(BACKUP_MANIFEST_FILE_NAME.to_owned());
    copy_directory_recursive(&backup_root_path, &target_root_path, &excluded)?;

    let restored_root = open_storage_root(target_root_path)?;
    let restored_recovery = recover_atomic_persistent_runtime_state_with_report(&restored_root)?;
    let restored_checkpoint = restored_recovery
        .report
        .checkpoint_sequence_number
        .ok_or_else(|| GraphStorageError::OperationFailed {
            operation: "restore_atomic_persistent_backup",
            message: "restored root has no checkpoint baseline".to_owned(),
        })?;
    if restored_checkpoint != validation.checkpoint_sequence_number {
        return Err(GraphStorageError::OperationFailed {
            operation: "restore_atomic_persistent_backup",
            message: format!(
                "restored checkpoint mismatch: expected {}, found {}",
                validation.checkpoint_sequence_number.0, restored_checkpoint.0
            ),
        });
    }

    Ok(AtomicPersistentRestoreOutcome {
        restored_root,
        file_count: validation.file_count,
        total_bytes: validation.total_bytes,
        checkpoint_sequence_number: validation.checkpoint_sequence_number,
    })
}

fn validate_transaction_continuity(transaction_wal_path: &Path) -> GraphStorageResult<usize> {
    if !transaction_wal_path.is_file() {
        return Err(GraphStorageError::OperationFailed {
            operation: "validate_atomic_persistent_backup",
            message: format!(
                "transaction WAL file is missing: {}",
                transaction_wal_path.display()
            ),
        });
    }

    let entries = read_json_lines_file::<DurableWalEntry>(
        transaction_wal_path,
        "validate_atomic_persistent_backup",
    )?;
    if entries.is_empty() {
        return Ok(0);
    }

    for entry in &entries {
        validate_durable_wal_entry(entry).map_err(|error| GraphStorageError::OperationFailed {
            operation: "validate_atomic_persistent_backup",
            message: error.to_string(),
        })?;
    }
    for pair in entries.windows(2) {
        if pair[1].sequence_number.0 < pair[0].sequence_number.0 {
            return Err(GraphStorageError::OperationFailed {
                operation: "validate_atomic_persistent_backup",
                message: "transaction WAL sequence order is not monotonic".to_owned(),
            });
        }
    }

    let mut by_transaction: HashMap<String, Vec<DurableWalEntry>> = HashMap::new();
    for entry in entries {
        by_transaction
            .entry(entry.transaction_id.value.clone())
            .or_default()
            .push(entry);
    }

    for transaction_entries in by_transaction.values() {
        classify_transaction_replay_status(transaction_entries.as_slice()).map_err(|error| {
            GraphStorageError::OperationFailed {
                operation: "validate_atomic_persistent_backup",
                message: format!("invalid transaction continuity in WAL entries: {error}"),
            }
        })?;
    }

    Ok(by_transaction.len())
}

fn validate_catalog_index_and_adjacency_consistency(
    state: &AtomicPersistentRuntimeState,
) -> GraphStorageResult<()> {
    let latest_nodes = &state.catalog.latest_node_records;
    let latest_relationships = &state.catalog.latest_relationship_records;

    for (label, entry) in &state.catalog.metadata_indexes.labels {
        for node in &entry.nodes {
            let Some(latest) = latest_nodes.get(&node.node_id) else {
                return Err(GraphStorageError::OperationFailed {
                    operation: "validate_atomic_persistent_backup",
                    message: format!(
                        "label index `{label}` references unknown node `{:?}`",
                        node.node_id
                    ),
                });
            };
            if let Some(index_storage_ref) = &node.latest_storage_ref
                && index_storage_ref != &latest.storage_ref
            {
                return Err(GraphStorageError::OperationFailed {
                    operation: "validate_atomic_persistent_backup",
                    message: format!(
                        "label index `{label}` storage_ref mismatch for node `{:?}`",
                        node.node_id
                    ),
                });
            }
        }
    }

    for (relationship_type, entry) in &state.catalog.metadata_indexes.relationship_types {
        for relationship in &entry.relationships {
            let Some(latest) = latest_relationships.get(&relationship.relationship_id) else {
                return Err(GraphStorageError::OperationFailed {
                    operation: "validate_atomic_persistent_backup",
                    message: format!(
                        "relationship index `{:?}` references unknown relationship `{:?}`",
                        relationship_type, relationship.relationship_id
                    ),
                });
            };
            if let Some(index_storage_ref) = &relationship.latest_storage_ref
                && index_storage_ref != &latest.storage_ref
            {
                return Err(GraphStorageError::OperationFailed {
                    operation: "validate_atomic_persistent_backup",
                    message: format!(
                        "relationship index `{:?}` storage_ref mismatch for relationship `{:?}`",
                        relationship_type, relationship.relationship_id
                    ),
                });
            }
        }
    }

    for adjacency_record in snapshot_persisted_adjacency_records(&state.adjacency_storage) {
        if !latest_nodes.contains_key(&adjacency_record.owner_node_id) {
            return Err(GraphStorageError::OperationFailed {
                operation: "validate_atomic_persistent_backup",
                message: format!(
                    "adjacency owner node `{:?}` is not present in latest node catalog",
                    adjacency_record.owner_node_id
                ),
            });
        }
        for entry in adjacency_record.entries {
            if !latest_nodes.contains_key(&entry.source_node_id)
                || !latest_nodes.contains_key(&entry.target_node_id)
            {
                return Err(GraphStorageError::OperationFailed {
                    operation: "validate_atomic_persistent_backup",
                    message: format!(
                        "adjacency entry `{:?}` references unknown source or target node",
                        entry.relationship_id
                    ),
                });
            }
            if !latest_relationships.contains_key(&entry.relationship_id) {
                return Err(GraphStorageError::OperationFailed {
                    operation: "validate_atomic_persistent_backup",
                    message: format!(
                        "adjacency entry references unknown relationship `{:?}`",
                        entry.relationship_id
                    ),
                });
            }
        }
    }

    Ok(())
}

fn validate_catalog_payload_checksums(
    root: &StorageRoot,
    catalog: &GraphCatalog,
) -> GraphStorageResult<()> {
    for storage_ref in collect_catalog_storage_refs(catalog) {
        let Some(expected_checksum) = &storage_ref.checksum else {
            continue;
        };
        if !matches!(
            storage_ref.segment,
            StorageSegment::NodeRecords | StorageSegment::RelationshipRecords
        ) {
            continue;
        }
        let bytes = read_storage_ref_bytes(root, storage_ref)?;
        validate_encoded_record_checksum(&JsonLinesRecordCodec, &bytes, expected_checksum)?;
    }
    Ok(())
}

fn collect_catalog_storage_refs(catalog: &GraphCatalog) -> Vec<&StorageRef> {
    let mut refs = Vec::new();
    refs.extend(
        catalog
            .latest_node_records
            .values()
            .map(|entry| &entry.storage_ref),
    );
    refs.extend(
        catalog
            .latest_relationship_records
            .values()
            .map(|entry| &entry.storage_ref),
    );
    refs.extend(
        catalog
            .historical_records
            .iter()
            .map(|entry| &entry.storage_ref),
    );
    refs
}

fn read_storage_ref_bytes(
    root: &StorageRoot,
    storage_ref: &StorageRef,
) -> GraphStorageResult<Vec<u8>> {
    let segment_path = match storage_ref.segment {
        StorageSegment::NodeRecords => root.path().join("nodes").join("node_records.log"),
        StorageSegment::RelationshipRecords => root
            .path()
            .join("relationships")
            .join("relationship_records.log"),
        _ => {
            return Err(GraphStorageError::OperationFailed {
                operation: "validate_atomic_persistent_backup",
                message: format!(
                    "checksum validation does not support segment {:?}",
                    storage_ref.segment
                ),
            });
        }
    };
    let bytes = fs::read(&segment_path).map_err(|error| GraphStorageError::IoOperationFailed {
        operation: "validate_atomic_persistent_backup",
        path: Some(segment_path.clone()),
        message: error.to_string(),
    })?;

    let start =
        usize::try_from(storage_ref.offset).map_err(|_| GraphStorageError::OperationFailed {
            operation: "validate_atomic_persistent_backup",
            message: "storage_ref offset does not fit usize".to_owned(),
        })?;
    let length =
        usize::try_from(storage_ref.length).map_err(|_| GraphStorageError::OperationFailed {
            operation: "validate_atomic_persistent_backup",
            message: "storage_ref length does not fit usize".to_owned(),
        })?;
    let end = start
        .checked_add(length)
        .ok_or_else(|| GraphStorageError::OperationFailed {
            operation: "validate_atomic_persistent_backup",
            message: "storage_ref end overflow".to_owned(),
        })?;
    if end > bytes.len() {
        return Err(GraphStorageError::OperationFailed {
            operation: "validate_atomic_persistent_backup",
            message: format!(
                "storage_ref range [{start}, {end}) exceeds segment size {}",
                bytes.len()
            ),
        });
    }
    Ok(bytes[start..end].to_vec())
}

fn ensure_target_root_is_empty(target_root_path: &Path) -> GraphStorageResult<()> {
    if !target_root_path.exists() {
        fs::create_dir_all(target_root_path).map_err(|error| {
            GraphStorageError::IoOperationFailed {
                operation: "restore_atomic_persistent_backup",
                path: Some(target_root_path.to_path_buf()),
                message: error.to_string(),
            }
        })?;
        return Ok(());
    }
    if !target_root_path.is_dir() {
        return Err(GraphStorageError::StorageRootAlreadyExists {
            path: target_root_path.to_path_buf(),
        });
    }
    let mut entries =
        fs::read_dir(target_root_path).map_err(|error| GraphStorageError::IoOperationFailed {
            operation: "restore_atomic_persistent_backup",
            path: Some(target_root_path.to_path_buf()),
            message: error.to_string(),
        })?;
    if entries.next().is_some() {
        return Err(GraphStorageError::StorageRootAlreadyExists {
            path: target_root_path.to_path_buf(),
        });
    }
    Ok(())
}

fn copy_directory_recursive(
    source_dir: &Path,
    destination_dir: &Path,
    excluded_root_file_names: &HashSet<String>,
) -> GraphStorageResult<()> {
    if !source_dir.is_dir() {
        return Err(GraphStorageError::StorageRootNotFound {
            path: source_dir.to_path_buf(),
        });
    }
    fs::create_dir_all(destination_dir).map_err(|error| GraphStorageError::IoOperationFailed {
        operation: "copy_directory_recursive",
        path: Some(destination_dir.to_path_buf()),
        message: error.to_string(),
    })?;
    copy_directory_recursive_internal(
        source_dir,
        destination_dir,
        source_dir,
        excluded_root_file_names,
    )
}

fn copy_directory_recursive_internal(
    source_dir: &Path,
    destination_dir: &Path,
    root_source_dir: &Path,
    excluded_root_file_names: &HashSet<String>,
) -> GraphStorageResult<()> {
    let entries =
        fs::read_dir(source_dir).map_err(|error| GraphStorageError::IoOperationFailed {
            operation: "copy_directory_recursive",
            path: Some(source_dir.to_path_buf()),
            message: error.to_string(),
        })?;

    for entry in entries {
        let entry = entry.map_err(|error| GraphStorageError::IoOperationFailed {
            operation: "copy_directory_recursive",
            path: Some(source_dir.to_path_buf()),
            message: error.to_string(),
        })?;
        let source_path = entry.path();
        let relative_path = source_path.strip_prefix(root_source_dir).map_err(|error| {
            GraphStorageError::OperationFailed {
                operation: "copy_directory_recursive",
                message: format!("failed to compute relative path: {error}"),
            }
        })?;

        if relative_path.components().count() == 1
            && let Some(file_name) = relative_path.file_name().and_then(|value| value.to_str())
            && excluded_root_file_names.contains(file_name)
        {
            continue;
        }

        let destination_path = destination_dir.join(relative_path);
        if source_path.is_dir() {
            fs::create_dir_all(&destination_path).map_err(|error| {
                GraphStorageError::IoOperationFailed {
                    operation: "copy_directory_recursive",
                    path: Some(destination_path.clone()),
                    message: error.to_string(),
                }
            })?;
            copy_directory_recursive_internal(
                &source_path,
                destination_dir,
                root_source_dir,
                excluded_root_file_names,
            )?;
            continue;
        }
        fs::copy(&source_path, &destination_path).map_err(|error| {
            GraphStorageError::IoOperationFailed {
                operation: "copy_directory_recursive",
                path: Some(source_path.clone()),
                message: error.to_string(),
            }
        })?;
    }
    Ok(())
}

fn collect_backup_files(
    backup_root_path: &Path,
    include_backup_manifest_file: bool,
) -> GraphStorageResult<Vec<BackupFileRecord>> {
    let mut files = Vec::new();
    collect_backup_files_recursive(
        backup_root_path,
        backup_root_path,
        include_backup_manifest_file,
        &mut files,
    )?;
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(files)
}

fn collect_backup_files_recursive(
    backup_root_path: &Path,
    current_dir: &Path,
    include_backup_manifest_file: bool,
    files: &mut Vec<BackupFileRecord>,
) -> GraphStorageResult<()> {
    let entries =
        fs::read_dir(current_dir).map_err(|error| GraphStorageError::IoOperationFailed {
            operation: "collect_backup_files",
            path: Some(current_dir.to_path_buf()),
            message: error.to_string(),
        })?;
    for entry in entries {
        let entry = entry.map_err(|error| GraphStorageError::IoOperationFailed {
            operation: "collect_backup_files",
            path: Some(current_dir.to_path_buf()),
            message: error.to_string(),
        })?;
        let path = entry.path();
        if path.is_dir() {
            collect_backup_files_recursive(
                backup_root_path,
                &path,
                include_backup_manifest_file,
                files,
            )?;
            continue;
        }
        if !include_backup_manifest_file
            && path.file_name().and_then(|value| value.to_str()) == Some(BACKUP_MANIFEST_FILE_NAME)
        {
            continue;
        }

        let bytes = fs::read(&path).map_err(|error| GraphStorageError::IoOperationFailed {
            operation: "collect_backup_files",
            path: Some(path.clone()),
            message: error.to_string(),
        })?;
        let relative_path = path
            .strip_prefix(backup_root_path)
            .map_err(|error| GraphStorageError::OperationFailed {
                operation: "collect_backup_files",
                message: format!("failed to compute relative path: {error}"),
            })?
            .to_string_lossy()
            .replace('\\', "/");
        files.push(BackupFileRecord {
            relative_path,
            byte_length: bytes.len() as u64,
            checksum: calculate_encoded_record_checksum(&JsonLinesRecordCodec, &bytes)?,
        });
    }
    Ok(())
}

fn manifest_files_by_relative_path(
    manifest: &BackupManifestRecord,
) -> BTreeMap<String, BackupFileRecord> {
    manifest
        .files
        .iter()
        .map(|record| (record.relative_path.clone(), record.clone()))
        .collect()
}

fn read_json_lines_file<T: for<'de> Deserialize<'de>>(
    path: &Path,
    operation: &'static str,
) -> GraphStorageResult<Vec<T>> {
    let content =
        fs::read_to_string(path).map_err(|error| GraphStorageError::IoOperationFailed {
            operation,
            path: Some(path.to_path_buf()),
            message: error.to_string(),
        })?;
    let mut records = Vec::new();
    for (line_index, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let record = serde_json::from_str::<T>(line).map_err(|error| {
            GraphStorageError::OperationFailed {
                operation,
                message: format!(
                    "failed to decode JSON line {} from {}: {error}",
                    line_index + 1,
                    path.display()
                ),
            }
        })?;
        records.push(record);
    }
    Ok(records)
}

fn read_json_file<T: for<'de> Deserialize<'de>>(
    path: &Path,
    operation: &'static str,
) -> GraphStorageResult<T> {
    let bytes = fs::read(path).map_err(|error| GraphStorageError::IoOperationFailed {
        operation,
        path: Some(path.to_path_buf()),
        message: error.to_string(),
    })?;
    serde_json::from_slice::<T>(&bytes).map_err(|error| GraphStorageError::OperationFailed {
        operation,
        message: format!("failed to decode JSON file {}: {error}", path.display()),
    })
}

fn write_json_file<T: Serialize>(
    path: &Path,
    value: &T,
    operation: &'static str,
) -> GraphStorageResult<()> {
    let bytes =
        serde_json::to_vec_pretty(value).map_err(|error| GraphStorageError::OperationFailed {
            operation,
            message: format!("failed to encode JSON: {error}"),
        })?;
    fs::write(path, bytes).map_err(|error| GraphStorageError::IoOperationFailed {
        operation,
        path: Some(path.to_path_buf()),
        message: error.to_string(),
    })
}

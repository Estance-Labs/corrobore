// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! Production database-operation orchestration.
//!
//! This module keeps canonical data authoritative while coordinating snapshots,
//! restores, schema migrations and derived-index rebuilds. Operation state is
//! persisted below `operations/` so interruption never turns partial work into
//! writable or query-ready state.

use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
    CanonicalEngineStore, GraphStorageError, GraphStorageResult, JsonLinesRecordCodec,
    RecordChecksum, StorageRoot, calculate_encoded_record_checksum,
    create_atomic_persistent_backup, open_storage_root, read_storage_manifest,
    restore_atomic_persistent_backup, validate_atomic_persistent_backup,
};

const SNAPSHOT_MANIFEST: &str = "snapshot_manifest.json";
const SNAPSHOT_DATA_DIRECTORY: &str = "data";
const SNAPSHOT_MANIFEST_VERSION: u32 = 1;
const OPERATIONS_DIRECTORY: &str = "operations";
const MIGRATION_STATE: &str = "storage-migration.json";
const MIGRATION_ROLLBACK_MANIFEST: &str = "manifest.v0.rollback.json";
const INDEX_REBUILD_STATE: &str = "index-rebuild.json";

/// Explicit readiness exposed while a database operation is incomplete.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationReadiness {
    /// The operation has completed and passed parity validation.
    Ready,
    /// Derived projections are being reconstructed and must not serve complete reads.
    Rebuilding,
    /// The operation requires exclusive offline ownership of the data directory.
    OfflineRequired,
    /// Validation failed and operator intervention is required.
    Failed,
    /// An operator requested cancellation at a safe resumable boundary.
    Cancelled,
}

/// Deterministic interruption points used to prove resumability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DatabaseOperationCrashStage {
    /// Interrupt after canonical migration but before projection rebuild/parity.
    AfterCanonicalMigration,
    /// Interrupt after canonical compact indexes but before remaining projections.
    AfterCanonicalIndexes,
}

/// Operator-provided snapshot metadata that never contains key material.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotRequest {
    /// Stable creation timestamp supplied by the operation boundary.
    pub created_at: String,
    /// Optional key-provider identity. Encryption keys remain outside the artifact.
    pub encryption_key_id: Option<String>,
    /// Optional lifecycle/retention hook recorded for the destination provider.
    pub retention_hook: Option<String>,
}

/// Completed coherent snapshot diagnostics.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SnapshotReport {
    /// Snapshot manifest schema version.
    pub manifest_version: u32,
    /// Final operation readiness.
    pub readiness: OperationReadiness,
    /// Stable graph identity.
    pub graph_id: String,
    /// Canonical checkpoint generation captured by the barrier.
    pub canonical_generation: u64,
    /// WAL boundary included in the artifact.
    pub wal_boundary: u64,
    /// Number of checksummed storage components.
    pub file_count: usize,
    /// Total checksummed bytes.
    pub total_bytes: u64,
    /// Local snapshot artifact root.
    pub snapshot_root: PathBuf,
}

/// Successful snapshot validation diagnostics.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SnapshotValidationReport {
    /// Number of component checksums validated.
    pub verified_files: usize,
    /// Number of verified bytes.
    pub verified_bytes: u64,
    /// Validated canonical/WAL generation.
    pub canonical_generation: u64,
}

/// Object-store boundary implemented by S3/MinIO providers and deterministic tests.
pub trait SnapshotArtifactStore {
    /// Upload or atomically replace one object.
    fn put(&mut self, key: &str, bytes: &[u8]) -> Result<(), String>;
    /// Download one object.
    fn get(&self, key: &str) -> Result<Vec<u8>, String>;
    /// List object keys below a bounded prefix.
    fn list(&self, prefix: &str) -> Result<Vec<String>, String>;
}

/// Snapshot export diagnostics.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SnapshotExportReport {
    /// Provider-relative destination prefix.
    pub destination: String,
    /// Number of objects uploaded, including the manifest published last.
    pub uploaded_objects: usize,
    /// Number of bytes uploaded.
    pub uploaded_bytes: u64,
}

/// Supported previous-to-current migration request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationRequest {
    /// Previous on-disk storage version.
    pub source_version: String,
    /// Current on-disk storage version.
    pub target_version: String,
    /// Stable operation timestamp.
    pub started_at: String,
}

impl MigrationRequest {
    /// Construct the supported previous-version upgrade plan.
    pub fn v0_to_v1(started_at: impl Into<String>) -> Self {
        Self {
            source_version: "V0".to_owned(),
            target_version: "V1".to_owned(),
            started_at: started_at.into(),
        }
    }
}

/// Migration or rollback diagnostics.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MigrationReport {
    /// Current readiness boundary.
    pub readiness: OperationReadiness,
    /// Whether durable progress from an earlier run was resumed.
    pub resumed: bool,
    /// Completed deterministic steps.
    pub completed_steps: usize,
    /// Total deterministic steps.
    pub total_steps: usize,
    /// Whether canonical and derived parity was verified.
    pub parity_verified: bool,
}

/// Every rebuildable Knowledge Data Engine projection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DerivedIndexKind {
    /// Stable identifier lookup.
    Identifier,
    /// Incoming and outgoing adjacency.
    Adjacency,
    /// Scalar property lookup.
    Property,
    /// Temporal scalar lookup.
    Temporal,
    /// Access-aware full-text search.
    FullText,
    /// Aggregation acceleration metadata.
    Aggregation,
    /// Extracted file-content search.
    FileContent,
    /// Node and relationship access-policy metadata.
    AccessPolicy,
}

/// Complete derived-index rebuild diagnostics.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct IndexRebuildReport {
    /// Current projection readiness.
    pub readiness: OperationReadiness,
    /// Whether the operation resumed durable progress.
    pub resumed: bool,
    /// Rebuilt projection kinds.
    pub completed_indexes: Vec<DerivedIndexKind>,
    /// Canonical records inspected.
    pub records_scanned: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SnapshotManifest {
    manifest_version: u32,
    readiness: OperationReadiness,
    graph_id: String,
    storage_version: String,
    record_format: String,
    created_at: String,
    encryption_key_id: Option<String>,
    retention_hook: Option<String>,
    canonical_generation: u64,
    wal_boundary: u64,
    components: Vec<SnapshotComponent>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SnapshotComponent {
    relative_path: String,
    byte_length: u64,
    checksum: RecordChecksum,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PersistedMigrationState {
    schema_version: u32,
    request: MigrationRequest,
    readiness: OperationReadiness,
    completed_steps: usize,
    total_steps: usize,
    parity_verified: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PersistedIndexRebuildState {
    schema_version: u32,
    readiness: OperationReadiness,
    completed_indexes: Vec<DerivedIndexKind>,
    records_scanned: u64,
}

/// Create one coherent canonical/WAL snapshot. Callers must hold the runtime's
/// exclusive write barrier while this synchronous operation executes.
pub fn create_consistent_snapshot(
    source_root: &StorageRoot,
    snapshot_root: impl AsRef<Path>,
    request: SnapshotRequest,
) -> GraphStorageResult<SnapshotReport> {
    validate_snapshot_request(&request)?;
    let snapshot_root = snapshot_root.as_ref();
    if snapshot_root.exists() {
        return Err(GraphStorageError::StorageRootAlreadyExists {
            path: snapshot_root.to_path_buf(),
        });
    }
    fs::create_dir_all(snapshot_root)
        .map_err(|error| io_error("create_consistent_snapshot", snapshot_root, error))?;
    let data_root = snapshot_root.join(SNAPSHOT_DATA_DIRECTORY);
    let backup = match create_atomic_persistent_backup(source_root, &data_root) {
        Ok(backup) => backup,
        Err(error) => {
            let _ = fs::remove_dir_all(snapshot_root);
            return Err(error);
        }
    };
    let storage = read_storage_manifest(source_root)?;
    let components = collect_components(snapshot_root, snapshot_root, false)?;
    let manifest = SnapshotManifest {
        manifest_version: SNAPSHOT_MANIFEST_VERSION,
        readiness: OperationReadiness::Ready,
        graph_id: storage.graph_id.value,
        storage_version: storage_version_label(&storage.storage_version),
        record_format: record_format_label(&storage.record_format),
        created_at: request.created_at,
        encryption_key_id: request.encryption_key_id,
        retention_hook: request.retention_hook,
        canonical_generation: backup.checkpoint_sequence_number.0,
        wal_boundary: backup.checkpoint_sequence_number.0,
        components,
    };
    write_json_atomic(
        snapshot_root,
        &snapshot_root.join(SNAPSHOT_MANIFEST),
        &manifest,
        "create_consistent_snapshot",
    )?;
    let validation =
        validate_consistent_snapshot(snapshot_root, manifest.encryption_key_id.as_deref())?;
    Ok(snapshot_report(snapshot_root, &manifest, &validation))
}

/// Validate manifest compatibility, key identity and every component checksum.
pub fn validate_consistent_snapshot(
    snapshot_root: impl AsRef<Path>,
    expected_encryption_key_id: Option<&str>,
) -> GraphStorageResult<SnapshotValidationReport> {
    let snapshot_root = snapshot_root.as_ref();
    let manifest: SnapshotManifest = read_json(
        &snapshot_root.join(SNAPSHOT_MANIFEST),
        "validate_consistent_snapshot",
    )?;
    if manifest.manifest_version != SNAPSHOT_MANIFEST_VERSION
        || manifest.readiness != OperationReadiness::Ready
        || manifest.storage_version != "V1"
        || manifest.record_format != "JsonLinesV1"
        || manifest.canonical_generation == 0
        || manifest.canonical_generation != manifest.wal_boundary
    {
        return operation_error(
            "validate_consistent_snapshot",
            "unsupported or incomplete snapshot manifest",
        );
    }
    if let Some(expected) = expected_encryption_key_id
        && manifest.encryption_key_id.as_deref() != Some(expected)
    {
        return operation_error(
            "validate_consistent_snapshot",
            "snapshot encryption key identity does not match configured key",
        );
    }
    let mut verified_bytes = 0_u64;
    for component in &manifest.components {
        let relative = safe_relative_path(&component.relative_path)?;
        let path = snapshot_root.join(relative);
        let bytes = fs::read(&path)
            .map_err(|error| io_error("validate_consistent_snapshot", &path, error))?;
        if bytes.len() as u64 != component.byte_length {
            return operation_error(
                "validate_consistent_snapshot",
                format!(
                    "snapshot component length mismatch: {}",
                    component.relative_path
                ),
            );
        }
        let actual = calculate_encoded_record_checksum(&JsonLinesRecordCodec, &bytes)?;
        if actual != component.checksum {
            return Err(GraphStorageError::ChecksumMismatch {
                expected: component.checksum.clone(),
                actual,
            });
        }
        verified_bytes = verified_bytes.saturating_add(component.byte_length);
    }
    let backup = validate_atomic_persistent_backup(snapshot_root.join(SNAPSHOT_DATA_DIRECTORY))?;
    if backup.checkpoint_sequence_number.0 != manifest.wal_boundary {
        return operation_error(
            "validate_consistent_snapshot",
            "snapshot WAL boundary differs from validated backup checkpoint",
        );
    }
    Ok(SnapshotValidationReport {
        verified_files: manifest.components.len(),
        verified_bytes,
        canonical_generation: manifest.canonical_generation,
    })
}

/// Validate first, then restore into a new empty data directory.
pub fn restore_consistent_snapshot(
    snapshot_root: impl AsRef<Path>,
    target_root: impl AsRef<Path>,
    expected_encryption_key_id: Option<&str>,
) -> GraphStorageResult<SnapshotReport> {
    let snapshot_root = snapshot_root.as_ref();
    let target_root = target_root.as_ref();
    let manifest: SnapshotManifest = read_json(
        &snapshot_root.join(SNAPSHOT_MANIFEST),
        "restore_consistent_snapshot",
    )?;
    if manifest.encryption_key_id.is_some() && expected_encryption_key_id.is_none() {
        return operation_error(
            "restore_consistent_snapshot",
            "snapshot requires its configured encryption key identity",
        );
    }
    let validation = validate_consistent_snapshot(snapshot_root, expected_encryption_key_id)?;
    restore_atomic_persistent_backup(snapshot_root.join(SNAPSHOT_DATA_DIRECTORY), target_root)?;
    // Reopening after the copy proves compatibility and recovery before the
    // caller is allowed to expose the target as writable.
    let restored = open_storage_root(target_root)?;
    let restored_manifest = read_storage_manifest(&restored)?;
    if restored_manifest.graph_id.value != manifest.graph_id {
        let _ = fs::remove_dir_all(target_root);
        return operation_error(
            "restore_consistent_snapshot",
            "restored graph identity differs from snapshot manifest",
        );
    }
    Ok(snapshot_report(snapshot_root, &manifest, &validation))
}

/// Upload checksummed components and publish the snapshot manifest last.
pub fn export_snapshot_to_store(
    snapshot_root: impl AsRef<Path>,
    store: &mut dyn SnapshotArtifactStore,
    destination_prefix: &str,
) -> GraphStorageResult<SnapshotExportReport> {
    let snapshot_root = snapshot_root.as_ref();
    validate_consistent_snapshot(snapshot_root, None)?;
    let prefix = destination_prefix.trim_matches('/');
    if prefix.is_empty() {
        return operation_error(
            "export_snapshot_to_store",
            "destination prefix must not be empty",
        );
    }
    let manifest_path = snapshot_root.join(SNAPSHOT_MANIFEST);
    let mut paths = collect_file_paths(snapshot_root)?;
    paths.retain(|path| path != &manifest_path);
    paths.sort();
    let mut uploaded_objects = 0_usize;
    let mut uploaded_bytes = 0_u64;
    for path in paths.into_iter().chain(std::iter::once(manifest_path)) {
        let relative = normalized_relative(snapshot_root, &path)?;
        let key = format!("{prefix}/{relative}");
        let bytes =
            fs::read(&path).map_err(|error| io_error("export_snapshot_to_store", &path, error))?;
        store
            .put(&key, &bytes)
            .map_err(|message| GraphStorageError::OperationFailed {
                operation: "export_snapshot_to_store",
                message,
            })?;
        let verified = store
            .get(&key)
            .map_err(|message| GraphStorageError::OperationFailed {
                operation: "export_snapshot_to_store",
                message,
            })?;
        if verified != bytes {
            return operation_error(
                "export_snapshot_to_store",
                format!("object verification failed for {key}"),
            );
        }
        uploaded_objects = uploaded_objects.saturating_add(1);
        uploaded_bytes = uploaded_bytes.saturating_add(bytes.len() as u64);
    }
    let published_manifest = format!("{prefix}/{SNAPSHOT_MANIFEST}");
    let listed =
        store
            .list(&published_manifest)
            .map_err(|message| GraphStorageError::OperationFailed {
                operation: "export_snapshot_to_store",
                message,
            })?;
    if !listed.contains(&published_manifest) {
        return operation_error(
            "export_snapshot_to_store",
            "destination object listing is missing the published snapshot manifest",
        );
    }
    Ok(SnapshotExportReport {
        destination: prefix.to_owned(),
        uploaded_objects,
        uploaded_bytes,
    })
}

/// Run or resume the supported previous-version migration.
pub fn migrate_storage(
    root: impl AsRef<Path>,
    request: MigrationRequest,
    crash_stage: Option<DatabaseOperationCrashStage>,
) -> GraphStorageResult<MigrationReport> {
    if request.source_version != "V0" || request.target_version != "V1" {
        return operation_error("migrate_storage", "unsupported storage migration plan");
    }
    let root = root.as_ref();
    let operations = root.join(OPERATIONS_DIRECTORY);
    fs::create_dir_all(&operations)
        .map_err(|error| io_error("migrate_storage", &operations, error))?;
    let state_path = operations.join(MIGRATION_STATE);
    let rollback_path = operations.join(MIGRATION_ROLLBACK_MANIFEST);
    let manifest_path = root.join("manifest.json");
    let existing = state_path
        .is_file()
        .then(|| read_json::<PersistedMigrationState>(&state_path, "migrate_storage"))
        .transpose()?;
    let resumed = existing.is_some();
    let mut state = existing.unwrap_or(PersistedMigrationState {
        schema_version: 1,
        request: request.clone(),
        readiness: OperationReadiness::Rebuilding,
        completed_steps: 0,
        total_steps: 4,
        parity_verified: false,
    });
    // `started_at` records the first attempt and naturally differs when an
    // operator resumes the same plan from a new CLI invocation. The durable
    // source/target pair is the operation identity.
    if state.schema_version != 1
        || state.request.source_version != request.source_version
        || state.request.target_version != request.target_version
    {
        return operation_error(
            "migrate_storage",
            "migration request conflicts with durable operation state",
        );
    }
    if state.readiness == OperationReadiness::Ready {
        return Ok(migration_report(&state, true));
    }
    if state.completed_steps == 0 {
        let manifest_bytes = fs::read(&manifest_path)
            .map_err(|error| io_error("migrate_storage", &manifest_path, error))?;
        let manifest_text = std::str::from_utf8(&manifest_bytes).map_err(|error| {
            GraphStorageError::OperationFailed {
                operation: "migrate_storage",
                message: error.to_string(),
            }
        })?;
        if !manifest_text.contains("\"storage_version\"") || !manifest_text.contains("\"V0\"") {
            return operation_error(
                "migrate_storage",
                "source manifest is not the supported previous V0 version",
            );
        }
        write_bytes_atomic(
            &operations,
            &rollback_path,
            &manifest_bytes,
            "migrate_storage",
        )?;
        state.completed_steps = 1;
        write_json_atomic(&operations, &state_path, &state, "migrate_storage")?;
    }
    if state.completed_steps == 1 {
        let manifest = fs::read_to_string(&manifest_path)
            .map_err(|error| io_error("migrate_storage", &manifest_path, error))?;
        let upgraded = manifest.replacen("\"V0\"", "\"V1\"", 1);
        write_bytes_atomic(root, &manifest_path, upgraded.as_bytes(), "migrate_storage")?;
        state.completed_steps = 2;
        write_json_atomic(&operations, &state_path, &state, "migrate_storage")?;
        if crash_stage == Some(DatabaseOperationCrashStage::AfterCanonicalMigration) {
            return operation_error(
                "migrate_storage",
                "injected crash after canonical migration",
            );
        }
    }
    if state.completed_steps == 2 {
        let storage_root = open_storage_root(root)?;
        let mut store = CanonicalEngineStore::open(storage_root, Default::default())?;
        rebuild_derived_indexes(&mut store, None)?;
        state.completed_steps = 3;
        write_json_atomic(&operations, &state_path, &state, "migrate_storage")?;
    }
    if state.completed_steps == 3 {
        let storage_root = open_storage_root(root)?;
        let mut store = CanonicalEngineStore::open(storage_root, Default::default())?;
        let graph = store.load_projection(crate::CanonicalProjectionRequest::all())?;
        let canonical_records = store.catalog().latest_node_records.len()
            + store.catalog().latest_relationship_records.len();
        let projected_records = graph
            .current_node_records()
            .map_err(graph_operation_error)?
            .len()
            + graph
                .current_relationship_records()
                .map_err(graph_operation_error)?
                .len();
        if canonical_records != projected_records || !store.full_text_projection_is_ready()? {
            state.readiness = OperationReadiness::Failed;
            write_json_atomic(&operations, &state_path, &state, "migrate_storage")?;
            return operation_error("migrate_storage", "post-migration parity validation failed");
        }
        state.completed_steps = 4;
        state.parity_verified = true;
        state.readiness = OperationReadiness::Ready;
        write_json_atomic(&operations, &state_path, &state, "migrate_storage")?;
    }
    Ok(migration_report(&state, resumed))
}

/// Roll back the manifest boundary while the canonical record format remains compatible.
pub fn rollback_storage_migration(root: impl AsRef<Path>) -> GraphStorageResult<MigrationReport> {
    let root = root.as_ref();
    let operations = root.join(OPERATIONS_DIRECTORY);
    let state_path = operations.join(MIGRATION_STATE);
    let rollback_path = operations.join(MIGRATION_ROLLBACK_MANIFEST);
    let mut state: PersistedMigrationState = read_json(&state_path, "rollback_storage_migration")?;
    if !state.parity_verified || state.readiness != OperationReadiness::Ready {
        return operation_error(
            "rollback_storage_migration",
            "rollback requires a completed compatible migration",
        );
    }
    let previous = fs::read(&rollback_path)
        .map_err(|error| io_error("rollback_storage_migration", &rollback_path, error))?;
    write_bytes_atomic(
        root,
        &root.join("manifest.json"),
        &previous,
        "rollback_storage_migration",
    )?;
    state.readiness = OperationReadiness::OfflineRequired;
    write_json_atomic(
        &operations,
        &state_path,
        &state,
        "rollback_storage_migration",
    )?;
    Ok(migration_report(&state, true))
}

/// Rebuild every derived projection from canonical data and indexable file metadata.
pub fn rebuild_derived_indexes(
    store: &mut CanonicalEngineStore,
    crash_stage: Option<DatabaseOperationCrashStage>,
) -> GraphStorageResult<IndexRebuildReport> {
    let operations = store.root().path().join(OPERATIONS_DIRECTORY);
    fs::create_dir_all(&operations)
        .map_err(|error| io_error("rebuild_derived_indexes", &operations, error))?;
    let state_path = operations.join(INDEX_REBUILD_STATE);
    let existing = state_path
        .is_file()
        .then(|| read_json::<PersistedIndexRebuildState>(&state_path, "rebuild_derived_indexes"))
        .transpose()?;
    let resumed = existing
        .as_ref()
        .is_some_and(|state| state.readiness != OperationReadiness::Ready);
    let mut state = existing
        .filter(|state| state.readiness != OperationReadiness::Ready)
        .unwrap_or(PersistedIndexRebuildState {
            schema_version: 1,
            readiness: OperationReadiness::Rebuilding,
            completed_indexes: Vec::new(),
            records_scanned: 0,
        });
    if state.schema_version != 1 {
        return operation_error(
            "rebuild_derived_indexes",
            "unsupported rebuild state version",
        );
    }
    state.readiness = OperationReadiness::Rebuilding;
    write_json_atomic(&operations, &state_path, &state, "rebuild_derived_indexes")?;
    if !state
        .completed_indexes
        .contains(&DerivedIndexKind::Identifier)
    {
        state.records_scanned = store.rebuild_compact_indexes()?;
        state.completed_indexes.extend([
            DerivedIndexKind::Identifier,
            DerivedIndexKind::Adjacency,
            DerivedIndexKind::Property,
            DerivedIndexKind::Temporal,
            DerivedIndexKind::Aggregation,
            DerivedIndexKind::AccessPolicy,
        ]);
        write_json_atomic(&operations, &state_path, &state, "rebuild_derived_indexes")?;
        if crash_stage == Some(DatabaseOperationCrashStage::AfterCanonicalIndexes) {
            return operation_error(
                "rebuild_derived_indexes",
                "injected crash after canonical indexes",
            );
        }
    }
    if !state
        .completed_indexes
        .contains(&DerivedIndexKind::FullText)
    {
        store.rebuild_full_text_index()?;
        state.completed_indexes.push(DerivedIndexKind::FullText);
        write_json_atomic(&operations, &state_path, &state, "rebuild_derived_indexes")?;
    }
    if !state
        .completed_indexes
        .contains(&DerivedIndexKind::FileContent)
    {
        store.rebuild_file_content_index()?;
        state.completed_indexes.push(DerivedIndexKind::FileContent);
    }
    state.completed_indexes.sort();
    state.completed_indexes.dedup();
    if !store.full_text_projection_is_ready()? {
        state.readiness = OperationReadiness::Failed;
        write_json_atomic(&operations, &state_path, &state, "rebuild_derived_indexes")?;
        return operation_error(
            "rebuild_derived_indexes",
            "full-text parity validation failed",
        );
    }
    state.readiness = OperationReadiness::Ready;
    write_json_atomic(&operations, &state_path, &state, "rebuild_derived_indexes")?;
    Ok(index_rebuild_report(&state, resumed))
}

/// Read durable rebuild readiness without starting or resuming work.
pub fn derived_index_rebuild_status(
    root: impl AsRef<Path>,
) -> GraphStorageResult<Option<IndexRebuildReport>> {
    let state_path = root
        .as_ref()
        .join(OPERATIONS_DIRECTORY)
        .join(INDEX_REBUILD_STATE);
    if !state_path.is_file() {
        return Ok(None);
    }
    let state: PersistedIndexRebuildState = read_json(&state_path, "derived_index_rebuild_status")?;
    if state.schema_version != 1 {
        return operation_error(
            "derived_index_rebuild_status",
            "unsupported rebuild state version",
        );
    }
    Ok(Some(index_rebuild_report(&state, true)))
}

/// Request cancellation at the next safe rebuild boundary. A later rebuild call
/// resumes from the durable completed-index list.
pub fn cancel_derived_index_rebuild(
    root: impl AsRef<Path>,
) -> GraphStorageResult<IndexRebuildReport> {
    let operations = root.as_ref().join(OPERATIONS_DIRECTORY);
    let state_path = operations.join(INDEX_REBUILD_STATE);
    let mut state: PersistedIndexRebuildState =
        read_json(&state_path, "cancel_derived_index_rebuild")?;
    if state.readiness == OperationReadiness::Ready {
        return operation_error(
            "cancel_derived_index_rebuild",
            "completed index rebuild cannot be cancelled",
        );
    }
    state.readiness = OperationReadiness::Cancelled;
    write_json_atomic(
        &operations,
        &state_path,
        &state,
        "cancel_derived_index_rebuild",
    )?;
    Ok(index_rebuild_report(&state, true))
}

fn validate_snapshot_request(request: &SnapshotRequest) -> GraphStorageResult<()> {
    if request.created_at.trim().is_empty()
        || request
            .encryption_key_id
            .as_ref()
            .is_some_and(|value| value.trim().is_empty())
        || request
            .retention_hook
            .as_ref()
            .is_some_and(|value| value.trim().is_empty())
    {
        return operation_error(
            "create_consistent_snapshot",
            "snapshot metadata values must not be empty",
        );
    }
    Ok(())
}

fn snapshot_report(
    snapshot_root: &Path,
    manifest: &SnapshotManifest,
    validation: &SnapshotValidationReport,
) -> SnapshotReport {
    SnapshotReport {
        manifest_version: manifest.manifest_version,
        readiness: manifest.readiness,
        graph_id: manifest.graph_id.clone(),
        canonical_generation: manifest.canonical_generation,
        wal_boundary: manifest.wal_boundary,
        file_count: validation.verified_files,
        total_bytes: validation.verified_bytes,
        snapshot_root: snapshot_root.to_path_buf(),
    }
}

fn collect_components(
    snapshot_root: &Path,
    current: &Path,
    include_manifest: bool,
) -> GraphStorageResult<Vec<SnapshotComponent>> {
    let mut components = Vec::new();
    for path in collect_file_paths(current)? {
        if !include_manifest && path == snapshot_root.join(SNAPSHOT_MANIFEST) {
            continue;
        }
        let bytes = fs::read(&path)
            .map_err(|error| io_error("collect_snapshot_components", &path, error))?;
        components.push(SnapshotComponent {
            relative_path: normalized_relative(snapshot_root, &path)?,
            byte_length: bytes.len() as u64,
            checksum: calculate_encoded_record_checksum(&JsonLinesRecordCodec, &bytes)?,
        });
    }
    components.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(components)
}

fn collect_file_paths(current: &Path) -> GraphStorageResult<Vec<PathBuf>> {
    let mut paths = Vec::new();
    let mut entries = fs::read_dir(current)
        .map_err(|error| io_error("collect_snapshot_files", current, error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| io_error("collect_snapshot_files", current, error))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            paths.extend(collect_file_paths(&path)?);
        } else if path.is_file() {
            paths.push(path);
        }
    }
    Ok(paths)
}

fn normalized_relative(root: &Path, path: &Path) -> GraphStorageResult<String> {
    path.strip_prefix(root)
        .map(|relative| relative.to_string_lossy().replace('\\', "/"))
        .map_err(|error| GraphStorageError::OperationFailed {
            operation: "normalize_snapshot_path",
            message: error.to_string(),
        })
}

fn safe_relative_path(value: &str) -> GraphStorageResult<PathBuf> {
    let path = Path::new(value);
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return operation_error(
            "validate_consistent_snapshot",
            "snapshot component path is not a safe relative path",
        );
    }
    Ok(path.to_path_buf())
}

fn migration_report(state: &PersistedMigrationState, resumed: bool) -> MigrationReport {
    MigrationReport {
        readiness: state.readiness,
        resumed,
        completed_steps: state.completed_steps,
        total_steps: state.total_steps,
        parity_verified: state.parity_verified,
    }
}

fn index_rebuild_report(state: &PersistedIndexRebuildState, resumed: bool) -> IndexRebuildReport {
    IndexRebuildReport {
        readiness: state.readiness,
        resumed,
        completed_indexes: state.completed_indexes.clone(),
        records_scanned: state.records_scanned,
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(
    path: &Path,
    operation: &'static str,
) -> GraphStorageResult<T> {
    let bytes = fs::read(path).map_err(|error| io_error(operation, path, error))?;
    serde_json::from_slice(&bytes).map_err(|error| GraphStorageError::OperationFailed {
        operation,
        message: format!("failed to decode {}: {error}", path.display()),
    })
}

fn write_json_atomic<T: Serialize>(
    directory: &Path,
    target: &Path,
    value: &T,
    operation: &'static str,
) -> GraphStorageResult<()> {
    let mut bytes =
        serde_json::to_vec_pretty(value).map_err(|error| GraphStorageError::OperationFailed {
            operation,
            message: error.to_string(),
        })?;
    bytes.push(b'\n');
    write_bytes_atomic(directory, target, &bytes, operation)
}

fn write_bytes_atomic(
    directory: &Path,
    target: &Path,
    bytes: &[u8],
    operation: &'static str,
) -> GraphStorageResult<()> {
    fs::create_dir_all(directory).map_err(|error| io_error(operation, directory, error))?;
    let temporary = target.with_extension("next");
    if temporary.exists() {
        fs::remove_file(&temporary).map_err(|error| io_error(operation, &temporary, error))?;
    }
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|error| io_error(operation, &temporary, error))?;
    file.write_all(bytes)
        .map_err(|error| io_error(operation, &temporary, error))?;
    file.sync_all()
        .map_err(|error| io_error(operation, &temporary, error))?;
    fs::rename(&temporary, target).map_err(|error| io_error(operation, target, error))?;
    File::open(directory)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| io_error(operation, directory, error))
}

fn storage_version_label(version: &crate::StorageVersion) -> String {
    match version {
        crate::StorageVersion::V1 => "V1".to_owned(),
        crate::StorageVersion::Unsupported(value) => value.clone(),
    }
}

fn record_format_label(format: &crate::RecordFormat) -> String {
    match format {
        crate::RecordFormat::JsonLinesV1 => "JsonLinesV1".to_owned(),
        crate::RecordFormat::Unsupported(value) => value.clone(),
    }
}

fn graph_operation_error(error: graph_core::GraphError) -> GraphStorageError {
    GraphStorageError::OperationFailed {
        operation: "validate_database_operation_parity",
        message: error.to_string(),
    }
}

fn io_error(operation: &'static str, path: &Path, error: std::io::Error) -> GraphStorageError {
    GraphStorageError::IoOperationFailed {
        operation,
        path: Some(path.to_path_buf()),
        message: error.to_string(),
    }
}

fn operation_error<T>(
    operation: &'static str,
    message: impl Into<String>,
) -> GraphStorageResult<T> {
    Err(GraphStorageError::OperationFailed {
        operation,
        message: message.into(),
    })
}

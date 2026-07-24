// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use graph_core::{Graph, GraphPersistenceSnapshot};
use serde::{Deserialize, Serialize};

use crate::{GraphStorageError, GraphStorageResult, RecordFormat, StorageRoot, StorageVersion};

const SNAPSHOT_DIRECTORY: &str = "runtime";
const SNAPSHOT_FILE: &str = "engine-graph.json";
const SNAPSHOT_BACKUP_FILE: &str = "engine-graph.previous.json";
const SNAPSHOT_TEMP_FILE: &str = "engine-graph.next.json";

#[derive(Serialize, Deserialize)]
struct EngineGraphSnapshot {
    storage_version: StorageVersion,
    record_format: RecordFormat,
    graph: GraphPersistenceSnapshot,
}

/// Load the durable in-memory engine graph associated with a storage root.
///
/// A missing snapshot represents a new empty graph. If an interrupted atomic
/// replacement left only the previous snapshot, that verified snapshot is used
/// as the recovery source.
pub fn load_engine_graph_snapshot(root: &StorageRoot) -> GraphStorageResult<Graph> {
    let directory = snapshot_directory(root);
    let current = directory.join(SNAPSHOT_FILE);
    let previous = directory.join(SNAPSHOT_BACKUP_FILE);
    let source = if current.is_file() {
        current
    } else if previous.is_file() {
        previous
    } else {
        return Ok(Graph::new());
    };

    let bytes = fs::read(&source).map_err(|error| GraphStorageError::OperationFailed {
        operation: "load_engine_graph_snapshot",
        message: format!("failed to read {}: {error}", source.display()),
    })?;
    let snapshot: EngineGraphSnapshot =
        serde_json::from_slice(&bytes).map_err(|error| GraphStorageError::DecodeFailed {
            format: "engine-graph-json-v1".to_owned(),
            reason: error.to_string(),
        })?;
    if snapshot.storage_version != StorageVersion::V1
        || snapshot.record_format != RecordFormat::JsonLinesV1
    {
        return Err(GraphStorageError::DecodeFailed {
            format: "engine-graph-json-v1".to_owned(),
            reason: format!(
                "unsupported engine snapshot compatibility: {:?}/{:?}",
                snapshot.storage_version, snapshot.record_format
            ),
        });
    }
    Graph::from_persistence_snapshot(snapshot.graph).map_err(|error| {
        GraphStorageError::DecodeFailed {
            format: "engine-graph-json-v1".to_owned(),
            reason: error.to_string(),
        }
    })
}

/// Atomically persist the engine graph under the versioned storage root.
///
/// The previous complete snapshot remains available until the replacement is
/// durable. `require_fsync` synchronizes both file contents and the containing
/// directory before the function reports success.
pub fn persist_engine_graph_snapshot(
    root: &StorageRoot,
    graph: &Graph,
    require_fsync: bool,
) -> GraphStorageResult<()> {
    let directory = snapshot_directory(root);
    fs::create_dir_all(&directory).map_err(|error| GraphStorageError::OperationFailed {
        operation: "persist_engine_graph_snapshot",
        message: format!("failed to create {}: {error}", directory.display()),
    })?;

    let current = directory.join(SNAPSHOT_FILE);
    let previous = directory.join(SNAPSHOT_BACKUP_FILE);
    let temporary = directory.join(SNAPSHOT_TEMP_FILE);
    if temporary.exists() {
        fs::remove_file(&temporary).map_err(|error| GraphStorageError::OperationFailed {
            operation: "persist_engine_graph_snapshot",
            message: format!("failed to remove stale {}: {error}", temporary.display()),
        })?;
    }

    let snapshot = EngineGraphSnapshot {
        storage_version: StorageVersion::V1,
        record_format: RecordFormat::JsonLinesV1,
        graph: graph.persistence_snapshot(),
    };
    let mut bytes =
        serde_json::to_vec(&snapshot).map_err(|error| GraphStorageError::OperationFailed {
            operation: "persist_engine_graph_snapshot",
            message: error.to_string(),
        })?;
    bytes.push(b'\n');

    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|error| GraphStorageError::OperationFailed {
            operation: "persist_engine_graph_snapshot",
            message: format!("failed to create {}: {error}", temporary.display()),
        })?;
    file.write_all(&bytes)
        .map_err(|error| GraphStorageError::OperationFailed {
            operation: "persist_engine_graph_snapshot",
            message: format!("failed to write {}: {error}", temporary.display()),
        })?;
    if require_fsync {
        file.sync_all()
            .map_err(|error| GraphStorageError::OperationFailed {
                operation: "persist_engine_graph_snapshot",
                message: format!("failed to sync {}: {error}", temporary.display()),
            })?;
    }
    drop(file);

    if previous.exists() {
        fs::remove_file(&previous).map_err(|error| GraphStorageError::OperationFailed {
            operation: "persist_engine_graph_snapshot",
            message: format!("failed to remove {}: {error}", previous.display()),
        })?;
    }
    if current.exists() {
        fs::rename(&current, &previous).map_err(|error| GraphStorageError::OperationFailed {
            operation: "persist_engine_graph_snapshot",
            message: format!(
                "failed to retain {} as {}: {error}",
                current.display(),
                previous.display()
            ),
        })?;
    }
    if let Err(error) = fs::rename(&temporary, &current) {
        if previous.exists() && !current.exists() {
            let _ = fs::rename(&previous, &current);
        }
        return Err(GraphStorageError::OperationFailed {
            operation: "persist_engine_graph_snapshot",
            message: format!(
                "failed to promote {} to {}: {error}",
                temporary.display(),
                current.display()
            ),
        });
    }

    if require_fsync {
        sync_directory(&directory)?;
    }
    if previous.exists() {
        fs::remove_file(&previous).map_err(|error| GraphStorageError::OperationFailed {
            operation: "persist_engine_graph_snapshot",
            message: format!("failed to remove {}: {error}", previous.display()),
        })?;
    }
    Ok(())
}

fn snapshot_directory(root: &StorageRoot) -> PathBuf {
    root.path().join(SNAPSHOT_DIRECTORY)
}

fn sync_directory(directory: &Path) -> GraphStorageResult<()> {
    File::open(directory)
        .and_then(|file| file.sync_all())
        .map_err(|error| GraphStorageError::OperationFailed {
            operation: "persist_engine_graph_snapshot",
            message: format!("failed to sync {}: {error}", directory.display()),
        })
}

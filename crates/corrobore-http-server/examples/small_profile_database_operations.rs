// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

use std::{fs, time::Instant};

use graph_core::{Graph, NodeInput, PropertyValue};
use graph_storage::{
    CanonicalEngineStore, CanonicalProjectionRequest, CanonicalStoreOptions, DurableTransactionId,
    GraphId, MigrationRequest, RecordFormat, SnapshotRequest, StorageManifest, StorageTimestamp,
    StorageVersion, create_consistent_snapshot, create_storage_root, migrate_storage,
    open_storage_root, rebuild_derived_indexes, restore_consistent_snapshot,
    rollback_storage_migration,
};
use serde::Serialize;

const RECORDS: usize = 1_000;

#[derive(Serialize)]
struct SmallProfileResult {
    profile: &'static str,
    records: usize,
    snapshot_bytes: u64,
    snapshot_ms: u128,
    restore_ms: u128,
    migration_ms: u128,
    rebuild_ms: u128,
    rollback_ms: u128,
    restored_records: usize,
    migration_parity_verified: bool,
    rollback_boundary: &'static str,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let base = std::env::temp_dir().join(format!(
        "corrobore-small-profile-database-operations-{}",
        std::process::id()
    ));
    if base.exists() {
        fs::remove_dir_all(&base)?;
    }
    fs::create_dir_all(&base)?;
    let source_path = base.join("source");
    let snapshot_path = base.join("snapshot");
    let restored_path = base.join("restored");
    let root = create_storage_root(source_path, manifest())?;
    let mut store = CanonicalEngineStore::open(
        root.clone(),
        CanonicalStoreOptions {
            max_hot_nodes: RECORDS as u64,
            max_hot_relationships: 1,
            max_warm_adjacency_entries: 1,
        },
    )?;
    let mut graph = Graph::new();
    for index in 0..RECORDS {
        graph.create_node(
            NodeInput::new(["Indicator"])
                .with_property("name", PropertyValue::String(format!("indicator-{index}")))
                .with_property(
                    "created_at",
                    PropertyValue::String(format!("2026-07-26T00:{:02}:00Z", index % 60)),
                ),
        )?;
    }
    store.commit_transition(
        &Graph::new(),
        &graph,
        DurableTransactionId::new("tx--small-profile-database-operations")?,
        None,
    )?;

    let started = Instant::now();
    let snapshot = create_consistent_snapshot(
        store.root(),
        &snapshot_path,
        SnapshotRequest {
            created_at: "2026-07-26T03:00:00Z".to_owned(),
            encryption_key_id: Some("benchmark-key".to_owned()),
            retention_hook: Some("benchmark-retention".to_owned()),
        },
    )?;
    let snapshot_ms = started.elapsed().as_millis();

    let started = Instant::now();
    restore_consistent_snapshot(&snapshot_path, &restored_path, Some("benchmark-key"))?;
    let restore_ms = started.elapsed().as_millis();

    let manifest_path = restored_path.join("manifest.json");
    let legacy = fs::read_to_string(&manifest_path)?.replace("\"V1\"", "\"V0\"");
    fs::write(&manifest_path, legacy)?;
    let started = Instant::now();
    let migration = migrate_storage(
        &restored_path,
        MigrationRequest::v0_to_v1("2026-07-26T03:01:00Z"),
        None,
    )?;
    let migration_ms = started.elapsed().as_millis();

    let mut restored = CanonicalEngineStore::open(
        open_storage_root(&restored_path)?,
        CanonicalStoreOptions {
            max_hot_nodes: RECORDS as u64,
            max_hot_relationships: 1,
            max_warm_adjacency_entries: 1,
        },
    )?;
    let started = Instant::now();
    rebuild_derived_indexes(&mut restored, None)?;
    let rebuild_ms = started.elapsed().as_millis();
    let restored_records = restored
        .load_projection(CanonicalProjectionRequest::all_nodes())?
        .list_nodes()?
        .len();
    drop(restored);

    let started = Instant::now();
    rollback_storage_migration(&restored_path)?;
    let rollback_ms = started.elapsed().as_millis();

    println!(
        "{}",
        serde_json::to_string_pretty(&SmallProfileResult {
            profile: "small",
            records: RECORDS,
            snapshot_bytes: snapshot.total_bytes,
            snapshot_ms,
            restore_ms,
            migration_ms,
            rebuild_ms,
            rollback_ms,
            restored_records,
            migration_parity_verified: migration.parity_verified,
            rollback_boundary: "V1 canonical format compatible with V0 manifest",
        })?
    );
    fs::remove_dir_all(base)?;
    Ok(())
}

fn manifest() -> StorageManifest {
    StorageManifest {
        storage_version: StorageVersion::V1,
        graph_id: GraphId {
            value: "graph--small-profile-database-operations".to_owned(),
        },
        created_at: StorageTimestamp {
            value: "2026-07-26T03:00:00Z".to_owned(),
        },
        updated_at: StorageTimestamp {
            value: "2026-07-26T03:00:00Z".to_owned(),
        },
        record_format: RecordFormat::JsonLinesV1,
    }
}

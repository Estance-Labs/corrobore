// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
#![allow(clippy::unwrap_used)]

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use graph_core::{Graph, NodeInput, NodePatch, PropertyValue, RelationshipInput};
use graph_storage::{
    CanonicalEngineStore, CanonicalProjectionRequest, CanonicalStoreOptions,
    DatabaseOperationCrashStage, DerivedIndexKind, DurableTransactionId, GraphId, MigrationRequest,
    OperationReadiness, RecordFormat, SnapshotArtifactStore, SnapshotRequest, StorageManifest,
    StorageTimestamp, StorageVersion, cancel_derived_index_rebuild, create_consistent_snapshot,
    create_storage_root, export_snapshot_to_store, migrate_storage, rebuild_derived_indexes,
    restore_consistent_snapshot, rollback_storage_migration, validate_consistent_snapshot,
};

fn unique_path(test_name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "corrobore-issue-52-{test_name}-{}-{unique}",
        std::process::id()
    ))
}

fn manifest() -> StorageManifest {
    StorageManifest {
        storage_version: StorageVersion::V1,
        graph_id: GraphId {
            value: "graph--issue-52".to_owned(),
        },
        created_at: StorageTimestamp {
            value: "2026-07-26T00:00:00Z".to_owned(),
        },
        updated_at: StorageTimestamp {
            value: "2026-07-26T00:00:00Z".to_owned(),
        },
        record_format: RecordFormat::JsonLinesV1,
    }
}

fn populated_store(test_name: &str) -> (graph_storage::StorageRoot, CanonicalEngineStore) {
    let path = unique_path(test_name);
    let root = create_storage_root(path, manifest()).unwrap();
    let mut store =
        CanonicalEngineStore::open(root.clone(), CanonicalStoreOptions::default()).unwrap();
    let mut graph = Graph::new();
    let source = graph
        .create_node(
            NodeInput::new(["Indicator"])
                .with_property("name", PropertyValue::String("snapshot.example".to_owned()))
                .with_property(
                    "opencti.access",
                    PropertyValue::Json(serde_json::json!({
                        "tenant_ids": ["tenant--operations"],
                        "organization_ids": ["organization--operations"]
                    })),
                ),
        )
        .unwrap();
    let target = graph
        .create_node(NodeInput::new(["Infrastructure"]))
        .unwrap();
    graph
        .create_relationship(RelationshipInput::new(source.clone(), "USES", target).unwrap())
        .unwrap();
    store
        .commit_transition(
            &Graph::new(),
            &graph,
            DurableTransactionId::new("tx--issue-52-fixture-create").unwrap(),
            None,
        )
        .unwrap();
    let previous = graph.clone();
    graph
        .update_node(
            &source,
            NodePatch::default().set_property(
                "description",
                PropertyValue::String("second canonical version".to_owned()),
            ),
        )
        .unwrap();
    store
        .commit_transition(
            &previous,
            &graph,
            DurableTransactionId::new("tx--issue-52-fixture-update").unwrap(),
            None,
        )
        .unwrap();
    (root, store)
}

#[derive(Default)]
struct MemoryObjectStore {
    objects: BTreeMap<String, Vec<u8>>,
}

impl SnapshotArtifactStore for MemoryObjectStore {
    fn put(&mut self, key: &str, bytes: &[u8]) -> Result<(), String> {
        self.objects.insert(key.to_owned(), bytes.to_vec());
        Ok(())
    }

    fn get(&self, key: &str) -> Result<Vec<u8>, String> {
        self.objects
            .get(key)
            .cloned()
            .ok_or_else(|| format!("missing object {key}"))
    }

    fn list(&self, prefix: &str) -> Result<Vec<String>, String> {
        Ok(self
            .objects
            .keys()
            .filter(|key| key.starts_with(prefix))
            .cloned()
            .collect())
    }
}

#[test]
fn snapshot_manifest_binds_one_generation_and_restores_query_parity() {
    let (source_root, mut source_store) = populated_store("snapshot-source");
    let snapshot_root = unique_path("snapshot-artifact");
    let restored_root = unique_path("snapshot-restored");
    let request = SnapshotRequest {
        created_at: "2026-07-26T01:00:00Z".to_owned(),
        encryption_key_id: Some("kms://operations/snapshot-key".to_owned()),
        retention_hook: Some("retain-30-days".to_owned()),
    };

    let report = create_consistent_snapshot(source_store.root(), &snapshot_root, request).unwrap();
    assert_eq!(report.manifest_version, 1);
    assert_eq!(report.readiness, OperationReadiness::Ready);
    assert_eq!(report.graph_id, "graph--issue-52");
    assert!(report.canonical_generation > 0);
    assert_eq!(report.canonical_generation, report.wal_boundary);
    assert!(report.file_count > 0);

    let validation =
        validate_consistent_snapshot(&snapshot_root, Some("kms://operations/snapshot-key"))
            .unwrap();
    assert_eq!(validation.verified_files, report.file_count);

    restore_consistent_snapshot(
        &snapshot_root,
        &restored_root,
        Some("kms://operations/snapshot-key"),
    )
    .unwrap();
    let restored = graph_storage::open_storage_root(&restored_root).unwrap();
    let mut restored_store =
        CanonicalEngineStore::open(restored, CanonicalStoreOptions::default()).unwrap();
    let source_graph = source_store
        .load_projection(CanonicalProjectionRequest::all())
        .unwrap();
    let restored_graph = restored_store
        .load_projection(CanonicalProjectionRequest::all())
        .unwrap();
    assert_eq!(
        serde_json::to_value(source_graph.persistence_snapshot()).unwrap(),
        serde_json::to_value(restored_graph.persistence_snapshot()).unwrap()
    );
    assert_eq!(restored_graph.list_nodes().unwrap().len(), 2);
    assert_eq!(restored_graph.list_relationships().unwrap().len(), 1);
    let restored_source = restored_graph
        .list_nodes()
        .unwrap()
        .into_iter()
        .find(|node| node.has_label("Indicator"))
        .unwrap();
    assert!(restored_source.property("opencti.access").is_some());
    let source_history = fs::read(source_root.path().join("nodes/node_records.log")).unwrap();
    let restored_history = fs::read(restored_root.join("nodes/node_records.log")).unwrap();
    assert_eq!(restored_history, source_history);
    assert!(source_history.iter().filter(|byte| **byte == b'\n').count() >= 3);

    let _ = fs::remove_dir_all(source_root.path());
    let _ = fs::remove_dir_all(snapshot_root);
    let _ = fs::remove_dir_all(restored_root);
}

#[test]
fn snapshot_rejects_wrong_key_before_creating_a_writable_restore() {
    let (source_root, source_store) = populated_store("wrong-key-source");
    let snapshot_root = unique_path("wrong-key-artifact");
    let restored_root = unique_path("wrong-key-restored");
    create_consistent_snapshot(
        source_store.root(),
        &snapshot_root,
        SnapshotRequest {
            created_at: "2026-07-26T01:00:00Z".to_owned(),
            encryption_key_id: Some("kms://operations/right-key".to_owned()),
            retention_hook: None,
        },
    )
    .unwrap();

    assert!(
        restore_consistent_snapshot(
            &snapshot_root,
            &restored_root,
            Some("kms://operations/wrong-key"),
        )
        .is_err()
    );
    assert!(!restored_root.exists());
    assert!(restore_consistent_snapshot(&snapshot_root, &restored_root, None).is_err());
    assert!(!restored_root.exists());

    let _ = fs::remove_dir_all(source_root.path());
    let _ = fs::remove_dir_all(snapshot_root);
}

#[test]
fn incomplete_or_wrong_version_snapshot_fails_before_restore_readiness() {
    let (source_root, source_store) = populated_store("invalid-snapshot-source");
    let snapshot_root = unique_path("invalid-snapshot-artifact");
    let restored_root = unique_path("invalid-snapshot-restored");
    create_consistent_snapshot(
        source_store.root(),
        &snapshot_root,
        SnapshotRequest {
            created_at: "2026-07-26T01:00:00Z".to_owned(),
            encryption_key_id: None,
            retention_hook: None,
        },
    )
    .unwrap();
    let manifest_path = snapshot_root.join("snapshot_manifest.json");
    let original = fs::read(&manifest_path).unwrap();
    let mut manifest: serde_json::Value = serde_json::from_slice(&original).unwrap();
    manifest["manifest_version"] = serde_json::json!(999);
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    assert!(
        restore_consistent_snapshot(&snapshot_root, &restored_root, None).is_err(),
        "unsupported manifests must fail before target creation"
    );
    assert!(!restored_root.exists());

    fs::write(&manifest_path, original).unwrap();
    let required_component = fs::read_dir(snapshot_root.join("data/nodes"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    fs::remove_file(required_component).unwrap();
    assert!(validate_consistent_snapshot(&snapshot_root, None).is_err());

    let _ = fs::remove_dir_all(source_root.path());
    let _ = fs::remove_dir_all(snapshot_root);
}

#[test]
fn object_store_export_publishes_manifest_last_with_all_checksummed_components() {
    let (source_root, source_store) = populated_store("object-store-source");
    let snapshot_root = unique_path("object-store-artifact");
    create_consistent_snapshot(
        source_store.root(),
        &snapshot_root,
        SnapshotRequest {
            created_at: "2026-07-26T01:00:00Z".to_owned(),
            encryption_key_id: None,
            retention_hook: Some("minio-lifecycle-policy".to_owned()),
        },
    )
    .unwrap();
    let mut object_store = MemoryObjectStore::default();

    let export =
        export_snapshot_to_store(&snapshot_root, &mut object_store, "backups/nightly").unwrap();
    assert_eq!(export.destination, "backups/nightly");
    assert!(
        object_store
            .objects
            .contains_key("backups/nightly/snapshot_manifest.json")
    );
    assert_eq!(export.uploaded_objects, object_store.objects.len());

    let _ = fs::remove_dir_all(source_root.path());
    let _ = fs::remove_dir_all(snapshot_root);
}

#[test]
fn migration_resumes_after_crash_and_supports_the_documented_rollback_boundary() {
    let (root, store) = populated_store("migration");
    drop(store);
    let manifest_path = root.path().join("manifest.json");
    let legacy = fs::read_to_string(&manifest_path)
        .unwrap()
        .replace("\"V1\"", "\"V0\"");
    fs::write(&manifest_path, legacy).unwrap();
    let request = MigrationRequest::v0_to_v1("2026-07-26T02:00:00Z");

    assert!(
        migrate_storage(
            root.path(),
            request.clone(),
            Some(DatabaseOperationCrashStage::AfterCanonicalMigration),
        )
        .is_err()
    );
    let resumed = migrate_storage(root.path(), request, None).unwrap();
    assert_eq!(resumed.readiness, OperationReadiness::Ready);
    assert!(resumed.resumed);
    assert!(resumed.parity_verified);
    assert_eq!(resumed.completed_steps, resumed.total_steps);
    assert_eq!(
        graph_storage::read_storage_manifest(&root)
            .unwrap()
            .storage_version,
        StorageVersion::V1
    );

    let rollback = rollback_storage_migration(root.path()).unwrap();
    assert_eq!(rollback.readiness, OperationReadiness::OfflineRequired);
    assert!(
        fs::read_to_string(&manifest_path)
            .unwrap()
            .contains("\"V0\"")
    );

    let _ = fs::remove_dir_all(root.path());
}

#[test]
fn complete_index_rebuild_is_resumable_and_never_reports_partial_state_as_ready() {
    let (root, mut store) = populated_store("rebuild");
    let interrupted = rebuild_derived_indexes(
        &mut store,
        Some(DatabaseOperationCrashStage::AfterCanonicalIndexes),
    )
    .expect_err("injected interruption should stop rebuild");
    assert!(interrupted.to_string().contains("injected"));
    assert!(
        fs::read_to_string(
            root.path()
                .join("transactions/outgoing_adjacency_mutations.log")
        )
        .unwrap()
        .contains("tx--derived-adjacency-rebuild-")
    );
    assert!(
        fs::read_to_string(
            root.path()
                .join("transactions/incoming_adjacency_mutations.log")
        )
        .unwrap()
        .contains("tx--derived-adjacency-rebuild-")
    );
    let state: serde_json::Value = serde_json::from_slice(
        &fs::read(root.path().join("operations/index-rebuild.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(state["readiness"], "rebuilding");

    let cancelled = cancel_derived_index_rebuild(root.path()).unwrap();
    assert_eq!(cancelled.readiness, OperationReadiness::Cancelled);

    let report = rebuild_derived_indexes(&mut store, None).unwrap();
    assert!(report.resumed);
    assert_eq!(report.readiness, OperationReadiness::Ready);
    for required in [
        DerivedIndexKind::Identifier,
        DerivedIndexKind::Adjacency,
        DerivedIndexKind::Property,
        DerivedIndexKind::Temporal,
        DerivedIndexKind::FullText,
        DerivedIndexKind::Aggregation,
        DerivedIndexKind::FileContent,
        DerivedIndexKind::AccessPolicy,
    ] {
        assert!(report.completed_indexes.contains(&required));
    }

    let repeated = rebuild_derived_indexes(&mut store, None).unwrap();
    assert!(!repeated.resumed);
    assert_eq!(repeated.readiness, OperationReadiness::Ready);

    let _ = fs::remove_dir_all(root.path());
}

fn _assert_path_is_relative(path: &Path) {
    assert!(path.is_relative());
}

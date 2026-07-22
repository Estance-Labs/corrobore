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
use std::path::{Path, PathBuf};

use crate::{
    CatalogRebuildOptions, CatalogRebuildRecord, CatalogRebuildReport, FileBackedGraphStore,
    GraphAdjacencyStorage, GraphCatalog, GraphStorageError, GraphStorageResult, StorageManifest,
    StorageRoot, StorageSegment, create_file_backed_graph_store, persist_graph_catalog_metadata,
    read_incoming_adjacency_log_for_catalog_rebuild,
    read_outgoing_adjacency_log_for_catalog_rebuild, read_persisted_graph_catalog_metadata,
    read_storage_manifest, rebuild_catalog_from_append_logs, validate_storage_manifest,
    write_incoming_adjacency_by_node_id, write_outgoing_adjacency_by_node_id,
};

/// Policy used when an existing graph store is reopened.
///
///
/// - Make the reopen path explicit instead of overloading raw storage-root open.
/// - Separate manifest validation from catalog recovery and pager construction.
/// - Allow future callers to choose whether a persisted catalog should be loaded,
///   rebuilt from append logs, or only validated without constructing a full store.
/// - Keep snapshot restore and WAL replay outside this issue.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GraphStoreOpenMode {
    /// Validate the root and manifest only, without constructing a file-backed
    /// store or loading catalog-owned metadata.
    ValidateOnly,

    /// Rebuild catalog-owned metadata from append-only node, relationship, and
    /// adjacency logs before constructing the file-backed store handle.
    #[default]
    RebuildCatalogFromAppendLogs,

    /// Load a future persisted catalog when available, falling back to rebuild
    /// only when the later implementation explicitly supports that policy.
    LoadCatalogWhenAvailable,
}

/// Options controlling graph store reopen and recovery behavior.
///
///
/// - Preserve a single API boundary for reopening a persisted local store.
/// - Make required storage components explicit so missing logs can become typed
///   recovery failures instead of implicit empty catalogs.
/// - Reuse the catalog rebuild options without tying reopen to a full
///   graph deserialization path.
/// - Keep snapshot and WAL recovery switches absent until dedicated issues own
///   those behaviors.
///
///
/// Future implementations should validate the manifest first, verify required
/// record-log components, load or rebuild catalog metadata, initialize adjacency
/// storage, and then create a file-backed graph store handle without loading all
/// node or relationship payloads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphStoreOpenOptions {
    /// Store open mode.
    pub mode: GraphStoreOpenMode,
    /// Catalog rebuild options.
    pub catalog_rebuild_options: CatalogRebuildOptions,
    /// Require node record log.
    pub require_node_record_log: bool,
    /// Require relationship record log.
    pub require_relationship_record_log: bool,
    /// Require outgoing adjacency log.
    pub require_outgoing_adjacency_log: bool,
    /// Require incoming adjacency log.
    pub require_incoming_adjacency_log: bool,
}

impl Default for GraphStoreOpenOptions {
    fn default() -> Self {
        Self {
            mode: GraphStoreOpenMode::default(),
            // Catalog rebuild options.
            catalog_rebuild_options: CatalogRebuildOptions::default(),
            // Require node record log.
            require_node_record_log: true,
            // Require relationship record log.
            require_relationship_record_log: true,
            // Require outgoing adjacency log.
            require_outgoing_adjacency_log: true,
            // Require incoming adjacency log.
            require_incoming_adjacency_log: true,
        }
    }
}

/// Source used to recover catalog metadata during reopen.
///
///
/// - Keep diagnostics explicit about whether metadata came from a future persisted
///   catalog or from append-log rebuild.
/// - Let later acceptance tests assert deterministic recovery paths without
///   inspecting private storage internals.
/// - Preserve the distinction between catalog metadata and graph payload loading.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphStoreCatalogRecoverySource {
    /// Persisted catalog.
    PersistedCatalog,
    /// Rebuilt from append logs.
    RebuiltFromAppendLogs,
}

/// Catalog recovery output for reopening a store.
///
///
/// - Return the recovered catalog with the same audit report shape introduced for
///   rebuild.
/// - Keep catalog recovery as a reusable step for the public reopen function.
/// - Avoid returning loaded node, relationship, or adjacency payloads here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphStoreCatalogRecoveryOutcome {
    /// Catalog.
    pub catalog: GraphCatalog,
    /// Source.
    pub source: GraphStoreCatalogRecoverySource,
    /// Report.
    pub report: CatalogRebuildReport,
}

/// Diagnostic report produced by graph store reopen.
///
///
/// - Capture the high-level reopen/recovery steps without exposing raw record
///   bytes or filesystem implementation details.
/// - Make manifest validation, component checks, catalog recovery, and adjacency
///   initialization auditable by later phase 4 acceptance tests.
/// - Keep this report separate from pager page-in diagnostics.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GraphStoreRecoveryReport {
    /// Manifest validated.
    pub manifest_validated: bool,
    /// Required components validated.
    pub required_components_validated: bool,
    /// Catalog recovered.
    pub catalog_recovered: bool,
    /// Adjacency storage recovered.
    pub adjacency_storage_recovered: bool,
    /// Catalog rebuild report.
    pub catalog_rebuild_report: Option<CatalogRebuildReport>,
    /// Warnings.
    pub warnings: Vec<String>,
}

/// Successful graph store reopen output.
///
///
/// - Return the reopened file-backed store handle together with validated manifest
///   metadata and recovery diagnostics.
/// - Keep the reopened store compatible with `FileBackedGraphPager` construction.
/// - Ensure reopen returns a lightweight handle, not a fully deserialized graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphStoreOpenOutcome {
    /// Root.
    pub root: StorageRoot,
    /// Manifest.
    pub manifest: StorageManifest,
    /// Store.
    pub store: FileBackedGraphStore,
    /// Recovery report.
    pub recovery_report: GraphStoreRecoveryReport,
}

/// Open an existing file-backed graph store from a local storage root.
///
///
/// - Provide the explicit entry point for graph store reopen.
/// - Validate the manifest before catalog or adjacency recovery starts.
/// - Recover catalog-owned lookup metadata deterministically by loading or
///   rebuilding metadata from persisted storage components.
/// - Recover node ID lookups, relationship ID lookups, outgoing adjacency, and
///   incoming adjacency as metadata paths available to the file-backed pager.
/// - Avoid full graph deserialization, snapshot restore, and WAL replay.
///
///
///   1. Reject a missing storage root or manifest with explicit storage errors.
/// 2. Reject corrupted manifests and unsupported storage versions before reading
///    record logs.
/// 3. Detect required missing or corrupted record logs during recovery.
/// 4. Rebuild or load catalog metadata needed by ID lookups and adjacency lookup.
/// 5. Return a `FileBackedGraphStore` ready for pager construction.
///
/// # Errors
///
///
/// Missing roots, missing manifests, corrupted manifests, unsupported storage
/// versions, missing required logs, corrupted record logs, duplicate latest-record
/// conflicts, and incomplete recovery state must return `GraphStorageError` values.
pub fn open_existing_file_backed_graph_store(
    path: impl Into<PathBuf>,
    options: GraphStoreOpenOptions,
) -> GraphStorageResult<GraphStoreOpenOutcome> {
    let path = path.into();
    if !path.is_dir() {
        return Err(GraphStorageError::StorageRootNotFound { path });
    }

    let root = StorageRoot { path };
    let manifest = validate_graph_store_reopen_manifest(&root)?;
    let mut recovery_report = GraphStoreRecoveryReport {
        manifest_validated: true,
        ..GraphStoreRecoveryReport::default()
    };

    let (catalog, catalog_rebuild_report) = match options.mode {
        GraphStoreOpenMode::ValidateOnly => (GraphCatalog::default(), None),
        GraphStoreOpenMode::RebuildCatalogFromAppendLogs => {
            validate_required_recovery_components(&root, &options)?;
            recovery_report.required_components_validated = true;
            let recovery = recover_graph_store_catalog(&root, &options)?;
            recovery_report.catalog_recovered = true;
            (recovery.catalog, Some(recovery.report))
        }
        GraphStoreOpenMode::LoadCatalogWhenAvailable => {
            let recovery = recover_graph_store_catalog(&root, &options)?;
            recovery_report.catalog_recovered = true;
            if recovery.source == GraphStoreCatalogRecoverySource::RebuiltFromAppendLogs {
                recovery_report.required_components_validated = true;
                recovery_report.warnings.push(
                    "persisted catalog metadata missing; rebuilt catalog from append logs"
                        .to_owned(),
                );
                (recovery.catalog, Some(recovery.report))
            } else {
                (recovery.catalog, None)
            }
        }
    };

    recovery_report.catalog_rebuild_report = catalog_rebuild_report;

    let (catalog, adjacency_storage) =
        recover_graph_store_adjacency_storage_with_catalog(&root, catalog)?;
    recovery_report.adjacency_storage_recovered = true;

    let store = build_recovered_file_backed_graph_store(root.clone(), catalog, adjacency_storage)?;

    Ok(GraphStoreOpenOutcome {
        root,
        manifest,
        store,
        recovery_report,
    })
}

/// Validate and return the manifest used by graph store reopen.
///
///
/// - Keep manifest validation as the first recoverable reopen step.
/// - Reuse the storage-root manifest contract while giving a named
///   boundary for reopen-specific diagnostics.
/// - Prevent catalog recovery from running against incompatible storage versions.
///
///
///   Future implementations should read the manifest from `root`, validate required fields,
///   reject unsupported storage versions and record formats, and return the trusted
///   manifest metadata.
pub fn validate_graph_store_reopen_manifest(
    root: &StorageRoot,
) -> GraphStorageResult<StorageManifest> {
    let manifest = read_storage_manifest(root)?;
    validate_storage_manifest(&manifest)?;
    Ok(manifest)
}

/// Validate that storage components required by the chosen reopen policy exist.
///
///
/// - Make missing node, relationship, outgoing adjacency, and incoming adjacency
///   logs explicit before catalog recovery starts.
/// - Keep required-component checks policy-driven instead of hard-coding one
///   layout into the public reopen function.
/// - Preserve the issue boundary that snapshot files and WAL files are not part of
///   this recovery pass.
///
///
/// Future implementations should inspect only the components requested by
/// `GraphStoreOpenOptions` and return typed missing-component errors instead of
/// silently accepting partial graph metadata.
pub fn validate_required_recovery_components(
    root: &StorageRoot,
    options: &GraphStoreOpenOptions,
) -> GraphStorageResult<()> {
    if !root.path().is_dir() {
        return Err(GraphStorageError::StorageRootNotFound {
            path: root.path().to_path_buf(),
        });
    }

    validate_required_component(
        root,
        options.require_node_record_log,
        StorageSegment::NodeRecords,
        node_record_log_path,
    )?;
    validate_required_component(
        root,
        options.require_relationship_record_log,
        StorageSegment::RelationshipRecords,
        relationship_record_log_path,
    )?;
    validate_required_component(
        root,
        options.require_outgoing_adjacency_log,
        StorageSegment::OutgoingAdjacency,
        outgoing_adjacency_log_path,
    )?;
    validate_required_component(
        root,
        options.require_incoming_adjacency_log,
        StorageSegment::IncomingAdjacency,
        incoming_adjacency_log_path,
    )?;

    Ok(())
}

/// Recover catalog metadata for an existing graph store.
///
///
/// - Load or rebuild the catalog needed by node lookup, relationship lookup,
///   label/type metadata, and adjacency storage references.
/// - Keep recovery at metadata level only: no full node payload, relationship
///   payload, or adjacency payload loading belongs here.
/// - Reuse append-log rebuild as the deterministic fallback for phase 3.
///
///
///   Future implementations should apply `GraphStoreOpenOptions::mode`, call the issue 58
///   rebuild path when needed, and return a catalog recovery outcome with audit
///   diagnostics.
pub fn recover_graph_store_catalog(
    root: &StorageRoot,
    options: &GraphStoreOpenOptions,
) -> GraphStorageResult<GraphStoreCatalogRecoveryOutcome> {
    match options.mode {
        GraphStoreOpenMode::ValidateOnly => Ok(GraphStoreCatalogRecoveryOutcome {
            catalog: GraphCatalog::default(),
            source: GraphStoreCatalogRecoverySource::PersistedCatalog,
            report: CatalogRebuildReport::default(),
        }),
        GraphStoreOpenMode::RebuildCatalogFromAppendLogs => {
            let outcome =
                rebuild_catalog_from_append_logs(root, options.catalog_rebuild_options.clone())?;
            persist_graph_catalog_metadata(root, &outcome.catalog)?;
            Ok(GraphStoreCatalogRecoveryOutcome {
                catalog: outcome.catalog,
                source: GraphStoreCatalogRecoverySource::RebuiltFromAppendLogs,
                report: outcome.report,
            })
        }
        GraphStoreOpenMode::LoadCatalogWhenAvailable => {
            if let Some(catalog) = read_persisted_graph_catalog_metadata(root)? {
                return Ok(GraphStoreCatalogRecoveryOutcome {
                    catalog,
                    source: GraphStoreCatalogRecoverySource::PersistedCatalog,
                    report: CatalogRebuildReport::default(),
                });
            }

            validate_required_recovery_components(root, options)?;
            let outcome =
                rebuild_catalog_from_append_logs(root, options.catalog_rebuild_options.clone())?;
            persist_graph_catalog_metadata(root, &outcome.catalog)?;
            Ok(GraphStoreCatalogRecoveryOutcome {
                catalog: outcome.catalog,
                source: GraphStoreCatalogRecoverySource::RebuiltFromAppendLogs,
                report: outcome.report,
            })
        }
    }
}

/// Recover the adjacency storage handle used by a reopened file-backed store.
///
///
/// - Provide a named boundary for making outgoing and incoming adjacency available
///   after reopen.
/// - Keep adjacency recovery separate from catalog rebuild and pager payload reads.
/// - Allow later implementations to initialize file-backed adjacency storage from
///   persisted adjacency logs or cataloged adjacency references without loading the
///   whole graph.
///
///
/// Future implementations should make adjacency reads deterministic after reopen and report
/// corrupted or missing adjacency components through `GraphStorageError`.
pub fn recover_graph_store_adjacency_storage(
    root: &StorageRoot,
    catalog: &GraphCatalog,
) -> GraphStorageResult<GraphAdjacencyStorage> {
    recover_graph_store_adjacency_storage_with_catalog(root, catalog.clone())
        .map(|(_, storage)| storage)
}

/// Build the file-backed graph store handle once recovery metadata is available.
///
///
/// - Centralize the final assembly step for reopened stores.
/// - Require the caller to provide an already validated root, recovered catalog,
///   and recovered adjacency storage handle.
/// - Keep this assembly lightweight so pager construction remains lazy and does
///   not deserialize full graph payloads.
///
///
/// Future implementations should delegate to the file-backed store constructor and attach
/// recovery diagnostics without repeating manifest or catalog work.
pub fn build_recovered_file_backed_graph_store(
    root: StorageRoot,
    catalog: GraphCatalog,
    adjacency_storage: GraphAdjacencyStorage,
) -> GraphStorageResult<FileBackedGraphStore> {
    create_file_backed_graph_store(root, catalog, adjacency_storage)
}

fn recover_graph_store_adjacency_storage_with_catalog(
    root: &StorageRoot,
    mut catalog: GraphCatalog,
) -> GraphStorageResult<(GraphCatalog, GraphAdjacencyStorage)> {
    let mut storage = GraphAdjacencyStorage::default();

    if outgoing_adjacency_log_path(root).is_file() {
        for record in read_outgoing_adjacency_log_for_catalog_rebuild(root)? {
            if let CatalogRebuildRecord::OutgoingAdjacency { record, .. } = record {
                let owner_node_id = record.owner_node_id.clone();
                write_outgoing_adjacency_by_node_id(
                    &mut storage,
                    &mut catalog,
                    &owner_node_id,
                    record.entries,
                )?;
            }
        }
    }

    if incoming_adjacency_log_path(root).is_file() {
        for record in read_incoming_adjacency_log_for_catalog_rebuild(root)? {
            if let CatalogRebuildRecord::IncomingAdjacency { record, .. } = record {
                let owner_node_id = record.owner_node_id.clone();
                write_incoming_adjacency_by_node_id(
                    &mut storage,
                    &mut catalog,
                    &owner_node_id,
                    record.entries,
                )?;
            }
        }
    }

    Ok((catalog, storage))
}

fn validate_required_component(
    root: &StorageRoot,
    required: bool,
    segment: StorageSegment,
    path: impl Fn(&StorageRoot) -> PathBuf,
) -> GraphStorageResult<()> {
    if !required {
        return Ok(());
    }

    let path = path(root);
    if path.is_file() {
        Ok(())
    } else {
        Err(GraphStorageError::CatalogRebuildSourceMissing { segment, path })
    }
}

fn node_record_log_path(root: &StorageRoot) -> PathBuf {
    root.path().join("nodes").join("node_records.log")
}

fn relationship_record_log_path(root: &StorageRoot) -> PathBuf {
    root.path()
        .join("relationships")
        .join("relationship_records.log")
}

fn outgoing_adjacency_log_path(root: &StorageRoot) -> PathBuf {
    root.path().join("adjacency").join("outgoing_adjacency.log")
}

fn incoming_adjacency_log_path(root: &StorageRoot) -> PathBuf {
    root.path().join("adjacency").join("incoming_adjacency.log")
}

#[allow(dead_code)]
fn path_display(path: &Path) -> String {
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_path(test_name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "intelligence_graph_engine_store_reopen_unit_{test_name}_{}_{}",
            std::process::id(),
            unique
        ))
    }

    fn manifest() -> StorageManifest {
        StorageManifest {
            // Storage version.
            storage_version: crate::StorageVersion::V1,
            // Graph id.
            graph_id: crate::GraphId {
                // Value.
                value: "graph--store-reopen-unit-tests".to_owned(),
            },
            // Created at.
            created_at: crate::StorageTimestamp {
                // Value.
                value: "2026-07-06T00:00:00Z".to_owned(),
            },
            // Updated at.
            updated_at: crate::StorageTimestamp {
                // Value.
                value: "2026-07-06T00:00:00Z".to_owned(),
            },
            // Record format.
            record_format: crate::RecordFormat::JsonLinesV1,
        }
    }

    fn root(test_name: &str) -> StorageRoot {
        let path = unique_temp_path(test_name);
        let _ = fs::remove_dir_all(&path);
        crate::create_storage_root(path, manifest())
            .expect("storage root fixture should be created")
    }

    #[test]
    fn validate_required_component_skips_check_when_component_not_required() {
        let root = root("optional_component");

        let result = validate_required_component(
            &root,
            false,
            StorageSegment::NodeRecords,
            node_record_log_path,
        );

        assert!(result.is_ok());
        let _ = fs::remove_dir_all(root.path());
    }

    #[test]
    fn validate_required_component_accepts_existing_component_file() {
        let root = root("existing_component");
        let path = node_record_log_path(&root);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("parent directories should be created");
        }
        fs::write(&path, b"{}\n").expect("component fixture should be written");

        let result = validate_required_component(
            &root,
            true,
            StorageSegment::NodeRecords,
            node_record_log_path,
        );

        assert!(result.is_ok());
        let _ = fs::remove_dir_all(root.path());
    }

    #[test]
    fn validate_required_component_reports_missing_required_file() {
        let root = root("missing_required_component");

        let error = validate_required_component(
            &root,
            true,
            StorageSegment::RelationshipRecords,
            relationship_record_log_path,
        )
        .expect_err("required component should fail when log is missing");

        assert!(matches!(
        error,
        GraphStorageError::CatalogRebuildSourceMissing { segment, .. }
        if segment == StorageSegment::RelationshipRecords
        ));
        let _ = fs::remove_dir_all(root.path());
    }

    #[test]
    fn segment_path_helpers_build_expected_layout_paths() {
        let root = root("segment_paths");

        assert!(node_record_log_path(&root).ends_with("nodes/node_records.log"));
        assert!(
            relationship_record_log_path(&root).ends_with("relationships/relationship_records.log")
        );
        assert!(outgoing_adjacency_log_path(&root).ends_with("adjacency/outgoing_adjacency.log"));
        assert!(incoming_adjacency_log_path(&root).ends_with("adjacency/incoming_adjacency.log"));

        let _ = fs::remove_dir_all(root.path());
    }

    #[test]
    fn path_display_returns_lossless_display_string() {
        let path = PathBuf::from("segment/path/manifest.json");

        let displayed = path_display(&path);

        assert!(displayed.ends_with("segment/path/manifest.json"));
    }

    #[test]
    fn load_catalog_mode_rebuilds_when_persisted_catalog_not_available() {
        let root = root("load_catalog_mode");
        let options = GraphStoreOpenOptions {
            mode: GraphStoreOpenMode::LoadCatalogWhenAvailable,
            catalog_rebuild_options: CatalogRebuildOptions {
                include_node_records: false,
                include_relationship_records: false,
                include_outgoing_adjacency: false,
                include_incoming_adjacency: false,
                fail_fast: true,
            },
            require_node_record_log: false,
            require_relationship_record_log: false,
            require_outgoing_adjacency_log: false,
            require_incoming_adjacency_log: false,
        };

        let outcome = recover_graph_store_catalog(&root, &options)
            .expect("load-catalog mode should rebuild metadata when no persisted catalog exists");

        assert_eq!(
            outcome.source,
            GraphStoreCatalogRecoverySource::RebuiltFromAppendLogs
        );
        assert_eq!(outcome.catalog, GraphCatalog::default());
        assert_eq!(outcome.report, CatalogRebuildReport::default());
        let _ = fs::remove_dir_all(root.path());
    }
}

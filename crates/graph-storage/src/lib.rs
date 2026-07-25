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
#![allow(clippy::filter_map_bool_then)]
#![warn(missing_docs)]

//! Persistent graph storage boundary for Corrobore.
//!
//! This crate owns on-disk persistence for the graph engine, keeping it cleanly
//! separated from the in-memory graph core. The graph core focuses on typed
//! records, lifecycle operations, working-set contracts, and pager traits, while
//! this crate handles local storage roots, manifests, file-backed persistence,
//! catalog indexing, catalog rebuild, and pager adapters.
//!
//! # Responsibilities
//!
//! - **Manifest and storage root** — create, validate, and reopen local storage
//!   directories with versioned manifests.
//! - **Records and envelopes** — define stable storage segments, byte-addressable
//!   references, persisted record IDs, kinds, checksums, and envelopes.
//! - **Record codec** — deterministic encode/decode boundary for persisted
//!   envelopes, with checksum calculation and validation.
//! - **Append-only record logs** — append-only node and relationship logs with
//!   deterministic offset/length addressing and version preservation.
//! - **Storage catalog** — latest-record lookup indexes for nodes and
//!   relationships, keyed by stable identifiers.
//! - **Catalog metadata indexes** — label and relationship-type indexes for
//!   lightweight metadata lookup without full payload loading.
//! - **Persistent adjacency** — outgoing and incoming adjacency storage records
//!   with catalog integration.
//! - **Catalog rebuild** — reconstruct catalog state from persisted append-only
//!   logs, including latest-record maps, label indexes, relationship-type
//!   indexes, and adjacency entries.
//! - **File-backed graph pager** — pager adapter mapping persistent storage
//!   into graph-core pager contracts.
//! - **Store reopen and recovery** — reopen existing storage roots with manifest
//!   validation, log integrity checks, catalog recovery, and adjacency recovery.

mod adjacency_storage;
mod atomic_mutation;
mod backup_restore;
mod canonical_engine_store;
mod catalog;
mod catalog_indexes;
mod catalog_metadata;
mod catalog_rebuild;
mod codec;
mod engine_snapshot;
mod error;
mod graph_pager;
mod log;
mod manifest;
mod record;
mod root;
mod store_reopen;
mod transaction_log;

pub use adjacency_storage::{
    AdjacencyStorageCatalogEntry, AdjacencyStorageLookupMode, GraphAdjacencyStorage,
    PersistedAdjacencyEntry, PersistedAdjacencyRecord, index_incoming_adjacency_storage_ref,
    index_outgoing_adjacency_storage_ref, read_incoming_adjacency_by_node_id,
    read_outgoing_adjacency_by_node_id, resolve_incoming_adjacency_storage_ref,
    resolve_outgoing_adjacency_storage_ref, restore_persisted_adjacency_records,
    snapshot_persisted_adjacency_records, write_incoming_adjacency_by_node_id,
    write_outgoing_adjacency_by_node_id,
};
pub use atomic_mutation::{
    AtomicPersistentCompactionOutcome, AtomicPersistentCompactionRequest,
    AtomicPersistentCompactionScope, AtomicPersistentMutationAdjacencyRecord,
    AtomicPersistentMutationBatch, AtomicPersistentMutationNodeRecord,
    AtomicPersistentMutationOutcome, AtomicPersistentMutationRelationshipRecord,
    AtomicPersistentRecoveryOutcome, AtomicPersistentRecoveryPath, AtomicPersistentRecoveryReport,
    AtomicPersistentRuntimeState, MutationCrashStage, apply_atomic_persistent_mutation_batch,
    compact_atomic_persistent_segments, recover_atomic_persistent_runtime_state,
    recover_atomic_persistent_runtime_state_with_report,
};
pub use backup_restore::{
    AtomicPersistentBackupOutcome, AtomicPersistentBackupValidationReport,
    AtomicPersistentRestoreOutcome, create_atomic_persistent_backup,
    restore_atomic_persistent_backup, validate_atomic_persistent_backup,
};
pub use canonical_engine_store::{
    CanonicalAdjacencyProjection, CanonicalEngineStore, CanonicalProjectionRequest,
    CanonicalProjectionStats, CanonicalPropertyFilter, CanonicalPropertyOperator,
    CanonicalStartupReport, CanonicalStoreOptions, CanonicalStoreStats,
};
pub use catalog::{
    GraphCatalog, HistoricalRecordCatalogEntry, LatestRecordCatalogEntry,
    check_duplicate_latest_record_conflict, create_empty_graph_catalog, index_appended_node_record,
    index_appended_relationship_record, resolve_latest_node_storage_ref,
    resolve_latest_relationship_storage_ref,
};
pub use catalog_indexes::{
    CatalogIndexLookupMode, GraphCatalogIndexes, LabelIndexCatalogEntry, LabelIndexNodeMetadata,
    NodeLabel, NodeReadIndexDocument, NodeReadIndexValue, RelationshipTypeIndexCatalogEntry,
    RelationshipTypeIndexRelationshipMetadata, index_node_labels, index_relationship_type,
    replace_node_read_indexes, replace_relationship_access_index, resolve_identifier_index_entries,
    resolve_label_index_entries, resolve_node_ids_by_label, resolve_property_index_entries,
    resolve_property_presence_entries, resolve_relationship_ids_by_type,
    resolve_relationship_type_index_entries,
};
pub use catalog_metadata::{persist_graph_catalog_metadata, read_persisted_graph_catalog_metadata};
pub use catalog_rebuild::{
    CatalogRebuildOptions, CatalogRebuildOutcome, CatalogRebuildRecord, CatalogRebuildRecordCounts,
    CatalogRebuildReport, catalog_rebuild_adjacency_direction_label,
    detect_corrupted_catalog_rebuild_records, detect_duplicate_latest_record_conflicts_for_rebuild,
    read_incoming_adjacency_log_for_catalog_rebuild, read_node_record_log_for_catalog_rebuild,
    read_outgoing_adjacency_log_for_catalog_rebuild,
    read_relationship_record_log_for_catalog_rebuild, rebuild_catalog_from_append_logs,
    reconstruct_adjacency_catalog_entries_from_rebuild_records,
    reconstruct_catalog_from_rebuild_records, reconstruct_label_indexes_from_rebuild_records,
    reconstruct_latest_node_records_from_rebuild_records,
    reconstruct_latest_relationship_records_from_rebuild_records,
    reconstruct_relationship_type_indexes_from_rebuild_records,
};
pub use codec::{
    EncodedRecord, JsonLinesRecordCodec, RecordCodec, calculate_encoded_record_checksum,
    decode_persisted_record_envelope, encode_persisted_record_envelope,
    validate_encoded_record_checksum,
};
pub use engine_snapshot::{load_engine_graph_snapshot, persist_engine_graph_snapshot};
pub use error::{GraphStorageError, GraphStorageResult};
pub use graph_pager::{
    FileBackedGraphPager, FileBackedGraphStore, create_file_backed_graph_pager,
    create_file_backed_graph_store, map_storage_error_to_graph_pager_error,
    pager_storage_ref_from_storage_ref,
};
pub use log::{
    AppendOnlyRecordLog, AppendOnlyRecordLogSegment, append_encoded_node_record_envelope,
    append_encoded_relationship_record_envelope, flush_append_only_record_log,
    open_append_only_node_record_log, open_append_only_relationship_record_log,
};
pub use manifest::{GraphId, RecordFormat, StorageManifest, StorageTimestamp, StorageVersion};
pub use opencti_access::AccessContext as CanonicalAccessContext;
pub use record::{
    GraphRecordVersion, PersistedRecordEnvelope, PersistedRecordId, PersistedRecordKind,
    RecordChecksum, StorageRef, StorageSegment, create_adjacency_record_envelope,
    create_node_record_envelope, create_relationship_record_envelope,
    validate_persisted_record_envelope, validate_storage_ref,
};
pub use root::{
    StorageRoot, create_storage_root, open_storage_root, read_storage_manifest,
    validate_storage_manifest,
};
pub use store_reopen::{
    GraphStoreCatalogRecoveryOutcome, GraphStoreCatalogRecoverySource, GraphStoreOpenMode,
    GraphStoreOpenOptions, GraphStoreOpenOutcome, GraphStoreRecoveryReport,
    build_recovered_file_backed_graph_store, open_existing_file_backed_graph_store,
    recover_graph_store_adjacency_storage, recover_graph_store_catalog,
    validate_graph_store_reopen_manifest, validate_required_recovery_components,
};
pub use transaction_log::{
    DurableMutationTarget, DurableReplayAction, DurableTransactionId,
    DurableTransactionReplayStatus, DurableWalEntry, DurableWalEntryKind, WalContractError,
    WalSequenceNumber, classify_replay_action, classify_transaction_replay_status,
    validate_durable_wal_entry,
};

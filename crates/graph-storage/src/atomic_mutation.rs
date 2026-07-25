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
use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use graph_core::{AdjacencyDirection, RelationshipType};
use opencti_access::AccessMetadata;
use serde::{Deserialize, Serialize};

use crate::{
    DurableMutationTarget, DurableTransactionId, DurableTransactionReplayStatus, DurableWalEntry,
    DurableWalEntryKind, EncodedRecord, GraphAdjacencyStorage, GraphCatalog, GraphRecordVersion,
    GraphStorageError, GraphStorageResult, LabelIndexNodeMetadata, NodeReadIndexDocument,
    PersistedAdjacencyEntry, PersistedAdjacencyRecord, PersistedRecordEnvelope, PersistedRecordId,
    RelationshipTypeIndexRelationshipMetadata, StorageRoot, WalSequenceNumber,
    append_encoded_node_record_envelope, append_encoded_relationship_record_envelope,
    classify_transaction_replay_status, index_appended_node_record,
    index_appended_relationship_record, index_relationship_type, open_append_only_node_record_log,
    open_append_only_relationship_record_log, persist_graph_catalog_metadata,
    read_persisted_graph_catalog_metadata, replace_node_read_indexes,
    replace_relationship_access_index, restore_persisted_adjacency_records,
    snapshot_persisted_adjacency_records, validate_durable_wal_entry,
    write_incoming_adjacency_by_node_id, write_outgoing_adjacency_by_node_id,
};

// A checkpoint is intentionally periodic rather than mutation-scoped. The WAL
// and projection journals remain the durable per-mutation boundary; rewriting a
// complete catalog on every small update would recreate the write amplification
// that the append-only store replaces.
const CHECKPOINT_INTERVAL_TRANSACTIONS: u64 = 128;

/// In-memory runtime state maintained by the atomic persistent mutation boundary.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AtomicPersistentRuntimeState {
    /// Catalog rebuilt and updated by committed persistent mutations.
    pub catalog: GraphCatalog,
    /// Adjacency storage rebuilt and updated by committed persistent mutations.
    pub adjacency_storage: GraphAdjacencyStorage,
}

/// One node record mutation payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AtomicPersistentMutationNodeRecord {
    /// Node envelope metadata.
    pub envelope: PersistedRecordEnvelope,
    /// Deterministic encoded node envelope.
    pub encoded_record: EncodedRecord,
    /// Labels used to update the label index projection.
    pub labels: Vec<String>,
    /// Payload-free identifier, property and temporal index values.
    pub read_index: NodeReadIndexDocument,
}

/// One relationship record mutation payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AtomicPersistentMutationRelationshipRecord {
    /// Relationship envelope metadata.
    pub envelope: PersistedRecordEnvelope,
    /// Deterministic encoded relationship envelope.
    pub encoded_record: EncodedRecord,
    /// Relationship type used to update the type index projection.
    pub relationship_type: RelationshipType,
    /// Whether this current relationship is visible rather than tombstoned.
    pub active: bool,
    /// Payload-free OpenCTI access document.
    pub access: AccessMetadata,
}

/// One adjacency mutation payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AtomicPersistentMutationAdjacencyRecord {
    /// Owner node whose adjacency is being updated.
    pub owner_node_id: graph_core::NodeId,
    /// Adjacency direction.
    pub direction: AdjacencyDirection,
    /// Adjacency entries persisted for this owner node and direction.
    pub entries: Vec<PersistedAdjacencyEntry>,
}

/// Full atomic persistent mutation payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AtomicPersistentMutationBatch {
    /// Durable transaction id.
    pub transaction_id: DurableTransactionId,
    /// Node records included in this mutation.
    pub node_records: Vec<AtomicPersistentMutationNodeRecord>,
    /// Relationship records included in this mutation.
    pub relationship_records: Vec<AtomicPersistentMutationRelationshipRecord>,
    /// Outgoing adjacency updates included in this mutation.
    pub outgoing_adjacency: Vec<AtomicPersistentMutationAdjacencyRecord>,
    /// Incoming adjacency updates included in this mutation.
    pub incoming_adjacency: Vec<AtomicPersistentMutationAdjacencyRecord>,
    /// Audit event messages persisted with the mutation.
    pub audit_events: Vec<String>,
}

/// Deterministic crash-injection stage used by acceptance tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MutationCrashStage {
    /// Fail after WAL and payload writes but before the applied marker is persisted.
    BeforeAppliedMarker,
    /// Fail after the applied marker is persisted but before checkpoint materialization.
    BeforeCheckpointWrite,
}

/// Outcome returned by atomic mutation apply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AtomicPersistentMutationOutcome {
    /// Whether the transaction was newly applied.
    pub applied: bool,
    /// Mutation sequence assigned to this transaction when available.
    pub mutation_sequence_number: Option<WalSequenceNumber>,
}

/// Compaction scope for atomic persistent segments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AtomicPersistentCompactionScope {
    /// Compact transaction WAL and index mutation segments.
    TransactionsAndIndexes,
}

/// Request for atomic persistent segment compaction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AtomicPersistentCompactionRequest {
    /// Target compaction scope.
    pub scope: AtomicPersistentCompactionScope,
    /// Snapshot-protected mutation sequences that must be retained.
    pub snapshot_protected_sequences: Vec<WalSequenceNumber>,
    /// Retention-protected mutation sequences that must be retained.
    pub retention_protected_sequences: Vec<WalSequenceNumber>,
}

/// Compaction result diagnostics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AtomicPersistentCompactionOutcome {
    /// Checkpoint sequence used as compaction safety boundary.
    pub safe_checkpoint_sequence_number: WalSequenceNumber,
    /// Bytes reclaimed by replacing compacted segments.
    pub reclaimed_bytes: u64,
    /// Number of mutation sequences compacted away.
    pub compacted_sequence_count: usize,
    /// Protected sequences that were retained.
    pub retained_protected_sequences: Vec<WalSequenceNumber>,
}

/// Recovery path used to reconstruct persistent runtime state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AtomicPersistentRecoveryPath {
    /// Recovery loaded a checkpoint and replayed only newer eligible mutation sequences.
    CheckpointAndBoundedReplay,
    /// Recovery replayed all eligible mutation sequences without a checkpoint baseline.
    FullReplay,
}

/// Recovery diagnostics for atomic persistent runtime reconstruction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AtomicPersistentRecoveryReport {
    /// Recovery path used.
    pub recovery_path: AtomicPersistentRecoveryPath,
    /// Sequence number of the checkpoint baseline when one was used.
    pub checkpoint_sequence_number: Option<WalSequenceNumber>,
    /// Number of eligible transactions replayed after baseline selection.
    pub replayed_transaction_count: usize,
    /// Non-fatal diagnostics captured during baseline selection.
    pub warnings: Vec<String>,
}

/// Recovery outcome containing state and diagnostics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AtomicPersistentRecoveryOutcome {
    /// Reconstructed runtime state.
    pub state: AtomicPersistentRuntimeState,
    /// Recovery diagnostics.
    pub report: AtomicPersistentRecoveryReport,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct NodeMutationLogRecord {
    transaction_id: DurableTransactionId,
    mutation_sequence_number: WalSequenceNumber,
    envelope: PersistedRecordEnvelope,
    storage_ref: crate::StorageRef,
    labels: Vec<String>,
    #[serde(default)]
    read_index: NodeReadIndexDocument,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RelationshipMutationLogRecord {
    transaction_id: DurableTransactionId,
    mutation_sequence_number: WalSequenceNumber,
    envelope: PersistedRecordEnvelope,
    storage_ref: crate::StorageRef,
    relationship_type: RelationshipType,
    #[serde(default = "default_true")]
    active: bool,
    #[serde(default)]
    access: AccessMetadata,
}

const fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AdjacencyMutationLogRecord {
    transaction_id: DurableTransactionId,
    mutation_sequence_number: WalSequenceNumber,
    record: PersistedAdjacencyRecord,
    storage_ref: crate::StorageRef,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AppliedMutationLogRecord {
    transaction_id: DurableTransactionId,
    mutation_sequence_number: WalSequenceNumber,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AuditMutationLogRecord {
    transaction_id: DurableTransactionId,
    mutation_sequence_number: WalSequenceNumber,
    message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AtomicPersistentCheckpointRecord {
    mutation_sequence_number: WalSequenceNumber,
    adjacency_records: Vec<PersistedAdjacencyRecord>,
}

struct LoadedCheckpoint {
    mutation_sequence_number: WalSequenceNumber,
    runtime_state: AtomicPersistentRuntimeState,
}

/// Applies one atomic persistent mutation batch.
pub fn apply_atomic_persistent_mutation_batch(
    root: &StorageRoot,
    state: &mut AtomicPersistentRuntimeState,
    batch: AtomicPersistentMutationBatch,
    crash_stage: Option<MutationCrashStage>,
) -> GraphStorageResult<AtomicPersistentMutationOutcome> {
    if !root.path().is_dir() {
        return Err(GraphStorageError::StorageRootNotFound {
            path: root.path().to_path_buf(),
        });
    }
    if mutation_targets(&batch).is_empty() {
        return Err(GraphStorageError::OperationFailed {
            operation: "apply_atomic_persistent_mutation_batch",
            message: "atomic mutation batch must contain at least one mutation target".to_owned(),
        });
    }

    let applied_index = read_applied_mutation_index(root)?;
    if let Some(existing_sequence) = applied_index.get(&batch.transaction_id) {
        return Ok(AtomicPersistentMutationOutcome {
            applied: false,
            mutation_sequence_number: Some(*existing_sequence),
        });
    }

    let (begin_sequence, mutation_sequence, commit_sequence) = next_transaction_sequences(root)?;
    let wal_entries = vec![
        DurableWalEntry {
            transaction_id: batch.transaction_id.clone(),
            sequence_number: begin_sequence,
            kind: DurableWalEntryKind::Begin,
            mutation_targets: Vec::new(),
            checksum: None,
        },
        DurableWalEntry {
            transaction_id: batch.transaction_id.clone(),
            sequence_number: mutation_sequence,
            kind: DurableWalEntryKind::Mutation,
            mutation_targets: mutation_targets(&batch),
            checksum: None,
        },
        DurableWalEntry {
            transaction_id: batch.transaction_id.clone(),
            sequence_number: commit_sequence,
            kind: DurableWalEntryKind::Commit,
            mutation_targets: Vec::new(),
            checksum: None,
        },
    ];
    append_wal_entries(root, &wal_entries)?;

    let mut working_state = state.clone();
    persist_node_mutations(root, &mut working_state, &batch, mutation_sequence)?;
    persist_relationship_mutations(root, &mut working_state, &batch, mutation_sequence)?;
    persist_adjacency_mutations(root, &mut working_state, &batch, mutation_sequence)?;
    persist_audit_events(root, &batch, mutation_sequence)?;

    if crash_stage == Some(MutationCrashStage::BeforeAppliedMarker) {
        return Err(GraphStorageError::OperationFailed {
            operation: "apply_atomic_persistent_mutation_batch",
            message: "injected crash before applied mutation marker".to_owned(),
        });
    }

    append_json_line_sync(
        &applied_mutation_log_path(root),
        &AppliedMutationLogRecord {
            transaction_id: batch.transaction_id.clone(),
            mutation_sequence_number: mutation_sequence,
        },
        "apply_atomic_persistent_mutation_batch",
    )?;

    if crash_stage == Some(MutationCrashStage::BeforeCheckpointWrite) {
        return Err(GraphStorageError::OperationFailed {
            operation: "apply_atomic_persistent_mutation_batch",
            message: "injected crash before checkpoint write".to_owned(),
        });
    }

    if should_write_checkpoint(mutation_sequence) {
        persist_graph_catalog_metadata(root, &working_state.catalog)?;
        write_checkpoint(root, mutation_sequence, &working_state)?;
    }
    *state = working_state;
    Ok(AtomicPersistentMutationOutcome {
        applied: true,
        mutation_sequence_number: Some(mutation_sequence),
    })
}

fn should_write_checkpoint(mutation_sequence: WalSequenceNumber) -> bool {
    let transaction_ordinal = mutation_sequence.0.saturating_add(1) / 3;
    transaction_ordinal == 1 || transaction_ordinal.is_multiple_of(CHECKPOINT_INTERVAL_TRANSACTIONS)
}

/// Recovers runtime state from atomic persistent mutation logs.
pub fn recover_atomic_persistent_runtime_state(
    root: &StorageRoot,
) -> GraphStorageResult<AtomicPersistentRuntimeState> {
    Ok(recover_atomic_persistent_runtime_state_with_report(root)?.state)
}

/// Recovers runtime state from atomic persistent mutation logs with diagnostics.
pub fn recover_atomic_persistent_runtime_state_with_report(
    root: &StorageRoot,
) -> GraphStorageResult<AtomicPersistentRecoveryOutcome> {
    if !root.path().is_dir() {
        return Err(GraphStorageError::StorageRootNotFound {
            path: root.path().to_path_buf(),
        });
    }

    let committed_sequences = read_committed_mutation_sequences(root)?;
    let applied_sequences = read_applied_mutation_index(root)?;
    let eligible = eligible_sequences(&committed_sequences, &applied_sequences);

    let checkpoint = load_latest_valid_checkpoint(root)?;
    let (mut recovered, recovery_path, checkpoint_sequence_number, warnings) = match checkpoint {
        Some(checkpoint) => (
            checkpoint.runtime_state,
            AtomicPersistentRecoveryPath::CheckpointAndBoundedReplay,
            Some(checkpoint.mutation_sequence_number),
            Vec::new(),
        ),
        None => (
            AtomicPersistentRuntimeState::default(),
            AtomicPersistentRecoveryPath::FullReplay,
            None,
            checkpoint_selection_warnings(root)?,
        ),
    };
    let replay_sequences = sequences_after_checkpoint(&eligible, checkpoint_sequence_number);

    replay_node_mutations(root, &replay_sequences, &mut recovered)?;
    replay_relationship_mutations(root, &replay_sequences, &mut recovered)?;
    replay_adjacency_mutations(root, &replay_sequences, &mut recovered)?;

    Ok(AtomicPersistentRecoveryOutcome {
        state: recovered,
        report: AtomicPersistentRecoveryReport {
            recovery_path,
            checkpoint_sequence_number,
            replayed_transaction_count: replay_sequences.len(),
            warnings,
        },
    })
}

/// Compact obsolete transaction and index segments using the latest safe checkpoint.
pub fn compact_atomic_persistent_segments(
    root: &StorageRoot,
    request: AtomicPersistentCompactionRequest,
) -> GraphStorageResult<AtomicPersistentCompactionOutcome> {
    if !root.path().is_dir() {
        return Err(GraphStorageError::StorageRootNotFound {
            path: root.path().to_path_buf(),
        });
    }

    let checkpoint =
        load_latest_valid_checkpoint(root)?.ok_or_else(|| GraphStorageError::OperationFailed {
            operation: "compact_atomic_persistent_segments",
            message: "safe checkpoint is required before compaction".to_owned(),
        })?;
    let safe_sequence = checkpoint.mutation_sequence_number;

    let mut protected_sequences: HashSet<WalSequenceNumber> = request
        .snapshot_protected_sequences
        .into_iter()
        .chain(request.retention_protected_sequences)
        .collect();
    protected_sequences.retain(|sequence| sequence.0 <= safe_sequence.0);

    let committed = read_committed_mutation_sequences(root)?;
    let applied = read_applied_mutation_index(root)?;
    let eligible = eligible_sequences(&committed, &applied);
    let mut transaction_mutation_sequences = committed.clone();
    for (transaction_id, sequence) in &applied {
        transaction_mutation_sequences
            .entry(transaction_id.clone())
            .or_insert(*sequence);
    }
    let protected_tx_ids: HashSet<DurableTransactionId> = eligible
        .iter()
        .filter_map(|(transaction_id, sequence)| {
            protected_sequences
                .contains(sequence)
                .then(|| transaction_id.clone())
        })
        .collect();

    let retained_protected_sequences: Vec<WalSequenceNumber> = eligible
        .values()
        .filter(|sequence| protected_sequences.contains(sequence))
        .copied()
        .collect();
    let compacted_sequences: HashSet<WalSequenceNumber> = eligible
        .values()
        .filter(|sequence| sequence.0 <= safe_sequence.0 && !protected_sequences.contains(sequence))
        .copied()
        .collect();

    let should_keep_sequence = |sequence: WalSequenceNumber| {
        sequence.0 > safe_sequence.0 || protected_sequences.contains(&sequence)
    };

    let mut reclaimed_bytes = 0_u64;
    match request.scope {
        AtomicPersistentCompactionScope::TransactionsAndIndexes => {
            let wal_entries: Vec<DurableWalEntry> = read_json_lines(
                &transaction_wal_path(root),
                "compact_atomic_persistent_segments",
            )?;
            let retained_wal: Vec<DurableWalEntry> = wal_entries
                .into_iter()
                .filter(|entry| {
                    transaction_mutation_sequences
                        .get(&entry.transaction_id)
                        .map(|mutation_sequence| should_keep_sequence(*mutation_sequence))
                        .unwrap_or(entry.sequence_number.0 > safe_sequence.0)
                        || protected_tx_ids.contains(&entry.transaction_id)
                })
                .collect();
            reclaimed_bytes += rewrite_compacted_segment(
                root,
                safe_sequence,
                &transaction_wal_path(root),
                retained_wal,
                "compact_atomic_persistent_segments",
            )?;

            let applied: Vec<AppliedMutationLogRecord> = read_json_lines(
                &applied_mutation_log_path(root),
                "compact_atomic_persistent_segments",
            )?;
            let retained_applied: Vec<AppliedMutationLogRecord> = applied
                .into_iter()
                .filter(|record| should_keep_sequence(record.mutation_sequence_number))
                .collect();
            reclaimed_bytes += rewrite_compacted_segment(
                root,
                safe_sequence,
                &applied_mutation_log_path(root),
                retained_applied,
                "compact_atomic_persistent_segments",
            )?;

            let node_logs: Vec<NodeMutationLogRecord> = read_json_lines(
                &node_mutation_log_path(root),
                "compact_atomic_persistent_segments",
            )?;
            let retained_node_logs: Vec<NodeMutationLogRecord> = node_logs
                .into_iter()
                .filter(|record| should_keep_sequence(record.mutation_sequence_number))
                .collect();
            reclaimed_bytes += rewrite_compacted_segment(
                root,
                safe_sequence,
                &node_mutation_log_path(root),
                retained_node_logs,
                "compact_atomic_persistent_segments",
            )?;

            let relationship_logs: Vec<RelationshipMutationLogRecord> = read_json_lines(
                &relationship_mutation_log_path(root),
                "compact_atomic_persistent_segments",
            )?;
            let retained_relationship_logs: Vec<RelationshipMutationLogRecord> = relationship_logs
                .into_iter()
                .filter(|record| should_keep_sequence(record.mutation_sequence_number))
                .collect();
            reclaimed_bytes += rewrite_compacted_segment(
                root,
                safe_sequence,
                &relationship_mutation_log_path(root),
                retained_relationship_logs,
                "compact_atomic_persistent_segments",
            )?;

            let outgoing_logs: Vec<AdjacencyMutationLogRecord> = read_json_lines(
                &outgoing_adjacency_mutation_log_path(root),
                "compact_atomic_persistent_segments",
            )?;
            let retained_outgoing_logs: Vec<AdjacencyMutationLogRecord> = outgoing_logs
                .into_iter()
                .filter(|record| should_keep_sequence(record.mutation_sequence_number))
                .collect();
            reclaimed_bytes += rewrite_compacted_segment(
                root,
                safe_sequence,
                &outgoing_adjacency_mutation_log_path(root),
                retained_outgoing_logs,
                "compact_atomic_persistent_segments",
            )?;

            let incoming_logs: Vec<AdjacencyMutationLogRecord> = read_json_lines(
                &incoming_adjacency_mutation_log_path(root),
                "compact_atomic_persistent_segments",
            )?;
            let retained_incoming_logs: Vec<AdjacencyMutationLogRecord> = incoming_logs
                .into_iter()
                .filter(|record| should_keep_sequence(record.mutation_sequence_number))
                .collect();
            reclaimed_bytes += rewrite_compacted_segment(
                root,
                safe_sequence,
                &incoming_adjacency_mutation_log_path(root),
                retained_incoming_logs,
                "compact_atomic_persistent_segments",
            )?;

            let audit_logs: Vec<AuditMutationLogRecord> = read_json_lines(
                &audit_mutation_log_path(root),
                "compact_atomic_persistent_segments",
            )?;
            let retained_audit_logs: Vec<AuditMutationLogRecord> = audit_logs
                .into_iter()
                .filter(|record| should_keep_sequence(record.mutation_sequence_number))
                .collect();
            reclaimed_bytes += rewrite_compacted_segment(
                root,
                safe_sequence,
                &audit_mutation_log_path(root),
                retained_audit_logs,
                "compact_atomic_persistent_segments",
            )?;
        }
    }

    let mut retained_protected_sequences = retained_protected_sequences;
    retained_protected_sequences.sort_by_key(|sequence| sequence.0);

    Ok(AtomicPersistentCompactionOutcome {
        safe_checkpoint_sequence_number: safe_sequence,
        reclaimed_bytes,
        compacted_sequence_count: compacted_sequences.len(),
        retained_protected_sequences,
    })
}

fn mutation_targets(batch: &AtomicPersistentMutationBatch) -> Vec<DurableMutationTarget> {
    let mut targets = Vec::new();
    for node in &batch.node_records {
        targets.push(DurableMutationTarget {
            record_id: node.envelope.record_id.clone(),
        });
    }
    for relationship in &batch.relationship_records {
        targets.push(DurableMutationTarget {
            record_id: relationship.envelope.record_id.clone(),
        });
    }
    for adjacency in &batch.outgoing_adjacency {
        targets.push(DurableMutationTarget {
            record_id: PersistedRecordId::Adjacency {
                owner_node_id: adjacency.owner_node_id.clone(),
                direction: AdjacencyDirection::Outgoing,
            },
        });
    }
    for adjacency in &batch.incoming_adjacency {
        targets.push(DurableMutationTarget {
            record_id: PersistedRecordId::Adjacency {
                owner_node_id: adjacency.owner_node_id.clone(),
                direction: AdjacencyDirection::Incoming,
            },
        });
    }
    targets
}

fn next_transaction_sequences(
    root: &StorageRoot,
) -> GraphStorageResult<(WalSequenceNumber, WalSequenceNumber, WalSequenceNumber)> {
    let entries: Vec<DurableWalEntry> =
        read_json_lines(&transaction_wal_path(root), "next_transaction_sequences")?;
    let highest = entries
        .iter()
        .map(|entry| entry.sequence_number.0)
        .max()
        .unwrap_or(0);

    let begin = WalSequenceNumber::new(highest + 1).map_err(wal_contract_error)?;
    let mutation = WalSequenceNumber::new(highest + 2).map_err(wal_contract_error)?;
    let commit = WalSequenceNumber::new(highest + 3).map_err(wal_contract_error)?;
    Ok((begin, mutation, commit))
}

fn append_wal_entries(root: &StorageRoot, entries: &[DurableWalEntry]) -> GraphStorageResult<()> {
    for entry in entries {
        validate_durable_wal_entry(entry).map_err(wal_contract_error)?;
        append_json_line_sync(
            &transaction_wal_path(root),
            entry,
            "apply_atomic_persistent_mutation_batch",
        )?;
    }
    Ok(())
}

fn persist_node_mutations(
    root: &StorageRoot,
    state: &mut AtomicPersistentRuntimeState,
    batch: &AtomicPersistentMutationBatch,
    mutation_sequence: WalSequenceNumber,
) -> GraphStorageResult<()> {
    let mut log = open_append_only_node_record_log(root)?;
    for node in &batch.node_records {
        let storage_ref =
            append_encoded_node_record_envelope(&mut log, &node.envelope, &node.encoded_record)?;
        index_appended_node_record(&mut state.catalog, &node.envelope, storage_ref.clone())?;
        if let (
            PersistedRecordId::Node(node_id),
            Some(GraphRecordVersion::Node { current: true, .. }),
        ) = (
            &node.envelope.record_id,
            node.envelope.graph_record_version.clone(),
        ) {
            replace_node_read_indexes(
                &mut state.catalog,
                &node.labels,
                &node.read_index,
                LabelIndexNodeMetadata {
                    node_id: node_id.clone(),
                    latest_storage_ref: Some(storage_ref.clone()),
                    graph_record_version: node_graph_version(&node.envelope)?,
                },
            )?;
        }
        append_json_line_sync(
            &node_mutation_log_path(root),
            &NodeMutationLogRecord {
                transaction_id: batch.transaction_id.clone(),
                mutation_sequence_number: mutation_sequence,
                envelope: node.envelope.clone(),
                storage_ref,
                labels: node.labels.clone(),
                read_index: node.read_index.clone(),
            },
            "apply_atomic_persistent_mutation_batch",
        )?;
    }
    Ok(())
}

fn persist_relationship_mutations(
    root: &StorageRoot,
    state: &mut AtomicPersistentRuntimeState,
    batch: &AtomicPersistentMutationBatch,
    mutation_sequence: WalSequenceNumber,
) -> GraphStorageResult<()> {
    let mut log = open_append_only_relationship_record_log(root)?;
    for relationship in &batch.relationship_records {
        let storage_ref = append_encoded_relationship_record_envelope(
            &mut log,
            &relationship.envelope,
            &relationship.encoded_record,
        )?;
        index_appended_relationship_record(
            &mut state.catalog,
            &relationship.envelope,
            storage_ref.clone(),
        )?;
        if let (
            PersistedRecordId::Relationship(relationship_id),
            Some(GraphRecordVersion::Relationship { current: true, .. }),
        ) = (
            &relationship.envelope.record_id,
            relationship.envelope.graph_record_version.as_ref(),
        ) {
            replace_relationship_access_index(
                &mut state.catalog,
                relationship_id,
                relationship.active,
                &relationship.access,
            );
            if relationship.active {
                index_relationship_type(
                    &mut state.catalog,
                    &relationship.relationship_type,
                    RelationshipTypeIndexRelationshipMetadata {
                        relationship_id: relationship_id.clone(),
                        latest_storage_ref: Some(storage_ref.clone()),
                        graph_record_version: relationship_graph_version(&relationship.envelope)?,
                    },
                )?;
            }
        }
        append_json_line_sync(
            &relationship_mutation_log_path(root),
            &RelationshipMutationLogRecord {
                transaction_id: batch.transaction_id.clone(),
                mutation_sequence_number: mutation_sequence,
                envelope: relationship.envelope.clone(),
                storage_ref,
                relationship_type: relationship.relationship_type.clone(),
                active: relationship.active,
                access: relationship.access.clone(),
            },
            "apply_atomic_persistent_mutation_batch",
        )?;
    }
    Ok(())
}

fn persist_adjacency_mutations(
    root: &StorageRoot,
    state: &mut AtomicPersistentRuntimeState,
    batch: &AtomicPersistentMutationBatch,
    mutation_sequence: WalSequenceNumber,
) -> GraphStorageResult<()> {
    for adjacency in &batch.outgoing_adjacency {
        if adjacency.direction != AdjacencyDirection::Outgoing {
            return Err(GraphStorageError::InvalidEnvelope {
                reason: "outgoing adjacency batch entries must use Outgoing direction".to_owned(),
            });
        }
        let storage_ref = write_outgoing_adjacency_by_node_id(
            &mut state.adjacency_storage,
            &mut state.catalog,
            &adjacency.owner_node_id,
            adjacency.entries.clone(),
        )?;
        append_json_line_sync(
            &outgoing_adjacency_mutation_log_path(root),
            &AdjacencyMutationLogRecord {
                transaction_id: batch.transaction_id.clone(),
                mutation_sequence_number: mutation_sequence,
                record: PersistedAdjacencyRecord {
                    owner_node_id: adjacency.owner_node_id.clone(),
                    direction: AdjacencyDirection::Outgoing,
                    entries: adjacency.entries.clone(),
                    storage_ref: Some(storage_ref.clone()),
                },
                storage_ref,
            },
            "apply_atomic_persistent_mutation_batch",
        )?;
    }

    for adjacency in &batch.incoming_adjacency {
        if adjacency.direction != AdjacencyDirection::Incoming {
            return Err(GraphStorageError::InvalidEnvelope {
                reason: "incoming adjacency batch entries must use Incoming direction".to_owned(),
            });
        }
        let storage_ref = write_incoming_adjacency_by_node_id(
            &mut state.adjacency_storage,
            &mut state.catalog,
            &adjacency.owner_node_id,
            adjacency.entries.clone(),
        )?;
        append_json_line_sync(
            &incoming_adjacency_mutation_log_path(root),
            &AdjacencyMutationLogRecord {
                transaction_id: batch.transaction_id.clone(),
                mutation_sequence_number: mutation_sequence,
                record: PersistedAdjacencyRecord {
                    owner_node_id: adjacency.owner_node_id.clone(),
                    direction: AdjacencyDirection::Incoming,
                    entries: adjacency.entries.clone(),
                    storage_ref: Some(storage_ref.clone()),
                },
                storage_ref,
            },
            "apply_atomic_persistent_mutation_batch",
        )?;
    }
    Ok(())
}

fn persist_audit_events(
    root: &StorageRoot,
    batch: &AtomicPersistentMutationBatch,
    mutation_sequence: WalSequenceNumber,
) -> GraphStorageResult<()> {
    for message in &batch.audit_events {
        append_json_line_sync(
            &audit_mutation_log_path(root),
            &AuditMutationLogRecord {
                transaction_id: batch.transaction_id.clone(),
                mutation_sequence_number: mutation_sequence,
                message: message.clone(),
            },
            "apply_atomic_persistent_mutation_batch",
        )?;
    }
    Ok(())
}

fn replay_node_mutations(
    root: &StorageRoot,
    eligible_sequences: &HashMap<DurableTransactionId, WalSequenceNumber>,
    state: &mut AtomicPersistentRuntimeState,
) -> GraphStorageResult<()> {
    let records: Vec<NodeMutationLogRecord> = read_json_lines(
        &node_mutation_log_path(root),
        "recover_atomic_persistent_runtime_state",
    )?;
    for record in records {
        if eligible_sequences.get(&record.transaction_id) != Some(&record.mutation_sequence_number)
        {
            continue;
        }
        index_appended_node_record(
            &mut state.catalog,
            &record.envelope,
            record.storage_ref.clone(),
        )?;
        if let (
            PersistedRecordId::Node(node_id),
            Some(GraphRecordVersion::Node { current: true, .. }),
        ) = (
            &record.envelope.record_id,
            record.envelope.graph_record_version.clone(),
        ) {
            replace_node_read_indexes(
                &mut state.catalog,
                &record.labels,
                &record.read_index,
                LabelIndexNodeMetadata {
                    node_id: node_id.clone(),
                    latest_storage_ref: Some(record.storage_ref.clone()),
                    graph_record_version: node_graph_version(&record.envelope)?,
                },
            )?;
        }
    }
    Ok(())
}

fn replay_relationship_mutations(
    root: &StorageRoot,
    eligible_sequences: &HashMap<DurableTransactionId, WalSequenceNumber>,
    state: &mut AtomicPersistentRuntimeState,
) -> GraphStorageResult<()> {
    let records: Vec<RelationshipMutationLogRecord> = read_json_lines(
        &relationship_mutation_log_path(root),
        "recover_atomic_persistent_runtime_state",
    )?;
    for record in records {
        if eligible_sequences.get(&record.transaction_id) != Some(&record.mutation_sequence_number)
        {
            continue;
        }
        index_appended_relationship_record(
            &mut state.catalog,
            &record.envelope,
            record.storage_ref.clone(),
        )?;
        if let (
            PersistedRecordId::Relationship(relationship_id),
            Some(GraphRecordVersion::Relationship { current: true, .. }),
        ) = (
            &record.envelope.record_id,
            record.envelope.graph_record_version.as_ref(),
        ) {
            replace_relationship_access_index(
                &mut state.catalog,
                relationship_id,
                record.active,
                &record.access,
            );
            if record.active {
                index_relationship_type(
                    &mut state.catalog,
                    &record.relationship_type,
                    RelationshipTypeIndexRelationshipMetadata {
                        relationship_id: relationship_id.clone(),
                        latest_storage_ref: Some(record.storage_ref.clone()),
                        graph_record_version: relationship_graph_version(&record.envelope)?,
                    },
                )?;
            }
        }
    }
    Ok(())
}

fn replay_adjacency_mutations(
    root: &StorageRoot,
    eligible_sequences: &HashMap<DurableTransactionId, WalSequenceNumber>,
    state: &mut AtomicPersistentRuntimeState,
) -> GraphStorageResult<()> {
    let outgoing_records: Vec<AdjacencyMutationLogRecord> = read_json_lines(
        &outgoing_adjacency_mutation_log_path(root),
        "recover_atomic_persistent_runtime_state",
    )?;
    for record in outgoing_records {
        if eligible_sequences.get(&record.transaction_id) != Some(&record.mutation_sequence_number)
        {
            continue;
        }
        write_outgoing_adjacency_by_node_id(
            &mut state.adjacency_storage,
            &mut state.catalog,
            &record.record.owner_node_id,
            record.record.entries,
        )?;
    }

    let incoming_records: Vec<AdjacencyMutationLogRecord> = read_json_lines(
        &incoming_adjacency_mutation_log_path(root),
        "recover_atomic_persistent_runtime_state",
    )?;
    for record in incoming_records {
        if eligible_sequences.get(&record.transaction_id) != Some(&record.mutation_sequence_number)
        {
            continue;
        }
        write_incoming_adjacency_by_node_id(
            &mut state.adjacency_storage,
            &mut state.catalog,
            &record.record.owner_node_id,
            record.record.entries,
        )?;
    }
    Ok(())
}

fn read_committed_mutation_sequences(
    root: &StorageRoot,
) -> GraphStorageResult<HashMap<DurableTransactionId, WalSequenceNumber>> {
    let entries: Vec<DurableWalEntry> = read_json_lines(
        &transaction_wal_path(root),
        "recover_atomic_persistent_runtime_state",
    )?;
    let mut grouped: HashMap<DurableTransactionId, Vec<DurableWalEntry>> = HashMap::new();
    for entry in entries {
        grouped
            .entry(entry.transaction_id.clone())
            .or_default()
            .push(entry);
    }

    let mut committed = HashMap::new();
    for (transaction_id, mut slice) in grouped {
        slice.sort_by_key(|entry| entry.sequence_number.0);
        let status = classify_transaction_replay_status(&slice).map_err(wal_contract_error)?;
        if status != DurableTransactionReplayStatus::Committed {
            continue;
        }
        let mutation = slice
            .iter()
            .find(|entry| entry.kind == DurableWalEntryKind::Mutation)
            .ok_or_else(|| GraphStorageError::OperationFailed {
                operation: "recover_atomic_persistent_runtime_state",
                message: format!(
                    "committed transaction {} is missing mutation WAL entry",
                    transaction_id.value
                ),
            })?;
        committed.insert(transaction_id, mutation.sequence_number);
    }

    Ok(committed)
}

fn read_applied_mutation_index(
    root: &StorageRoot,
) -> GraphStorageResult<HashMap<DurableTransactionId, WalSequenceNumber>> {
    let records: Vec<AppliedMutationLogRecord> = read_json_lines(
        &applied_mutation_log_path(root),
        "read_applied_mutation_index",
    )?;
    let mut index: HashMap<DurableTransactionId, WalSequenceNumber> = HashMap::new();
    for record in records {
        if let Some(existing) = index.get(&record.transaction_id)
            && existing != &record.mutation_sequence_number
        {
            return Err(GraphStorageError::OperationFailed {
                operation: "read_applied_mutation_index",
                message: format!(
                    "transaction {} has conflicting applied mutation sequences: {} and {}",
                    record.transaction_id.value, existing.0, record.mutation_sequence_number.0
                ),
            });
        }
        index.insert(record.transaction_id, record.mutation_sequence_number);
    }
    Ok(index)
}

fn eligible_sequences(
    committed: &HashMap<DurableTransactionId, WalSequenceNumber>,
    applied: &HashMap<DurableTransactionId, WalSequenceNumber>,
) -> HashMap<DurableTransactionId, WalSequenceNumber> {
    let mut eligible = HashMap::new();
    for (transaction_id, applied_sequence) in applied {
        if committed.get(transaction_id) == Some(applied_sequence) {
            eligible.insert(transaction_id.clone(), *applied_sequence);
        }
    }
    eligible
}

fn sequences_after_checkpoint(
    eligible: &HashMap<DurableTransactionId, WalSequenceNumber>,
    checkpoint_sequence_number: Option<WalSequenceNumber>,
) -> HashMap<DurableTransactionId, WalSequenceNumber> {
    let Some(checkpoint_sequence_number) = checkpoint_sequence_number else {
        return eligible.clone();
    };
    eligible
        .iter()
        .filter_map(|(transaction_id, sequence)| {
            (sequence.0 > checkpoint_sequence_number.0).then(|| (transaction_id.clone(), *sequence))
        })
        .collect()
}

fn write_checkpoint(
    root: &StorageRoot,
    mutation_sequence_number: WalSequenceNumber,
    state: &AtomicPersistentRuntimeState,
) -> GraphStorageResult<()> {
    let checkpoint = AtomicPersistentCheckpointRecord {
        mutation_sequence_number,
        adjacency_records: snapshot_persisted_adjacency_records(&state.adjacency_storage),
    };
    write_json_file_sync(
        &checkpoint_file_path(root, mutation_sequence_number),
        &checkpoint,
        "apply_atomic_persistent_mutation_batch",
    )
}

fn load_latest_valid_checkpoint(
    root: &StorageRoot,
) -> GraphStorageResult<Option<LoadedCheckpoint>> {
    let mut candidates = checkpoint_candidates(root)?;
    candidates.sort_by_key(|(_, sequence)| std::cmp::Reverse(*sequence));
    for (path, _) in candidates {
        match read_json_file::<AtomicPersistentCheckpointRecord>(&path, "recover_checkpoint") {
            Ok(checkpoint) => {
                let Some(catalog) = read_persisted_graph_catalog_metadata(root)? else {
                    continue;
                };
                let adjacency_storage =
                    restore_persisted_adjacency_records(&checkpoint.adjacency_records)?;
                return Ok(Some(LoadedCheckpoint {
                    mutation_sequence_number: checkpoint.mutation_sequence_number,
                    runtime_state: AtomicPersistentRuntimeState {
                        catalog,
                        adjacency_storage,
                    },
                }));
            }
            Err(_) => continue,
        }
    }
    Ok(None)
}

fn checkpoint_selection_warnings(root: &StorageRoot) -> GraphStorageResult<Vec<String>> {
    let mut candidates = checkpoint_candidates(root)?;
    candidates.sort_by_key(|(_, sequence)| std::cmp::Reverse(*sequence));
    let mut warnings = Vec::new();
    for (path, sequence) in candidates {
        match read_json_file::<AtomicPersistentCheckpointRecord>(&path, "recover_checkpoint") {
            Ok(_) => {
                if read_persisted_graph_catalog_metadata(root)?.is_none() {
                    warnings.push(format!(
                        "checkpoint {} at {} ignored: persisted catalog metadata is missing",
                        sequence,
                        path.to_string_lossy()
                    ));
                    continue;
                }
                break;
            }
            Err(error) => warnings.push(format!(
                "checkpoint {} at {} ignored: {}",
                sequence,
                path.to_string_lossy(),
                error
            )),
        }
    }
    Ok(warnings)
}

fn checkpoint_candidates(root: &StorageRoot) -> GraphStorageResult<Vec<(PathBuf, u64)>> {
    let directory = checkpoints_dir(root);
    if !directory.is_dir() {
        return Ok(Vec::new());
    }

    let mut candidates = Vec::new();
    let entries =
        fs::read_dir(&directory).map_err(|error| GraphStorageError::IoOperationFailed {
            operation: "checkpoint_candidates",
            path: Some(directory.clone()),
            message: error.to_string(),
        })?;
    for entry in entries {
        let entry = entry.map_err(|error| GraphStorageError::IoOperationFailed {
            operation: "checkpoint_candidates",
            path: Some(directory.clone()),
            message: error.to_string(),
        })?;
        let path = entry.path();
        let Some(sequence) = checkpoint_sequence_from_file_name(&path) else {
            continue;
        };
        candidates.push((path, sequence));
    }
    Ok(candidates)
}

fn checkpoint_sequence_from_file_name(path: &Path) -> Option<u64> {
    let file_name = path.file_name()?.to_str()?;
    let stripped = file_name
        .strip_prefix("checkpoint-")?
        .strip_suffix(".json")?;
    stripped.parse::<u64>().ok()
}

fn node_graph_version(
    envelope: &PersistedRecordEnvelope,
) -> GraphStorageResult<Option<GraphRecordVersion>> {
    match &envelope.graph_record_version {
        Some(version @ GraphRecordVersion::Node { .. }) => Ok(Some(version.clone())),
        Some(_) => Err(GraphStorageError::InvalidEnvelope {
            reason: "node mutation envelope requires node graph_record_version".to_owned(),
        }),
        None => Ok(None),
    }
}

fn relationship_graph_version(
    envelope: &PersistedRecordEnvelope,
) -> GraphStorageResult<Option<GraphRecordVersion>> {
    match &envelope.graph_record_version {
        Some(version @ GraphRecordVersion::Relationship { .. }) => Ok(Some(version.clone())),
        Some(_) => Err(GraphStorageError::InvalidEnvelope {
            reason: "relationship mutation envelope requires relationship graph_record_version"
                .to_owned(),
        }),
        None => Ok(None),
    }
}

fn append_json_line_sync<T: Serialize>(
    path: &Path,
    value: &T,
    operation: &'static str,
) -> GraphStorageResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| GraphStorageError::IoOperationFailed {
            operation,
            path: Some(parent.to_path_buf()),
            message: error.to_string(),
        })?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| GraphStorageError::IoOperationFailed {
            operation,
            path: Some(path.to_path_buf()),
            message: error.to_string(),
        })?;
    let mut line =
        serde_json::to_vec(value).map_err(|error| GraphStorageError::OperationFailed {
            operation,
            message: error.to_string(),
        })?;
    line.push(b'\n');
    file.write_all(&line)
        .map_err(|error| GraphStorageError::IoOperationFailed {
            operation,
            path: Some(path.to_path_buf()),
            message: error.to_string(),
        })?;
    file.flush()
        .map_err(|error| GraphStorageError::IoOperationFailed {
            operation,
            path: Some(path.to_path_buf()),
            message: error.to_string(),
        })?;
    file.sync_data()
        .map_err(|error| GraphStorageError::IoOperationFailed {
            operation,
            path: Some(path.to_path_buf()),
            message: error.to_string(),
        })
}

fn read_json_lines<T: for<'de> Deserialize<'de>>(
    path: &Path,
    operation: &'static str,
) -> GraphStorageResult<Vec<T>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = OpenOptions::new().read(true).open(path).map_err(|error| {
        GraphStorageError::IoOperationFailed {
            operation,
            path: Some(path.to_path_buf()),
            message: error.to_string(),
        }
    })?;
    let mut records = Vec::new();
    for line in BufReader::new(file).lines() {
        let line = line.map_err(|error| GraphStorageError::IoOperationFailed {
            operation,
            path: Some(path.to_path_buf()),
            message: error.to_string(),
        })?;
        if line.trim().is_empty() {
            continue;
        }
        let record = serde_json::from_str::<T>(&line).map_err(|error| {
            GraphStorageError::OperationFailed {
                operation,
                message: format!(
                    "failed to decode JSON line from {}: {error}",
                    path.to_string_lossy()
                ),
            }
        })?;
        records.push(record);
    }
    Ok(records)
}

fn write_json_file_sync<T: Serialize>(
    path: &Path,
    value: &T,
    operation: &'static str,
) -> GraphStorageResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| GraphStorageError::IoOperationFailed {
            operation,
            path: Some(parent.to_path_buf()),
            message: error.to_string(),
        })?;
    }
    let bytes = serde_json::to_vec(value).map_err(|error| GraphStorageError::OperationFailed {
        operation,
        message: error.to_string(),
    })?;
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)
        .map_err(|error| GraphStorageError::IoOperationFailed {
            operation,
            path: Some(path.to_path_buf()),
            message: error.to_string(),
        })?;
    file.write_all(&bytes)
        .map_err(|error| GraphStorageError::IoOperationFailed {
            operation,
            path: Some(path.to_path_buf()),
            message: error.to_string(),
        })?;
    file.flush()
        .map_err(|error| GraphStorageError::IoOperationFailed {
            operation,
            path: Some(path.to_path_buf()),
            message: error.to_string(),
        })?;
    file.sync_data()
        .map_err(|error| GraphStorageError::IoOperationFailed {
            operation,
            path: Some(path.to_path_buf()),
            message: error.to_string(),
        })
}

fn rewrite_compacted_segment<T>(
    root: &StorageRoot,
    safe_sequence: WalSequenceNumber,
    path: &Path,
    records: Vec<T>,
    operation: &'static str,
) -> GraphStorageResult<u64>
where
    T: Serialize + for<'de> Deserialize<'de>,
{
    let previous_bytes = if path.is_file() {
        fs::read(path).map_err(|error| GraphStorageError::IoOperationFailed {
            operation,
            path: Some(path.to_path_buf()),
            message: error.to_string(),
        })?
    } else {
        Vec::new()
    };

    let replacement_bytes = json_lines_bytes(&records, operation)?;
    if previous_bytes == replacement_bytes {
        return Ok(0);
    }

    if !previous_bytes.is_empty() {
        let sealed_path = sealed_segment_path(root, safe_sequence, path)?;
        if let Some(parent) = sealed_path.parent() {
            fs::create_dir_all(parent).map_err(|error| GraphStorageError::IoOperationFailed {
                operation,
                path: Some(parent.to_path_buf()),
                message: error.to_string(),
            })?;
        }
        fs::write(&sealed_path, &previous_bytes).map_err(|error| {
            GraphStorageError::IoOperationFailed {
                operation,
                path: Some(sealed_path.clone()),
                message: error.to_string(),
            }
        })?;
        fs::remove_file(&sealed_path).map_err(|error| GraphStorageError::IoOperationFailed {
            operation,
            path: Some(sealed_path),
            message: error.to_string(),
        })?;
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| GraphStorageError::IoOperationFailed {
            operation,
            path: Some(parent.to_path_buf()),
            message: error.to_string(),
        })?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)
        .map_err(|error| GraphStorageError::IoOperationFailed {
            operation,
            path: Some(path.to_path_buf()),
            message: error.to_string(),
        })?;
    file.write_all(&replacement_bytes)
        .map_err(|error| GraphStorageError::IoOperationFailed {
            operation,
            path: Some(path.to_path_buf()),
            message: error.to_string(),
        })?;
    file.flush()
        .map_err(|error| GraphStorageError::IoOperationFailed {
            operation,
            path: Some(path.to_path_buf()),
            message: error.to_string(),
        })?;
    file.sync_data()
        .map_err(|error| GraphStorageError::IoOperationFailed {
            operation,
            path: Some(path.to_path_buf()),
            message: error.to_string(),
        })?;

    let previous_size = u64::try_from(previous_bytes.len()).unwrap_or(u64::MAX);
    let replacement_size = u64::try_from(replacement_bytes.len()).unwrap_or(u64::MAX);
    Ok(previous_size.saturating_sub(replacement_size))
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
        message: format!(
            "failed to decode JSON file from {}: {error}",
            path.to_string_lossy()
        ),
    })
}

fn json_lines_bytes<T: Serialize>(
    records: &[T],
    operation: &'static str,
) -> GraphStorageResult<Vec<u8>> {
    let mut bytes = Vec::new();
    for record in records {
        let mut line =
            serde_json::to_vec(record).map_err(|error| GraphStorageError::OperationFailed {
                operation,
                message: error.to_string(),
            })?;
        line.push(b'\n');
        bytes.extend_from_slice(&line);
    }
    Ok(bytes)
}

fn transaction_dir(root: &StorageRoot) -> PathBuf {
    root.path().join("transactions")
}

fn sealed_segment_path(
    root: &StorageRoot,
    sequence: WalSequenceNumber,
    path: &Path,
) -> GraphStorageResult<PathBuf> {
    let file_name = path
        .file_name()
        .ok_or_else(|| GraphStorageError::OperationFailed {
            operation: "sealed_segment_path",
            message: format!("path {} has no file name", path.to_string_lossy()),
        })?;
    Ok(transaction_dir(root)
        .join("segments")
        .join(format!("segment-{}", sequence.0))
        .join(file_name))
}

fn checkpoints_dir(root: &StorageRoot) -> PathBuf {
    transaction_dir(root).join("checkpoints")
}

fn checkpoint_file_path(root: &StorageRoot, sequence: WalSequenceNumber) -> PathBuf {
    checkpoints_dir(root).join(format!("checkpoint-{}.json", sequence.0))
}

fn transaction_wal_path(root: &StorageRoot) -> PathBuf {
    transaction_dir(root).join("transaction_wal.log")
}

fn applied_mutation_log_path(root: &StorageRoot) -> PathBuf {
    transaction_dir(root).join("applied_mutations.log")
}

fn node_mutation_log_path(root: &StorageRoot) -> PathBuf {
    transaction_dir(root).join("node_mutations.log")
}

fn relationship_mutation_log_path(root: &StorageRoot) -> PathBuf {
    transaction_dir(root).join("relationship_mutations.log")
}

fn outgoing_adjacency_mutation_log_path(root: &StorageRoot) -> PathBuf {
    transaction_dir(root).join("outgoing_adjacency_mutations.log")
}

fn incoming_adjacency_mutation_log_path(root: &StorageRoot) -> PathBuf {
    transaction_dir(root).join("incoming_adjacency_mutations.log")
}

fn audit_mutation_log_path(root: &StorageRoot) -> PathBuf {
    transaction_dir(root).join("audit_events.log")
}

fn wal_contract_error(error: crate::WalContractError) -> GraphStorageError {
    GraphStorageError::OperationFailed {
        operation: "atomic_persistent_mutation_contract",
        message: error.to_string(),
    }
}

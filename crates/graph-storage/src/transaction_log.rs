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
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{PersistedRecordId, RecordChecksum};

/// Typed durable transaction identifier used by WAL entries.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DurableTransactionId {
    /// Stable transaction identifier value.
    pub value: String,
}

impl DurableTransactionId {
    /// Creates a typed durable transaction identifier.
    pub fn new(value: impl Into<String>) -> Result<Self, WalContractError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(WalContractError::InvalidTransactionId {
                reason: "transaction id must not be empty".to_owned(),
            });
        }
        Ok(Self { value })
    }
}

/// Monotonic durable sequence number assigned to one WAL entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WalSequenceNumber(pub u64);

impl WalSequenceNumber {
    /// Creates a sequence number and validates its domain.
    pub fn new(value: u64) -> Result<Self, WalContractError> {
        if value == 0 {
            return Err(WalContractError::InvalidSequenceNumber {
                reason: "sequence number must be greater than zero".to_owned(),
            });
        }
        Ok(Self(value))
    }
}

/// Durable WAL entry kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DurableWalEntryKind {
    /// Start marker for one durable transaction.
    Begin,
    /// Mutation payload marker for one durable transaction.
    Mutation,
    /// Successful durable commit marker.
    Commit,
    /// Explicit transaction abort marker.
    Abort,
}

/// One logical persisted mutation target referenced by a WAL mutation entry.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DurableMutationTarget {
    /// Canonical record identifier touched by the mutation.
    pub record_id: PersistedRecordId,
}

/// One typed durable WAL entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DurableWalEntry {
    /// Transaction owning this WAL entry.
    pub transaction_id: DurableTransactionId,
    /// Monotonic entry sequence.
    pub sequence_number: WalSequenceNumber,
    /// Entry kind.
    pub kind: DurableWalEntryKind,
    /// Mutation targets for `Mutation` entries.
    pub mutation_targets: Vec<DurableMutationTarget>,
    /// Optional payload checksum.
    pub checksum: Option<RecordChecksum>,
}

/// Replay status resolved from a deterministic validation of one transaction WAL slice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DurableTransactionReplayStatus {
    /// The transaction has a valid commit marker.
    Committed,
    /// The transaction has a valid abort marker.
    Aborted,
    /// The transaction has no terminal marker.
    Incomplete,
}

/// Deterministic replay action for one validated transaction WAL slice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DurableReplayAction {
    /// Apply committed mutation effects.
    ApplyCommitted,
    /// Skip replay because the committed transaction was already applied.
    SkipDuplicate,
    /// Skip replay because the transaction is incomplete.
    SkipIncomplete,
    /// Skip replay because the transaction is explicitly aborted.
    SkipAborted,
}

/// WAL contract error model.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum WalContractError {
    /// Invalid transaction identifier.
    #[error("invalid durable transaction id: {reason}")]
    InvalidTransactionId {
        /// Rejection reason.
        reason: String,
    },
    /// Invalid sequence number.
    #[error("invalid WAL sequence number: {reason}")]
    InvalidSequenceNumber {
        /// Rejection reason.
        reason: String,
    },
    /// Invalid WAL entry.
    #[error("invalid WAL entry at sequence {sequence}: {reason}")]
    InvalidWalEntry {
        /// Entry sequence number.
        sequence: u64,
        /// Rejection reason.
        reason: String,
    },
    /// Invalid transaction WAL ordering.
    #[error("invalid transaction WAL ordering: {reason}")]
    InvalidOrdering {
        /// Rejection reason.
        reason: String,
    },
}

/// Validates one durable WAL entry against its entry-kind contract.
pub fn validate_durable_wal_entry(entry: &DurableWalEntry) -> Result<(), WalContractError> {
    if let Some(checksum) = &entry.checksum
        && (checksum.algorithm.trim().is_empty() || checksum.value.trim().is_empty())
    {
        return Err(WalContractError::InvalidWalEntry {
            sequence: entry.sequence_number.0,
            reason: "checksum algorithm and value must not be empty".to_owned(),
        });
    }

    match entry.kind {
        DurableWalEntryKind::Mutation if entry.mutation_targets.is_empty() => {
            Err(WalContractError::InvalidWalEntry {
                sequence: entry.sequence_number.0,
                reason: "mutation entry requires at least one mutation target".to_owned(),
            })
        }
        DurableWalEntryKind::Mutation => Ok(()),
        _ if !entry.mutation_targets.is_empty() => Err(WalContractError::InvalidWalEntry {
            sequence: entry.sequence_number.0,
            reason: "only mutation entries may carry mutation targets".to_owned(),
        }),
        _ => Ok(()),
    }
}

/// Validates one transaction WAL slice and classifies replay status deterministically.
pub fn classify_transaction_replay_status(
    entries: &[DurableWalEntry],
) -> Result<DurableTransactionReplayStatus, WalContractError> {
    let first = entries
        .first()
        .ok_or_else(|| WalContractError::InvalidOrdering {
            reason: "transaction WAL slice must not be empty".to_owned(),
        })?;
    if first.kind != DurableWalEntryKind::Begin {
        return Err(WalContractError::InvalidOrdering {
            reason: "transaction WAL slice must start with BEGIN".to_owned(),
        });
    }

    let transaction_id = &first.transaction_id;
    let mut previous_sequence = first.sequence_number.0;
    let mut terminal: Option<DurableTransactionReplayStatus> = None;

    for (index, entry) in entries.iter().enumerate() {
        validate_durable_wal_entry(entry)?;

        if &entry.transaction_id != transaction_id {
            return Err(WalContractError::InvalidOrdering {
                reason: "all WAL entries in one slice must share the same transaction id"
                    .to_owned(),
            });
        }

        if index > 0 {
            if entry.sequence_number.0 <= previous_sequence {
                return Err(WalContractError::InvalidOrdering {
                    reason: "WAL sequence numbers must be strictly increasing".to_owned(),
                });
            }
            previous_sequence = entry.sequence_number.0;
        }

        if index > 0 && entry.kind == DurableWalEntryKind::Begin {
            return Err(WalContractError::InvalidOrdering {
                reason: "BEGIN marker may appear only as the first transaction entry".to_owned(),
            });
        }

        if terminal.is_some() {
            return Err(WalContractError::InvalidOrdering {
                reason: "transaction WAL slice must not contain entries after terminal marker"
                    .to_owned(),
            });
        }

        match entry.kind {
            DurableWalEntryKind::Commit => {
                terminal = Some(DurableTransactionReplayStatus::Committed)
            }
            DurableWalEntryKind::Abort => terminal = Some(DurableTransactionReplayStatus::Aborted),
            DurableWalEntryKind::Begin | DurableWalEntryKind::Mutation => {}
        }
    }

    Ok(terminal.unwrap_or(DurableTransactionReplayStatus::Incomplete))
}

/// Resolves replay action from validated transaction status and duplicate knowledge.
pub fn classify_replay_action(
    status: DurableTransactionReplayStatus,
    already_applied: bool,
) -> DurableReplayAction {
    match (status, already_applied) {
        (DurableTransactionReplayStatus::Committed, false) => DurableReplayAction::ApplyCommitted,
        (DurableTransactionReplayStatus::Committed, true) => DurableReplayAction::SkipDuplicate,
        (DurableTransactionReplayStatus::Incomplete, _) => DurableReplayAction::SkipIncomplete,
        (DurableTransactionReplayStatus::Aborted, _) => DurableReplayAction::SkipAborted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PersistedRecordId, RecordChecksum, WalSequenceNumber};
    use graph_core::NodeId;

    fn transaction_id() -> DurableTransactionId {
        DurableTransactionId::new("tx--alpha").expect("fixture tx id should be valid")
    }

    fn checksum() -> RecordChecksum {
        RecordChecksum {
            algorithm: "sha256".to_owned(),
            value: "cafebabe".to_owned(),
        }
    }

    fn mutation_target() -> DurableMutationTarget {
        DurableMutationTarget {
            record_id: PersistedRecordId::Node(
                NodeId::new("node--42").expect("fixture node id should be valid"),
            ),
        }
    }

    fn entry(
        sequence: u64,
        kind: DurableWalEntryKind,
        mutation_targets: Vec<DurableMutationTarget>,
    ) -> DurableWalEntry {
        DurableWalEntry {
            transaction_id: transaction_id(),
            sequence_number: WalSequenceNumber::new(sequence)
                .expect("fixture sequence should be valid"),
            kind,
            mutation_targets,
            checksum: Some(checksum()),
        }
    }

    #[test]
    fn durable_transaction_id_rejects_empty_values() {
        let error = DurableTransactionId::new(" ").expect_err("empty tx id should fail");
        assert!(matches!(
            error,
            WalContractError::InvalidTransactionId { reason } if reason.contains("must not be empty")
        ));
    }

    #[test]
    fn wal_sequence_number_rejects_zero() {
        let error = WalSequenceNumber::new(0).expect_err("zero sequence should fail");
        assert!(matches!(
            error,
            WalContractError::InvalidSequenceNumber { reason } if reason.contains("greater than zero")
        ));
    }

    #[test]
    fn wal_entry_contract_requires_mutation_targets_for_mutation_kind() {
        let mutation = entry(2, DurableWalEntryKind::Mutation, vec![]);
        let error = validate_durable_wal_entry(&mutation)
            .expect_err("mutation without targets should fail");
        assert!(matches!(
            error,
            WalContractError::InvalidWalEntry { reason, .. } if reason.contains("requires at least one")
        ));
    }

    #[test]
    fn replay_status_classifies_complete_committed_slice() {
        let entries = vec![
            entry(1, DurableWalEntryKind::Begin, vec![]),
            entry(2, DurableWalEntryKind::Mutation, vec![mutation_target()]),
            entry(3, DurableWalEntryKind::Commit, vec![]),
        ];
        let status =
            classify_transaction_replay_status(&entries).expect("committed slice should validate");
        assert_eq!(status, DurableTransactionReplayStatus::Committed);
    }

    #[test]
    fn replay_status_classifies_incomplete_slice_without_terminal_marker() {
        let entries = vec![
            entry(1, DurableWalEntryKind::Begin, vec![]),
            entry(2, DurableWalEntryKind::Mutation, vec![mutation_target()]),
        ];
        let status = classify_transaction_replay_status(&entries)
            .expect("incomplete slice should still classify deterministically");
        assert_eq!(status, DurableTransactionReplayStatus::Incomplete);
    }

    #[test]
    fn replay_status_rejects_mixed_transaction_ids() {
        let mut other_tx = entry(2, DurableWalEntryKind::Mutation, vec![mutation_target()]);
        other_tx.transaction_id =
            DurableTransactionId::new("tx--beta").expect("fixture tx id should be valid");
        let entries = vec![entry(1, DurableWalEntryKind::Begin, vec![]), other_tx];

        let error = classify_transaction_replay_status(&entries)
            .expect_err("mixed transaction ids should fail");
        assert!(matches!(
            error,
            WalContractError::InvalidOrdering { reason } if reason.contains("same transaction id")
        ));
    }

    #[test]
    fn replay_status_rejects_non_monotonic_sequence_numbers() {
        let entries = vec![
            entry(2, DurableWalEntryKind::Begin, vec![]),
            entry(2, DurableWalEntryKind::Mutation, vec![mutation_target()]),
        ];
        let error = classify_transaction_replay_status(&entries)
            .expect_err("duplicate sequence should fail");
        assert!(matches!(
            error,
            WalContractError::InvalidOrdering { reason } if reason.contains("strictly increasing")
        ));
    }

    #[test]
    fn replay_action_is_idempotent_for_already_applied_commit() {
        let action = classify_replay_action(DurableTransactionReplayStatus::Committed, true);
        assert_eq!(action, DurableReplayAction::SkipDuplicate);
    }
}

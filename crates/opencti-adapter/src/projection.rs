// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! Transactional, rebuildable OpenCTI identifier projection contract.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{Identifier, RecordRef};

/// Canonical record state consumed by the identifier projection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionRecord {
    pub(crate) record_ref: RecordRef,
    pub(crate) revision: u64,
    pub(crate) identifiers: BTreeSet<Identifier>,
    pub(crate) deleted: bool,
}

impl ProjectionRecord {
    /// Stable record reference.
    pub const fn record_ref(&self) -> &RecordRef {
        &self.record_ref
    }

    /// Current canonical revision.
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Current identifier set.
    pub const fn identifiers(&self) -> &BTreeSet<Identifier> {
        &self.identifiers
    }
}

/// One source removed by an atomic merge.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeSource {
    pub(crate) record_ref: RecordRef,
    pub(crate) expected_revision: u64,
    pub(crate) tombstone_revision: u64,
}

impl MergeSource {
    /// Describe one source and its deterministic tombstone revision.
    pub fn new(record_ref: RecordRef, expected_revision: u64, tombstone_revision: u64) -> Self {
        Self {
            record_ref,
            expected_revision,
            tombstone_revision,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum IdentifierOperation {
    Upsert {
        record: ProjectionRecord,
        expected_revision: Option<u64>,
    },
    Delete {
        record_ref: RecordRef,
        expected_revision: u64,
        tombstone_revision: u64,
    },
    Merge {
        target: ProjectionRecord,
        expected_revision: Option<u64>,
        sources: Vec<MergeSource>,
    },
}

/// Replay-safe batch of identifier changes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentifierTransaction {
    id: String,
    operations: Vec<IdentifierOperation>,
}

impl IdentifierTransaction {
    /// Start an empty transaction with a stable replay identity.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            operations: Vec::new(),
        }
    }

    /// Append a create, update, or migration upsert.
    pub fn upsert(mut self, record: ProjectionRecord, expected_revision: Option<u64>) -> Self {
        self.operations.push(IdentifierOperation::Upsert {
            record,
            expected_revision,
        });
        self
    }

    /// Append a tombstone operation.
    pub fn delete(
        mut self,
        record_ref: RecordRef,
        expected_revision: u64,
        tombstone_revision: u64,
    ) -> Self {
        self.operations.push(IdentifierOperation::Delete {
            record_ref,
            expected_revision,
            tombstone_revision,
        });
        self
    }

    /// Append an atomic survivor/source merge.
    pub fn merge<I>(
        mut self,
        target: ProjectionRecord,
        expected_revision: Option<u64>,
        sources: I,
    ) -> Self
    where
        I: IntoIterator<Item = MergeSource>,
    {
        self.operations.push(IdentifierOperation::Merge {
            target,
            expected_revision,
            sources: sources.into_iter().collect(),
        });
        self
    }
}

/// Result of applying or replaying a transaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectionApply {
    /// Transaction changed projection state.
    Applied,
    /// Byte-equivalent logical transaction was already applied.
    Replayed,
}

/// In-memory identifier projection rebuilt from canonical graph records.
#[derive(Clone, Debug, Default)]
pub struct IdentifierProjection {
    by_identifier: BTreeMap<Identifier, RecordRef>,
    by_record: BTreeMap<RecordRef, ProjectionRecord>,
    applied_transactions: BTreeMap<String, IdentifierTransaction>,
}

impl IdentifierProjection {
    /// Empty projection.
    pub fn new() -> Self {
        Self::default()
    }

    /// Build the complete projection from current canonical records.
    pub fn rebuild<I>(records: I) -> Result<Self, ProjectionError>
    where
        I: IntoIterator<Item = ProjectionRecord>,
    {
        let mut records = records.into_iter().collect::<Vec<_>>();
        records.sort_by(|left, right| left.record_ref.cmp(&right.record_ref));
        let mut projection = Self::new();
        for record in records {
            projection.validate_record(&record)?;
            if projection.by_record.contains_key(&record.record_ref) {
                return Err(ProjectionError::InvalidInput {
                    reason: format!(
                        "duplicate canonical record {}",
                        record.record_ref.canonical_id()
                    ),
                });
            }
            if !record.deleted {
                projection.claim_identifiers(&record)?;
            }
            projection
                .by_record
                .insert(record.record_ref.clone(), record);
        }
        Ok(projection)
    }

    /// Atomically apply every ordered operation or leave the projection unchanged.
    pub fn apply(
        &mut self,
        transaction: IdentifierTransaction,
    ) -> Result<ProjectionApply, ProjectionError> {
        if transaction.id.trim().is_empty() {
            return Err(ProjectionError::InvalidInput {
                reason: "transaction identity cannot be empty".to_owned(),
            });
        }
        if transaction.operations.is_empty() {
            return Err(ProjectionError::InvalidInput {
                reason: format!("transaction {} has no operations", transaction.id),
            });
        }
        if let Some(applied) = self.applied_transactions.get(&transaction.id) {
            return if applied == &transaction {
                Ok(ProjectionApply::Replayed)
            } else {
                Err(ProjectionError::TransactionReplayConflict {
                    transaction_id: transaction.id,
                })
            };
        }

        let mut next = self.clone();
        for operation in &transaction.operations {
            match operation {
                IdentifierOperation::Upsert {
                    record,
                    expected_revision,
                } => next.apply_upsert(record.clone(), *expected_revision)?,
                IdentifierOperation::Delete {
                    record_ref,
                    expected_revision,
                    tombstone_revision,
                } => next.apply_delete(record_ref, *expected_revision, *tombstone_revision)?,
                IdentifierOperation::Merge {
                    target,
                    expected_revision,
                    sources,
                } => next.apply_merge(target.clone(), *expected_revision, sources)?,
            }
        }
        next.applied_transactions
            .insert(transaction.id.clone(), transaction);
        *self = next;
        Ok(ProjectionApply::Applied)
    }

    /// Resolve one typed identifier to the current canonical record.
    pub fn lookup(&self, kind: crate::IdentifierKind, value: &str) -> Option<&RecordRef> {
        let value = value.trim();
        if value.is_empty() {
            return None;
        }
        self.by_identifier.get(&Identifier {
            kind,
            value: value.to_owned(),
        })
    }

    /// Whether a canonical record has a current tombstone.
    pub fn is_deleted(&self, record_ref: &RecordRef) -> bool {
        self.by_record
            .get(record_ref)
            .is_some_and(|record| record.deleted)
    }

    /// Current non-tombstoned records in deterministic order.
    pub fn active_records(&self) -> Vec<ProjectionRecord> {
        self.by_record
            .values()
            .filter(|record| !record.deleted)
            .cloned()
            .collect()
    }

    /// Whether no current or tombstoned record state exists.
    pub fn is_empty(&self) -> bool {
        self.by_record.is_empty()
    }
}

impl IdentifierProjection {
    fn validate_record(&self, record: &ProjectionRecord) -> Result<(), ProjectionError> {
        if record.revision == 0 {
            return Err(ProjectionError::InvalidInput {
                reason: format!(
                    "record {} has revision zero",
                    record.record_ref.canonical_id()
                ),
            });
        }
        if !record.deleted && record.identifiers.is_empty() {
            return Err(ProjectionError::InvalidInput {
                reason: format!(
                    "record {} has no identifiers",
                    record.record_ref.canonical_id()
                ),
            });
        }
        if record.deleted && !record.identifiers.is_empty() {
            return Err(ProjectionError::InvalidInput {
                reason: format!(
                    "tombstoned record {} still owns identifiers",
                    record.record_ref.canonical_id()
                ),
            });
        }
        Ok(())
    }

    fn check_revision(
        &self,
        record_ref: &RecordRef,
        expected: Option<u64>,
    ) -> Result<Option<&ProjectionRecord>, ProjectionError> {
        let current = self.by_record.get(record_ref);
        let actual = current.map(|record| record.revision);
        if expected != actual {
            return Err(ProjectionError::RevisionConflict {
                record: record_ref.clone(),
                expected,
                actual,
            });
        }
        Ok(current)
    }

    fn release_identifiers(&mut self, record: &ProjectionRecord) {
        if record.deleted {
            return;
        }
        for identifier in &record.identifiers {
            if self
                .by_identifier
                .get(identifier)
                .is_some_and(|owner| owner == &record.record_ref)
            {
                self.by_identifier.remove(identifier);
            }
        }
    }

    fn claim_identifiers(&mut self, record: &ProjectionRecord) -> Result<(), ProjectionError> {
        for identifier in &record.identifiers {
            if let Some(existing) = self.by_identifier.get(identifier)
                && existing != &record.record_ref
            {
                return Err(ProjectionError::IdentifierConflict {
                    identifier: identifier.clone(),
                    existing: existing.clone(),
                    incoming: record.record_ref.clone(),
                });
            }
        }
        for identifier in &record.identifiers {
            self.by_identifier
                .insert(identifier.clone(), record.record_ref.clone());
        }
        Ok(())
    }

    fn apply_upsert(
        &mut self,
        record: ProjectionRecord,
        expected_revision: Option<u64>,
    ) -> Result<(), ProjectionError> {
        self.validate_record(&record)?;
        if record.deleted {
            return Err(ProjectionError::InvalidInput {
                reason: "upsert cannot carry a tombstone".to_owned(),
            });
        }
        let current = self
            .check_revision(&record.record_ref, expected_revision)?
            .cloned();
        if let Some(current) = &current
            && record.revision <= current.revision
        {
            return Err(ProjectionError::RevisionConflict {
                record: record.record_ref.clone(),
                expected: Some(current.revision.saturating_add(1)),
                actual: Some(record.revision),
            });
        }
        if let Some(current) = &current {
            self.release_identifiers(current);
        }
        self.claim_identifiers(&record)?;
        self.by_record.insert(record.record_ref.clone(), record);
        Ok(())
    }

    fn apply_delete(
        &mut self,
        record_ref: &RecordRef,
        expected_revision: u64,
        tombstone_revision: u64,
    ) -> Result<(), ProjectionError> {
        let current = self
            .check_revision(record_ref, Some(expected_revision))?
            .cloned()
            .ok_or_else(|| ProjectionError::RevisionConflict {
                record: record_ref.clone(),
                expected: Some(expected_revision),
                actual: None,
            })?;
        if current.deleted || tombstone_revision <= current.revision {
            return Err(ProjectionError::RevisionConflict {
                record: record_ref.clone(),
                expected: Some(current.revision.saturating_add(1)),
                actual: Some(tombstone_revision),
            });
        }
        self.release_identifiers(&current);
        self.by_record.insert(
            record_ref.clone(),
            ProjectionRecord {
                record_ref: record_ref.clone(),
                revision: tombstone_revision,
                identifiers: BTreeSet::new(),
                deleted: true,
            },
        );
        Ok(())
    }

    fn apply_merge(
        &mut self,
        target: ProjectionRecord,
        expected_revision: Option<u64>,
        sources: &[MergeSource],
    ) -> Result<(), ProjectionError> {
        if sources.is_empty() {
            return Err(ProjectionError::InvalidMerge {
                reason: "merge requires at least one source".to_owned(),
            });
        }
        let mut seen = BTreeSet::new();
        for source in sources {
            if source.record_ref == target.record_ref {
                return Err(ProjectionError::InvalidMerge {
                    reason: "merge target cannot also be a source".to_owned(),
                });
            }
            if !seen.insert(source.record_ref.clone()) {
                return Err(ProjectionError::InvalidMerge {
                    reason: format!(
                        "duplicate merge source {}",
                        source.record_ref.canonical_id()
                    ),
                });
            }
        }

        let current_target = self
            .check_revision(&target.record_ref, expected_revision)?
            .cloned();
        if let Some(current_target) = &current_target
            && target.revision <= current_target.revision
        {
            return Err(ProjectionError::RevisionConflict {
                record: target.record_ref.clone(),
                expected: Some(current_target.revision.saturating_add(1)),
                actual: Some(target.revision),
            });
        }
        let mut current_sources = Vec::with_capacity(sources.len());
        for source in sources {
            let current = self
                .check_revision(&source.record_ref, Some(source.expected_revision))?
                .cloned()
                .ok_or_else(|| ProjectionError::InvalidMerge {
                    reason: format!(
                        "merge source {} does not exist",
                        source.record_ref.canonical_id()
                    ),
                })?;
            if current.deleted || source.tombstone_revision <= current.revision {
                return Err(ProjectionError::RevisionConflict {
                    record: source.record_ref.clone(),
                    expected: Some(current.revision.saturating_add(1)),
                    actual: Some(source.tombstone_revision),
                });
            }
            current_sources.push((source, current));
        }

        if let Some(current_target) = &current_target {
            self.release_identifiers(current_target);
        }
        for (_, current) in &current_sources {
            self.release_identifiers(current);
        }
        self.validate_record(&target)?;
        self.claim_identifiers(&target)?;
        self.by_record.insert(target.record_ref.clone(), target);
        for (source, _) in current_sources {
            self.by_record.insert(
                source.record_ref.clone(),
                ProjectionRecord {
                    record_ref: source.record_ref.clone(),
                    revision: source.tombstone_revision,
                    identifiers: BTreeSet::new(),
                    deleted: true,
                },
            );
        }
        Ok(())
    }
}

/// Deterministic identifier projection failures.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ProjectionError {
    /// One identifier is already owned by another current record.
    #[error("identifier {identifier:?} is owned by {existing:?}, cannot assign it to {incoming:?}")]
    IdentifierConflict {
        /// Conflicting identifier.
        identifier: Identifier,
        /// Current owner.
        existing: RecordRef,
        /// Proposed owner.
        incoming: RecordRef,
    },
    /// Optimistic record revision did not match.
    #[error("record {record:?} revision conflict: expected {expected:?}, actual {actual:?}")]
    RevisionConflict {
        /// Conflicting record.
        record: RecordRef,
        /// Transaction precondition.
        expected: Option<u64>,
        /// Current projection revision.
        actual: Option<u64>,
    },
    /// A transaction ID was replayed with different operations.
    #[error("transaction {transaction_id} was replayed with different operations")]
    TransactionReplayConflict {
        /// Conflicting transaction identity.
        transaction_id: String,
    },
    /// Merge source or target invariants are invalid.
    #[error("invalid identifier merge: {reason}")]
    InvalidMerge {
        /// Safe validation detail.
        reason: String,
    },
    /// Transaction or record input is malformed.
    #[error("invalid identifier projection input: {reason}")]
    InvalidInput {
        /// Safe validation detail.
        reason: String,
    },
}

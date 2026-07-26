// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! OpenCTI compatibility mapping outside generic graph-core domain rules.

#![warn(missing_docs)]

mod mapping;
mod merge;
mod projection;
mod reconciliation;
mod synchronization;
mod transactional_write;

pub use mapping::{
    AccessMetadata, Identifier, IdentifierKind, MappedObject, MappedRecord, MappedRelationship,
    MappingError, MappingVersion, OpenCtiAdapter, OpenCtiTimestamps, Provenance, RecordFamily,
    RecordKind, RecordRef, Reference,
};
pub use merge::{
    MergeConflict, MergeError, MergeLimits, OpenCtiMergeExecutor, OpenCtiMergeOutcome,
    OpenCtiMergeRequest,
};
pub use projection::{
    IdentifierProjection, IdentifierTransaction, MergeSource, ProjectionApply, ProjectionError,
    ProjectionRecord,
};
pub use reconciliation::{
    DivergenceKind, OpenCtiReconciler, OpenCtiReconciliationCommand, OpenCtiReconciliationOutcome,
    ReconciliationDifference, ReconciliationError, ReconciliationLimits, ReconciliationMode,
    ReconciliationReport, ReconciliationScope, RepairAction,
};
pub use synchronization::{
    BulkLimits, DeadLetterRecord, DivergenceStatus, GraphDigest, MutationClass, OpenCtiMutation,
    OpenCtiSyncBatch, OpenCtiSynchronizer, OperationResult, OperationStatus, SyncBatchResult,
    SyncCheckpoint, SyncError, SyncPhase, SyncValidationReport,
};
pub use transactional_write::{
    OpenCtiWriteBatch, OpenCtiWriteBatchOutcome, OpenCtiWriteExecutor, OpenCtiWriteOperation,
    OpenCtiWriteOperationKind, WriteError, WriteLimits, WriteOperationOutcome,
    WriteOperationStatus,
};

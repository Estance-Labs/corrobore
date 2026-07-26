// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! Durable Corrobore-primary OpenCTI writes, reference projection, and legacy recovery.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use corrobore_engine::{
    BulkResult, KnowledgeDataError, KnowledgeDataErrorCode, KnowledgeDataOperation,
    KnowledgeDataRequest, KnowledgeDataResponse, MergeResult, RequestContext, WriteResult,
};
use graph_storage::{
    CanonicalEngineStore, CanonicalProjectionRequest, DurableTransactionId,
    read_atomic_persistent_audit_events,
};
use opencti_adapter::{
    MergeError, MergeLimits, OpenCtiAdapter, OpenCtiMergeExecutor, OpenCtiMergeRequest,
    OpenCtiWriteBatch, OpenCtiWriteExecutor, OpenCtiWriteOperation, WriteError, WriteLimits,
    WriteOperationOutcome, WriteOperationStatus,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

const STATE_SCHEMA_VERSION: u32 = 2;
const RECEIPT_SCHEMA_VERSION: u32 = 1;
const MAX_RECONCILIATION_ATTEMPTS: u32 = 3;

/// Outcome observed after attempting both migration-period write targets.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DualWriteOutcome {
    /// Non-reversible hash of the caller idempotency identity.
    pub idempotency_key_hash: String,
    /// Payload-free request correlation identity.
    pub correlation_id: String,
    /// Whether the authoritative reference acknowledged the write.
    pub reference_applied: bool,
    /// Whether Corrobore durably acknowledged the write.
    pub corrobore_applied: bool,
    /// Safe bounded failure diagnostic.
    pub diagnostic: Option<String>,
}

/// Durable reconciliation lifecycle for a partial dual write.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconciliationStatus {
    /// One provider still requires replay or operator quarantine handling.
    Pending,
    /// Both providers have acknowledged the same logical mutation.
    Reconciled,
    /// Automatic retries are exhausted and operator action is required.
    Quarantined,
}

/// One durable partial-write reconciliation record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReconciliationRecord {
    /// Non-reversible idempotency identity.
    pub idempotency_key_hash: String,
    /// Payload-free correlation identity.
    pub correlation_id: String,
    /// Current reconciliation lifecycle.
    pub status: ReconciliationStatus,
    /// Number of bounded replay attempts.
    pub attempts: u32,
    /// Safe bounded diagnostic.
    pub diagnostic: Option<String>,
}

/// WAL-bound mutation audit decoded for authenticated operator inspection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenCtiWriteAuditRecord {
    /// Non-reversible idempotency identity.
    pub idempotency_key_hash: String,
    /// Payload-free request correlation identity.
    pub correlation_id: String,
    /// Optional connector offset preserved without source content.
    pub source_offset: Option<String>,
    /// Revision observed before mutation.
    pub before_revision: Option<u64>,
    /// Revision committed by mutation.
    pub after_revision: Option<u64>,
    /// Stable outcome classification.
    pub outcome: String,
}

/// Bounded operational write/reconciliation summary.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenCtiWriteStatus {
    /// Partial writes still requiring reconciliation.
    pub pending_reconciliation: usize,
    /// Whether no pending or quarantined partial writes remain.
    pub fully_reconciled: bool,
    /// Canonical mutation items durably applied in this process.
    pub applied_operations: u64,
    /// Rejected, conflicting, retryable, or aborted items observed in this process.
    pub failed_operations: u64,
    /// Transactions served from a durable idempotency receipt.
    pub idempotent_replays: u64,
    /// Partial writes isolated after exhausting automatic retries.
    pub quarantined_reconciliation: usize,
    /// Accepted canonical writes waiting for verified reference projection.
    pub projection_outbox_depth: usize,
    /// Number of accepted sequences not yet verified on the reference.
    pub projection_lag: u64,
    /// Projection attempts retried after a reference failure.
    pub projection_retries: u64,
    /// Projection records isolated after an outcome divergence.
    pub projection_quarantined: usize,
    /// Current exclusive write authority.
    pub write_authority: WriteAuthority,
    /// Whether canonical and reference state can currently be claimed synchronized.
    pub fully_synchronized: bool,
    /// Lossless reconstruction plans generated for operator rebuilds.
    pub reconstruction_runs: u64,
}

/// Exclusive authority used for OpenCTI mutations.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteAuthority {
    /// Corrobore commits canonical state before acknowledging a mutation.
    #[default]
    CorroborePrimary,
    /// Mutations are rejected while rollback safety gates are evaluated.
    WritesSuspended,
    /// The reference provider is authoritative after a verified rollback.
    ReferencePrimary,
}

/// Durable reference-projection lifecycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionStatus {
    /// Outbox intent is durable, while the canonical transaction is unresolved.
    Prepared,
    /// Canonical commit is durable and reference delivery is pending.
    Pending,
    /// Reference returned the exact canonical result.
    Delivered,
    /// Reference returned a divergent result and requires operator action.
    Quarantined,
}

/// One ordered, replay-safe reference projection record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectionRecord {
    /// Monotonic global ordering sequence.
    pub sequence: u64,
    /// Stable entity or transaction ordering key.
    pub ordering_key: String,
    /// Durable transaction identity used for crash recovery.
    pub transaction_id: String,
    /// Typed request with a non-reversible idempotency identity.
    pub request: KnowledgeDataRequest,
    /// Canonical result that reference delivery must match exactly.
    pub expected_response: Option<KnowledgeDataResponse>,
    /// Current delivery lifecycle.
    pub status: ProjectionStatus,
    /// Failed delivery attempts.
    pub attempts: u32,
    /// Safe bounded diagnostic for the last failed attempt.
    pub diagnostic: Option<String>,
}

/// Payload-free projection state safe for authenticated operator inspection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionRecordSummary {
    /// Monotonic global ordering sequence.
    pub sequence: u64,
    /// Non-reversible identity of the entity or transaction ordering key.
    pub ordering_key_hash: String,
    /// Current delivery lifecycle.
    pub status: ProjectionStatus,
    /// Failed delivery attempts.
    pub attempts: u32,
    /// Safe bounded diagnostic for the last failed attempt.
    pub diagnostic: Option<String>,
}

/// Safety signals that may initiate a write-authority rollback.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RollbackTrigger {
    /// Access-control or security outcome divergence.
    SecurityDivergence,
    /// Canonical corruption or failed integrity verification.
    Corruption,
    /// Primary write latency exceeded the approved envelope.
    LatencyRegression,
    /// Schema or data migration failed.
    MigrationFailure,
    /// Reference projection returned a divergent mutation result.
    WriteDivergence,
    /// Required reference availability is degraded.
    ReferenceAvailability,
}

/// Evidence required before assigning authority back to the reference.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityTransitionReadiness {
    /// Reference write endpoint passed its health gate.
    pub reference_healthy: bool,
    /// Every accepted canonical write was replayed.
    pub replay_complete: bool,
    /// The approved parity corpus matches both providers.
    pub parity_verified: bool,
}

/// Complete, lossless reference reconstruction input.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReferenceReconstructionPlan {
    /// Canonical high-water sequence captured with the export.
    pub high_water_sequence: u64,
    /// Losslessly restored OpenCTI records.
    pub records: Vec<Value>,
}

/// Durable direct-write coordinator restored before server readiness.
#[derive(Clone, Debug)]
pub struct OpenCtiWriteRuntime {
    state_path: Option<PathBuf>,
    limits: WriteLimits,
    max_reconciliation_records: usize,
    reconciliations: Vec<ReconciliationRecord>,
    audits: Vec<OpenCtiWriteAuditRecord>,
    applied_operations: u64,
    failed_operations: u64,
    idempotent_replays: u64,
    projection_records: Vec<ProjectionRecord>,
    next_projection_sequence: u64,
    projection_retries: u64,
    write_authority: WriteAuthority,
    rollback_trigger: Option<RollbackTrigger>,
    reconstruction_runs: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PersistedWriteState {
    schema_version: u32,
    reconciliations: Vec<ReconciliationRecord>,
    #[serde(default)]
    projection_records: Vec<ProjectionRecord>,
    #[serde(default = "initial_projection_sequence")]
    next_projection_sequence: u64,
    #[serde(default)]
    projection_retries: u64,
    #[serde(default)]
    write_authority: WriteAuthority,
    #[serde(default)]
    rollback_trigger: Option<RollbackTrigger>,
    #[serde(default)]
    reconstruction_runs: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PersistedWriteReceipt {
    schema_version: u32,
    idempotency_key_hash: String,
    operation_fingerprint: String,
    response: KnowledgeDataResponse,
    audits: Vec<OpenCtiWriteAuditRecord>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
enum BulkItem {
    Create {
        #[serde(default)]
        operation_id: Option<String>,
        record: Value,
    },
    Update {
        #[serde(default)]
        operation_id: Option<String>,
        id: String,
        #[serde(default)]
        expected_revision: Option<u64>,
        patch: Value,
    },
    AccessPolicyUpdate {
        #[serde(default)]
        operation_id: Option<String>,
        id: String,
        #[serde(default)]
        expected_revision: Option<u64>,
        patch: Value,
    },
    Delete {
        #[serde(default)]
        operation_id: Option<String>,
        id: String,
        #[serde(default)]
        expected_revision: Option<u64>,
    },
}

impl OpenCtiWriteRuntime {
    /// Hash an idempotency key for audit and reconciliation without retaining
    /// the original credential-like caller value.
    pub fn hash_idempotency_key(value: &str) -> String {
        hash_text(value)
    }

    /// Restore bounded reconciliation state and configure direct-write limits.
    pub fn open(
        state_path: Option<PathBuf>,
        limits: WriteLimits,
        max_reconciliation_records: usize,
    ) -> Result<Self, String> {
        if max_reconciliation_records == 0 {
            return Err("max_reconciliation_records must be greater than zero".to_owned());
        }
        let persisted = state_path
            .as_deref()
            .filter(|path| path.is_file())
            .map(read_state)
            .transpose()?;
        if persisted
            .as_ref()
            .is_some_and(|state| !matches!(state.schema_version, 1 | STATE_SCHEMA_VERSION))
        {
            return Err("unsupported OpenCTI write state version".to_owned());
        }
        let persisted = persisted.unwrap_or(PersistedWriteState {
            schema_version: STATE_SCHEMA_VERSION,
            reconciliations: Vec::new(),
            projection_records: Vec::new(),
            next_projection_sequence: initial_projection_sequence(),
            projection_retries: 0,
            write_authority: WriteAuthority::CorroborePrimary,
            rollback_trigger: None,
            reconstruction_runs: 0,
        });
        let legacy_unresolved = persisted.schema_version == 1
            && persisted
                .reconciliations
                .iter()
                .any(|record| record.status != ReconciliationStatus::Reconciled);
        let write_authority = if legacy_unresolved {
            WriteAuthority::WritesSuspended
        } else {
            persisted.write_authority
        };
        let rollback_trigger = if legacy_unresolved {
            Some(RollbackTrigger::MigrationFailure)
        } else {
            persisted.rollback_trigger
        };
        Ok(Self {
            state_path,
            limits,
            max_reconciliation_records,
            reconciliations: persisted.reconciliations,
            audits: Vec::new(),
            applied_operations: 0,
            failed_operations: 0,
            idempotent_replays: 0,
            projection_records: persisted.projection_records,
            next_projection_sequence: persisted.next_projection_sequence,
            projection_retries: persisted.projection_retries,
            write_authority,
            rollback_trigger,
            reconstruction_runs: persisted.reconstruction_runs,
        })
    }

    /// Persist a sanitized projection intent before canonical mutation commit.
    pub fn prepare_projection(&mut self, request: &KnowledgeDataRequest) -> Result<u64, String> {
        if self.write_authority != WriteAuthority::CorroborePrimary {
            return Err(format!(
                "OpenCTI writes are not accepted while authority is {:?}",
                self.write_authority
            ));
        }
        let idempotency_key = request
            .context
            .idempotency_key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .ok_or_else(|| "transactional mutation requires an idempotency_key".to_owned())?;
        let idempotency_hash = hash_text(idempotency_key);
        let transaction_id = format!(
            "tx--opencti-write-{}",
            idempotency_hash.trim_start_matches("sha256:")
        );
        if let Some(existing) = self
            .projection_records
            .iter()
            .find(|record| record.transaction_id == transaction_id)
        {
            if existing.request.operation != request.operation {
                return Err(
                    "idempotency key was reused with a different projection payload".to_owned(),
                );
            }
            return Ok(existing.sequence);
        }
        let active_records = self
            .projection_records
            .iter()
            .filter(|record| record.status != ProjectionStatus::Delivered)
            .count();
        if active_records >= self.max_reconciliation_records {
            return Err(
                "projection outbox backpressure: pending record capacity is exhausted".to_owned(),
            );
        }
        while self.projection_records.len() >= self.max_reconciliation_records {
            let Some(index) = self
                .projection_records
                .iter()
                .position(|record| record.status == ProjectionStatus::Delivered)
            else {
                break;
            };
            self.projection_records.remove(index);
        }
        let sequence = self.next_projection_sequence;
        let mut sanitized = request.clone();
        sanitized.context.idempotency_key = Some(idempotency_hash);
        self.projection_records.push(ProjectionRecord {
            sequence,
            ordering_key: projection_ordering_key(&request.operation, sequence),
            transaction_id,
            request: sanitized,
            expected_response: None,
            status: ProjectionStatus::Prepared,
            attempts: 0,
            diagnostic: None,
        });
        self.next_projection_sequence = self.next_projection_sequence.saturating_add(1);
        if let Err(error) = self.persist() {
            self.projection_records.pop();
            self.next_projection_sequence = sequence;
            return Err(error);
        }
        Ok(sequence)
    }

    /// Mark a prepared projection eligible for ordered reference delivery.
    pub fn activate_projection(
        &mut self,
        sequence: u64,
        response: KnowledgeDataResponse,
    ) -> Result<(), String> {
        let record = self.projection_record_mut(sequence)?;
        if record.status != ProjectionStatus::Prepared {
            if record.expected_response.as_ref() == Some(&response)
                && matches!(
                    record.status,
                    ProjectionStatus::Pending | ProjectionStatus::Delivered
                )
            {
                return Ok(());
            }
            return Err("projection replay does not match its canonical receipt".to_owned());
        }
        record.expected_response = Some(response);
        record.status = ProjectionStatus::Pending;
        record.diagnostic = None;
        self.persist()
    }

    /// Recover prepared projection intents against canonical WAL receipts.
    pub fn recover_projection_outbox(
        &mut self,
        store: &mut CanonicalEngineStore,
    ) -> Result<(), String> {
        let prepared = self
            .projection_records
            .iter()
            .filter(|record| record.status == ProjectionStatus::Prepared)
            .map(|record| (record.sequence, record.transaction_id.clone()))
            .collect::<Vec<_>>();
        let mut abandoned = Vec::new();
        for (sequence, transaction_id) in prepared {
            let transaction_id =
                DurableTransactionId::new(transaction_id).map_err(|error| error.to_string())?;
            let events = read_atomic_persistent_audit_events(store.root(), &transaction_id)
                .map_err(|error| error.to_string())?;
            if events.is_empty() {
                abandoned.push(sequence);
                continue;
            }
            let receipt = events
                .iter()
                .find_map(|event| serde_json::from_str::<PersistedWriteReceipt>(event).ok())
                .ok_or_else(|| {
                    format!(
                        "committed projection sequence {sequence} has no readable write receipt"
                    )
                })?;
            if receipt.schema_version != RECEIPT_SCHEMA_VERSION {
                return Err(format!(
                    "projection sequence {sequence} has an unsupported write receipt"
                ));
            }
            let record = self.projection_record_mut(sequence)?;
            record.expected_response = Some(receipt.response);
            record.status = ProjectionStatus::Pending;
            record.diagnostic = Some("recovered committed write after restart".to_owned());
        }
        self.projection_records
            .retain(|record| !abandoned.contains(&record.sequence));
        self.next_projection_sequence = self.next_projection_sequence.max(
            self.projection_records
                .iter()
                .map(|record| record.sequence.saturating_add(1))
                .max()
                .unwrap_or(initial_projection_sequence()),
        );
        self.persist()
    }

    /// Return the oldest canonical write that still needs reference delivery.
    pub fn pending_projection(&self) -> Option<&ProjectionRecord> {
        self.projection_records
            .iter()
            .find(|record| record.status != ProjectionStatus::Delivered)
            .filter(|record| record.status == ProjectionStatus::Pending)
    }

    /// Return ordered durable projection state for authenticated operators.
    pub fn projection_records(&self) -> &[ProjectionRecord] {
        &self.projection_records
    }

    /// Return payload-free ordered projection state for authenticated operators.
    pub fn projection_summaries(&self) -> Vec<ProjectionRecordSummary> {
        self.projection_records
            .iter()
            .map(|record| ProjectionRecordSummary {
                sequence: record.sequence,
                ordering_key_hash: hash_text(&record.ordering_key),
                status: record.status,
                attempts: record.attempts,
                diagnostic: record.diagnostic.clone(),
            })
            .collect()
    }

    /// Record a retryable reference projection failure.
    pub fn record_projection_failure(
        &mut self,
        sequence: u64,
        diagnostic: &str,
    ) -> Result<(), String> {
        let record = self.projection_record_mut(sequence)?;
        if record.status != ProjectionStatus::Pending {
            return Err("only a pending projection can record a retry".to_owned());
        }
        record.attempts = record.attempts.saturating_add(1);
        record.diagnostic = Some(bounded_diagnostic(diagnostic));
        self.projection_retries = self.projection_retries.saturating_add(1);
        self.persist()
    }

    /// Verify the exact reference result before marking a projection delivered.
    pub fn verify_projection(
        &mut self,
        sequence: u64,
        actual: &KnowledgeDataResponse,
    ) -> Result<(), String> {
        let record = self.projection_record_mut(sequence)?;
        if record.status == ProjectionStatus::Delivered {
            return Ok(());
        }
        if record.status != ProjectionStatus::Pending {
            return Err("only a pending projection can be verified".to_owned());
        }
        let matches = record.expected_response.as_ref() == Some(actual);
        if matches {
            record.status = ProjectionStatus::Delivered;
            record.diagnostic = None;
            self.persist()?;
            return Ok(());
        }
        record.status = ProjectionStatus::Quarantined;
        record.diagnostic =
            Some("reference projection outcome diverged from canonical receipt".to_owned());
        self.write_authority = WriteAuthority::WritesSuspended;
        self.rollback_trigger = Some(RollbackTrigger::WriteDivergence);
        self.persist()?;
        Err("reference projection outcome diverged from canonical receipt".to_owned())
    }

    /// Export all canonical records losslessly for a clean reference rebuild.
    pub fn reconstruction_plan(
        &mut self,
        store: &mut CanonicalEngineStore,
    ) -> Result<ReferenceReconstructionPlan, String> {
        let graph = store
            .load_projection(CanonicalProjectionRequest::all())
            .map_err(|error| error.to_string())?;
        let adapter = OpenCtiAdapter::pinned();
        let mut records = graph
            .list_nodes()
            .map_err(|error| error.to_string())?
            .iter()
            .map(|node| {
                adapter
                    .restore_node(node)
                    .map(|record| record.raw().clone())
                    .map_err(|error| error.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        records.extend(
            graph
                .list_relationships()
                .map_err(|error| error.to_string())?
                .iter()
                .map(|relationship| {
                    adapter
                        .restore_relationship(relationship)
                        .map(|record| record.raw().clone())
                        .map_err(|error| error.to_string())
                })
                .collect::<Result<Vec<_>, _>>()?,
        );
        records.sort_by(|left, right| {
            record_identifier(left)
                .cmp(&record_identifier(right))
                .then_with(|| left.to_string().cmp(&right.to_string()))
        });
        let plan = ReferenceReconstructionPlan {
            high_water_sequence: self.next_projection_sequence.saturating_sub(1),
            records,
        };
        self.reconstruction_runs = self.reconstruction_runs.saturating_add(1);
        self.persist()?;
        Ok(plan)
    }

    /// Stop accepting writes immediately after a declared rollback trigger.
    pub fn suspend_writes(&mut self, trigger: RollbackTrigger) -> Result<(), String> {
        self.write_authority = WriteAuthority::WritesSuspended;
        self.rollback_trigger = Some(trigger);
        self.persist()
    }

    /// Assign write authority only after the target-specific safety gates pass.
    pub fn transition_authority(
        &mut self,
        target: WriteAuthority,
        readiness: AuthorityTransitionReadiness,
    ) -> Result<(), String> {
        match target {
            WriteAuthority::WritesSuspended => {
                self.write_authority = target;
            }
            WriteAuthority::ReferencePrimary => {
                if self.write_authority != WriteAuthority::WritesSuspended {
                    return Err(
                        "writes must be suspended before assigning reference authority".to_owned(),
                    );
                }
                if !readiness.reference_healthy {
                    return Err("reference health gate is not verified".to_owned());
                }
                if !readiness.replay_complete || self.has_pending_projection() {
                    return Err("projection replay is not complete".to_owned());
                }
                if !readiness.parity_verified {
                    return Err("reference parity corpus is not verified".to_owned());
                }
                for record in &mut self.projection_records {
                    if record.status == ProjectionStatus::Quarantined {
                        record.status = ProjectionStatus::Delivered;
                        record.diagnostic = Some(
                            "resolved by operator-verified full parity during rollback".to_owned(),
                        );
                    }
                }
                self.write_authority = target;
            }
            WriteAuthority::CorroborePrimary => {
                if !readiness.replay_complete
                    || !readiness.parity_verified
                    || self.has_pending_projection()
                {
                    return Err(
                        "canonical authority requires complete replay and verified parity"
                            .to_owned(),
                    );
                }
                for record in &mut self.projection_records {
                    if record.status == ProjectionStatus::Quarantined {
                        record.status = ProjectionStatus::Delivered;
                        record.diagnostic = Some(
                            "resolved by operator-verified full parity before primary resume"
                                .to_owned(),
                        );
                    }
                }
                self.write_authority = target;
                self.rollback_trigger = None;
            }
        }
        self.persist()
    }

    /// Remove a projection intent after a canonical mutation was proven unsuccessful.
    pub fn abort_projection(&mut self, sequence: u64) -> Result<(), String> {
        let Some(index) = self
            .projection_records
            .iter()
            .position(|record| record.sequence == sequence)
        else {
            return Ok(());
        };
        if self.projection_records[index].status != ProjectionStatus::Prepared {
            return Err("only an uncommitted projection prepare can be aborted".to_owned());
        }
        self.projection_records.remove(index);
        self.persist()
    }

    fn projection_record_mut(&mut self, sequence: u64) -> Result<&mut ProjectionRecord, String> {
        self.projection_records
            .iter_mut()
            .find(|record| record.sequence == sequence)
            .ok_or_else(|| format!("unknown projection sequence {sequence}"))
    }

    fn has_pending_projection(&self) -> bool {
        self.projection_records.iter().any(|record| {
            matches!(
                record.status,
                ProjectionStatus::Prepared | ProjectionStatus::Pending
            )
        })
    }

    /// Validate a typed mutation, recover or create its WAL-bound idempotency
    /// receipt, atomically commit canonical records/projections, and only then
    /// return the original stable contract response.
    pub fn apply(
        &mut self,
        store: &mut CanonicalEngineStore,
        operation: &KnowledgeDataOperation,
        context: &RequestContext,
    ) -> Result<KnowledgeDataResponse, KnowledgeDataError> {
        let idempotency_key = context
            .idempotency_key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .ok_or_else(|| invalid("transactional mutation requires an idempotency_key"))?;
        let idempotency_hash = hash_text(idempotency_key);
        let transaction_id = DurableTransactionId::new(format!(
            "tx--opencti-write-{}",
            idempotency_hash.trim_start_matches("sha256:")
        ))
        .map_err(|error| invalid(&error.to_string()))?;
        let fingerprint = fingerprint_operation(operation)?;

        let existing = read_atomic_persistent_audit_events(store.root(), &transaction_id)
            .map_err(|error| unavailable(&error.to_string()))?;
        if !existing.is_empty() {
            return self.replay_receipt(existing, &idempotency_hash, &fingerprint);
        }

        if let KnowledgeDataOperation::Merge(request) = operation {
            return self.apply_merge(
                store,
                request,
                context,
                transaction_id,
                idempotency_hash,
                fingerprint,
            );
        }

        let batch = write_batch(operation, context, transaction_id.value.clone())?;
        let previous = store
            .load_projection(CanonicalProjectionRequest::all())
            .map_err(|error| unavailable(&error.to_string()))?;
        let planned = OpenCtiWriteExecutor::new(self.limits)
            .apply(&previous, &batch)
            .map_err(map_write_error)?;
        let response = match response_for(operation, &planned.operations, planned.committed) {
            Ok(response) => response,
            Err(error) => {
                self.record_operation_counts(&planned.operations);
                return Err(error);
            }
        };
        if !planned.committed {
            self.record_operation_counts(&planned.operations);
            return Ok(response);
        }

        let audits = audit_records(context, &idempotency_hash, &planned.operations);
        let receipt = PersistedWriteReceipt {
            schema_version: RECEIPT_SCHEMA_VERSION,
            idempotency_key_hash: idempotency_hash,
            operation_fingerprint: fingerprint,
            response: response.clone(),
            audits: audits.clone(),
        };
        let receipt = serde_json::to_string(&receipt)
            .map_err(|error| unavailable(&format!("failed to encode write receipt: {error}")))?;
        store
            .commit_transition_with_audit(
                &previous,
                &planned.graph,
                transaction_id,
                vec![receipt],
                None,
            )
            .map_err(|error| unavailable(&error.to_string()))?;
        self.record_operation_counts(&planned.operations);
        self.audits.extend(audits);
        Ok(response)
    }

    fn apply_merge(
        &mut self,
        store: &mut CanonicalEngineStore,
        request: &corrobore_engine::MergeRequest,
        context: &RequestContext,
        transaction_id: DurableTransactionId,
        idempotency_hash: String,
        fingerprint: String,
    ) -> Result<KnowledgeDataResponse, KnowledgeDataError> {
        let previous = store
            .load_projection(CanonicalProjectionRequest::all())
            .map_err(|error| unavailable(&error.to_string()))?;
        let merge_request = OpenCtiMergeRequest::new(
            context.request_id.clone(),
            request.target_id.clone(),
            request.source_ids.clone(),
            request.expected_revisions.clone(),
        )
        .map_err(map_merge_error)?;
        let outcome = OpenCtiMergeExecutor::new(MergeLimits {
            max_sources: self.limits.max_operations,
            ..MergeLimits::default()
        })
        .apply(&previous, &merge_request)
        .map_err(map_merge_error)?;
        let response = KnowledgeDataResponse::Merge(MergeResult {
            target_id: outcome.target_id.clone(),
            target_revision: outcome.target_revision,
            deleted_source_ids: outcome.deleted_source_ids.clone(),
            redirected_relationship_ids: outcome.redirected_relationship_ids.clone(),
            redirected_reference_ids: outcome.redirected_reference_ids.clone(),
            deduplicated_relationship_ids: outcome.deduplicated_relationship_ids.clone(),
            conflict_count: outcome.conflicts.len() as u64,
        });
        let mut audits = vec![OpenCtiWriteAuditRecord {
            idempotency_key_hash: idempotency_hash.clone(),
            correlation_id: context.correlation_id.clone(),
            source_offset: context.access.attributes.get("source_offset").cloned(),
            before_revision: outcome.target_revision.checked_sub(1),
            after_revision: Some(outcome.target_revision),
            outcome: "merged_survivor".to_owned(),
        }];
        audits.extend(
            outcome
                .deleted_source_ids
                .iter()
                .map(|source_id| OpenCtiWriteAuditRecord {
                    idempotency_key_hash: idempotency_hash.clone(),
                    correlation_id: context.correlation_id.clone(),
                    source_offset: context.access.attributes.get("source_offset").cloned(),
                    before_revision: request.expected_revisions.get(source_id).copied(),
                    after_revision: None,
                    outcome: "merged_source_tombstoned".to_owned(),
                }),
        );
        let receipt = PersistedWriteReceipt {
            schema_version: RECEIPT_SCHEMA_VERSION,
            idempotency_key_hash: idempotency_hash,
            operation_fingerprint: fingerprint,
            response: response.clone(),
            audits: audits.clone(),
        };
        let receipt = serde_json::to_string(&receipt)
            .map_err(|error| unavailable(&format!("failed to encode merge receipt: {error}")))?;
        store
            .commit_transition_with_audit(
                &previous,
                &outcome.graph,
                transaction_id,
                vec![receipt],
                None,
            )
            .map_err(|error| unavailable(&error.to_string()))?;
        self.applied_operations = self.applied_operations.saturating_add(1);
        self.audits.extend(audits);
        Ok(response)
    }

    /// Persist partial dual-write state before any response can claim complete
    /// reconciliation. Equivalent replays update rather than duplicate state.
    pub fn record_dual_write(&mut self, outcome: DualWriteOutcome) -> Result<(), String> {
        if outcome.idempotency_key_hash.trim().is_empty()
            || outcome.correlation_id.trim().is_empty()
        {
            return Err("dual-write identity and correlation cannot be blank".to_owned());
        }
        let reconciled = outcome.reference_applied && outcome.corrobore_applied;
        if let Some(record) = self
            .reconciliations
            .iter_mut()
            .find(|record| record.idempotency_key_hash == outcome.idempotency_key_hash)
        {
            record.correlation_id = outcome.correlation_id;
            record.attempts = record.attempts.saturating_add(1);
            record.status = if reconciled {
                ReconciliationStatus::Reconciled
            } else if record.attempts >= MAX_RECONCILIATION_ATTEMPTS {
                ReconciliationStatus::Quarantined
            } else {
                ReconciliationStatus::Pending
            };
            record.diagnostic = outcome.diagnostic;
        } else {
            if self.reconciliations.len() == self.max_reconciliation_records {
                if let Some(index) = self
                    .reconciliations
                    .iter()
                    .position(|record| record.status == ReconciliationStatus::Reconciled)
                {
                    self.reconciliations.remove(index);
                } else {
                    return Err(
                        "reconciliation backpressure: pending record capacity is exhausted"
                            .to_owned(),
                    );
                }
            }
            self.reconciliations.push(ReconciliationRecord {
                idempotency_key_hash: outcome.idempotency_key_hash,
                correlation_id: outcome.correlation_id,
                status: if reconciled {
                    ReconciliationStatus::Reconciled
                } else {
                    ReconciliationStatus::Pending
                },
                attempts: 1,
                diagnostic: outcome.diagnostic,
            });
        }
        self.persist()
    }

    /// Current bounded reconciliation summary.
    pub fn status(&self) -> OpenCtiWriteStatus {
        OpenCtiWriteStatus {
            pending_reconciliation: self
                .reconciliations
                .iter()
                .filter(|record| record.status == ReconciliationStatus::Pending)
                .count(),
            fully_reconciled: self
                .reconciliations
                .iter()
                .all(|record| record.status == ReconciliationStatus::Reconciled),
            applied_operations: self.applied_operations,
            failed_operations: self.failed_operations,
            idempotent_replays: self.idempotent_replays,
            quarantined_reconciliation: self
                .reconciliations
                .iter()
                .filter(|record| record.status == ReconciliationStatus::Quarantined)
                .count(),
            projection_outbox_depth: self
                .projection_records
                .iter()
                .filter(|record| {
                    matches!(
                        record.status,
                        ProjectionStatus::Prepared | ProjectionStatus::Pending
                    )
                })
                .count(),
            projection_lag: self
                .projection_records
                .iter()
                .filter(|record| record.status != ProjectionStatus::Delivered)
                .count() as u64,
            projection_retries: self.projection_retries,
            projection_quarantined: self
                .projection_records
                .iter()
                .filter(|record| record.status == ProjectionStatus::Quarantined)
                .count(),
            write_authority: self.write_authority,
            fully_synchronized: self
                .reconciliations
                .iter()
                .all(|record| record.status == ReconciliationStatus::Reconciled)
                && self
                    .projection_records
                    .iter()
                    .all(|record| record.status == ProjectionStatus::Delivered),
            reconstruction_runs: self.reconstruction_runs,
        }
    }

    /// Durable partial-write records, oldest first.
    pub fn reconciliation_records(&self) -> &[ReconciliationRecord] {
        &self.reconciliations
    }

    /// Payload-free WAL-bound audit receipts, oldest first.
    pub fn audit_records(&self) -> &[OpenCtiWriteAuditRecord] {
        &self.audits
    }

    fn replay_receipt(
        &mut self,
        events: Vec<String>,
        idempotency_hash: &str,
        fingerprint: &str,
    ) -> Result<KnowledgeDataResponse, KnowledgeDataError> {
        let receipt = events
            .iter()
            .find_map(|event| serde_json::from_str::<PersistedWriteReceipt>(event).ok())
            .ok_or_else(|| unavailable("committed transaction has no readable write receipt"))?;
        if receipt.schema_version != RECEIPT_SCHEMA_VERSION {
            return Err(unavailable("unsupported committed write receipt version"));
        }
        if receipt.idempotency_key_hash != idempotency_hash
            || receipt.operation_fingerprint != fingerprint
        {
            return Err(conflict(
                "idempotency key was replayed with a different mutation payload",
            ));
        }
        self.audits = receipt.audits;
        self.idempotent_replays = self.idempotent_replays.saturating_add(1);
        Ok(receipt.response)
    }

    fn persist(&self) -> Result<(), String> {
        let Some(path) = &self.state_path else {
            return Ok(());
        };
        write_state(
            path,
            &PersistedWriteState {
                schema_version: STATE_SCHEMA_VERSION,
                reconciliations: self.reconciliations.clone(),
                projection_records: self.projection_records.clone(),
                next_projection_sequence: self.next_projection_sequence,
                projection_retries: self.projection_retries,
                write_authority: self.write_authority,
                rollback_trigger: self.rollback_trigger,
                reconstruction_runs: self.reconstruction_runs,
            },
        )
    }

    fn record_operation_counts(&mut self, outcomes: &[WriteOperationOutcome]) {
        for outcome in outcomes {
            if outcome.status == WriteOperationStatus::Applied {
                self.applied_operations = self.applied_operations.saturating_add(1);
            } else {
                self.failed_operations = self.failed_operations.saturating_add(1);
            }
        }
    }
}

const fn initial_projection_sequence() -> u64 {
    1
}

fn projection_ordering_key(operation: &KnowledgeDataOperation, sequence: u64) -> String {
    let key = match operation {
        KnowledgeDataOperation::Create(request) => record_identifier(&request.record),
        KnowledgeDataOperation::Update(request) => Some(request.id.as_str()),
        KnowledgeDataOperation::Delete(request) => Some(request.id.as_str()),
        KnowledgeDataOperation::Merge(request) => Some(request.target_id.as_str()),
        KnowledgeDataOperation::Bulk(_) => None,
        _ => None,
    };
    key.filter(|key| !key.trim().is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("transaction--{sequence:020}"))
}

fn record_identifier(record: &Value) -> Option<&str> {
    record
        .get("internal_id")
        .or_else(|| record.get("id"))
        .and_then(Value::as_str)
}

fn bounded_diagnostic(diagnostic: &str) -> String {
    diagnostic.chars().take(512).collect()
}

fn write_batch(
    operation: &KnowledgeDataOperation,
    context: &RequestContext,
    transaction_id: String,
) -> Result<OpenCtiWriteBatch, KnowledgeDataError> {
    let request_id = context.request_id.trim();
    let (atomic, operations) = match operation {
        KnowledgeDataOperation::Create(request) => (
            true,
            vec![OpenCtiWriteOperation::create(
                request_id,
                request.record.clone(),
            )],
        ),
        KnowledgeDataOperation::Update(request) => (
            true,
            vec![OpenCtiWriteOperation::update(
                request_id,
                request.id.clone(),
                request.expected_revision,
                request.patch.clone(),
            )],
        ),
        KnowledgeDataOperation::Delete(request) => (
            true,
            vec![OpenCtiWriteOperation::delete(
                request_id,
                request.id.clone(),
                request.expected_revision,
            )],
        ),
        KnowledgeDataOperation::Bulk(request) => {
            let operations = request
                .operations
                .iter()
                .cloned()
                .enumerate()
                .map(|(index, value)| parse_bulk_item(index, value))
                .collect::<Result<Vec<_>, _>>()?;
            (request.atomic, operations)
        }
        _ => {
            return Err(invalid(
                "OpenCTI write runtime accepts create, update, delete, or bulk",
            ));
        }
    };
    OpenCtiWriteBatch::new(transaction_id, atomic, operations)
        .map_err(|error| invalid(&error.to_string()))
}

fn parse_bulk_item(
    index: usize,
    value: Value,
) -> Result<OpenCtiWriteOperation, KnowledgeDataError> {
    let item: BulkItem = serde_json::from_value(value)
        .map_err(|error| invalid(&format!("invalid bulk item {index}: {error}")))?;
    let default_id = || format!("bulk-item-{index}");
    Ok(match item {
        BulkItem::Create {
            operation_id,
            record,
        } => OpenCtiWriteOperation::create(operation_id.unwrap_or_else(default_id), record),
        BulkItem::Update {
            operation_id,
            id,
            expected_revision,
            patch,
        }
        | BulkItem::AccessPolicyUpdate {
            operation_id,
            id,
            expected_revision,
            patch,
        } => OpenCtiWriteOperation::update(
            operation_id.unwrap_or_else(default_id),
            id,
            expected_revision,
            patch,
        ),
        BulkItem::Delete {
            operation_id,
            id,
            expected_revision,
        } => OpenCtiWriteOperation::delete(
            operation_id.unwrap_or_else(default_id),
            id,
            expected_revision,
        ),
    })
}

fn response_for(
    operation: &KnowledgeDataOperation,
    outcomes: &[WriteOperationOutcome],
    committed: bool,
) -> Result<KnowledgeDataResponse, KnowledgeDataError> {
    if matches!(operation, KnowledgeDataOperation::Bulk(_)) {
        let results = outcomes
            .iter()
            .map(|outcome| {
                serde_json::to_value(outcome).map_err(|error| unavailable(&error.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(KnowledgeDataResponse::Bulk(BulkResult { results }));
    }
    let outcome = outcomes
        .first()
        .ok_or_else(|| invalid("single mutation produced no operation result"))?;
    if committed && outcome.status == WriteOperationStatus::Applied {
        return Ok(KnowledgeDataResponse::Write(WriteResult {
            id: outcome.id.clone().unwrap_or_default(),
            revision: outcome.after_revision.unwrap_or_default(),
        }));
    }
    Err(match outcome.status {
        WriteOperationStatus::Conflict => conflict(
            outcome
                .diagnostic
                .as_deref()
                .unwrap_or("optimistic concurrency conflict"),
        ),
        WriteOperationStatus::Retryable => unavailable(
            outcome
                .diagnostic
                .as_deref()
                .unwrap_or("write dependency is not available"),
        ),
        WriteOperationStatus::Rejected | WriteOperationStatus::Aborted => invalid(
            outcome
                .diagnostic
                .as_deref()
                .unwrap_or("write was rejected"),
        ),
        WriteOperationStatus::Applied => unavailable("write result was not durably committed"),
    })
}

fn audit_records(
    context: &RequestContext,
    idempotency_key_hash: &str,
    outcomes: &[WriteOperationOutcome],
) -> Vec<OpenCtiWriteAuditRecord> {
    let source_offset = context.access.attributes.get("source_offset").cloned();
    outcomes
        .iter()
        .map(|outcome| OpenCtiWriteAuditRecord {
            idempotency_key_hash: idempotency_key_hash.to_owned(),
            correlation_id: context.correlation_id.clone(),
            source_offset: source_offset.clone(),
            before_revision: outcome.before_revision,
            after_revision: outcome.after_revision,
            outcome: serde_json::to_value(outcome.status)
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned))
                .unwrap_or_else(|| "unknown".to_owned()),
        })
        .collect()
}

fn fingerprint_operation(operation: &KnowledgeDataOperation) -> Result<String, KnowledgeDataError> {
    let value = serde_json::to_value(operation).map_err(|error| invalid(&error.to_string()))?;
    let canonical = canonicalize_json(value);
    let bytes = serde_json::to_vec(&canonical).map_err(|error| invalid(&error.to_string()))?;
    Ok(hash_bytes(&bytes))
}

fn canonicalize_json(value: Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut entries = object.into_iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key, canonicalize_json(value)))
                    .collect(),
            )
        }
        Value::Array(values) => Value::Array(values.into_iter().map(canonicalize_json).collect()),
        scalar => scalar,
    }
}

fn hash_text(value: &str) -> String {
    hash_bytes(value.as_bytes())
}

fn hash_bytes(value: &[u8]) -> String {
    let digest = Sha256::digest(value);
    let mut encoded = String::with_capacity(7 + digest.len() * 2);
    encoded.push_str("sha256:");
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

fn map_write_error(error: WriteError) -> KnowledgeDataError {
    match error {
        WriteError::InvalidInput(message) | WriteError::LimitExceeded(message) => invalid(&message),
        WriteError::Graph(message) => unavailable(&message),
    }
}

fn map_merge_error(error: MergeError) -> KnowledgeDataError {
    match error {
        MergeError::InvalidInput(message) | MergeError::LimitExceeded(message) => invalid(&message),
        MergeError::Conflict(message) => conflict(&message),
        MergeError::Graph(message) => unavailable(&message),
        MergeError::NotImplemented => unavailable("merge implementation is not available"),
    }
}

fn invalid(message: &str) -> KnowledgeDataError {
    KnowledgeDataError {
        code: KnowledgeDataErrorCode::InvalidRequest,
        message: message.to_owned(),
        retryable: false,
    }
}

fn conflict(message: &str) -> KnowledgeDataError {
    KnowledgeDataError {
        code: KnowledgeDataErrorCode::Conflict,
        message: message.to_owned(),
        retryable: false,
    }
}

fn unavailable(message: &str) -> KnowledgeDataError {
    KnowledgeDataError {
        code: KnowledgeDataErrorCode::BackendUnavailable,
        message: message.to_owned(),
        retryable: true,
    }
}

fn read_state(path: &Path) -> Result<PersistedWriteState, String> {
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    serde_json::from_slice(&bytes).map_err(|error| error.to_string())
}

fn write_state(path: &Path, state: &PersistedWriteState) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "OpenCTI write state path has no parent".to_owned())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temporary = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(state).map_err(|error| error.to_string())?;
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&temporary)
        .map_err(|error| error.to_string())?;
    file.write_all(&bytes).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    fs::rename(&temporary, path).map_err(|error| error.to_string())?;
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| error.to_string())
}

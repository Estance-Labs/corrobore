// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! Durable OpenCTI transactional-write coordination and dual-write recovery.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use corrobore_engine::{
    BulkResult, KnowledgeDataError, KnowledgeDataErrorCode, KnowledgeDataOperation,
    KnowledgeDataResponse, RequestContext, WriteResult,
};
use graph_storage::{
    CanonicalEngineStore, CanonicalProjectionRequest, DurableTransactionId,
    read_atomic_persistent_audit_events,
};
use opencti_adapter::{
    OpenCtiWriteBatch, OpenCtiWriteExecutor, OpenCtiWriteOperation, WriteError, WriteLimits,
    WriteOperationOutcome, WriteOperationStatus,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

const STATE_SCHEMA_VERSION: u32 = 1;
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
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PersistedWriteState {
    schema_version: u32,
    reconciliations: Vec<ReconciliationRecord>,
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
            .is_some_and(|state| state.schema_version != STATE_SCHEMA_VERSION)
        {
            return Err("unsupported OpenCTI write state version".to_owned());
        }
        Ok(Self {
            state_path,
            limits,
            max_reconciliation_records,
            reconciliations: persisted
                .map(|state| state.reconciliations)
                .unwrap_or_default(),
            audits: Vec::new(),
            applied_operations: 0,
            failed_operations: 0,
            idempotent_replays: 0,
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

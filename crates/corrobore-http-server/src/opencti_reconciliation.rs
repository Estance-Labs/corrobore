// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! Restart-safe targeted OpenCTI reconciliation coordination.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use graph_storage::{
    CanonicalEngineStore, CanonicalProjectionRequest, DurableTransactionId,
    read_atomic_persistent_audit_events,
};
use opencti_adapter::{
    OpenCtiReconciler, OpenCtiReconciliationCommand, ReconciliationLimits, ReconciliationMode,
    ReconciliationReport, RepairAction,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const STATE_SCHEMA_VERSION: u32 = 1;

/// Deterministic phase boundary used to prove resume behavior.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReconciliationCrashStage {
    /// Canonical WAL commit is durable but projections/report publication is pending.
    AfterCanonicalCommit,
}

/// Bounded operational reconciliation state.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenCtiReconciliationStatus {
    /// Retained completed/dry-run reports.
    pub retained_reports: usize,
    /// Reports containing at least one unsafe quarantined record.
    pub quarantined_commands: usize,
    /// Reports that completed post-repair parity verification.
    pub parity_verified_commands: usize,
}

/// Durable coordinator; canonical changes use the graph WAL and report receipts
/// use an atomic fsynced state file after projection publication succeeds.
#[derive(Clone, Debug)]
pub struct OpenCtiReconciliationRuntime {
    state_path: Option<PathBuf>,
    limits: ReconciliationLimits,
    max_reports: usize,
    reports: Vec<ReconciliationReport>,
    receipts: Vec<PersistedReceipt>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PersistedState {
    schema_version: u32,
    receipts: Vec<PersistedReceipt>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PersistedReceipt {
    fingerprint: String,
    report: ReconciliationReport,
}

impl OpenCtiReconciliationRuntime {
    /// Restore bounded reports before readiness and reject unknown state schemas.
    pub fn open(
        state_path: Option<PathBuf>,
        limits: ReconciliationLimits,
        max_reports: usize,
    ) -> Result<Self, String> {
        if max_reports == 0 {
            return Err("max_reports must be greater than zero".to_owned());
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
            return Err("unsupported OpenCTI reconciliation state version".to_owned());
        }
        let receipts = persisted.map(|state| state.receipts).unwrap_or_default();
        let reports = receipts
            .iter()
            .map(|receipt| receipt.report.clone())
            .collect();
        Ok(Self {
            state_path,
            limits,
            max_reports,
            reports,
            receipts,
        })
    }

    /// Execute without crash injection.
    pub fn execute(
        &mut self,
        store: &mut CanonicalEngineStore,
        command: OpenCtiReconciliationCommand,
    ) -> Result<ReconciliationReport, String> {
        self.execute_with_crash(store, command, None)
    }

    /// Plan, WAL-commit safe canonical changes, rebuild required projections,
    /// verify parity, and only then publish the durable idempotent report.
    pub fn execute_with_crash(
        &mut self,
        store: &mut CanonicalEngineStore,
        command: OpenCtiReconciliationCommand,
        crash_stage: Option<ReconciliationCrashStage>,
    ) -> Result<ReconciliationReport, String> {
        let command_fingerprint = fingerprint(&command)?;
        if let Some(receipt) = self
            .receipts
            .iter()
            .find(|receipt| receipt.report.command_id == command.command_id)
        {
            if receipt.fingerprint != command_fingerprint {
                return Err(
                    "reconciliation command_id was replayed with a different payload".to_owned(),
                );
            }
            return Ok(receipt.report.clone());
        }

        let transaction_id = DurableTransactionId::new(format!(
            "tx--opencti-reconcile-{}",
            hash_text(&command.command_id)
        ))
        .map_err(|error| error.to_string())?;
        let previous = store
            .load_projection(CanonicalProjectionRequest::all())
            .map_err(|error| error.to_string())?;
        let full_text_stale = !store
            .full_text_projection_is_ready()
            .map_err(|error| error.to_string())?;
        let stale_ids = if full_text_stale {
            command
                .reference_records
                .iter()
                .filter_map(canonical_id)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let outcome = OpenCtiReconciler::new(self.limits)
            .execute(&previous, &command, &stale_ids)
            .map_err(|error| error.to_string())?;
        let mut report = outcome.report;

        if command.mode == ReconciliationMode::Repair {
            let already_committed =
                !read_atomic_persistent_audit_events(store.root(), &transaction_id)
                    .map_err(|error| error.to_string())?
                    .is_empty();
            if !already_committed {
                let audit = serde_json::json!({
                    "kind": "opencti_reconciliation",
                    "command_id_hash": hash_text(&command.command_id),
                    "fingerprint": command_fingerprint.clone(),
                });
                store
                    .commit_transition_with_audit(
                        &previous,
                        &outcome.graph,
                        transaction_id,
                        vec![audit.to_string()],
                        None,
                    )
                    .map_err(|error| error.to_string())?;
            }
            if crash_stage == Some(ReconciliationCrashStage::AfterCanonicalCommit) {
                return Err("injected reconciliation crash after canonical commit".to_owned());
            }
            if report.mutated || !report.projection_rebuild_ids.is_empty() {
                store
                    .rebuild_full_text_index()
                    .map_err(|error| error.to_string())?;
            }
            for difference in &mut report.differences {
                if difference.action == RepairAction::PlannedProjectionRebuild {
                    difference.action = RepairAction::Applied;
                }
            }
            let repaired = store
                .load_projection(CanonicalProjectionRequest::all())
                .map_err(|error| error.to_string())?;
            let mut verification_command = command.clone();
            verification_command.mode = ReconciliationMode::DryRun;
            let verification = OpenCtiReconciler::new(self.limits)
                .execute(&repaired, &verification_command, &[])
                .map_err(|error| error.to_string())?;
            for mut difference in verification.report.differences {
                difference.action = RepairAction::Quarantined;
                difference.diagnostic = format!(
                    "post-repair parity verification failed: {}",
                    difference.diagnostic
                );
                report
                    .quarantined_record_ids
                    .push(difference.record_id.clone());
                report.differences.push(difference);
            }
            report.quarantined_record_ids.sort();
            report.quarantined_record_ids.dedup();
            report.parity_verified = report.quarantined_record_ids.is_empty();
        }
        self.remember(command_fingerprint, report.clone())?;
        Ok(report)
    }

    /// Oldest-first retained reports.
    pub fn reports(&self) -> &[ReconciliationReport] {
        &self.reports
    }

    /// Bounded payload-free status.
    pub fn status(&self) -> OpenCtiReconciliationStatus {
        OpenCtiReconciliationStatus {
            retained_reports: self.reports.len(),
            quarantined_commands: self
                .reports
                .iter()
                .filter(|report| !report.quarantined_record_ids.is_empty())
                .count(),
            parity_verified_commands: self
                .reports
                .iter()
                .filter(|report| report.parity_verified)
                .count(),
        }
    }

    fn remember(
        &mut self,
        fingerprint: String,
        report: ReconciliationReport,
    ) -> Result<(), String> {
        if self.receipts.len() == self.max_reports {
            let removable = self
                .receipts
                .iter()
                .position(|receipt| receipt.report.quarantined_record_ids.is_empty())
                .ok_or_else(|| {
                    "reconciliation backpressure: quarantine capacity is exhausted".to_owned()
                })?;
            self.receipts.remove(removable);
            self.reports.remove(removable);
        }
        self.receipts.push(PersistedReceipt {
            fingerprint,
            report: report.clone(),
        });
        self.reports.push(report);
        self.persist()
    }

    fn persist(&self) -> Result<(), String> {
        let Some(path) = &self.state_path else {
            return Ok(());
        };
        write_state(
            path,
            &PersistedState {
                schema_version: STATE_SCHEMA_VERSION,
                receipts: self.receipts.clone(),
            },
        )
    }
}

fn canonical_id(record: &serde_json::Value) -> Option<&str> {
    record
        .get("internal_id")
        .or_else(|| record.get("id"))
        .and_then(serde_json::Value::as_str)
}

fn fingerprint(command: &OpenCtiReconciliationCommand) -> Result<String, String> {
    serde_json::to_vec(command)
        .map(|bytes| hash_bytes(&bytes))
        .map_err(|error| error.to_string())
}

fn hash_text(value: &str) -> String {
    hash_bytes(value.as_bytes())
}

fn hash_bytes(value: &[u8]) -> String {
    let digest = Sha256::digest(value);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn read_state(path: &Path) -> Result<PersistedState, String> {
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    serde_json::from_slice(&bytes).map_err(|error| error.to_string())
}

fn write_state(path: &Path, state: &PersistedState) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "OpenCTI reconciliation state path has no parent".to_owned())?;
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

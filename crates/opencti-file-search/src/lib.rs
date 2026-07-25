// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

#![warn(missing_docs)]

//! Sandboxed OpenCTI file extraction and content-search boundary.

use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{Cursor, Write as _},
    path::{Component, Path, PathBuf},
    sync::{Arc, RwLock},
    time::Instant,
};

use calamine::{Reader as _, open_workbook_auto_from_rs};
use fs4::fs_std::FileExt as _;
use opencti_access::{AccessContext, AccessMetadata};
use opencti_search::{
    FullTextDocument, FullTextFieldFilter, FullTextIndex, FullTextIndexSettings, FullTextMatchMode,
    FullTextQuery, FullTextRebuildOutcome, FullTextRecordClass, FullTextSearchHit,
    FullTextSearchPage, FullTextSearchReadiness,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Hard resource limits enforced before and during document extraction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractionLimits {
    /// Maximum source bytes fetched from object storage.
    pub max_input_bytes: usize,
    /// Maximum UTF-8 bytes emitted by all chunks.
    pub max_extracted_bytes: usize,
    /// Maximum PDF pages.
    pub max_pages: usize,
    /// Maximum spreadsheet sheets.
    pub max_sheets: usize,
    /// Maximum rows extracted from one sheet.
    pub max_rows_per_sheet: usize,
    /// Maximum spreadsheet cells across the workbook.
    pub max_cells: usize,
    /// Maximum emitted chunks.
    pub max_chunks: usize,
    /// Maximum Unicode scalar values per chunk.
    pub max_chunk_chars: usize,
}

impl Default for ExtractionLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: 50 * 1024 * 1024,
            max_extracted_bytes: 10 * 1024 * 1024,
            max_pages: 1_000,
            max_sheets: 100,
            max_rows_per_sheet: 100_000,
            max_cells: 1_000_000,
            max_chunks: 10_000,
            max_chunk_chars: 4_096,
        }
    }
}

/// Canonical OpenCTI file identity and object-storage provenance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileDescriptor {
    /// Stable OpenCTI file identifier.
    pub file_id: String,
    /// Canonical object to which the file is attached.
    pub source_object_id: String,
    /// Opaque S3/MinIO object key.
    pub blob_key: String,
    /// Original file name.
    pub name: String,
    /// Declared media type.
    pub mime_type: String,
    /// Lowercase SHA-256 source digest.
    pub content_hash: String,
    /// Monotonic OpenCTI file version.
    pub version: u64,
    /// Access policy copied from the file and its source object.
    pub access: AccessMetadata,
}

/// One bounded worker request after object-storage retrieval.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileExtractionRequest {
    /// Canonical descriptor.
    pub descriptor: FileDescriptor,
    /// Untrusted source bytes.
    pub content: Vec<u8>,
}

/// Page, sheet and row coordinates for one extracted chunk.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkProvenance {
    /// One-based PDF page.
    pub page: Option<u32>,
    /// Spreadsheet sheet name.
    pub sheet: Option<String>,
    /// One-based first source row.
    pub row_start: Option<u32>,
    /// One-based last source row.
    pub row_end: Option<u32>,
}

/// One searchable bounded text segment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractedChunk {
    /// Stable ordinal within the source version.
    pub ordinal: u32,
    /// Normalized extracted text.
    pub text: String,
    /// Exact source coordinates.
    pub provenance: ChunkProvenance,
}

/// Rebuildable extraction output persisted beside canonical metadata.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractionArtifact {
    /// Canonical source descriptor.
    pub descriptor: FileDescriptor,
    /// Ordered text chunks.
    pub chunks: Vec<ExtractedChunk>,
    /// Total UTF-8 bytes across chunks.
    pub extracted_bytes: u64,
}

/// Stable bounded extraction failure category.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionErrorCode {
    /// Media type is outside the compatibility subset.
    UnsupportedFormat,
    /// Parser rejected malformed input.
    Malformed,
    /// Encrypted input requires unavailable credentials.
    Encrypted,
    /// Source bytes exceed the configured limit.
    InputLimitExceeded,
    /// Pages, rows, cells, chunks, time or memory exceeded a hard bound.
    ResourceLimitExceeded,
    /// The isolated worker exceeded its execution deadline.
    Timeout,
    /// Source content hash does not match canonical metadata.
    ContentHashMismatch,
}

/// Safe extraction failure without untrusted payload fragments.
#[derive(Clone, Debug, Error, PartialEq, Eq, Serialize, Deserialize)]
#[error("{code:?}: {diagnostic}")]
pub struct ExtractionFailure {
    /// Stable error code.
    pub code: ExtractionErrorCode,
    /// Bounded operator-facing diagnostic.
    pub diagnostic: String,
}

/// Extract one supported file under explicit resource limits.
///
/// The parser is selected from the declared and sniffed media type, then the
/// hash is verified, text normalized, and provenance-aware chunking applied.
pub fn extract_file(
    request: FileExtractionRequest,
    limits: &ExtractionLimits,
) -> Result<ExtractionArtifact, ExtractionFailure> {
    validate_extraction_request(&request, limits)?;
    let extracted = match normalized_mime_type(&request.descriptor.mime_type) {
        "text/plain" => extract_plain_text(&request.content)?,
        "text/csv" | "application/csv" => extract_csv(&request.content, limits)?,
        "text/html" | "application/xhtml+xml" => extract_html(&request.content)?,
        "application/pdf" => extract_pdf(&request.content, limits)?,
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        | "application/vnd.ms-excel"
        | "application/vnd.ms-excel.sheet.binary.macroenabled.12"
        | "application/vnd.oasis.opendocument.spreadsheet" => {
            extract_spreadsheet(&request.content, limits)?
        }
        _ => {
            return Err(failure(
                ExtractionErrorCode::UnsupportedFormat,
                "declared media type is outside the supported file extraction subset",
            ));
        }
    };
    let chunks = bounded_chunks(extracted, limits)?;
    let extracted_bytes = chunks
        .iter()
        .map(|chunk| chunk.text.len() as u64)
        .sum::<u64>();
    if extracted_bytes > limits.max_extracted_bytes as u64 {
        return Err(failure(
            ExtractionErrorCode::ResourceLimitExceeded,
            "extracted text exceeds max_extracted_bytes",
        ));
    }
    Ok(ExtractionArtifact {
        descriptor: request.descriptor,
        chunks,
        extracted_bytes,
    })
}

/// Result of deduplicating one durable enqueue request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnqueueOutcome {
    /// Deterministic job identity.
    pub job_id: String,
    /// Whether the same file/hash/version was already known.
    pub duplicate: bool,
}

/// One leased job safe to process outside the server process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeasedExtractionJob {
    /// Stable job identity.
    pub job_id: String,
    /// Unpredictable token fencing stale workers.
    pub lease_token: String,
    /// Canonical file descriptor.
    pub descriptor: FileDescriptor,
}

/// Durable failure transition selected by the retry policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobDisposition {
    /// A later lease may retry the job.
    RetryScheduled,
    /// Retry budget is exhausted and the job is quarantined.
    Quarantined,
}

/// Low-cardinality extraction and queue measurements.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileJobMetrics {
    /// Pending or retryable jobs.
    pub queue_depth: u64,
    /// Cumulative completed jobs.
    pub completed_jobs: u64,
    /// Cumulative parser failures.
    pub failures: u64,
    /// Cumulative scheduled retries.
    pub retries: u64,
    /// Cumulative quarantines.
    pub quarantines: u64,
    /// Cumulative extracted bytes.
    pub extracted_bytes: u64,
    /// Oldest pending job age in milliseconds.
    pub index_lag_ms: u64,
    /// Cumulative worker processing latency in milliseconds.
    pub processing_latency_ms_total: u64,
    /// Most recent worker processing latency in milliseconds.
    pub last_processing_latency_ms: u64,
}

/// Canonical file lifecycle transition that changes searchable visibility.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileLifecycleEvent {
    /// Remove one file and all derived content.
    Delete {
        /// Stable file identifier.
        file_id: String,
    },
    /// Replace content and metadata under the same file identity.
    Replace {
        /// New canonical descriptor.
        descriptor: FileDescriptor,
        /// New untrusted source bytes.
        content: Vec<u8>,
    },
    /// Redirect one file identity to another and remove the source.
    Merge {
        /// Identity being merged away.
        source_file_id: String,
        /// Surviving identity.
        target_file_id: String,
    },
    /// Replace only access metadata without re-reading untrusted content.
    PolicyChange {
        /// Stable file identifier.
        file_id: String,
        /// New canonical access metadata.
        access: AccessMetadata,
    },
}

/// Durable file job and extraction-artifact repository.
#[derive(Clone, Debug)]
pub struct FileJobStore {
    root: PathBuf,
    max_attempts: u32,
    lease_ms: u64,
    state: DurableFileState,
}

/// Durable queue, extraction, or index failure.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum FileContentError {
    /// Persistent state could not be read or committed.
    #[error("file content persistence failed: {0}")]
    Persistence(String),
    /// Extraction failed safely.
    #[error(transparent)]
    Extraction(#[from] ExtractionFailure),
}

impl FileJobStore {
    /// Open or initialize one atomically persisted queue.
    pub fn open(root: PathBuf, max_attempts: u32, lease_ms: u64) -> Result<Self, FileContentError> {
        if max_attempts == 0 || lease_ms == 0 {
            return Err(FileContentError::Persistence(
                "max_attempts and lease_ms must be non-zero".to_owned(),
            ));
        }
        fs::create_dir_all(&root).map_err(persistence_error)?;
        let state_path = root.join("file-jobs.json");
        let state = if state_path.exists() {
            serde_json::from_slice(&fs::read(&state_path).map_err(persistence_error)?)
                .map_err(|error| FileContentError::Persistence(error.to_string()))?
        } else {
            DurableFileState::default()
        };
        Ok(Self {
            root,
            max_attempts,
            lease_ms,
            state,
        })
    }

    /// Enqueue one deterministic file/hash/version identity.
    pub fn enqueue(
        &mut self,
        descriptor: FileDescriptor,
        now_ms: u64,
    ) -> Result<EnqueueOutcome, FileContentError> {
        let _lock = self.begin_update()?;
        validate_descriptor(&descriptor).map_err(FileContentError::Extraction)?;
        let job_id = job_id(&descriptor);
        let duplicate = self.state.jobs.contains_key(&job_id);
        if !duplicate {
            self.state.jobs.insert(
                job_id.clone(),
                JobRecord {
                    descriptor,
                    state: JobState::Pending {
                        eligible_at_ms: now_ms,
                    },
                    attempts: 0,
                    enqueued_at_ms: now_ms,
                    last_error: None,
                },
            );
            self.persist()?;
        }
        Ok(EnqueueOutcome { job_id, duplicate })
    }

    /// Lease the oldest eligible job and recover expired crash leases.
    pub fn lease_next(
        &mut self,
        now_ms: u64,
    ) -> Result<Option<LeasedExtractionJob>, FileContentError> {
        let _lock = self.begin_update()?;
        let mut recovered_failures = 0_u64;
        let mut recovered_retries = 0_u64;
        let mut recovered_quarantines = 0_u64;
        for record in self.state.jobs.values_mut() {
            if matches!(
                record.state,
                JobState::Leased {
                    expires_at_ms,
                    ..
                } if expires_at_ms <= now_ms
            ) {
                record.attempts = record.attempts.saturating_add(1);
                record.last_error = Some(SafeJobError {
                    code: ExtractionErrorCode::Timeout,
                    diagnostic: "isolated worker lease expired before completion".to_owned(),
                });
                recovered_failures = recovered_failures.saturating_add(1);
                if record.attempts < self.max_attempts {
                    record.state = JobState::Pending {
                        eligible_at_ms: now_ms,
                    };
                    recovered_retries = recovered_retries.saturating_add(1);
                } else {
                    record.state = JobState::Quarantined;
                    recovered_quarantines = recovered_quarantines.saturating_add(1);
                }
            }
        }
        self.state.metrics.failures = self
            .state
            .metrics
            .failures
            .saturating_add(recovered_failures);
        self.state.metrics.retries = self.state.metrics.retries.saturating_add(recovered_retries);
        self.state.metrics.quarantines = self
            .state
            .metrics
            .quarantines
            .saturating_add(recovered_quarantines);
        let selected = self
            .state
            .jobs
            .iter()
            .filter(|(_, record)| {
                matches!(
                    record.state,
                    JobState::Pending { eligible_at_ms } if eligible_at_ms <= now_ms
                )
            })
            .min_by_key(|(_, record)| (record.enqueued_at_ms, record.descriptor.file_id.clone()))
            .map(|(job_id, _)| job_id.clone());
        let Some(job_id) = selected else {
            self.persist()?;
            return Ok(None);
        };
        let lease_token = random_token()?;
        let record =
            self.state.jobs.get_mut(&job_id).ok_or_else(|| {
                FileContentError::Persistence("selected job disappeared".to_owned())
            })?;
        record.state = JobState::Leased {
            token: lease_token.clone(),
            expires_at_ms: now_ms.saturating_add(self.lease_ms),
        };
        let lease = LeasedExtractionJob {
            job_id,
            lease_token,
            descriptor: record.descriptor.clone(),
        };
        self.persist()?;
        Ok(Some(lease))
    }

    /// Record a fenced failure and apply bounded retry/quarantine policy.
    pub fn fail(
        &mut self,
        lease: &LeasedExtractionJob,
        code: ExtractionErrorCode,
        diagnostic: &str,
        now_ms: u64,
    ) -> Result<JobDisposition, FileContentError> {
        let _lock = self.begin_update()?;
        let max_attempts = self.max_attempts;
        let lease_ms = self.lease_ms;
        let disposition = {
            let record = self.validate_lease_mut(lease)?;
            record.attempts = record.attempts.saturating_add(1);
            record.last_error = Some(SafeJobError {
                code,
                diagnostic: safe_diagnostic(diagnostic),
            });
            if record.attempts < max_attempts {
                record.state = JobState::Pending {
                    eligible_at_ms: now_ms.saturating_add(lease_ms),
                };
                JobDisposition::RetryScheduled
            } else {
                record.state = JobState::Quarantined;
                JobDisposition::Quarantined
            }
        };
        self.state.metrics.failures = self.state.metrics.failures.saturating_add(1);
        match disposition {
            JobDisposition::RetryScheduled => {
                self.state.metrics.retries = self.state.metrics.retries.saturating_add(1);
            }
            JobDisposition::Quarantined => {
                self.state.metrics.quarantines = self.state.metrics.quarantines.saturating_add(1);
            }
        }
        self.persist()?;
        Ok(disposition)
    }

    /// Publish an extraction artifact idempotently.
    pub fn publish_artifact(
        &mut self,
        artifact: ExtractionArtifact,
        _now_ms: u64,
    ) -> Result<(), FileContentError> {
        let _lock = self.begin_update()?;
        self.publish_artifact_unlocked(artifact)
    }

    fn publish_artifact_unlocked(
        &mut self,
        artifact: ExtractionArtifact,
    ) -> Result<(), FileContentError> {
        let artifact_job_id = job_id(&artifact.descriptor);
        self.state.jobs.retain(|job_id, record| {
            record.descriptor.file_id != artifact.descriptor.file_id || job_id == &artifact_job_id
        });
        if let Some(record) = self.state.jobs.get_mut(&artifact_job_id) {
            record.state = JobState::Completed;
        }
        let previous = self
            .state
            .artifacts
            .insert(artifact.descriptor.file_id.clone(), artifact.clone());
        if previous.as_ref() != Some(&artifact) {
            self.state.artifact_generation = self.state.artifact_generation.saturating_add(1);
            self.state.metrics.completed_jobs = self.state.metrics.completed_jobs.saturating_add(1);
            self.state.metrics.extracted_bytes = self
                .state
                .metrics
                .extracted_bytes
                .saturating_add(artifact.extracted_bytes);
        }
        self.persist()
    }

    /// Fence a worker completion by its active lease and publish exactly once.
    pub fn complete(
        &mut self,
        lease: &LeasedExtractionJob,
        artifact: ExtractionArtifact,
        now_ms: u64,
    ) -> Result<(), FileContentError> {
        let _lock = self.begin_update()?;
        self.validate_lease_mut(lease)?;
        if artifact.descriptor != lease.descriptor {
            return Err(FileContentError::Persistence(
                "worker artifact descriptor does not match its leased job".to_owned(),
            ));
        }
        let _ = now_ms;
        self.publish_artifact_unlocked(artifact)
    }

    /// Return the complete canonical artifact set used for rebuild.
    pub fn artifacts(&self) -> Result<Vec<ExtractionArtifact>, FileContentError> {
        Ok(self.read_state()?.artifacts.into_values().collect())
    }

    /// Return one consistent generation and canonical artifact snapshot.
    pub fn artifact_snapshot(&self) -> Result<(u64, Vec<ExtractionArtifact>), FileContentError> {
        let state = self.read_state()?;
        Ok((
            state.artifact_generation,
            state.artifacts.into_values().collect(),
        ))
    }

    /// Remove one file artifact and every queued version of that identity.
    pub fn delete_file(&mut self, file_id: &str) -> Result<(), FileContentError> {
        let _lock = self.begin_update()?;
        self.delete_file_unlocked(file_id)
    }

    fn delete_file_unlocked(&mut self, file_id: &str) -> Result<(), FileContentError> {
        if self.state.artifacts.remove(file_id).is_some() {
            self.state.artifact_generation = self.state.artifact_generation.saturating_add(1);
        }
        self.state
            .jobs
            .retain(|_, record| record.descriptor.file_id != file_id);
        self.persist()
    }

    /// Apply delete, replacement, merge or policy-bearing replacement.
    pub fn apply_lifecycle(
        &mut self,
        event: FileLifecycleEvent,
        limits: &ExtractionLimits,
        now_ms: u64,
    ) -> Result<(), FileContentError> {
        let _lock = self.begin_update()?;
        match event {
            FileLifecycleEvent::Delete { file_id } => self.delete_file_unlocked(&file_id),
            FileLifecycleEvent::Replace {
                descriptor,
                content,
            } => {
                let artifact = extract_file(
                    FileExtractionRequest {
                        descriptor,
                        content,
                    },
                    limits,
                )?;
                let _ = now_ms;
                self.publish_artifact_unlocked(artifact)
            }
            FileLifecycleEvent::Merge {
                source_file_id,
                target_file_id,
            } => {
                if source_file_id == target_file_id {
                    return Ok(());
                }
                if let Some(mut artifact) = self.state.artifacts.remove(&source_file_id) {
                    artifact.descriptor.file_id = target_file_id.clone();
                    self.state.artifacts.insert(target_file_id, artifact);
                    self.state.artifact_generation =
                        self.state.artifact_generation.saturating_add(1);
                }
                self.state
                    .jobs
                    .retain(|_, record| record.descriptor.file_id != source_file_id);
                self.persist()
            }
            FileLifecycleEvent::PolicyChange { file_id, access } => {
                let Some(artifact) = self.state.artifacts.get_mut(&file_id) else {
                    return Err(FileContentError::Persistence(format!(
                        "cannot change policy for unknown file {file_id}"
                    )));
                };
                if artifact.descriptor.access != access {
                    artifact.descriptor.access = access;
                    self.state.artifact_generation =
                        self.state.artifact_generation.saturating_add(1);
                }
                self.persist()
            }
        }
    }

    /// Return bounded queue and extraction metrics.
    pub fn metrics(&self, now_ms: u64) -> FileJobMetrics {
        let state = self.read_state().unwrap_or_else(|_| self.state.clone());
        let mut metrics = state.metrics.clone();
        let pending = state
            .jobs
            .values()
            .filter(|record| matches!(record.state, JobState::Pending { .. }))
            .collect::<Vec<_>>();
        metrics.queue_depth = pending.len() as u64;
        metrics.index_lag_ms = pending
            .iter()
            .map(|record| now_ms.saturating_sub(record.enqueued_at_ms))
            .max()
            .unwrap_or(0);
        metrics
    }

    fn record_processing_latency(&mut self, latency_ms: u64) -> Result<(), FileContentError> {
        let _lock = self.begin_update()?;
        self.state.metrics.processing_latency_ms_total = self
            .state
            .metrics
            .processing_latency_ms_total
            .saturating_add(latency_ms);
        self.state.metrics.last_processing_latency_ms = latency_ms;
        self.persist()
    }

    fn validate_lease_mut(
        &mut self,
        lease: &LeasedExtractionJob,
    ) -> Result<&mut JobRecord, FileContentError> {
        let record = self
            .state
            .jobs
            .get_mut(&lease.job_id)
            .ok_or_else(|| FileContentError::Persistence("leased job is unknown".to_owned()))?;
        if !matches!(
            &record.state,
            JobState::Leased { token, .. } if token == &lease.lease_token
        ) {
            return Err(FileContentError::Persistence(
                "lease token is stale or invalid".to_owned(),
            ));
        }
        Ok(record)
    }

    fn persist(&self) -> Result<(), FileContentError> {
        atomic_write_json(&self.root.join("file-jobs.json"), &self.state)
    }

    fn begin_update(&mut self) -> Result<fs::File, FileContentError> {
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.root.join("file-jobs.lock"))
            .map_err(persistence_error)?;
        lock.lock_exclusive().map_err(persistence_error)?;
        self.state = self.read_state()?;
        Ok(lock)
    }

    fn read_state(&self) -> Result<DurableFileState, FileContentError> {
        let path = self.root.join("file-jobs.json");
        if !path.exists() {
            return Ok(DurableFileState::default());
        }
        serde_json::from_slice(&fs::read(path).map_err(persistence_error)?)
            .map_err(|error| FileContentError::Persistence(error.to_string()))
    }
}

/// Object-storage boundary used by the dedicated extraction worker.
pub trait FileBlobSource: std::fmt::Debug {
    /// Fetch at most `max_bytes` for one opaque S3/MinIO-compatible key.
    fn fetch(&mut self, blob_key: &str, max_bytes: usize) -> Result<Vec<u8>, ExtractionFailure>;
}

/// Filesystem implementation used by local deployments and deterministic tests.
#[derive(Clone, Debug)]
pub struct FilesystemBlobSource {
    root: PathBuf,
}

impl FilesystemBlobSource {
    /// Bind the source to one root; keys may never escape it.
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

impl FileBlobSource for FilesystemBlobSource {
    fn fetch(&mut self, blob_key: &str, max_bytes: usize) -> Result<Vec<u8>, ExtractionFailure> {
        let key = Path::new(blob_key);
        if key.is_absolute()
            || key
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(failure(
                ExtractionErrorCode::Malformed,
                "object-storage key contains an unsafe path component",
            ));
        }
        let path = self.root.join(key);
        let metadata = fs::metadata(&path).map_err(|_| {
            failure(
                ExtractionErrorCode::Malformed,
                "object-storage source is unavailable",
            )
        })?;
        if metadata.len() > max_bytes as u64 {
            return Err(failure(
                ExtractionErrorCode::InputLimitExceeded,
                "object-storage source exceeds max_input_bytes",
            ));
        }
        fs::read(path).map_err(|_| {
            failure(
                ExtractionErrorCode::Malformed,
                "object-storage source could not be read",
            )
        })
    }
}

/// Result of one bounded worker polling cycle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkerRunOutcome {
    /// No eligible job exists.
    Idle,
    /// One artifact was fenced and durably published.
    Published {
        /// Stable job identity.
        job_id: String,
        /// Searchable UTF-8 bytes emitted.
        extracted_bytes: u64,
    },
    /// One failure remains eligible for bounded retry.
    RetryScheduled {
        /// Stable job identity.
        job_id: String,
    },
    /// One permanent or exhausted job entered quarantine.
    Quarantined {
        /// Stable job identity.
        job_id: String,
    },
}

/// Dedicated worker that owns untrusted parser execution outside graph-core.
#[derive(Debug)]
pub struct FileExtractionWorker<S> {
    source: S,
    limits: ExtractionLimits,
    max_runtime_ms: u64,
}

impl<S: FileBlobSource> FileExtractionWorker<S> {
    /// Create a worker with explicit source, parser and deadline bounds.
    pub fn new(
        source: S,
        limits: ExtractionLimits,
        max_runtime_ms: u64,
    ) -> Result<Self, FileContentError> {
        if max_runtime_ms == 0 {
            return Err(FileContentError::Persistence(
                "worker max_runtime_ms must be non-zero".to_owned(),
            ));
        }
        Ok(Self {
            source,
            limits,
            max_runtime_ms,
        })
    }

    /// Lease, fetch, extract and publish at most one job.
    ///
    /// The production launcher supervises this worker process with the same
    /// deadline so a parser that does not return is killed and its lease later
    /// resumes. This method also rejects results that finish after the bound.
    pub fn run_once(
        &mut self,
        store: &mut FileJobStore,
        now_ms: u64,
    ) -> Result<WorkerRunOutcome, FileContentError> {
        let Some(lease) = store.lease_next(now_ms)? else {
            return Ok(WorkerRunOutcome::Idle);
        };
        let started = Instant::now();
        let result = self
            .source
            .fetch(&lease.descriptor.blob_key, self.limits.max_input_bytes)
            .and_then(|content| {
                extract_file(
                    FileExtractionRequest {
                        descriptor: lease.descriptor.clone(),
                        content,
                    },
                    &self.limits,
                )
            });
        let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        store.record_processing_latency(elapsed_ms)?;
        let result = if elapsed_ms > self.max_runtime_ms {
            Err(failure(
                ExtractionErrorCode::Timeout,
                "isolated parser exceeded max_runtime_ms",
            ))
        } else {
            result
        };
        match result {
            Ok(artifact) => {
                let extracted_bytes = artifact.extracted_bytes;
                store.complete(&lease, artifact, now_ms.saturating_add(elapsed_ms))?;
                Ok(WorkerRunOutcome::Published {
                    job_id: lease.job_id,
                    extracted_bytes,
                })
            }
            Err(error) => {
                let disposition = store.fail(
                    &lease,
                    error.code,
                    &error.diagnostic,
                    now_ms.saturating_add(elapsed_ms),
                )?;
                Ok(match disposition {
                    JobDisposition::RetryScheduled => WorkerRunOutcome::RetryScheduled {
                        job_id: lease.job_id,
                    },
                    JobDisposition::Quarantined => WorkerRunOutcome::Quarantined {
                        job_id: lease.job_id,
                    },
                })
            }
        }
    }
}

/// File-specific full-text query and structured filters.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileContentQuery {
    /// User search text.
    pub text: String,
    /// Term, phrase, prefix or bounded fuzzy matching.
    pub mode: FullTextMatchMode,
    /// Allowed media types.
    pub mime_types: Vec<String>,
    /// Allowed owner identities.
    pub owner_ids: Vec<String>,
    /// Allowed canonical source objects.
    pub source_object_ids: Vec<String>,
    /// Maximum hits.
    pub limit: u32,
    /// Opaque generation and policy-bound cursor.
    pub cursor: Option<String>,
}

impl Default for FileContentQuery {
    fn default() -> Self {
        Self {
            text: String::new(),
            mode: FullTextMatchMode::Term,
            mime_types: Vec::new(),
            owner_ids: Vec::new(),
            source_object_ids: Vec::new(),
            limit: 20,
            cursor: None,
        }
    }
}

/// Durable file-content index settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileContentIndexSettings {
    /// Versioned schema identifier.
    pub schema_version: String,
    /// HMAC key used to authenticate cursors.
    pub cursor_key: Vec<u8>,
    /// Tantivy writer memory bound.
    pub writer_memory_bytes: usize,
    /// Maximum candidates evaluated before authorization.
    pub max_candidates: usize,
    /// Maximum highlighted snippet characters.
    pub snippet_chars: usize,
}

impl FileContentIndexSettings {
    /// Small deterministic settings for tests and embedded use.
    pub fn testing(cursor_key: Vec<u8>) -> Self {
        Self {
            schema_version: "opencti-file-content-v1".to_owned(),
            cursor_key,
            writer_memory_bytes: 15_000_000,
            max_candidates: 10_000,
            snippet_chars: 160,
        }
    }
}

/// Rebuildable Corrobore-owned file-content full-text index.
#[derive(Clone, Debug)]
pub struct FileContentIndex {
    root: PathBuf,
    settings: FileContentIndexSettings,
    index: FullTextIndex,
    artifacts: Arc<RwLock<BTreeMap<String, ExtractionArtifact>>>,
}

impl FileContentIndex {
    /// Open one versioned index root.
    pub fn open(
        root: PathBuf,
        settings: FileContentIndexSettings,
    ) -> Result<Self, FileContentError> {
        let index = FullTextIndex::open(
            root.clone(),
            FullTextIndexSettings {
                schema_version: settings.schema_version.clone(),
                cursor_key: settings.cursor_key.clone(),
                writer_memory_bytes: settings.writer_memory_bytes,
                max_candidates: settings.max_candidates,
            },
        )
        .map_err(|error| FileContentError::Persistence(error.to_string()))?;
        let artifacts = if root.join("file-artifacts.json").is_file() {
            read_artifacts(&root.join("file-artifacts.json"))?
        } else {
            BTreeMap::new()
        };
        Ok(Self {
            root,
            settings,
            index,
            artifacts: Arc::new(RwLock::new(artifacts)),
        })
    }

    /// Published Tantivy directory, used only for corruption/rebuild probes.
    pub fn index_path(&self) -> PathBuf {
        self.root.join("published")
    }

    /// Inspect whether a complete compatible generation is queryable.
    pub fn readiness(&self) -> FullTextSearchReadiness {
        self.index.inspect().readiness
    }

    /// Reconstruct an atomic generation from durable artifacts.
    pub fn rebuild(
        &self,
        artifacts: Vec<ExtractionArtifact>,
    ) -> Result<FullTextRebuildOutcome, FileContentError> {
        self.rebuild_generation(artifacts, 0)
    }

    /// Synchronize the derived index only when the durable artifact generation
    /// changed, including changes published by another process.
    pub fn rebuild_from_store(
        &self,
        store: &FileJobStore,
    ) -> Result<FullTextRebuildOutcome, FileContentError> {
        let (source_generation, artifacts) = store.artifact_snapshot()?;
        let marker = fs::read(self.root.join("file-source-generation.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<u64>(&bytes).ok());
        if marker == Some(source_generation) && self.readiness() == FullTextSearchReadiness::Ready {
            let status = self.index.inspect();
            return Ok(FullTextRebuildOutcome {
                readiness: status.readiness,
                processed_documents: status.processed_documents,
                total_documents: status.total_documents,
                generation_changed: false,
                generation: status.generation.unwrap_or_default(),
            });
        }
        self.rebuild_generation(artifacts, source_generation)
    }

    fn rebuild_generation(
        &self,
        artifacts: Vec<ExtractionArtifact>,
        source_generation: u64,
    ) -> Result<FullTextRebuildOutcome, FileContentError> {
        let documents = artifacts.iter().map(artifact_document).collect::<Vec<_>>();
        self.index
            .invalidate()
            .map_err(|error| FileContentError::Persistence(error.to_string()))?;
        atomic_write_json(&self.root.join("file-artifacts.json"), &artifacts)?;
        let outcome = self
            .index
            .rebuild(&documents)
            .map_err(|error| FileContentError::Persistence(error.to_string()))?;
        let artifacts = artifacts
            .into_iter()
            .map(|artifact| (artifact.descriptor.file_id.clone(), artifact))
            .collect();
        *self.artifacts.write().map_err(|_| {
            FileContentError::Persistence("artifact cache lock poisoned".to_owned())
        })? = artifacts;
        atomic_write_json(
            &self.root.join("file-source-generation.json"),
            &source_generation,
        )?;
        Ok(outcome)
    }

    /// Search authorized file content with snippets and provenance.
    pub fn search(
        &self,
        query: &FileContentQuery,
        access: &AccessContext,
    ) -> Result<FullTextSearchPage, FileContentError> {
        if query.limit == 0 || query.limit > 1_000 || query.text.trim().is_empty() {
            return Err(FileContentError::Persistence(
                "file content query requires text and limit 1..=1000".to_owned(),
            ));
        }
        let mut filters = Vec::new();
        extend_filters(&mut filters, "mime_type", &query.mime_types);
        extend_filters(&mut filters, "owner_id", &query.owner_ids);
        extend_filters(&mut filters, "source_object_id", &query.source_object_ids);
        let mut page = self
            .index
            .search(
                &FullTextQuery {
                    text: query.text.clone(),
                    mode: query.mode.clone(),
                    fields: vec!["content".to_owned(), "name".to_owned()],
                    kinds: vec!["file".to_owned()],
                    filters,
                    limit: query.limit,
                    cursor: query.cursor.clone(),
                },
                access,
            )
            .map_err(|error| FileContentError::Persistence(error.to_string()))?;
        let artifacts = self.artifacts.read().map_err(|_| {
            FileContentError::Persistence("artifact cache lock poisoned".to_owned())
        })?;
        for hit in &mut page.hits {
            if let Some(artifact) = artifacts.get(&hit.id) {
                decorate_hit(hit, artifact, &query.text, self.settings.snippet_chars);
            }
        }
        Ok(page)
    }
}

#[derive(Clone, Debug)]
struct RawExtractedChunk {
    text: String,
    provenance: ChunkProvenance,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct DurableFileState {
    jobs: BTreeMap<String, JobRecord>,
    artifacts: BTreeMap<String, ExtractionArtifact>,
    metrics: FileJobMetrics,
    #[serde(default)]
    artifact_generation: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct JobRecord {
    descriptor: FileDescriptor,
    state: JobState,
    attempts: u32,
    enqueued_at_ms: u64,
    last_error: Option<SafeJobError>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum JobState {
    Pending { eligible_at_ms: u64 },
    Leased { token: String, expires_at_ms: u64 },
    Completed,
    Quarantined,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SafeJobError {
    code: ExtractionErrorCode,
    diagnostic: String,
}

fn validate_extraction_request(
    request: &FileExtractionRequest,
    limits: &ExtractionLimits,
) -> Result<(), ExtractionFailure> {
    validate_descriptor(&request.descriptor)?;
    if limits.max_input_bytes == 0
        || limits.max_extracted_bytes == 0
        || limits.max_pages == 0
        || limits.max_sheets == 0
        || limits.max_rows_per_sheet == 0
        || limits.max_cells == 0
        || limits.max_chunks == 0
        || limits.max_chunk_chars == 0
    {
        return Err(failure(
            ExtractionErrorCode::ResourceLimitExceeded,
            "all extraction limits must be non-zero",
        ));
    }
    if request.content.len() > limits.max_input_bytes {
        return Err(failure(
            ExtractionErrorCode::InputLimitExceeded,
            "source file exceeds max_input_bytes",
        ));
    }
    let actual_hash = format!("{:x}", Sha256::digest(&request.content));
    if actual_hash != request.descriptor.content_hash.to_ascii_lowercase() {
        return Err(failure(
            ExtractionErrorCode::ContentHashMismatch,
            "source content hash does not match canonical metadata",
        ));
    }
    Ok(())
}

fn validate_descriptor(descriptor: &FileDescriptor) -> Result<(), ExtractionFailure> {
    if descriptor.file_id.trim().is_empty()
        || descriptor.source_object_id.trim().is_empty()
        || descriptor.blob_key.trim().is_empty()
        || descriptor.name.trim().is_empty()
        || descriptor.mime_type.trim().is_empty()
        || descriptor.version == 0
    {
        return Err(failure(
            ExtractionErrorCode::Malformed,
            "file descriptor is missing a required identity or version",
        ));
    }
    if descriptor.content_hash.len() != 64
        || !descriptor
            .content_hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(failure(
            ExtractionErrorCode::Malformed,
            "content_hash must be a lowercase SHA-256 digest",
        ));
    }
    Ok(())
}

fn normalized_mime_type(value: &str) -> &str {
    value.split(';').next().map(str::trim).unwrap_or_default()
}

fn extract_plain_text(content: &[u8]) -> Result<Vec<RawExtractedChunk>, ExtractionFailure> {
    let text = std::str::from_utf8(content).map_err(|_| {
        failure(
            ExtractionErrorCode::Malformed,
            "plain text is not valid UTF-8",
        )
    })?;
    Ok(vec![RawExtractedChunk {
        text: text.to_owned(),
        provenance: ChunkProvenance::default(),
    }])
}

fn extract_csv(
    content: &[u8],
    limits: &ExtractionLimits,
) -> Result<Vec<RawExtractedChunk>, ExtractionFailure> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(content);
    let mut rows = Vec::new();
    for record in reader.records() {
        if rows.len() == limits.max_rows_per_sheet {
            return Err(failure(
                ExtractionErrorCode::ResourceLimitExceeded,
                "CSV exceeds max_rows_per_sheet",
            ));
        }
        let record = record.map_err(|_| {
            failure(
                ExtractionErrorCode::Malformed,
                "CSV parser rejected malformed input",
            )
        })?;
        if record.len() > limits.max_cells
            || rows
                .iter()
                .map(Vec::len)
                .sum::<usize>()
                .saturating_add(record.len())
                > limits.max_cells
        {
            return Err(failure(
                ExtractionErrorCode::ResourceLimitExceeded,
                "CSV exceeds max_cells",
            ));
        }
        rows.push(record.iter().map(str::to_owned).collect::<Vec<_>>());
    }
    let row_count = u32::try_from(rows.len()).unwrap_or(u32::MAX);
    Ok(vec![RawExtractedChunk {
        text: rows
            .into_iter()
            .map(|row| row.join(" "))
            .collect::<Vec<_>>()
            .join("\n"),
        provenance: ChunkProvenance {
            row_start: (row_count > 0).then_some(1),
            row_end: (row_count > 0).then_some(row_count),
            ..ChunkProvenance::default()
        },
    }])
}

fn extract_html(content: &[u8]) -> Result<Vec<RawExtractedChunk>, ExtractionFailure> {
    let html = std::str::from_utf8(content).map_err(|_| {
        failure(
            ExtractionErrorCode::Malformed,
            "HTML source is not valid UTF-8",
        )
    })?;
    let mut output = String::new();
    let mut chars = html.chars().peekable();
    let mut suppressed = false;
    while let Some(character) = chars.next() {
        if character != '<' {
            if !suppressed {
                output.push(character);
            }
            continue;
        }
        let mut tag = String::new();
        for next in chars.by_ref() {
            if next == '>' {
                break;
            }
            tag.push(next);
        }
        let normalized = tag.trim().to_ascii_lowercase();
        if normalized.starts_with("script") || normalized.starts_with("style") {
            suppressed = true;
        } else if normalized.starts_with("/script") || normalized.starts_with("/style") {
            suppressed = false;
        } else if !suppressed {
            output.push(' ');
        }
    }
    let output = output
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">");
    Ok(vec![RawExtractedChunk {
        text: output,
        provenance: ChunkProvenance::default(),
    }])
}

fn extract_pdf(
    content: &[u8],
    limits: &ExtractionLimits,
) -> Result<Vec<RawExtractedChunk>, ExtractionFailure> {
    if content
        .windows(b"/Encrypt".len())
        .any(|part| part == b"/Encrypt")
    {
        return Err(failure(
            ExtractionErrorCode::Encrypted,
            "encrypted PDF files are not supported",
        ));
    }
    let document = lopdf::Document::load_mem(content).map_err(|_| {
        failure(
            ExtractionErrorCode::Malformed,
            "PDF parser rejected malformed input",
        )
    })?;
    if document.is_encrypted() {
        return Err(failure(
            ExtractionErrorCode::Encrypted,
            "encrypted PDF files are not supported",
        ));
    }
    let pages = document.get_pages().keys().copied().collect::<Vec<_>>();
    if pages.is_empty() {
        return Err(failure(
            ExtractionErrorCode::Malformed,
            "PDF contains no readable pages",
        ));
    }
    if pages.len() > limits.max_pages {
        return Err(failure(
            ExtractionErrorCode::ResourceLimitExceeded,
            "PDF exceeds max_pages",
        ));
    }
    let per_page_limit = limits
        .max_extracted_bytes
        .min(limits.max_chunk_chars.saturating_mul(4));
    pages
        .into_iter()
        .map(|page| {
            document
                .extract_text_with_limit(&[page], per_page_limit)
                .map(|text| RawExtractedChunk {
                    text,
                    provenance: ChunkProvenance {
                        page: Some(page),
                        ..ChunkProvenance::default()
                    },
                })
                .map_err(|error| {
                    let message = error.to_string().to_ascii_lowercase();
                    if message.contains("limit") || message.contains("memory") {
                        failure(
                            ExtractionErrorCode::ResourceLimitExceeded,
                            "PDF decompression exceeded its resource bound",
                        )
                    } else {
                        failure(ExtractionErrorCode::Malformed, "PDF text extraction failed")
                    }
                })
        })
        .collect()
}

fn extract_spreadsheet(
    content: &[u8],
    limits: &ExtractionLimits,
) -> Result<Vec<RawExtractedChunk>, ExtractionFailure> {
    let mut workbook = open_workbook_auto_from_rs(Cursor::new(content.to_vec())).map_err(|_| {
        failure(
            ExtractionErrorCode::Malformed,
            "spreadsheet parser rejected malformed or encrypted input",
        )
    })?;
    let sheet_names = workbook.sheet_names();
    if sheet_names.len() > limits.max_sheets {
        return Err(failure(
            ExtractionErrorCode::ResourceLimitExceeded,
            "workbook exceeds max_sheets",
        ));
    }
    let mut chunks = Vec::new();
    let mut total_cells = 0_usize;
    for sheet in sheet_names {
        let range = workbook.worksheet_range(&sheet).map_err(|_| {
            failure(
                ExtractionErrorCode::Malformed,
                "spreadsheet worksheet could not be decoded",
            )
        })?;
        let mut rows = Vec::new();
        for row in range.rows() {
            if rows.len() == limits.max_rows_per_sheet {
                return Err(failure(
                    ExtractionErrorCode::ResourceLimitExceeded,
                    "worksheet exceeds max_rows_per_sheet",
                ));
            }
            total_cells = total_cells.saturating_add(row.len());
            if total_cells > limits.max_cells {
                return Err(failure(
                    ExtractionErrorCode::ResourceLimitExceeded,
                    "workbook exceeds max_cells",
                ));
            }
            rows.push(
                row.iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(" "),
            );
        }
        let row_count = u32::try_from(rows.len()).unwrap_or(u32::MAX);
        chunks.push(RawExtractedChunk {
            text: rows.join("\n"),
            provenance: ChunkProvenance {
                sheet: Some(sheet),
                row_start: (row_count > 0).then_some(1),
                row_end: (row_count > 0).then_some(row_count),
                ..ChunkProvenance::default()
            },
        });
    }
    Ok(chunks)
}

fn bounded_chunks(
    raw: Vec<RawExtractedChunk>,
    limits: &ExtractionLimits,
) -> Result<Vec<ExtractedChunk>, ExtractionFailure> {
    let mut chunks = Vec::new();
    let mut extracted_bytes = 0_usize;
    for source in raw {
        let normalized = normalize_text(&source.text);
        if normalized.is_empty() {
            continue;
        }
        let characters = normalized.chars().collect::<Vec<_>>();
        for part in characters.chunks(limits.max_chunk_chars) {
            if chunks.len() == limits.max_chunks {
                return Err(failure(
                    ExtractionErrorCode::ResourceLimitExceeded,
                    "extraction exceeds max_chunks",
                ));
            }
            let text = part.iter().collect::<String>();
            extracted_bytes = extracted_bytes.saturating_add(text.len());
            if extracted_bytes > limits.max_extracted_bytes {
                return Err(failure(
                    ExtractionErrorCode::ResourceLimitExceeded,
                    "extraction exceeds max_extracted_bytes",
                ));
            }
            chunks.push(ExtractedChunk {
                ordinal: u32::try_from(chunks.len()).unwrap_or(u32::MAX),
                text,
                provenance: source.provenance.clone(),
            });
        }
    }
    if chunks.is_empty() {
        return Err(failure(
            ExtractionErrorCode::Malformed,
            "document contains no searchable text",
        ));
    }
    Ok(chunks)
}

fn normalize_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn failure(code: ExtractionErrorCode, diagnostic: &str) -> ExtractionFailure {
    ExtractionFailure {
        code,
        diagnostic: safe_diagnostic(diagnostic),
    }
}

fn safe_diagnostic(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(256)
        .collect()
}

fn job_id(descriptor: &FileDescriptor) -> String {
    let mut digest = Sha256::new();
    digest.update(b"corrobore-file-job-v1\0");
    digest.update(descriptor.file_id.as_bytes());
    digest.update([0]);
    digest.update(descriptor.content_hash.as_bytes());
    digest.update([0]);
    digest.update(descriptor.version.to_be_bytes());
    format!("file-job--{:x}", digest.finalize())
}

fn random_token() -> Result<String, FileContentError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|error| FileContentError::Persistence(format!("lease entropy failed: {error}")))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn atomic_write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), FileContentError> {
    let parent = path
        .parent()
        .ok_or_else(|| FileContentError::Persistence("state path has no parent".to_owned()))?;
    fs::create_dir_all(parent).map_err(persistence_error)?;
    let temporary = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec(value)
        .map_err(|error| FileContentError::Persistence(error.to_string()))?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&temporary)
        .map_err(persistence_error)?;
    file.write_all(&bytes).map_err(persistence_error)?;
    file.sync_all().map_err(persistence_error)?;
    fs::rename(&temporary, path).map_err(persistence_error)?;
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(persistence_error)
}

fn persistence_error(error: std::io::Error) -> FileContentError {
    FileContentError::Persistence(error.to_string())
}

fn artifact_document(artifact: &ExtractionArtifact) -> FullTextDocument {
    let mut fields = BTreeMap::new();
    fields.insert(
        "content".to_owned(),
        artifact
            .chunks
            .iter()
            .map(|chunk| chunk.text.clone())
            .collect(),
    );
    fields.insert("name".to_owned(), vec![artifact.descriptor.name.clone()]);
    fields.insert(
        "mime_type".to_owned(),
        vec![artifact.descriptor.mime_type.clone()],
    );
    fields.insert(
        "source_object_id".to_owned(),
        vec![artifact.descriptor.source_object_id.clone()],
    );
    fields.insert(
        "content_hash".to_owned(),
        vec![artifact.descriptor.content_hash.clone()],
    );
    fields.insert(
        "owner_id".to_owned(),
        artifact.descriptor.access.owner_ids.clone(),
    );
    FullTextDocument {
        id: artifact.descriptor.file_id.clone(),
        record_class: FullTextRecordClass::FileContent,
        kind: "file".to_owned(),
        revision: artifact.descriptor.version,
        fields,
        access: artifact.descriptor.access.clone(),
    }
}

fn extend_filters(filters: &mut Vec<FullTextFieldFilter>, field: &str, values: &[String]) {
    for value in values {
        filters.push(FullTextFieldFilter {
            field: field.to_owned(),
            value: value.clone(),
        });
    }
}

fn read_artifacts(path: &Path) -> Result<BTreeMap<String, ExtractionArtifact>, FileContentError> {
    let artifacts = serde_json::from_slice::<Vec<ExtractionArtifact>>(
        &fs::read(path).map_err(persistence_error)?,
    )
    .map_err(|error| FileContentError::Persistence(error.to_string()))?;
    Ok(artifacts
        .into_iter()
        .map(|artifact| (artifact.descriptor.file_id.clone(), artifact))
        .collect())
}

fn decorate_hit(
    hit: &mut FullTextSearchHit,
    artifact: &ExtractionArtifact,
    query: &str,
    snippet_chars: usize,
) {
    let query_lower = query.to_lowercase();
    let selected = artifact
        .chunks
        .iter()
        .find(|chunk| chunk.text.to_lowercase().contains(&query_lower))
        .or_else(|| artifact.chunks.first());
    let Some(chunk) = selected else {
        return;
    };
    let text = chunk.text.chars().take(snippet_chars).collect::<String>();
    let marked = if let Some(offset) = text.to_lowercase().find(&query_lower) {
        let end = offset.saturating_add(query.len());
        if text.is_char_boundary(offset) && text.is_char_boundary(end) {
            format!(
                "{}<mark>{}</mark>{}",
                &text[..offset],
                &text[offset..end],
                &text[end..]
            )
        } else {
            text
        }
    } else {
        text
    };
    hit.snippet = Some(marked);
    hit.highlights = vec![query.to_owned()];
    hit.metadata.insert(
        "source_object_id".to_owned(),
        artifact.descriptor.source_object_id.clone(),
    );
    hit.metadata.insert(
        "content_hash".to_owned(),
        artifact.descriptor.content_hash.clone(),
    );
    hit.metadata.insert(
        "version".to_owned(),
        artifact.descriptor.version.to_string(),
    );
    if let Some(page) = chunk.provenance.page {
        hit.metadata.insert("page".to_owned(), page.to_string());
    }
    if let Some(sheet) = &chunk.provenance.sheet {
        hit.metadata.insert("sheet".to_owned(), sheet.clone());
    }
    if let Some(row) = chunk.provenance.row_start {
        hit.metadata.insert("row_start".to_owned(), row.to_string());
    }
    if let Some(row) = chunk.provenance.row_end {
        hit.metadata.insert("row_end".to_owned(), row.to_string());
    }
}

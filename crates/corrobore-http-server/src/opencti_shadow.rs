// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! Asynchronous shadow dispatch and durable privacy-safe parity reporting.

use std::{
    collections::VecDeque,
    fs::{self, OpenOptions},
    future::Future,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use corrobore_engine::{
    DivergenceBaseline, KnowledgeDataRequest, QueryClass, ShadowComparisonReport, ShadowMetrics,
    ShadowRequestMetadata, ShadowSamplingPolicy,
};
use serde::{Deserialize, Serialize};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

const SHADOW_STATE_SCHEMA_VERSION: u32 = 1;

/// Reason one eligible request did not start shadow work.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ShadowShedReason {
    /// Snapshot/catch-up parity has not opened the shadow gate.
    SynchronizationGate,
    /// The deterministic sampling policy excluded the request.
    SamplingPolicy,
    /// All independently bounded shadow execution permits are in use.
    ConcurrencyLimit,
}

/// Admission decision for one reference read.
#[derive(Debug)]
pub enum ShadowAdmission {
    /// Shadow work may start while this permit remains alive.
    Accepted(OwnedSemaphorePermit),
    /// Reference execution continues without shadow work.
    Shed(ShadowShedReason),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PersistedShadowReports {
    schema_version: u32,
    reports: VecDeque<ShadowComparisonReport>,
}

/// Durable coordinator for sampling, load shedding, baselines, reports, and metrics.
#[derive(Debug)]
pub struct OpenCtiShadowRuntime {
    state_path: Option<PathBuf>,
    policy: ShadowSamplingPolicy,
    baselines: Vec<DivergenceBaseline>,
    permits: Arc<Semaphore>,
    max_reports: usize,
    reports: VecDeque<ShadowComparisonReport>,
    metrics: ShadowMetrics,
}

impl OpenCtiShadowRuntime {
    /// Restore bounded reports and initialize admission controls.
    pub fn open(
        state_path: Option<PathBuf>,
        policy: ShadowSamplingPolicy,
        baselines: Vec<DivergenceBaseline>,
        max_concurrency: usize,
        max_reports: usize,
    ) -> Result<Self, String> {
        if max_concurrency == 0 {
            return Err("shadow max concurrency must be positive".to_owned());
        }
        if max_reports == 0 {
            return Err("shadow max reports must be positive".to_owned());
        }
        if policy.default_percentage_basis_points > 10_000
            || policy
                .rules
                .iter()
                .any(|rule| rule.percentage_basis_points > 10_000)
        {
            return Err(
                "shadow sampling percentages must not exceed 10000 basis points".to_owned(),
            );
        }
        if baselines
            .iter()
            .any(|baseline| baseline.owner.trim().is_empty())
        {
            return Err("shadow divergence baselines require an owner".to_owned());
        }
        let persisted = state_path
            .as_deref()
            .filter(|path| path.is_file())
            .map(read_reports)
            .transpose()?;
        if persisted
            .as_ref()
            .is_some_and(|state| state.schema_version != SHADOW_STATE_SCHEMA_VERSION)
        {
            return Err("unsupported OpenCTI shadow report state version".to_owned());
        }
        let mut reports = persisted.map_or_else(VecDeque::new, |state| state.reports);
        while reports.len() > max_reports {
            reports.pop_front();
        }
        let mut metrics = ShadowMetrics::default();
        for report in &reports {
            metrics.record(report);
        }
        Ok(Self {
            state_path,
            policy,
            baselines,
            permits: Arc::new(Semaphore::new(max_concurrency)),
            max_reports,
            reports,
            metrics,
        })
    }

    /// Apply synchronization, deterministic sampling, and concurrency gates.
    pub fn admit(
        &self,
        request: &KnowledgeDataRequest,
        metadata: &ShadowRequestMetadata,
        synchronization_gate_open: bool,
    ) -> ShadowAdmission {
        if !synchronization_gate_open {
            return ShadowAdmission::Shed(ShadowShedReason::SynchronizationGate);
        }
        if !self.policy.should_sample(request, metadata) {
            return ShadowAdmission::Shed(ShadowShedReason::SamplingPolicy);
        }
        match Arc::clone(&self.permits).try_acquire_owned() {
            Ok(permit) => ShadowAdmission::Accepted(permit),
            Err(_) => ShadowAdmission::Shed(ShadowShedReason::ConcurrencyLimit),
        }
    }

    /// Baselines used by background comparison.
    #[must_use]
    pub fn baselines(&self) -> Vec<DivergenceBaseline> {
        self.baselines.clone()
    }

    /// Record and atomically persist one bounded comparison report.
    pub fn record(&mut self, report: ShadowComparisonReport) -> Result<(), String> {
        self.metrics.record(&report);
        self.reports.push_back(report);
        while self.reports.len() > self.max_reports {
            self.reports.pop_front();
        }
        if let Some(path) = &self.state_path {
            write_reports(
                path,
                &PersistedShadowReports {
                    schema_version: SHADOW_STATE_SCHEMA_VERSION,
                    reports: self.reports.clone(),
                },
            )?;
        }
        Ok(())
    }

    /// Query recent reports by bounded low-cardinality dimensions.
    #[must_use]
    pub fn reports(
        &self,
        query_class: Option<QueryClass>,
        release: Option<&str>,
        limit: usize,
    ) -> Vec<ShadowComparisonReport> {
        self.reports
            .iter()
            .rev()
            .filter(|report| query_class.is_none_or(|value| report.query_class == value))
            .filter(|report| {
                release.is_none_or(|value| report.shadow_provider.release.as_str() == value)
            })
            .take(limit.min(self.max_reports))
            .cloned()
            .collect()
    }

    /// Current low-cardinality parity and latency aggregates.
    #[must_use]
    pub const fn metrics(&self) -> &ShadowMetrics {
        &self.metrics
    }
}

fn read_reports(path: &Path) -> Result<PersistedShadowReports, String> {
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    serde_json::from_slice(&bytes).map_err(|error| error.to_string())
}

fn write_reports(path: &Path, state: &PersistedShadowReports) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "shadow report state path has no parent".to_owned())?;
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
/// Terminal state of one independently budgeted shadow execution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShadowCompletion<T> {
    /// Shadow provider completed successfully.
    Completed(T),
    /// Shadow provider failed; provider details are deliberately discarded.
    Failed,
    /// Independent shadow deadline elapsed.
    TimedOut,
    /// Sampling or concurrency policy shed the shadow execution.
    Shed,
}

/// Return the reference future without waiting for shadow work.
///
/// The implementation will start accepted shadow work independently, wait only
/// for the reference future on the caller path, and finish comparison/reporting
/// in a detached task bounded by `shadow_timeout`. Reference response content
/// and latency therefore remain independent of success, failure, timeout, or
/// shedding in the shadow path.
pub async fn dispatch_shadowed<
    ReferenceFuture,
    ShadowFuture,
    Response,
    ShadowValue,
    ShadowError,
    F,
>(
    reference_future: ReferenceFuture,
    shadow_future: Option<ShadowFuture>,
    _shadow_timeout: Duration,
    report: F,
) -> Response
where
    ReferenceFuture: Future<Output = Response>,
    ShadowFuture: Future<Output = Result<ShadowValue, ShadowError>> + Send + 'static,
    Response: Clone + Send + 'static,
    ShadowValue: Send + 'static,
    ShadowError: Send + 'static,
    F: FnOnce(Response, ShadowCompletion<ShadowValue>) + Send + 'static,
{
    let response = reference_future.await;
    let report_response = response.clone();
    if let Some(shadow_future) = shadow_future {
        tokio::spawn(async move {
            let completion = match tokio::time::timeout(_shadow_timeout, shadow_future).await {
                Ok(Ok(value)) => ShadowCompletion::Completed(value),
                Ok(Err(_)) => ShadowCompletion::Failed,
                Err(_) => ShadowCompletion::TimedOut,
            };
            report(report_response, completion);
        });
    } else {
        report(report_response, ShadowCompletion::Shed);
    }
    response
}

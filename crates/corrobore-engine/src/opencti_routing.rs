// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! Deterministic progressive OpenCTI read routing and rollback contracts.
//!
//! The router owns provider selection only. Provider execution stays behind
//! the Knowledge Data Engine boundary, allowing HTTP and embedded hosts to use
//! the same policy, sticky-session, circuit-breaker, and privacy-safe audit
//! decisions.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{KnowledgeDataRequest, OperationKind, QueryClass};

const ROUTING_STATE_SCHEMA_VERSION: u32 = 1;

/// Provider eligible to serve one visible read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderTarget {
    /// Existing Elasticsearch or OpenSearch provider.
    Reference,
    /// Corrobore Knowledge Data Engine provider.
    Corrobore,
}

/// Progressive read migration mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadRoutingMode {
    /// All visible reads remain on the reference provider.
    ReferenceOnly,
    /// Reference reads may execute Corrobore asynchronously for comparison.
    Shadow,
    /// Selected reads use Corrobore according to deterministic rules.
    Canary,
    /// Graph-native reads use Corrobore; other reads remain on the reference.
    GraphReads,
    /// Every supported read uses Corrobore.
    PrimaryReads,
}

/// Bounded non-sensitive dimensions supplied by the OpenCTI integration.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadRoutingMetadata {
    /// Deployment environment.
    pub environment: String,
    /// Optional logical entity type.
    pub entity_type: Option<String>,
    /// Optional bounded user cohort.
    pub user_cohort: Option<String>,
    /// Enabled feature flags relevant to provider selection.
    pub feature_flags: BTreeSet<String>,
    /// Optional pagination or application session identity.
    pub session_id: Option<String>,
    /// Provider-compatible index generation for this request.
    pub index_generation: Option<String>,
}

/// First-match deterministic canary routing rule.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadRoutingRule {
    /// Optional environment selector.
    pub environment: Option<String>,
    /// Optional operation selector.
    pub operation: Option<OperationKind>,
    /// Optional query-class selector.
    pub query_class: Option<QueryClass>,
    /// Optional entity-type selector.
    pub entity_type: Option<String>,
    /// Optional organization selector.
    pub organization_id: Option<String>,
    /// Optional tenant selector.
    pub tenant_id: Option<String>,
    /// Optional bounded cohort selector.
    pub user_cohort: Option<String>,
    /// Optional feature flag required for selection.
    pub required_feature_flag: Option<String>,
    /// Selected percentage from zero through 10,000 basis points.
    pub percentage_basis_points: u16,
}

/// SLO gates used for canary promotion and automatic rollback.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadRoutingThresholds {
    /// Maximum rolling provider error rate.
    pub max_error_rate_basis_points: u16,
    /// Maximum rolling P95 latency.
    pub max_latency_p95_ms: u64,
    /// Minimum request count required before promotion.
    pub minimum_soak_requests: u64,
}

/// Versioned progressive routing policy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadRoutingPolicy {
    /// Operator-owned stable policy version.
    pub policy_version: String,
    /// Current migration mode.
    pub mode: ReadRoutingMode,
    /// Canary fallback percentage when no rule matches.
    pub default_percentage_basis_points: u16,
    /// First-match deterministic routing rules.
    pub rules: Vec<ReadRoutingRule>,
    /// Continuous SLO thresholds.
    pub thresholds: ReadRoutingThresholds,
}

/// Current health, synchronization, security, parity, and performance gates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadRoutingGates {
    /// Canonical synchronization and validation are complete.
    pub synchronization_ready: bool,
    /// Reference provider is current enough for immediate rollback.
    pub reference_fresh: bool,
    /// Corrobore provider is available.
    pub corrobore_available: bool,
    /// Canonical or derived corruption was detected.
    pub corruption_detected: bool,
    /// Authorization or information-disclosure divergence was detected.
    pub security_divergence: bool,
    /// Functional parity breached the configured gate.
    pub parity_breach: bool,
    /// Rolling Corrobore error rate.
    pub error_rate_basis_points: u16,
    /// Rolling Corrobore P95 latency.
    pub latency_p95_ms: u64,
}

/// Stable reason for automatic traffic rollback.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RollbackReason {
    /// Authorization or information disclosure diverged.
    SecurityDivergence,
    /// Corruption was detected.
    Corruption,
    /// Corrobore became unavailable.
    Unavailability,
    /// Functional parity breached the gate.
    ParityBreach,
    /// Rolling provider error rate exceeded its threshold.
    ErrorRate,
    /// Rolling P95 latency exceeded its threshold.
    ExcessiveLatency,
    /// Synchronization freshness gate closed.
    Synchronization,
    /// Operator requested immediate rollback.
    OperatorRequested,
}

/// Stable explanation for a provider decision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutingDecisionReason {
    /// Policy deliberately keeps reads on the reference provider.
    ReferenceMode,
    /// Shadow mode keeps reference visible and compares Corrobore optionally.
    ShadowMode,
    /// Canary rule selected the request.
    MatchedRule {
        /// Zero-based rule index.
        index: usize,
    },
    /// Default canary percentage selected the request.
    DefaultCanary,
    /// Canary sampling or selectors did not select the request.
    CanaryNotSelected,
    /// Graph-native migration mode selected Corrobore.
    GraphReadMode,
    /// Operation is outside the graph-native surface.
    UnsupportedGraphRead,
    /// Primary-read mode selected Corrobore.
    PrimaryReadMode,
    /// Existing session binding was reused.
    StickySession,
    /// A gate or circuit breaker restored reference traffic.
    AutomaticRollback(RollbackReason),
}

/// One provider selection for a visible read and optional detached comparison.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadRoutingDecision {
    /// Exactly one provider that owns the visible response.
    pub primary: ProviderTarget,
    /// Optional detached provider whose response cannot become visible.
    pub shadow: Option<ProviderTarget>,
    /// Stable explanation for the selection.
    pub reason: RoutingDecisionReason,
    /// Policy version used for the decision.
    pub policy_version: String,
}

/// Stable reason a request cannot be served safely.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutingBlockReason {
    /// Neither Corrobore nor a fresh reference can safely serve the read.
    ReferenceNotFresh,
    /// Session continuation attempted another provider/index generation.
    IncompatibleSessionGeneration,
    /// Only read operations may use the routing surface.
    UnsupportedOperation,
    /// Durable decision evidence could not be committed.
    StatePersistenceFailed,
}

/// Explicit fail-closed routing result.
#[derive(Clone, Debug, PartialEq, Eq, Error, Serialize, Deserialize)]
#[error("OpenCTI read routing blocked: {reason:?}")]
pub struct ReadRoutingBlock {
    /// Stable non-sensitive block reason.
    pub reason: RoutingBlockReason,
}

/// Runtime signal that opens the circuit breaker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutingSignal {
    /// Authorization or leakage divergence.
    SecurityDivergence,
    /// Canonical or derived corruption.
    Corruption,
    /// Provider unavailability.
    Unavailability,
    /// Functional parity breach.
    ParityBreach,
    /// Excessive rolling error rate.
    ErrorRate,
    /// Excessive rolling latency.
    ExcessiveLatency,
    /// Operator-triggered rollback.
    OperatorRollback,
}

/// Privacy-safe decision evidence correlated with the original request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadRoutingAuditEvent {
    /// Request correlation identity.
    pub correlation_id: String,
    /// Bounded query class.
    pub query_class: QueryClass,
    /// Selected visible provider.
    pub primary: ProviderTarget,
    /// Stable decision reason.
    pub reason: RoutingDecisionReason,
    /// Policy version.
    pub policy_version: String,
    /// Event time supplied by the host.
    pub timestamp_unix_ms: u64,
}

/// Rolling canary or full-read evidence window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutingWindow {
    /// Completed routed reads.
    pub requests: u64,
    /// Failed routed reads.
    pub errors: u64,
    /// Observed P95 latency.
    pub latency_p95_ms: u64,
    /// Functional parity breaches.
    pub parity_breaches: u64,
    /// Security divergences; promotion requires exactly zero.
    pub security_divergences: u64,
}

impl RoutingWindow {
    /// Return whether this window satisfies promotion gates.
    #[must_use]
    pub fn promotion_ready(&self, thresholds: &ReadRoutingThresholds) -> bool {
        if self.requests < thresholds.minimum_soak_requests
            || self.latency_p95_ms > thresholds.max_latency_p95_ms
            || self.parity_breaches != 0
            || self.security_divergences != 0
        {
            return false;
        }
        let error_basis_points = self
            .errors
            .saturating_mul(10_000)
            .checked_div(self.requests)
            .unwrap_or(u64::MAX);
        error_basis_points <= u64::from(thresholds.max_error_rate_basis_points)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct SessionBinding {
    provider: ProviderTarget,
    generation: Option<String>,
}

/// Stateful deterministic router with sticky bindings and circuit breaker.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OpenCtiReadRoutingRuntime {
    policy: ReadRoutingPolicy,
    sessions: BTreeMap<String, SessionBinding>,
    circuit: Option<RollbackReason>,
    audits: Vec<ReadRoutingAuditEvent>,
    #[serde(skip)]
    state_path: Option<PathBuf>,
    max_audits: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PersistedReadRoutingState {
    schema_version: u32,
    policy_version: String,
    sessions: BTreeMap<String, SessionBinding>,
    circuit: Option<RollbackReason>,
    audits: Vec<ReadRoutingAuditEvent>,
}

impl OpenCtiReadRoutingRuntime {
    /// Validate policy bounds and initialize an empty runtime.
    pub fn new(policy: ReadRoutingPolicy) -> Result<Self, String> {
        Self::open(None, policy, 10_000)
    }

    /// Restore compatible sticky bindings, circuit state, and bounded audits.
    pub fn open(
        state_path: Option<PathBuf>,
        policy: ReadRoutingPolicy,
        max_audits: usize,
    ) -> Result<Self, String> {
        if policy.policy_version.trim().is_empty() {
            return Err("read routing policy version must not be empty".to_owned());
        }
        if policy.default_percentage_basis_points > 10_000
            || policy
                .rules
                .iter()
                .any(|rule| rule.percentage_basis_points > 10_000)
        {
            return Err("read routing percentages must not exceed 10000 basis points".to_owned());
        }
        if policy.thresholds.max_error_rate_basis_points > 10_000 {
            return Err(
                "read routing error threshold must not exceed 10000 basis points".to_owned(),
            );
        }
        if policy.thresholds.max_latency_p95_ms == 0 || policy.thresholds.minimum_soak_requests == 0
        {
            return Err("read routing latency and soak thresholds must be positive".to_owned());
        }
        if max_audits == 0 {
            return Err("read routing audit retention must be positive".to_owned());
        }
        let persisted = state_path
            .as_deref()
            .filter(|path| path.is_file())
            .map(read_routing_state)
            .transpose()?;
        if persisted
            .as_ref()
            .is_some_and(|state| state.schema_version != ROUTING_STATE_SCHEMA_VERSION)
        {
            return Err("unsupported OpenCTI read routing state version".to_owned());
        }
        let compatible = persisted.filter(|state| state.policy_version == policy.policy_version);
        let mut audits = compatible
            .as_ref()
            .map(|state| state.audits.clone())
            .unwrap_or_default();
        if audits.len() > max_audits {
            audits.drain(0..audits.len() - max_audits);
        }
        Ok(Self {
            policy,
            sessions: compatible
                .as_ref()
                .map(|state| state.sessions.clone())
                .unwrap_or_default(),
            circuit: compatible.as_ref().and_then(|state| state.circuit),
            audits,
            state_path,
            max_audits,
        })
    }

    /// Select exactly one primary provider, optional shadow work, and preserve
    /// privacy-safe evidence. Phase 3 will evaluate selectors, sticky bindings,
    /// gates, and circuit state in that order.
    pub fn decide(
        &mut self,
        request: &KnowledgeDataRequest,
        metadata: &ReadRoutingMetadata,
        gates: &ReadRoutingGates,
        timestamp_unix_ms: u64,
    ) -> Result<ReadRoutingDecision, ReadRoutingBlock> {
        let query_class =
            QueryClass::from_operation(&request.operation).ok_or(ReadRoutingBlock {
                reason: RoutingBlockReason::UnsupportedOperation,
            })?;
        let (mut primary, mut shadow, mut reason) =
            self.policy_selection(request, metadata, query_class);

        let rollback = self.circuit.or_else(|| self.gate_rollback(gates));
        if primary == ProviderTarget::Corrobore
            && let Some(rollback_reason) = rollback
        {
            if !gates.reference_fresh {
                return Err(ReadRoutingBlock {
                    reason: RoutingBlockReason::ReferenceNotFresh,
                });
            }
            primary = ProviderTarget::Reference;
            shadow = None;
            reason = RoutingDecisionReason::AutomaticRollback(rollback_reason);
            self.circuit.get_or_insert(rollback_reason);
        }

        if let Some(session_id) = metadata.session_id.as_ref() {
            if let Some(binding) = self.sessions.get(session_id) {
                if binding.generation != metadata.index_generation {
                    return Err(ReadRoutingBlock {
                        reason: RoutingBlockReason::IncompatibleSessionGeneration,
                    });
                }
                if binding.provider != primary {
                    if matches!(reason, RoutingDecisionReason::AutomaticRollback(_)) {
                        return Err(ReadRoutingBlock {
                            reason: RoutingBlockReason::IncompatibleSessionGeneration,
                        });
                    }
                    primary = binding.provider;
                    shadow = opposite_provider(primary);
                    reason = RoutingDecisionReason::StickySession;
                }
            } else {
                self.sessions.insert(
                    session_id.clone(),
                    SessionBinding {
                        provider: primary,
                        generation: metadata.index_generation.clone(),
                    },
                );
            }
        }

        let decision = ReadRoutingDecision {
            primary,
            shadow,
            reason: reason.clone(),
            policy_version: self.policy.policy_version.clone(),
        };
        self.audits.push(ReadRoutingAuditEvent {
            correlation_id: request.context.correlation_id.clone(),
            query_class,
            primary,
            reason,
            policy_version: self.policy.policy_version.clone(),
            timestamp_unix_ms,
        });
        if self.audits.len() > self.max_audits {
            self.audits.drain(0..self.audits.len() - self.max_audits);
        }
        self.persist().map_err(|_| ReadRoutingBlock {
            reason: RoutingBlockReason::StatePersistenceFailed,
        })?;
        Ok(decision)
    }

    /// Open the circuit breaker while retaining the first rollback cause.
    pub fn record_signal(
        &mut self,
        signal: RoutingSignal,
        _timestamp_unix_ms: u64,
    ) -> Result<(), String> {
        self.circuit.get_or_insert(match signal {
            RoutingSignal::SecurityDivergence => RollbackReason::SecurityDivergence,
            RoutingSignal::Corruption => RollbackReason::Corruption,
            RoutingSignal::Unavailability => RollbackReason::Unavailability,
            RoutingSignal::ParityBreach => RollbackReason::ParityBreach,
            RoutingSignal::ErrorRate => RollbackReason::ErrorRate,
            RoutingSignal::ExcessiveLatency => RollbackReason::ExcessiveLatency,
            RoutingSignal::OperatorRollback => RollbackReason::OperatorRequested,
        });
        self.persist()
    }

    /// Return current rollback cause, if the circuit is open.
    #[must_use]
    pub const fn rollback_reason(&self) -> Option<RollbackReason> {
        self.circuit
    }

    /// Explain the newest decision for one correlated request.
    #[must_use]
    pub fn explain(&self, correlation_id: &str) -> Option<&ReadRoutingAuditEvent> {
        self.audits
            .iter()
            .rev()
            .find(|event| event.correlation_id == correlation_id)
    }

    /// Return bounded newest-first provider-decision evidence.
    #[must_use]
    pub fn audits(&self, limit: usize) -> Vec<ReadRoutingAuditEvent> {
        self.audits.iter().rev().take(limit).cloned().collect()
    }

    fn persist(&self) -> Result<(), String> {
        let Some(path) = self.state_path.as_deref() else {
            return Ok(());
        };
        write_routing_state(
            path,
            &PersistedReadRoutingState {
                schema_version: ROUTING_STATE_SCHEMA_VERSION,
                policy_version: self.policy.policy_version.clone(),
                sessions: self.sessions.clone(),
                circuit: self.circuit,
                audits: self.audits.clone(),
            },
        )
    }

    fn policy_selection(
        &self,
        request: &KnowledgeDataRequest,
        metadata: &ReadRoutingMetadata,
        query_class: QueryClass,
    ) -> (
        ProviderTarget,
        Option<ProviderTarget>,
        RoutingDecisionReason,
    ) {
        match self.policy.mode {
            ReadRoutingMode::ReferenceOnly => (
                ProviderTarget::Reference,
                None,
                RoutingDecisionReason::ReferenceMode,
            ),
            ReadRoutingMode::Shadow => (
                ProviderTarget::Reference,
                Some(ProviderTarget::Corrobore),
                RoutingDecisionReason::ShadowMode,
            ),
            ReadRoutingMode::GraphReads if query_class == QueryClass::Graph => (
                ProviderTarget::Corrobore,
                Some(ProviderTarget::Reference),
                RoutingDecisionReason::GraphReadMode,
            ),
            ReadRoutingMode::GraphReads => (
                ProviderTarget::Reference,
                Some(ProviderTarget::Corrobore),
                RoutingDecisionReason::UnsupportedGraphRead,
            ),
            ReadRoutingMode::PrimaryReads => (
                ProviderTarget::Corrobore,
                Some(ProviderTarget::Reference),
                RoutingDecisionReason::PrimaryReadMode,
            ),
            ReadRoutingMode::Canary => {
                let matching = self
                    .policy
                    .rules
                    .iter()
                    .enumerate()
                    .find(|(_, rule)| rule.matches(request, metadata, query_class));
                let percentage = matching
                    .map_or(self.policy.default_percentage_basis_points, |(_, rule)| {
                        rule.percentage_basis_points
                    });
                if selected_by_percentage(request, metadata, query_class, percentage) {
                    (
                        ProviderTarget::Corrobore,
                        Some(ProviderTarget::Reference),
                        matching.map_or(RoutingDecisionReason::DefaultCanary, |(index, _)| {
                            RoutingDecisionReason::MatchedRule { index }
                        }),
                    )
                } else {
                    (
                        ProviderTarget::Reference,
                        Some(ProviderTarget::Corrobore),
                        RoutingDecisionReason::CanaryNotSelected,
                    )
                }
            }
        }
    }

    fn gate_rollback(&self, gates: &ReadRoutingGates) -> Option<RollbackReason> {
        if gates.security_divergence {
            Some(RollbackReason::SecurityDivergence)
        } else if gates.corruption_detected {
            Some(RollbackReason::Corruption)
        } else if !gates.corrobore_available {
            Some(RollbackReason::Unavailability)
        } else if gates.parity_breach {
            Some(RollbackReason::ParityBreach)
        } else if !gates.synchronization_ready {
            Some(RollbackReason::Synchronization)
        } else if gates.error_rate_basis_points > self.policy.thresholds.max_error_rate_basis_points
        {
            Some(RollbackReason::ErrorRate)
        } else if gates.latency_p95_ms > self.policy.thresholds.max_latency_p95_ms {
            Some(RollbackReason::ExcessiveLatency)
        } else {
            None
        }
    }
}

impl ReadRoutingRule {
    fn matches(
        &self,
        request: &KnowledgeDataRequest,
        metadata: &ReadRoutingMetadata,
        query_class: QueryClass,
    ) -> bool {
        self.environment
            .as_deref()
            .is_none_or(|value| value == metadata.environment)
            && self
                .operation
                .is_none_or(|value| value == request.operation.kind())
            && self.query_class.is_none_or(|value| value == query_class)
            && self
                .entity_type
                .as_deref()
                .is_none_or(|value| Some(value) == metadata.entity_type.as_deref())
            && self.organization_id.as_deref().is_none_or(|value| {
                request
                    .context
                    .access
                    .organization_ids
                    .iter()
                    .any(|candidate| candidate == value)
            })
            && self
                .tenant_id
                .as_deref()
                .is_none_or(|value| Some(value) == request.context.access.tenant_id.as_deref())
            && self
                .user_cohort
                .as_deref()
                .is_none_or(|value| Some(value) == metadata.user_cohort.as_deref())
            && self
                .required_feature_flag
                .as_deref()
                .is_none_or(|value| metadata.feature_flags.contains(value))
    }
}

fn selected_by_percentage(
    request: &KnowledgeDataRequest,
    metadata: &ReadRoutingMetadata,
    query_class: QueryClass,
    percentage_basis_points: u16,
) -> bool {
    if percentage_basis_points == 0 {
        return false;
    }
    if percentage_basis_points >= 10_000 {
        return true;
    }
    let mut hasher = Sha256::new();
    for value in [
        metadata
            .session_id
            .as_deref()
            .unwrap_or(request.context.correlation_id.as_str()),
        metadata.environment.as_str(),
        metadata.entity_type.as_deref().unwrap_or_default(),
        metadata.user_cohort.as_deref().unwrap_or_default(),
        request
            .context
            .access
            .tenant_id
            .as_deref()
            .unwrap_or_default(),
        query_class.as_str(),
    ] {
        hasher.update(value.as_bytes());
        hasher.update([0]);
    }
    let digest = hasher.finalize();
    let selector = u64::from_be_bytes([
        digest[0], digest[1], digest[2], digest[3], digest[4], digest[5], digest[6], digest[7],
    ]) % 10_000;
    selector < u64::from(percentage_basis_points)
}

const fn opposite_provider(provider: ProviderTarget) -> Option<ProviderTarget> {
    Some(match provider {
        ProviderTarget::Reference => ProviderTarget::Corrobore,
        ProviderTarget::Corrobore => ProviderTarget::Reference,
    })
}

fn read_routing_state(path: &Path) -> Result<PersistedReadRoutingState, String> {
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    serde_json::from_slice(&bytes).map_err(|error| error.to_string())
}

fn write_routing_state(path: &Path, state: &PersistedReadRoutingState) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "read routing state path has no parent".to_owned())?;
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

// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

//! Provider-neutral OpenCTI shadow-read normalization and parity contracts.
//!
//! The implementation compares typed Knowledge Data Engine responses only.
//! Reports retain pseudonymous identifiers and field names, never property
//! values or provider error messages, so security divergences remain useful
//! without widening operator access.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::{
    KnowledgeDataOperation, KnowledgeDataOutcome, KnowledgeDataRequest, KnowledgeDataResponse,
    KnowledgeDataResponseEnvelope, KnowledgeRecord, OperationKind,
};

/// Stable read family used for comparison, sampling, reports, and metrics.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryClass {
    /// One record by stable identifier.
    PointRead,
    /// Bounded record collection.
    Collection,
    /// Snapshot-bound cursor page.
    Pagination,
    /// Structured or full-text search.
    Search,
    /// Scalar count.
    Count,
    /// Bucket aggregation.
    Aggregation,
    /// Neighbor, traversal, or subgraph projection.
    Graph,
}

impl QueryClass {
    /// Low-cardinality label used by metrics.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PointRead => "point_read",
            Self::Collection => "collection",
            Self::Pagination => "pagination",
            Self::Search => "search",
            Self::Count => "count",
            Self::Aggregation => "aggregation",
            Self::Graph => "graph",
        }
    }

    /// Classifies supported read operations; mutations return `None`.
    pub const fn from_operation(operation: &KnowledgeDataOperation) -> Option<Self> {
        match operation {
            KnowledgeDataOperation::GetById(_) => Some(Self::PointRead),
            KnowledgeDataOperation::List(_) => Some(Self::Collection),
            KnowledgeDataOperation::Paginate(_) => Some(Self::Pagination),
            KnowledgeDataOperation::Search(_) => Some(Self::Search),
            KnowledgeDataOperation::Count(_) => Some(Self::Count),
            KnowledgeDataOperation::Aggregate(_) => Some(Self::Aggregation),
            KnowledgeDataOperation::Neighbors(_)
            | KnowledgeDataOperation::Traverse(_)
            | KnowledgeDataOperation::Subgraph(_) => Some(Self::Graph),
            _ => None,
        }
    }
}

/// Version identity retained for one provider execution.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderDescriptor {
    /// Stable provider name such as `opensearch` or `corrobore`.
    pub name: String,
    /// Provider implementation version.
    pub version: String,
    /// Bounded deployment release label.
    pub release: String,
}

/// One completed provider execution.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderExecution {
    /// Provider identity.
    pub provider: ProviderDescriptor,
    /// Wall-clock execution latency.
    pub latency_ms: u64,
    /// Typed provider outcome.
    pub envelope: KnowledgeDataResponseEnvelope,
}

/// Non-sensitive dimensions used to select a sampling rule.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShadowRequestMetadata {
    /// Deployment environment.
    pub environment: String,
    /// Optional logical entity type.
    pub entity_type: Option<String>,
    /// Optional bounded user cohort.
    pub user_cohort: Option<String>,
}

/// One first-match deterministic sampling rule.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShadowSamplingRule {
    /// Optional environment selector.
    pub environment: Option<String>,
    /// Optional typed operation selector.
    pub operation: Option<OperationKind>,
    /// Optional query-class selector.
    pub query_class: Option<QueryClass>,
    /// Optional entity-type selector.
    pub entity_type: Option<String>,
    /// Optional organization selector.
    pub organization_id: Option<String>,
    /// Optional tenant selector.
    pub tenant_id: Option<String>,
    /// Optional bounded user-cohort selector.
    pub user_cohort: Option<String>,
    /// Sampling percentage expressed as 0 through 10,000 basis points.
    pub percentage_basis_points: u16,
}

/// Deterministic sampling policy.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShadowSamplingPolicy {
    /// Fallback percentage expressed as basis points.
    pub default_percentage_basis_points: u16,
    /// First matching rule wins.
    pub rules: Vec<ShadowSamplingRule>,
}

impl ShadowSamplingPolicy {
    /// Selects traffic by hashing stable request dimensions. The implementation
    /// will validate percentages, apply every selector, and avoid random state
    /// so retries produce the same decision.
    pub fn should_sample(
        &self,
        request: &KnowledgeDataRequest,
        metadata: &ShadowRequestMetadata,
    ) -> bool {
        let Some(query_class) = QueryClass::from_operation(&request.operation) else {
            return false;
        };
        let percentage = self
            .rules
            .iter()
            .find(|rule| rule.matches(request, metadata, query_class))
            .map_or(self.default_percentage_basis_points, |rule| {
                rule.percentage_basis_points
            });
        if percentage > 10_000 {
            return false;
        }
        if percentage == 0 {
            return false;
        }
        if percentage == 10_000 {
            return true;
        }
        let mut hasher = Sha256::new();
        for value in [
            request.context.correlation_id.as_str(),
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
        selector < u64::from(percentage)
    }
}

impl ShadowSamplingRule {
    fn matches(
        &self,
        request: &KnowledgeDataRequest,
        metadata: &ShadowRequestMetadata,
        query_class: QueryClass,
    ) -> bool {
        self.percentage_basis_points <= 10_000
            && self
                .environment
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
    }
}

/// Redacted property mismatch for one pseudonymous record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PropertyDifference {
    /// SHA-256 evidence identity, never the source identifier.
    pub record: String,
    /// Differing property paths without values.
    pub fields: Vec<String>,
}

/// Redacted ordering mismatch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderingDifference {
    /// Zero-based result position.
    pub position: usize,
    /// Pseudonymous reference identifier.
    pub reference: Option<String>,
    /// Pseudonymous shadow identifier.
    pub shadow: Option<String>,
}

/// Severity gate applied to routing promotion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShadowComparisonGate {
    /// Equivalent result.
    Pass,
    /// Known non-security difference with an owned, unexpired baseline.
    BaselineAccepted,
    /// Unapproved functional, performance, or security divergence.
    Blocked,
}

/// Highest severity represented by a comparison report.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DivergenceSeverity {
    /// No divergence.
    None,
    /// Owned, unexpired non-security difference.
    Warning,
    /// Difference that blocks routing promotion.
    Blocking,
}

/// Explicitly owned temporary allowance for one deterministic divergence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DivergenceBaseline {
    /// Stable baseline identity.
    pub id: String,
    /// Query class covered by the allowance.
    pub query_class: QueryClass,
    /// Exact deterministic divergence fingerprint.
    pub fingerprint: String,
    /// Accountable owner.
    pub owner: String,
    /// Absolute expiry in Unix epoch milliseconds.
    pub expires_at_unix_ms: u64,
}

/// Safe baseline details attached to a report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppliedBaseline {
    /// Stable baseline identity.
    pub id: String,
    /// Accountable owner.
    pub owner: String,
    /// Absolute expiry in Unix epoch milliseconds.
    pub expires_at_unix_ms: u64,
}

/// Durable privacy-safe comparison report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShadowComparisonReport {
    /// Correlation identity shared by request, executions, report, and metrics.
    pub correlation_id: String,
    /// Typed query class.
    pub query_class: QueryClass,
    /// Reference provider identity and version.
    pub reference_provider: ProviderDescriptor,
    /// Shadow provider identity and version.
    pub shadow_provider: ProviderDescriptor,
    /// Reference latency.
    pub reference_latency_ms: u64,
    /// Shadow latency, absent for timeout or shedding.
    pub shadow_latency_ms: Option<u64>,
    /// Exact functional equivalence before baselines.
    pub equivalent: bool,
    /// Pseudonymous records absent from the shadow.
    pub missing_ids: Vec<String>,
    /// Pseudonymous records unexpectedly visible in the shadow.
    pub unexpected_ids: Vec<String>,
    /// Significant property differences without values.
    pub property_differences: Vec<PropertyDifference>,
    /// Result-order differences.
    pub ordering_differences: Vec<OrderingDifference>,
    /// Cursor-page differences without token contents.
    pub cursor_differences: Vec<String>,
    /// Aggregation paths that differ without bucket values.
    pub aggregation_differences: Vec<String>,
    /// Relationship or path structure differences.
    pub relationship_differences: Vec<String>,
    /// Stable performance gate categories.
    pub performance_differences: Vec<String>,
    /// Permission or authorization divergence categories.
    pub security_differences: Vec<String>,
    /// Stable error-category difference, without provider messages.
    pub error_difference: Option<String>,
    /// Routing gate.
    pub gate: ShadowComparisonGate,
    /// Highest functional, performance, or security severity.
    pub severity: DivergenceSeverity,
    /// Deterministic fingerprint used by baselines.
    pub divergence_fingerprint: String,
    /// Applied baseline, when allowed.
    pub baseline: Option<AppliedBaseline>,
}

/// Stable independently budgeted shadow failure category.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShadowFailureKind {
    /// The shadow provider failed before returning a typed result.
    Failed,
    /// The independent shadow deadline elapsed.
    TimedOut,
}

/// Compare two typed read executions.
///
/// The implementation will normalize provider metadata, canonicalize JSON,
/// compare every result dimension, pseudonymize evidence before constructing
/// the report, classify all possible overexposure as blocking, and permit only
/// owned, unexpired non-security baselines.
#[must_use]
pub fn compare_shadow_read(
    request: &KnowledgeDataRequest,
    reference: ProviderExecution,
    shadow: ProviderExecution,
    baselines: &[DivergenceBaseline],
    now_unix_ms: u64,
) -> ShadowComparisonReport {
    let query_class =
        QueryClass::from_operation(&request.operation).unwrap_or(QueryClass::Collection);
    let reference_result = CanonicalOutcome::from_envelope(&reference.envelope);
    let shadow_result = CanonicalOutcome::from_envelope(&shadow.envelope);
    let mut differences = ComparisonDifferences::default();
    compare_outcomes(&reference_result, &shadow_result, &mut differences);
    if reference.envelope.correlation_id != request.context.correlation_id
        || shadow.envelope.correlation_id != request.context.correlation_id
    {
        differences.error_difference = Some("correlation_id_mismatch".to_owned());
    }
    if shadow.latency_ms
        > reference
            .latency_ms
            .saturating_mul(2)
            .max(reference.latency_ms.saturating_add(100))
    {
        differences
            .performance
            .push("shadow_latency_budget".to_owned());
    }
    differences.normalize();
    if request
        .context
        .access
        .attributes
        .get("policy_version")
        .is_some_and(|version| !version.trim().is_empty())
        && !differences.is_functionally_equivalent()
    {
        differences
            .security
            .push("authorization_result_mismatch".to_owned());
        differences.normalize();
    }
    let equivalent = differences.is_functionally_equivalent();
    let fingerprint = divergence_fingerprint(query_class, &differences);
    let baseline =
        (!equivalent && differences.security.is_empty() && differences.performance.is_empty())
            .then(|| {
                baselines.iter().find(|baseline| {
                    baseline.query_class == query_class
                        && baseline.fingerprint == fingerprint
                        && !baseline.owner.trim().is_empty()
                        && baseline.expires_at_unix_ms > now_unix_ms
                })
            })
            .flatten()
            .map(|baseline| AppliedBaseline {
                id: baseline.id.clone(),
                owner: baseline.owner.clone(),
                expires_at_unix_ms: baseline.expires_at_unix_ms,
            });
    let gate = if equivalent && differences.performance.is_empty() {
        ShadowComparisonGate::Pass
    } else if baseline.is_some() {
        ShadowComparisonGate::BaselineAccepted
    } else {
        ShadowComparisonGate::Blocked
    };
    let severity = match gate {
        ShadowComparisonGate::Pass => DivergenceSeverity::None,
        ShadowComparisonGate::BaselineAccepted => DivergenceSeverity::Warning,
        ShadowComparisonGate::Blocked => DivergenceSeverity::Blocking,
    };
    ShadowComparisonReport {
        correlation_id: request.context.correlation_id.clone(),
        query_class,
        reference_provider: reference.provider,
        shadow_provider: shadow.provider,
        reference_latency_ms: reference.latency_ms,
        shadow_latency_ms: Some(shadow.latency_ms),
        equivalent,
        missing_ids: differences.missing_ids,
        unexpected_ids: differences.unexpected_ids,
        property_differences: differences.properties,
        ordering_differences: differences.ordering,
        cursor_differences: differences.cursor,
        aggregation_differences: differences.aggregation,
        relationship_differences: differences.relationships,
        performance_differences: differences.performance,
        security_differences: differences.security,
        error_difference: differences.error_difference,
        gate,
        severity,
        divergence_fingerprint: fingerprint,
        baseline,
    }
}

/// Build a privacy-safe blocking report when shadow execution did not produce
/// a typed result.
#[must_use]
pub fn shadow_failure_report(
    request: &KnowledgeDataRequest,
    reference: ProviderExecution,
    shadow_provider: ProviderDescriptor,
    failure: ShadowFailureKind,
) -> ShadowComparisonReport {
    let query_class =
        QueryClass::from_operation(&request.operation).unwrap_or(QueryClass::Collection);
    let failure_name = match failure {
        ShadowFailureKind::Failed => "shadow_failed",
        ShadowFailureKind::TimedOut => "shadow_timed_out",
    };
    let mut differences = ComparisonDifferences {
        error_difference: Some(failure_name.to_owned()),
        ..ComparisonDifferences::default()
    };
    if failure == ShadowFailureKind::TimedOut {
        differences.performance.push(failure_name.to_owned());
    }
    let fingerprint = divergence_fingerprint(query_class, &differences);
    ShadowComparisonReport {
        correlation_id: request.context.correlation_id.clone(),
        query_class,
        reference_provider: reference.provider,
        shadow_provider,
        reference_latency_ms: reference.latency_ms,
        shadow_latency_ms: None,
        equivalent: false,
        missing_ids: Vec::new(),
        unexpected_ids: Vec::new(),
        property_differences: Vec::new(),
        ordering_differences: Vec::new(),
        cursor_differences: Vec::new(),
        aggregation_differences: Vec::new(),
        relationship_differences: Vec::new(),
        performance_differences: differences.performance,
        security_differences: Vec::new(),
        error_difference: differences.error_difference,
        gate: ShadowComparisonGate::Blocked,
        severity: DivergenceSeverity::Blocking,
        divergence_fingerprint: fingerprint,
        baseline: None,
    }
}

/// Fixed latency upper bounds in milliseconds plus an overflow bucket.
pub const SHADOW_LATENCY_BUCKETS_MS: [u64; 9] = [10, 25, 50, 100, 250, 500, 1_000, 2_500, u64::MAX];

/// One bounded low-cardinality metrics series.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ShadowMetricSeries {
    /// Query class label.
    pub query_class: QueryClass,
    /// Deployment release label.
    pub release: String,
    /// Total comparisons.
    pub comparisons: u64,
    /// Equivalent comparisons.
    pub equivalent: u64,
    /// Blocking security divergences.
    pub security_blocking: u64,
    /// Cumulative reference latency histogram buckets.
    pub reference_latency_buckets: Vec<u64>,
    /// Cumulative shadow latency histogram buckets.
    pub shadow_latency_buckets: Vec<u64>,
}

/// In-memory low-cardinality parity and latency aggregates.
#[derive(Clone, Debug, Default)]
pub struct ShadowMetrics {
    series: BTreeMap<(QueryClass, String), ShadowMetricSeries>,
}

impl ShadowMetrics {
    /// Records one comparison without adding correlation, user, tenant,
    /// organization, entity, or record identifiers as labels.
    pub fn record(&mut self, report: &ShadowComparisonReport) {
        let key = (report.query_class, report.shadow_provider.release.clone());
        let series = self
            .series
            .entry(key)
            .or_insert_with(|| ShadowMetricSeries {
                query_class: report.query_class,
                release: report.shadow_provider.release.clone(),
                comparisons: 0,
                equivalent: 0,
                security_blocking: 0,
                reference_latency_buckets: vec![0; SHADOW_LATENCY_BUCKETS_MS.len()],
                shadow_latency_buckets: vec![0; SHADOW_LATENCY_BUCKETS_MS.len()],
            });
        series.comparisons = series.comparisons.saturating_add(1);
        series.equivalent = series
            .equivalent
            .saturating_add(u64::from(report.equivalent));
        series.security_blocking = series
            .security_blocking
            .saturating_add(u64::from(!report.security_differences.is_empty()));
        record_latency(
            &mut series.reference_latency_buckets,
            report.reference_latency_ms,
        );
        if let Some(latency) = report.shadow_latency_ms {
            record_latency(&mut series.shadow_latency_buckets, latency);
        }
    }

    /// Returns deterministically ordered metric series.
    #[must_use]
    pub fn series(&self) -> Vec<ShadowMetricSeries> {
        self.series.values().cloned().collect()
    }
}

fn record_latency(buckets: &mut [u64], latency_ms: u64) {
    for (count, upper_bound) in buckets.iter_mut().zip(SHADOW_LATENCY_BUCKETS_MS) {
        if latency_ms <= upper_bound {
            *count = count.saturating_add(1);
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum CanonicalOutcome {
    Records {
        records: BTreeMap<String, CanonicalRecord>,
        order: Vec<String>,
        has_next_page: bool,
    },
    Count(u64),
    Aggregation(Value),
    Graph {
        records: BTreeMap<String, CanonicalRecord>,
        order: Vec<String>,
        relationships: Vec<Value>,
    },
    Error(String),
    Other(Value),
}

#[derive(Clone, Debug, PartialEq)]
struct CanonicalRecord {
    kind: String,
    body: Value,
}

impl CanonicalOutcome {
    fn from_envelope(envelope: &KnowledgeDataResponseEnvelope) -> Self {
        match &envelope.outcome {
            KnowledgeDataOutcome::Failure { error } => Self::Error(error.code.as_str().to_owned()),
            KnowledgeDataOutcome::Success { response } => match response {
                KnowledgeDataResponse::Record(record) => {
                    let records = record
                        .iter()
                        .map(canonical_record)
                        .collect::<BTreeMap<_, _>>();
                    let order = record.iter().map(|record| record.id.clone()).collect();
                    Self::Records {
                        records,
                        order,
                        has_next_page: false,
                    }
                }
                KnowledgeDataResponse::Records(page) => Self::Records {
                    records: page
                        .records
                        .iter()
                        .map(canonical_record)
                        .collect::<BTreeMap<_, _>>(),
                    order: page
                        .records
                        .iter()
                        .map(|record| record.id.clone())
                        .collect(),
                    has_next_page: page.next_token.is_some(),
                },
                KnowledgeDataResponse::Count(result) => Self::Count(result.count),
                KnowledgeDataResponse::Aggregation(result) => {
                    Self::Aggregation(canonical_json(&Value::Array(result.buckets.clone()), false))
                }
                KnowledgeDataResponse::Graph(result) => {
                    let mut relationships = result
                        .relationships
                        .iter()
                        .map(|value| canonical_json(value, true))
                        .collect::<Vec<_>>();
                    relationships.sort_by_key(canonical_value_key);
                    Self::Graph {
                        records: result
                            .records
                            .iter()
                            .map(canonical_record)
                            .collect::<BTreeMap<_, _>>(),
                        order: result
                            .records
                            .iter()
                            .map(|record| record.id.clone())
                            .collect(),
                        relationships,
                    }
                }
                other => Self::Other(
                    serde_json::to_value(other)
                        .map(|value| canonical_json(&value, true))
                        .unwrap_or(Value::Null),
                ),
            },
        }
    }
}

fn canonical_record(record: &KnowledgeRecord) -> (String, CanonicalRecord) {
    (
        record.id.clone(),
        CanonicalRecord {
            kind: record.kind.to_ascii_lowercase().replace('_', "-"),
            body: canonical_json(&record.body, true),
        },
    )
}

fn canonical_json(value: &Value, sort_arrays: bool) -> Value {
    match value {
        Value::Object(object) => {
            let mut normalized = Map::new();
            let root_identity_fields = [
                "id",
                "standard_id",
                "internal_id",
                "entity_type",
                "type",
                "parent_types",
            ];
            let provider_fields = ["_score", "sort", "highlight", "__typename"];
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort();
            for key in keys {
                if root_identity_fields.contains(&key.as_str())
                    || provider_fields.contains(&key.as_str())
                {
                    continue;
                }
                normalized.insert(key.clone(), canonical_json(&object[key], sort_arrays));
            }
            Value::Object(normalized)
        }
        Value::Array(values) => {
            let mut normalized = values
                .iter()
                .map(|value| canonical_json(value, sort_arrays))
                .collect::<Vec<_>>();
            if sort_arrays {
                normalized.sort_by_key(canonical_value_key);
            }
            Value::Array(normalized)
        }
        Value::Number(number) => number
            .as_f64()
            .and_then(serde_json::Number::from_f64)
            .map_or_else(|| value.clone(), Value::Number),
        _ => value.clone(),
    }
}

fn canonical_value_key(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

#[derive(Default, Serialize)]
struct ComparisonDifferences {
    missing_ids: Vec<String>,
    unexpected_ids: Vec<String>,
    properties: Vec<PropertyDifference>,
    ordering: Vec<OrderingDifference>,
    cursor: Vec<String>,
    aggregation: Vec<String>,
    relationships: Vec<String>,
    performance: Vec<String>,
    security: Vec<String>,
    error_difference: Option<String>,
}

impl ComparisonDifferences {
    fn is_functionally_equivalent(&self) -> bool {
        self.missing_ids.is_empty()
            && self.unexpected_ids.is_empty()
            && self.properties.is_empty()
            && self.ordering.is_empty()
            && self.cursor.is_empty()
            && self.aggregation.is_empty()
            && self.relationships.is_empty()
            && self.security.is_empty()
            && self.error_difference.is_none()
    }

    fn normalize(&mut self) {
        for values in [
            &mut self.missing_ids,
            &mut self.unexpected_ids,
            &mut self.cursor,
            &mut self.aggregation,
            &mut self.relationships,
            &mut self.performance,
            &mut self.security,
        ] {
            values.sort();
            values.dedup();
        }
        self.properties
            .sort_by(|left, right| left.record.cmp(&right.record));
        self.ordering.sort_by_key(|difference| difference.position);
    }
}

fn compare_outcomes(
    reference: &CanonicalOutcome,
    shadow: &CanonicalOutcome,
    differences: &mut ComparisonDifferences,
) {
    match (reference, shadow) {
        (
            CanonicalOutcome::Records {
                records: reference_records,
                order: reference_order,
                has_next_page: reference_next,
            },
            CanonicalOutcome::Records {
                records: shadow_records,
                order: shadow_order,
                has_next_page: shadow_next,
            },
        ) => {
            compare_records(reference_records, shadow_records, differences);
            compare_order(reference_order, shadow_order, differences);
            if reference_next != shadow_next {
                differences.cursor.push("next_page_presence".to_owned());
            }
        }
        (CanonicalOutcome::Count(reference), CanonicalOutcome::Count(shadow)) => {
            if reference != shadow {
                differences.aggregation.push("$count".to_owned());
            }
        }
        (CanonicalOutcome::Aggregation(reference), CanonicalOutcome::Aggregation(shadow)) => {
            diff_value_paths(reference, shadow, "$buckets", &mut differences.aggregation);
        }
        (
            CanonicalOutcome::Graph {
                records: reference_records,
                order: reference_order,
                relationships: reference_relationships,
            },
            CanonicalOutcome::Graph {
                records: shadow_records,
                order: shadow_order,
                relationships: shadow_relationships,
            },
        ) => {
            compare_records(reference_records, shadow_records, differences);
            compare_order(reference_order, shadow_order, differences);
            diff_value_paths(
                &Value::Array(reference_relationships.clone()),
                &Value::Array(shadow_relationships.clone()),
                "$relationships",
                &mut differences.relationships,
            );
        }
        (CanonicalOutcome::Error(reference), CanonicalOutcome::Error(shadow)) => {
            if reference != shadow {
                differences.error_difference =
                    Some(format!("reference={reference};shadow={shadow}"));
                if reference == "UNAUTHORIZED" || shadow == "UNAUTHORIZED" {
                    differences
                        .security
                        .push("authorization_error_mismatch".to_owned());
                }
            }
        }
        (CanonicalOutcome::Error(reference), _) | (_, CanonicalOutcome::Error(reference)) => {
            differences.error_difference = Some(format!("outcome_mismatch={reference}"));
            if reference == "UNAUTHORIZED" {
                differences
                    .security
                    .push("authorization_outcome_mismatch".to_owned());
            }
        }
        (CanonicalOutcome::Other(reference), CanonicalOutcome::Other(shadow)) => {
            diff_value_paths(reference, shadow, "$response", &mut differences.aggregation);
        }
        _ => {
            differences.error_difference = Some("response_kind_mismatch".to_owned());
        }
    }
}

fn compare_records(
    reference: &BTreeMap<String, CanonicalRecord>,
    shadow: &BTreeMap<String, CanonicalRecord>,
    differences: &mut ComparisonDifferences,
) {
    let reference_ids = reference.keys().cloned().collect::<BTreeSet<_>>();
    let shadow_ids = shadow.keys().cloned().collect::<BTreeSet<_>>();
    differences.missing_ids = reference_ids
        .difference(&shadow_ids)
        .map(|id| evidence_id(id))
        .collect();
    differences.unexpected_ids = shadow_ids
        .difference(&reference_ids)
        .map(|id| evidence_id(id))
        .collect();
    if !differences.unexpected_ids.is_empty() {
        differences
            .security
            .push("shadow_exposes_unexpected_records".to_owned());
    }
    for id in reference_ids.intersection(&shadow_ids) {
        let reference_record = &reference[id];
        let shadow_record = &shadow[id];
        let mut fields = Vec::new();
        if reference_record.kind != shadow_record.kind {
            fields.push("$kind".to_owned());
        }
        diff_value_paths(
            &reference_record.body,
            &shadow_record.body,
            "$body",
            &mut fields,
        );
        fields.sort();
        fields.dedup();
        if !fields.is_empty() {
            if fields.iter().any(|field| {
                [
                    "access",
                    "marking",
                    "organization",
                    "tenant",
                    "permission",
                    "authorized",
                ]
                .iter()
                .any(|security_field| field.to_ascii_lowercase().contains(security_field))
            }) {
                differences
                    .security
                    .push("access_policy_mismatch".to_owned());
            }
            differences.properties.push(PropertyDifference {
                record: evidence_id(id),
                fields,
            });
        }
    }
}

fn compare_order(reference: &[String], shadow: &[String], differences: &mut ComparisonDifferences) {
    if reference == shadow {
        return;
    }
    for position in 0..reference.len().max(shadow.len()) {
        let reference_id = reference.get(position);
        let shadow_id = shadow.get(position);
        if reference_id != shadow_id {
            differences.ordering.push(OrderingDifference {
                position,
                reference: reference_id.map(|id| evidence_id(id)),
                shadow: shadow_id.map(|id| evidence_id(id)),
            });
        }
    }
}

fn diff_value_paths(reference: &Value, shadow: &Value, path: &str, output: &mut Vec<String>) {
    match (reference, shadow) {
        (Value::Object(reference), Value::Object(shadow)) => {
            let keys = reference
                .keys()
                .chain(shadow.keys())
                .collect::<BTreeSet<_>>();
            for key in keys {
                match (reference.get(key), shadow.get(key)) {
                    (Some(reference), Some(shadow)) => {
                        diff_value_paths(reference, shadow, &format!("{path}.{key}"), output);
                    }
                    _ => output.push(format!("{path}.{key}")),
                }
            }
        }
        (Value::Array(reference), Value::Array(shadow)) => {
            if reference.len() != shadow.len() {
                output.push(format!("{path}.length"));
            }
            for (index, (reference, shadow)) in reference.iter().zip(shadow).enumerate() {
                diff_value_paths(reference, shadow, &format!("{path}[{index}]"), output);
            }
        }
        _ if reference != shadow => output.push(path.to_owned()),
        _ => {}
    }
}

fn evidence_id(id: &str) -> String {
    let digest = Sha256::digest(id.as_bytes());
    let mut encoded = String::with_capacity(23);
    encoded.push_str("sha256:");
    for byte in &digest[..8] {
        let _ = write!(&mut encoded, "{byte:02x}");
    }
    encoded
}

fn divergence_fingerprint(query_class: QueryClass, differences: &ComparisonDifferences) -> String {
    let payload = json!({
        "query_class": query_class,
        "missing_ids": differences.missing_ids,
        "unexpected_ids": differences.unexpected_ids,
        "properties": differences.properties,
        "ordering": differences.ordering,
        "cursor": differences.cursor,
        "aggregation": differences.aggregation,
        "relationships": differences.relationships,
        "security": differences.security,
        "error": differences.error_difference,
    });
    evidence_id(&serde_json::to_string(&payload).unwrap_or_default())
}

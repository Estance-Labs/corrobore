# OpenCTI shadow reads and parity reports

Issue #43 adds a read-only migration gate on top of the Knowledge Data Engine
contract and the durable OpenCTI synchronization gate. The reference
Elasticsearch/OpenSearch provider remains authoritative: its typed response is
the only response returned to the caller.

## Execution boundary

`POST /v1/opencti/shadow/reads` accepts supported read operations only. The
server starts Corrobore work only after all three controls permit it:

1. snapshot and catch-up validation reports zero persistent divergence;
2. the deterministic sampling policy selects the correlation ID;
3. an independent concurrency permit is available.

The reference future is awaited on the caller path. Corrobore completion and
comparison run in a detached task with their own deadline. Failure, timeout, or
load shedding therefore cannot extend reference latency or replace its result.

## Canonical comparison

Both providers use `KnowledgeDataResponseEnvelope`. Normalization removes
provider-only search metadata, canonicalizes object keys and set-like property
arrays, and compares:

- record IDs and significant property paths;
- record ordering and cursor continuation presence;
- counts and aggregation buckets;
- relationship and path structure;
- stable error categories and authorization outcomes;
- reference and shadow latency.

Provider versions and the Corrobore release are retained in every report.
Correlation IDs link the original request, provider executions, report, and
metrics.

## Privacy and security

Durable evidence never contains record property values, provider response
bodies, bearer tokens, or remote error messages. Record identifiers are
represented by stable truncated SHA-256 handles and property evidence contains
paths only. Any record visible only in Corrobore or any authorization-outcome
mismatch is classified as blocking and cannot be baselined.

Reports are stored under the persistent runtime root in
`runtime/opencti-shadow-reports.json`. Writes use an fsynced temporary file,
atomic rename, and parent-directory fsync. Retention is bounded.

## Sampling and baselines

The optional sampling-policy JSON is a `ShadowSamplingPolicy`. Rules use
first-match semantics and can select environment, typed operation, query class,
entity type, organization, tenant, bounded user cohort, and percentage in basis
points. Decisions are deterministic for retries.
[A complete sampling-policy example](../examples/opencti-shadow-sampling.json)
shows every supported selector.

The optional baseline JSON is a list of `DivergenceBaseline` objects. A
baseline applies only to an exact deterministic fingerprint and query class,
must have a non-empty owner, and must not be expired. Security and performance
divergences remain blocking. Copy the actual report fingerprint into an
[owned baseline entry](../examples/opencti-shadow-baselines.json); the example
fingerprint is intentionally a placeholder.

## Observability

`GET /v1/opencti/shadow/reports` returns privacy-safe evidence. Prometheus
metrics expose comparison totals, equivalent totals, security-blocking totals,
and cumulative reference/shadow latency buckets. Labels are deliberately
limited to query class, release, and provider; correlation, record, tenant,
organization, user, and entity identifiers never become metric labels.

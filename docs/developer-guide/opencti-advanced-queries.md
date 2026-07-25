# OpenCTI advanced queries

Issue #47 implements the bounded advanced-query subset needed by the pinned
OpenCTI 7.260722.0 workload. It extends the backend-neutral Knowledge Data
Engine contract; it does not expose Elasticsearch or OpenSearch Query DSL,
scripts, pipeline aggregations, geospatial queries or vector queries.

## Structural predicates and stable pages

`ListRequest.predicate` is a typed tree made of `condition`, `and` and `or`
nodes. Conditions support equality, inequality, inclusion, exclusion,
existence and scalar ranges. Legacy `filters` remain conjunctive and are
evaluated in addition to the structural tree.

The tree is limited to depth 16 and 128 nodes. Empty boolean nodes, blank
fields, missing comparison values and empty `in`/`not_in` sets are rejected.
The persistent planner pushes safe outer conjunctive conditions into compact
property or temporal indexes. An `or` branch remains structural and is
evaluated exactly against the bounded candidate projection.

`order_by` accepts multiple ascending or descending keys. Missing values sort
before present values in ascending order and after them in descending order.
Canonical record identity is always the final ascending tie-breaker.

Cursor tokens bind the complete normalized request, schema, policy and access
fingerprints, result snapshot and last multi-key cursor. A changed query,
policy or snapshot rejects the token. `include_total_count` returns the
authorization-filtered total from that same snapshot; the declared query limit
still bounds all pages.

## Aggregation contract

`AggregateRequest` contains a typed `AggregationPlan`:

```json
{
  "plan": {
    "kinds": ["indicator"],
    "predicate": null,
    "aggregation": {
      "kind": "date_histogram",
      "field": "valid_from",
      "interval": "day",
      "time_zone_offset_minutes": 0,
      "include_empty": false
    },
    "candidate_limit": 600000
  }
}
```

The supported expressions are:

- `count`, `cardinality`, `terms` and `date_histogram`;
- `sum`, `average`, `minimum` and `maximum`;
- `filter`, `nested` and `reverse_nested`.

Terms buckets sort by count descending and then by canonical JSON key
ascending. Array values contribute at most once per root record to a terms
bucket. Cardinality is exact and ignores missing and null values.

Numeric metrics accept finite integers and floats. Missing, null and
non-numeric values are ignored. An empty numeric input returns no scalar value;
non-finite output is rejected. The Cypher subset exposes the same rules through
`COUNT`, `SUM`, `AVG`, `MIN` and `MAX`.

Date histograms accept RFC 3339 timestamps and fixed offsets from -18 to +18
hours. The offset is applied before calendar bucketing and removed from the
canonical UTC millisecond key. Daylight-saving rules are deliberately not
inferred. Empty buckets, when requested, are emitted only between the first
and last observed bucket.

Nested aggregation expands an array into `$value` subjects.
`reverse_nested` deduplicates and returns to their canonical parent records.

## Authorization, planning and lifecycle

Authorization is evaluated before totals, sort boundaries, buckets and scalar
metrics. Relationships require access to their own metadata and to both
endpoints. Denied records therefore cannot affect counts, cardinality,
histograms or continuation state.

Persistent reads use identifier, type, property, temporal, relationship and
full-text indexes where applicable. The exact evaluator refuses a zero bound
or more than 1,000,000 candidates and returns `QUERY_BUDGET_EXCEEDED` when the
declared bound is exhausted; it never silently scans an unbounded fallback.

Aggregation results carry an `examined_records` count and a deterministic
generation fingerprint over canonical IDs and revisions. Warm results are
cached by typed plan and access-policy fingerprint. Any graph replacement or
mutation invalidates the cache. The cache is derived, versioned and fully
rebuildable from canonical graph state; it is never authoritative.

## Pinned parity and performance

The corpus acceptance tests assert exact OpenCTI reference IDs, page boundaries
and buckets for terms and day histograms. They also cover restricted access,
nested/reverse-nested behavior, numeric null rules and cache rebuild after a
mutation.

The reproducible release benchmark uses the PRD small profile: 100,000 objects,
500,000 relationships, 20 warmups and 60 measurements per warm aggregation.
The recorded run reached:

- terms P95: 0.009 ms, below the 5.814 ms parity ceiling;
- date-histogram P95: 0.011333 ms, below the 5.8284 ms parity ceiling;
- maximum resident set: 3,077,865,472 bytes.

Inputs, environment and exact measurements are stored in
`compatibility/opencti/7.260722.0/advanced-query-benchmark-results.json`.

Reproduce the latency run with:

```bash
cargo run --release -p corrobore-engine \
  --example small_profile_advanced_query_benchmark --locked
```

On macOS, prefix the built executable with `/usr/bin/time -l` to capture the
same maximum-resident-set measurement.

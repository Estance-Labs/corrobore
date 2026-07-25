# OpenCTI transactional writes

Issue #50 adds reference-compatible create, update, delete, relationship,
access-policy and ordered bulk mutations to the backend-neutral Knowledge Data
Engine contract. During migration the configured Elasticsearch/OpenSearch
provider remains authoritative: `POST /v1/opencti/writes` applies there first,
mirrors the exact request into Corrobore, and returns the reference envelope.

Persistent storage is required for Corrobore acknowledgement. Each mutation
uses a non-empty caller idempotency key. Corrobore hashes that key, derives a
stable transaction identity, persists WAL intent before payload records, and
only acknowledges after the canonical records, adjacency and derived indexes
cross the applied marker. A retry after restart reads the WAL-bound receipt and
returns the original response without applying the mutation twice. Reusing the
same key for a different request is a conflict.

## Mutation and bulk semantics

Creates require a stable OpenCTI `id` and `type`. Updates use JSON merge-patch
semantics; access-policy fields are updated through the same typed path.
Relationships validate both endpoints and persist their adjacency projections
in the canonical transaction. Update and delete accept `expected_revision`;
a stale revision returns a conflict and cannot overwrite a newer value.

Bulk items are evaluated in request order and return a stable status for every
item. With `atomic: true`, any rejected or conflicting item aborts the entire
batch. With `atomic: false`, valid items commit and failures remain explicit in
their original positions. The operation count, JSON body size, concurrent
request count and per-provider deadline are bounded respectively by:

- `CORROBORE_OPENCTI_SYNC_MAX_OPERATIONS`;
- `CORROBORE_HTTP_IMPORT_MAX_BODY_BYTES`;
- `CORROBORE_OPENCTI_SHADOW_MAX_CONCURRENCY`;
- `CORROBORE_OPENCTI_SHADOW_TIMEOUT_MS`.

Saturation returns explicit backpressure. Clients retry with the same
idempotency key. The complete request shape is illustrated by
[`opencti-transactional-write.json`](../examples/opencti-transactional-write.json).

## Recovery, reconciliation and audit

Recovery ignores WAL transactions that did not reach their applied marker, so
crashes after intent, payload journals, audit persistence or before a catalog
checkpoint cannot expose a partial mutation. Canonical data remains the source
for rebuilding indexes and projections. The full-text projection is durably
invalidated before mutation; the first subsequent search rebuilds and atomically
publishes the complete canonical generation before returning results, so stale
search data is never served and write acknowledgement does not rebuild the
entire corpus.

If only one dual-write target succeeds, Corrobore stores a bounded
reconciliation record in `runtime/opencti-write-state.json`. Replaying the same
request advances it to `reconciled`; three failed attempts quarantine it for
operator action. Unresolved records are never evicted to admit new work.

`GET /v1/opencti/writes/status` returns counters, reconciliation state and
committed audit receipts. Audit fields are limited to the hashed idempotency
identity, correlation ID, optional source offset, before/after revisions and
outcome. Record content, bearer tokens and the original idempotency key are not
stored there. The same state is exported through the
`corrobore_opencti_write_*` and reconciliation metrics.

## Performance acceptance

The reproducible small-profile benchmark uses the pinned profile's exact
5,000-document reference bulk size (1,000 objects and 4,000 relationships) and
commits it as one WAL-backed canonical transition after unmeasured WAL and
periodic-checkpoint warmups.
It reports planning, durable commit and end-to-end records per second, plus a
gate against the pinned OpenSearch 3.7.0 small-profile ingestion reference in
`compatibility/opencti/7.260722.0/benchmark-results.json`. The target profile
remains 100,000 objects and 500,000 relationships; the bounded measured unit is
the same bulk size used by the reference runner.

Run it with:

```bash
cargo run --release -p corrobore-http-server \
  --example small_profile_transactional_write_benchmark --locked
```

The exact recorded environment and result are stored in
`compatibility/opencti/7.260722.0/transactional-write-benchmark-results.json`.
The recorded native run reached 54,823.759 records/s end-to-end and passed the
33,639.092 records/s parity floor derived from the 42,048.865 records/s
OpenSearch reference.

Merge/deduplication, endpoint-moving relationship updates, source-of-truth
inversion and distributed transactions remain outside issue #50.

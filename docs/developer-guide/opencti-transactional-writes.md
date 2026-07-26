# OpenCTI transactional writes

Corrobore is the exclusive primary for create, update, delete, relationship,
access-policy, merge and ordered bulk mutations. `POST /v1/opencti/writes`
durably prepares a reference-projection intent, commits canonical Corrobore
state, binds the canonical response to the outbox, and only then acknowledges
the caller. Elasticsearch/OpenSearch is a derived, reversible projection; its
availability never decides whether an accepted canonical write succeeds.

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

## Ordered reference projection

The durable outbox is stored in `runtime/opencti-write-state.json`. Its global
monotonic sequence is the primary ordering boundary. Create operations also use
`internal_id` or `id` as their entity ordering key, updates and deletes use the
target ID, merges use the survivor ID, and bulk operations fall back to their
transaction sequence. The persisted request replaces the caller idempotency key
with its SHA-256 identity; projection replay therefore remains idempotent without
retaining credential-like source material.

Projection always drains the oldest pending sequence first. A transport error,
timeout or reference rejection increments retry and lag counters and leaves the
entry pending. Only an exact match with the canonical response marks it
`delivered`. A different successful result is quarantined, suspends writes, and
records `write_divergence` as the rollback trigger. Outbox capacity is bounded;
when unresolved entries fill it, new writes receive explicit backpressure rather
than losing an accepted mutation.

During projection lag, a routed request with `consistency: read_your_writes` is
served directly from Corrobore, regardless of the progressive read-routing
policy. Eventual reads may still observe the reference's older index generation.
Search/index visibility on Corrobore follows the canonical generation boundary:
the first search after a mutation rebuilds and atomically publishes the complete
generation, never a partial projection.

## Recovery, reconstruction and audit

At startup, every `prepared` outbox intent is compared with its deterministic
canonical WAL transaction. A readable applied receipt promotes it to `pending`
with the original canonical response. An intent with no applied WAL event is
proven abandoned and removed. A committed transaction with an unreadable receipt
blocks readiness instead of being discarded. This closes the crash window
between outbox preparation, canonical commit, and outbox activation without a
distributed transaction.

When upgrading a pre-inversion state file, any unresolved legacy dual-write
record starts in `writes_suspended` with a `migration_failure` trigger. An
operator must reconcile that historical partial write and verify parity before
enabling Corrobore-primary traffic.

`POST /v1/admin/opencti/reconstruction` reads a consistent complete canonical
projection and losslessly restores every node and relationship from
`opencti.raw`. It returns deterministic records plus the captured outbox
high-water sequence. Operators load these records into a clean reference,
replay sequences above the high-water mark, run the approved parity corpus, and
only then make the rebuilt reference eligible for reads or rollback.

`GET /v1/opencti/writes/status` returns counters, reconciliation state and
committed audit receipts, ordered projection entries, outbox depth, lag,
retries, quarantine, reconstruction count, synchronization state and current
write authority. Audit fields are limited to the hashed idempotency identity,
correlation ID, optional source offset, before/after revisions and outcome.
Bearer tokens and original idempotency keys are never persisted.

`POST /v1/admin/opencti/projection/drain` retries the ordered outbox after a
reference outage. Prometheus exposes
`corrobore_opencti_projection_outbox_depth`, `corrobore_opencti_projection_lag`,
`corrobore_opencti_projection_retries_total`,
`corrobore_opencti_projection_quarantined`,
`corrobore_opencti_projection_reconstruction_total`, and the one-hot
`corrobore_opencti_write_authority{authority=...}` gauge.

Issue #51 extends this endpoint with merge. It atomically preserves the target,
unions identifiers and access metadata, redirects relationships and embedded
STIX references, deduplicates equivalent edges without weakening authorization,
retains source provenance and history, and tombstones duplicates. See
[OpenCTI merge and targeted reconciliation](opencti-merge-reconciliation.md).

## Authority rollback runbook

Rollback triggers are `security_divergence`, `corruption`,
`latency_regression`, `migration_failure`, `write_divergence`, and
`reference_availability`.

1. Stop unsafe mutations with `POST /v1/admin/opencti/authority/suspend` and
   the observed trigger. Confirm the authority gauge is `writes_suspended`.
2. Inspect outbox depth, lag, retries and quarantine. Resolve quarantined
   divergence; restore reference health; call the drain endpoint until replay is
   complete.
3. Reconstruct a clean reference when corruption or migration failure makes
   incremental replay unsafe. Apply the returned corpus, then replay mutations
   above its high-water sequence.
4. Run the parity corpus, including records, relationships, full-text results,
   access decisions and index generations. Do not proceed on any mismatch.
5. Assign `reference_primary` with `POST /v1/admin/opencti/authority`, setting
   `reference_healthy`, `replay_complete`, and `parity_verified` to true. The
   server rejects the transition if writes were not suspended, projection state
   remains prepared or pending, or any evidence is false. Verified full parity
   resolves quarantined divergence as part of the durable transition.
6. Restore traffic gradually, monitor authority, availability, p95 latency,
   parity, and quarantine, and keep the canonical store and outbox intact for a
   reversible return to `corrobore_primary`.

## Performance and soak acceptance

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

`opencti_primary_projection` also runs a primary soak corpus through durable
commits, repeated simulated reference outages, runtime/store restart, ordered
replay and exact verification. It asserts canonical record durability, bounded
latency, retained retry evidence, final parity, and zero original idempotency-key
leakage in the persisted outbox. Multi-primary and distributed replication are
deliberately unsupported.

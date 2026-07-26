# OpenCTI merge and targeted reconciliation

Issue #51 adds deterministic entity merge, edge deduplication, and bounded
provider-to-canonical reconciliation. Persistent storage is required for every
repair. A dry-run also uses persistent mode so its report can be audited and
replayed after restart.

## Merge contract

`POST /v1/opencti/writes` accepts the Knowledge Data Engine `merge` operation.
The request declares one survivor, at least one duplicate, optional optimistic
revision preconditions, and the normal mandatory idempotency key.

The planner rejects missing records, repeated sources, a source equal to the
survivor, incompatible entity types, stale revisions, too many sources, or a
relationship scan above the configured bound before changing the input graph.
For an accepted merge it performs one WAL-backed graph transition:

1. Target scalar values win deterministic conflicts.
2. Arrays, identifiers, markings, organizations, tenants, file metadata, and
   other structured properties are canonicalized and unioned.
3. Source records and conflicts are retained in payload provenance on the
   survivor; normal graph version history retains the previous survivor.
4. STIX `_ref` and `_refs` values and relationship endpoints are redirected.
5. Edges that become duplicates are reduced deterministically; the retained
   edge unions authorization and other array metadata before duplicates are
   tombstoned.
6. Duplicate entities are tombstoned last.

The response identifies the survivor revision, deleted sources, redirected
relationships and object references, deduplicated relationships, and the
payload-free conflict count. The response and audit evidence share the same WAL
transaction. A replay returns the original response without creating another
version.

## Reconciliation contract

`POST /v1/opencti/reconciliation` accepts the body shown in
[`opencti-reconciliation.json`](../examples/opencti-reconciliation.json).
Selection is explicit and bounded:

- `records` selects exact canonical IDs;
- `range` selects a lexicographic half-open ID interval;
- `partition` selects one stable hash partition;
- `full` selects the complete supplied/reference universe up to its hard cap.

Each command compares lossless reference records with the canonical graph and
reports `missing`, `extra`, `property_divergent`, `relationship_divergent`,
`permission_divergent`, and `stale_index` dimensions. The report contains IDs,
bounded diagnostics, and planned actions, never graph payloads or credentials.

`dry_run` persists the exact report but cannot mutate canonical data or derived
indexes. `repair` creates missing records, replaces safe divergences, optionally
tombstones explicitly authorized extras, and rebuilds the stale full-text
projection. A record-category conflict and an extra record without
`allow_extra_deletion` are quarantined. Any quarantine blocks the parity gate.
An extra node is also quarantined when an attached relationship is outside the
declared deletion scope, preventing a targeted repair from creating a dangling
edge or silently broadening its mutation set.

## Restart and replay

Canonical repair uses the graph WAL. The coordinator persists its bounded
report only after the canonical applied marker and required index rebuilds are
durable. If the process stops after the canonical commit, replay discovers the
existing WAL audit, completes projections and parity verification, and then
publishes the report. Reusing a `command_id` with a changed payload is rejected.

The WAL itself resumes committed-but-not-applied transaction IDs. It validates
the mutation targets, already-written payload records, and audit messages before
finishing the applied marker. This keeps merge visibility, receipts, adjacency,
and history atomic at every tested crash boundary.

`GET /v1/opencti/reconciliation/status` returns oldest-first retained reports
and aggregate parity/quarantine counts. Quarantined reports are never evicted to
admit new work; capacity exhaustion produces backpressure. Metrics are exported
as `corrobore_opencti_reconciliation_reports`,
`corrobore_opencti_reconciliation_quarantined`, and
`corrobore_opencti_reconciliation_parity_verified`.

## Operational sequence

1. Submit `dry_run` and review the exact planned actions.
2. Resolve quarantined category or deletion-policy conflicts.
3. Submit a new `repair` command with the approved reference snapshot.
4. Require `parity_verified: true` before advancing migration routing.
5. Use range, partition, or bounded full scopes for larger resynchronization.

The body limit is `limits.import_max_body_bytes`. The selected-record and
retained-report limits are `limits.opencti_sync_max_replay_identities`; merge
source count is bounded by `limits.opencti_sync_max_operations`.

## Performance acceptance

The reproducible release benchmark uses the pinned small profile's 5,000-record
bounded unit: 1,000 objects and a 4,000-edge supernode. The merge rewires 998
edges, safely deduplicates 3,002 more, and commits through fsynced WAL. The
repair then replaces all 1,997 surviving canonical records and rebuilds the
full-text projection before parity verification.

The workload-specific gates are 10,000 scanned records/s for supernode merge
and 1,500 repaired records/s including WAL, index rebuild, and a second parity
comparison. The pinned
42,048.865 records/s OpenSearch number remains ingestion context only because
the reference bundle does not contain an equivalent merge/repair workload.
This distinction prevents an ingestion metric from being mislabeled as merge
latency evidence.

Run:

```bash
cargo run --release -p corrobore-http-server \
  --example small_profile_merge_reconciliation_benchmark --locked
```

The recorded macOS ARM64 run reached 15,470.728 scanned records/s for merge and
2,100.037 records/s for repair. Its complete environment, bounds, timings, and
passing gates are stored in
`compatibility/opencti/7.260722.0/merge-reconciliation-benchmark-results.json`.

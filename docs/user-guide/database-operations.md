# Database operations

Corrobore snapshots are coherent copies of one committed canonical generation. The
snapshot barrier serializes with canonical writes, records the same checkpoint and
WAL boundary, and verifies every copied component before returning `ready`.

## Online snapshot

Use the administrative API when the server is running:

```bash
curl -X POST http://127.0.0.1:8080/v1/admin/storage/snapshots \
  -H "Authorization: Bearer $CORROBORE_HTTP_ADMIN_AUTH_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"destination":"/var/backups/corrobore/2026-07-26","encryption_key_id":"kms://corrobore/snapshots","retention_hook":"retain-30-days"}'
```

The key identifier is a configuration boundary, not key material. Encryption is
provided by the encrypted volume or the configured S3/MinIO server-side KMS policy.
A restore configured with another key identity fails before the target becomes
writable.

## Offline commands

Offline commands acquire the same exclusive data-directory ownership as the server.
They fail if a live server owns the directory.

```bash
corrobore server snapshot --storage-dir /var/lib/corrobore/graph \
  --destination /var/backups/corrobore/2026-07-26 \
  --encryption-key-id kms://corrobore/snapshots \
  --retention-hook retain-30-days

corrobore server validate-snapshot \
  --snapshot /var/backups/corrobore/2026-07-26 \
  --encryption-key-id kms://corrobore/snapshots

corrobore server restore \
  --snapshot /var/backups/corrobore/2026-07-26 \
  --target /var/lib/corrobore-restored/graph \
  --encryption-key-id kms://corrobore/snapshots
```

Restore accepts only a missing or empty target and validates manifest version,
key identity, component checksums, WAL continuity, checkpoint, catalog, adjacency
and payload checksums before opening the restored root.

## S3 and MinIO export

The provider uses path-style AWS Signature V4 and works with HTTPS S3 endpoints or
HTTP(S) MinIO endpoints. Credentials are read from the environment and are never
accepted in the snapshot manifest or printed in reports.

```bash
export CORROBORE_S3_ACCESS_KEY='...'
export CORROBORE_S3_SECRET_KEY='...'
# Optional for temporary AWS credentials:
export CORROBORE_S3_SESSION_TOKEN='...'

corrobore server export-snapshot-s3 \
  --snapshot /var/backups/corrobore/2026-07-26 \
  --endpoint https://s3.example.net \
  --bucket corrobore-backups \
  --prefix production/2026-07-26 \
  --region us-east-1
```

Every object is read back after upload. The versioned snapshot manifest is
published last, so an incomplete prefix cannot look like a completed snapshot.

## Object storage and Parquet export

`corrobore server import-snapshot` downloads a snapshot exported with
`export-snapshot-s3` (or any exporter publishing the same layout) from an
object store URL into a new local snapshot directory. It reads the manifest
first, writes it last, and validates every component checksum, so an
interrupted download never looks complete. Then `restore` it as usual. URLs
are `s3://bucket` (with `--endpoint`, `--region` and the `CORROBORE_S3_*`
credentials), `file:///path` or `memory://`, served by the `object_store`
crate.

```bash
corrobore server import-snapshot --source s3://corrobore-backups \
  --endpoint https://s3.example.net --prefix production/2026-07-26 \
  --destination /var/backups/corrobore/2026-07-26
corrobore server restore --snapshot /var/backups/corrobore/2026-07-26 --target /var/lib/corrobore/graph
```

`corrobore server export-parquet` writes one commit (`--commit`, else the head
of `--branch`, else `main`) to Apache Parquet files that DuckDB, Polars or
Spark read directly:

- **Nodes:** one `nodes/<label>.parquet` file per label.
- **Relationships:** one `relationships/<type>.parquet` file per relationship
  type.
- **`manifest.json`:** the commit and the files written.

Each file has these columns:

- the native fields: `id`, `version_id`, `status`, then `labels`, or `type`,
  `source` and `target` for relationships, then `first_seen` and `last_seen`;
- `properties`, holding every property as JSON;
- one typed `prop_<key>` column for every property whose values share one
  scalar type at that commit. The type is `Int64`, `Float64` (integers and
  floats mixed), `Utf8` or `Boolean`.

The online equivalent is `POST /v1/admin/storage/export/parquet` with
`{destination, branch?, commit?}` and the admin token. The destination must
not exist yet.

```bash
corrobore server export-parquet --storage-dir /var/lib/corrobore/graph --destination /var/exports/graph-2026-11-15
duckdb -c "SELECT prop_name, prop_score FROM '/var/exports/graph-2026-11-15/nodes/Indicator.parquet'"
```

## Migration, rollback and index rebuild

Migration is offline, versioned, idempotent and resumable. Durable progress lives
under `operations/`. The supported boundary is `V0` to `V1`; rollback is allowed
only while canonical records remain compatible and restores the saved V0 manifest.

```bash
corrobore server migrate --storage-dir /var/lib/corrobore/graph --from V0 --to V1
corrobore server rollback --storage-dir /var/lib/corrobore/graph
corrobore server rebuild-indexes --storage-dir /var/lib/corrobore/graph
corrobore server cancel-rebuild --storage-dir /var/lib/corrobore/graph
```

Rebuild covers identifiers, properties, temporal values, adjacency, access policy,
full text, aggregation metadata and extracted file content. Canonical graph and
indexable file metadata remain authoritative. `rebuilding`, `cancelled` and
`failed` states are never reported as complete; startup resumes an interrupted
rebuild before readiness.

## History retention

Graph history keeps every commit until you clean it up. Retention keeps, on
every branch, the newest first-parent commits (`keep`), plus any commit created
within the last `keep_seconds` and recent work merged from other branches. The
first commit past the window on each branch becomes a *baseline*: it keeps its
id and the full view of the graph at that commit, so every retained commit
still reads exactly as before. Older commits become unreadable. Reads at them
answer `404 GRAPH_COMMIT_NOT_FOUND`, and change-feed cursors on them answer
`410 CHANGE_FEED_GAP`.

The same pass rewrites the branch payloads into a new
`history/branch_payloads.<generation>.log` holding only payloads that retained
commits reference, and compacts the WAL and transaction journals behind the
latest checkpoint. The rewritten history log is published by an atomic rename,
so an interrupted cleanup leaves the previous history intact.

```bash
corrobore server cleanup --storage-dir /var/lib/corrobore/graph --keep 100 --dry-run
corrobore server cleanup --storage-dir /var/lib/corrobore/graph --keep 100 --keep-seconds 2592000
```

Online, with the admin token:

```bash
curl -X POST -H "Authorization: Bearer $ADMIN_TOKEN" -H 'content-type: application/json' \
  -d '{"keep_commits": 100, "keep_seconds": 2592000, "dry_run": true}' \
  https://corrobore.example/v1/admin/storage/cleanup
```

The report gives `pruned_commits`, `retained_commits`, `baselines`, history and
branch payload bytes before and after, and the WAL bytes reclaimed. When no
checkpoint exists yet, WAL compaction is skipped and `wal_skipped` says why.

## Observability and recovery checks

`GET /v1/admin/storage/operations` provides bounded counters. `/metrics` exports
snapshot/rebuild totals, failures, duration, bytes and scanned-record counts.
Every offline command also emits a stable operation name, completion/failure
status and `duration_ms` on standard error while its JSON report exposes bytes or
durable migration/rebuild progress.
After restore or migration, compare graph IDs, record counts, histories, access
results and representative queries before promoting the new data directory.

## Reproducible small-profile drill

The release-profile acceptance drill creates 1,000 canonical records, takes a
snapshot, restores it into a clean directory, upgrades the supported previous
manifest, rebuilds every derived index, verifies record parity and exercises the
rollback boundary:

```bash
cargo run --release -p corrobore-http-server \
  --example small_profile_database_operations --locked
```

The captured reference result is versioned in
`compatibility/opencti/7.260722.0/database-operations-small-profile.json`.

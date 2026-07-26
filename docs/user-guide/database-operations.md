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

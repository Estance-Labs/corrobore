# Elastic-free OpenCTI operations

This guide operates the certified small and conditional medium single-node OpenCTI stack
with Corrobore as its Knowledge Data Engine. It does not replace Redis,
RabbitMQ, or MinIO and does not claim high availability.

## Install

Copy `packaging/opencti-elastic-free/.env.example` to a protected environment
file and create every referenced secret below a directory with mode `0700`.
Set each secret source file to read-only mode `0444`: Docker Compose bind-mounts
these files into containers that deliberately run under different non-root
UIDs, and Compose does not apply the declared secret `uid`, `gid`, or `mode` to
file-backed secrets. The protected parent directory prevents host users from
traversing to those files, while each container receives only its explicitly
declared secret. Generate a TLS certificate whose SAN includes `corrobore`.
Generate the mandatory OpenCTI encryption secret with the following commands
and keep it with the backups; encrypted platform values cannot be recovered
without it:

```bash
mkdir -p packaging/opencti-elastic-free/secrets
chmod 0700 packaging/opencti-elastic-free/secrets
openssl rand -base64 -out packaging/opencti-elastic-free/secrets/opencti-encryption-key 32
openssl req -x509 -newkey rsa:2048 -sha256 -nodes \
  -keyout packaging/opencti-elastic-free/secrets/tls.key \
  -out packaging/opencti-elastic-free/secrets/tls.crt -days 365 \
  -subj /CN=corrobore \
  -addext basicConstraints=critical,CA:FALSE \
  -addext keyUsage=critical,digitalSignature,keyEncipherment \
  -addext extendedKeyUsage=serverAuth \
  -addext subjectAltName=DNS:corrobore
chmod 0444 packaging/opencti-elastic-free/secrets/*
```

Select either
`profiles/small.env` or `profiles/medium.env`, then validate before starting:

```bash
docker compose --env-file packaging/opencti-elastic-free/profiles/small.env \
  -f packaging/opencti-elastic-free/compose.yml config --quiet
scripts/opencti-elastic-free-migrate.sh install
docker compose -f packaging/opencti-elastic-free/compose.yml up --build --wait
```

The OpenCTI image is rebuilt from the exact supported commit in the Estance
fork; that commit is itself source-locked to the declared upstream release.
The native provider tests and TypeScript checks run inside the image build.
`DATABASE_ENGINE=corrobore` is fixed in Compose. Do not add
`ELASTICSEARCH__*` variables; startup rejects them. Corrobore starts with
`CORROBORE_OPENCTI_ELASTIC_FREE=true`, which requires persistent storage,
forbids a reference endpoint, skips obsolete projection-outbox entries, and
lets a fresh canonical instance serve primary reads without a synchronization
checkpoint.

The distribution reserves a separate authenticated rate bucket for the native
OpenCTI provider (`250` requests/second with a `10000` request burst). This
absorbs the bounded schema bootstrap without changing the general Corrobore API
defaults (`50` requests/second, burst `200`). Tune
`CORROBORE_OPENCTI_RATE_LIMIT_PER_SECOND` and
`CORROBORE_OPENCTI_RATE_LIMIT_BURST` only from captured bootstrap and steady
state traffic; the provider still retries `429` responses within its request
deadline.

## Capacity profiles

The small profile covers 100,000 objects and 500,000 relationships. It starts
with 4 GiB for OpenCTI, 2 GiB for Corrobore, and bounded 16K/32K hot records.
The conditional medium profile targets 1,000,000 objects and 5,000,000 relationships. It
starts with 8 GiB for OpenCTI, 6 GiB for Corrobore, and 65K/131K hot records.
Promote it to certified use only after the compatibility gate publishes and
passes the Corrobore medium measurements declared in `compatibility.json`.
Keep at least twice the canonical data size free while taking a snapshot or
rebuilding indexes. Change a limit only with captured P95, CPU, memory, disk,
snapshot and restore evidence.

## Migration

`scripts/opencti-elastic-free-migrate.sh` is a locked, durable state machine.
For an existing Elasticsearch/OpenSearch installation, set
`OPENCTI_MIGRATION_FROM_REFERENCE=true`, configure the three reference values
documented in `.env.example`, and combine `compose.yml` with
`compose.migration.yml`. Run each phase once, in order:

1. `install` validates the stack and creates reference-only routing.
2. `initial-import` imports `OPENCTI_MIGRATION_BUNDLE` from a consistent source snapshot.
3. `catch-up` applies `OPENCTI_CATCH_UP_BATCH` with source sequence and high-water mark.
4. `validate` requires zero lag, empty queues, `InSync` parity, no quarantine, and an executable full parity hook.
5. `shadow` compares Corrobore without changing visible results.
6. `canary` routes the deterministic percentage in `CORROBORE_CANARY_BASIS_POINTS`.
7. `primary-read` sends supported reads to Corrobore while retaining the reference.
8. `primary-write` assigns canonical write authority only after replay and parity.
9. `safety-delay` records the beginning of the observation window.
10. `shutdown-elastic` rechecks all gates, enforces `CORROBORE_MIGRATION_SAFETY_DELAY_SECONDS`, then calls the explicit reference shutdown hook.

After the shutdown hook succeeds, the command atomically selects
`primary_reads`, restarts Corrobore with `CORROBORE_OPENCTI_ELASTIC_FREE=true`,
and its configuration rejects any remaining reference endpoint. A fresh
installation skips the reference override; `install` creates the final
primary-read policy directly.

State and routing policies are written atomically below
`CORROBORE_MIGRATION_STATE_DIR`. Never edit `migration.json` by hand.

## Rollback

Before final reference shutdown, set `OPENCTI_REFERENCE_RESTORE_COMMAND` to an
audited executable and run `rollback`. The command suspends writes with a
bounded trigger such as `security_divergence`, restores and replays the
reference, requires `replay_complete` and `parity_verified`, assigns reference
authority, and returns routing to `reference_only`. It never deletes the
canonical Corrobore data or projection outbox.

After the final safety delay and reference shutdown, rollback means restoring
the last verified reference snapshot, replaying the retained reconstruction
plan and outbox, rerunning the complete parity matrix, then explicitly changing
authority. There is no blind downgrade.

## Backup, restore, upgrade, and index rebuild

Use the offline database commands described in
[Standalone operations](standalone-operations.md):

- `corrobore server snapshot` creates a coherent canonical snapshot;
- `corrobore server validate-snapshot` checks every component checksum;
- `corrobore server restore` requires a clean target directory;
- `corrobore server migrate` and `rollback` enforce the supported schema boundary;
- `corrobore server rebuild-indexes` reconstructs every derived index from canonical graph data.

Back up the OpenCTI, Redis, RabbitMQ, and MinIO volumes in the same maintenance
window. Restore Corrobore first, validate it, rebuild indexes if required, then
start dependencies and OpenCTI in health order. Upgrade only between entries in
`compatibility.json`; retain the previous images, canonical snapshot, migration
state, and reference projection until the new acceptance matrix passes.

## Metrics and alerts

Scrape Corrobore `/metrics` over authenticated TLS. Alerts must cover:

- non-zero synchronization lag or queue depth;
- any functional or security divergence;
- projection outbox growth, retry, or quarantine;
- write authority suspension and automatic routing rollback;
- WAL recovery or storage corruption;
- snapshot failure or excessive snapshot duration;
- restore validation failure;
- index rebuild failure, cancellation, or excessive duration;
- file extraction failures, quarantine, or index lag;
- P95 latency, backpressure, memory, disk, and profile capacity thresholds.

Diagnostics include `/health/ready`, `/version`, `/metrics`,
`/v1/opencti/sync/status`, `/v1/opencti/writes/status`,
`/v1/opencti/reconciliation/status`, and
`/v1/opencti/routing/decisions`. They intentionally omit graph payloads,
credentials, original idempotency keys, and policy subjects.

## Troubleshooting

- Readiness blocked after a kill: retain the volume, inspect recovery status,
  and never bypass strict recovery or exclusive ownership.
- Lag or projection outbox increasing: stop promotion, restore reference
  reachability, drain in order, and require exact verification.
- Security divergence: fail closed, suspend writes if affected, roll routing
  back immediately, and preserve redacted evidence.
- Corrupt derived index: keep canonical data offline, validate a snapshot, then
  run index rebuild. Do not reimport over the damaged store.
- File search lag: inspect worker bounds and quarantine without granting the
  worker network access or write access to source blobs.
- Capacity alert: lower concurrency or enlarge the selected certified profile;
  do not remove traversal, body, queue, or working-set bounds.

## Known limitations

Only OpenCTI `7.260722.0` is source-locked. The large profile, clustering,
replication, failover, rolling upgrades, multi-region operation, and replacement
of Redis/RabbitMQ/MinIO are outside this release. A later OpenCTI version must
refresh the native provider, full acceptance corpus, performance
evidence, and compatibility entry before use.

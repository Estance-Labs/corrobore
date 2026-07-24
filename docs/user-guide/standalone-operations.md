# Standalone Operations Guide

This runbook covers the supported single-node standalone deployment. It assumes
required bearer authentication, TLS for non-loopback exposure, and persistent
storage. Bolt, clustering, replication, high availability, and managed-cloud
operations are outside this guide.

Read the [configuration reference](standalone-configuration.md) before changing
production settings.

## Native deployment

Download the release archive for the host target from GitHub Releases. The
archive contains the binaries and their `SHA256SUMS` file:

```bash
install -d corrobore-release
tar --extract --gzip \
  --file release-artifacts-v<version>-<target>.tar.gz \
  --directory corrobore-release
cd corrobore-release
sha256sum --check SHA256SUMS
./corrobore server version
```

Windows release archives use ZIP and include `corrobore.exe`.

For a foreground development start on loopback:

```bash
CORROBORE_HTTP_AUTH_TOKEN_FILE="$PWD/.corrobore-secrets/http-token" \
  ./corrobore server start \
  --host 127.0.0.1 \
  --storage-mode persistent \
  --storage-dir "$PWD/.corrobore-runtime/graph"
```

For a production configuration, validate before opening storage or a listener:

```bash
corrobore server validate-config --config /etc/corrobore/corrobore.toml
corrobore server start --config /etc/corrobore/corrobore.toml
```

The second command remains in the foreground. Use a supervisor for restart
policy and process identity.

## Docker deployment

The production image runs as uid and gid `65532`, reads secrets and TLS material
from mounted files, and persists `/data`. The repository Compose stack also
mounts the persistent graph root at `/graph-data`.

Prepare `.env`, the token file, and TLS files as shown in
[Getting Started with Docker](../getting-started-docker.md), then validate and
start:

```bash
docker compose config
docker compose up --wait
docker compose ps
```

Use `docker compose up --build --wait` only when building the local checkout.
Published images are produced only by version-tag release runs and carry OCI
version and source-revision labels.

Follow structured output and probe the authenticated readiness endpoint:

```bash
docker compose logs --follow corrobore-http-server
curl --fail --silent --show-error --insecure \
  --header "Authorization: Bearer ${CORROBORE_HTTP_AUTH_TOKEN}" \
  https://127.0.0.1:8080/health/ready
```

`docker compose down` removes containers but preserves named volumes.
`docker compose down --volumes` deletes persistent state and must never be used
as a routine stop command.

## systemd deployment

The repository provides matching files:

- `packaging/systemd/corrobore.service`;
- `packaging/systemd/corrobore.toml`.

Install the release executable and create a dedicated identity:

```bash
sudo install -D -m 0755 corrobore /usr/local/bin/corrobore
sudo useradd --system --home-dir /var/lib/corrobore \
  --shell /usr/sbin/nologin corrobore
sudo install -d -o corrobore -g corrobore -m 0750 \
  /var/lib/corrobore/runtime \
  /var/lib/corrobore/graph \
  /var/log/corrobore
sudo install -d -o root -g corrobore -m 0750 \
  /etc/corrobore/secrets \
  /etc/corrobore/tls
sudo install -o root -g corrobore -m 0640 \
  packaging/systemd/corrobore.toml /etc/corrobore/corrobore.toml
sudo install -o root -g root -m 0644 \
  packaging/systemd/corrobore.service /etc/systemd/system/corrobore.service
```

Install the bearer token, certificate, and private key at the paths declared in
the TOML file with group-readable mode `0640`. Never put their values in the
unit or commit them to the repository.

Validate as the service identity, then enable the foreground service:

```bash
sudo -u corrobore \
  /usr/local/bin/corrobore server validate-config --config /etc/corrobore/corrobore.toml
sudo systemctl daemon-reload
sudo systemctl enable --now corrobore.service
systemctl status corrobore.service
```

The exact supervised command is:

```console
corrobore server start --config /etc/corrobore/corrobore.toml
```

The unit uses `Type=simple`; Corrobore does not fork or write a PID file.

## Operational endpoints

Operational routes match the
[OpenAPI specification](../api/openapi.yaml). Production non-loopback
configuration sets `operations.endpoint_policy = "authenticated"`, so include
the primary bearer token.

| Endpoint | Meaning | Expected healthy result |
| --- | --- | --- |
| `GET /health/live` | The event loop can continue running. | HTTP 200 with `live: true`. |
| `GET /health/ready` | Initialization and recovery are complete and the lifecycle accepts work. | HTTP 200 with `ready: true` and lifecycle `ready`. |
| `GET /version` | Product, build, protocol, and storage compatibility. | Supported versions/formats plus active persistent format when configured. |
| `GET /metrics` | Prometheus text metrics with bounded-cardinality labels. | HTTP 200 without graph content or credentials. |

Example:

```bash
curl --fail --silent --show-error \
  --cacert /etc/corrobore/tls/server.crt \
  --header "Authorization: Bearer $(cat /etc/corrobore/secrets/http-token)" \
  https://127.0.0.1:8080/version
```

The CLI performs the readiness and compatibility checks with the same resolved
configuration:

```bash
corrobore server status --config /etc/corrobore/corrobore.toml
```

An unavailable endpoint exits `8`; an incompatible storage contract exits `9`.

## Logging and correlation

The standalone service writes structured JSONL logs under `logging.directory`
and emits operational messages to the supervisor. HTTP responses carry an
`X-Request-Id`. A valid client identifier is echoed; otherwise the server
generates one. Request, runtime, and error events use that same identifier.

For systemd:

```bash
journalctl --unit corrobore.service --follow
```

For Docker:

```bash
docker compose logs --follow corrobore-http-server
```

Do not send graph content, token values, private keys, or signed license
material to log aggregation. Effective configuration and field diagnostics
redact secret values by design.

## Graceful shutdown

Use the supervisor rather than sending `SIGKILL`:

```bash
sudo systemctl stop corrobore.service
```

or:

```bash
docker compose stop --timeout 15 corrobore-http-server
```

`SIGTERM` changes readiness to false, rejects new protected work with HTTP 503,
waits for accepted work within `server.shutdown_timeout_ms`, cancels remaining
work when the bound expires, flushes persistent files, and releases directory
ownership. Exit code `0` means a clean stop; code `7` reports a timeout or final
durability-flush failure.

## Backup

Use a **consistent offline backup** for this release. Stop the service cleanly
before copying its persistent roots. Copying live append logs independently can
combine incompatible points in time.

The systemd configuration keeps session state and graph storage under
`/var/lib/corrobore`. The following creates a timestamped archive without
including token or private-key files:

```bash
sudo systemctl stop corrobore.service
if systemctl is-active --quiet corrobore.service; then
  echo "corrobore is still active" >&2
  exit 1
fi

BACKUP_DIR="/srv/backups/corrobore/$(date -u +%Y%m%dT%H%M%SZ)"
sudo install -d -m 0750 "${BACKUP_DIR}"
sudo tar --create --gzip \
  --file "${BACKUP_DIR}/data.tar.gz" \
  --directory /var/lib/corrobore .
sudo sha256sum "${BACKUP_DIR}/data.tar.gz" |
  sudo tee "${BACKUP_DIR}/SHA256SUMS"
sudo systemctl start corrobore.service
```

The archive must contain the graph root's `manifest.json`, append logs,
transaction state, catalog metadata, and the runtime session store as one
consistent set. Protect backups as sensitive operational data.

For Compose, stop the service and copy both mounted roots while the container is
stopped:

```bash
docker compose stop --timeout 15 corrobore-http-server
install -d -m 0750 backup/data backup/graph-data
docker compose cp --archive corrobore-http-server:/data/. backup/data/
docker compose cp --archive corrobore-http-server:/graph-data/. backup/graph-data/
tar --create --gzip --file backup/corrobore-volumes.tar.gz \
  --directory backup data graph-data
sha256sum backup/corrobore-volumes.tar.gz > backup/SHA256SUMS
docker compose start corrobore-http-server
```

Storage-level backup integrity and semantic equivalence are continuously
validated against a persistent dataset with:

```console
cargo test -p graph-storage --test backup_restore_integrity --locked
```

## Restore

Restore only while Corrobore is stopped and only into an **empty restore
target**. Keep the displaced data until readiness and application checks pass.

Systemd procedure:

```bash
sudo systemctl stop corrobore.service
cd /srv/backups/corrobore/<timestamp>
sudo sha256sum --check SHA256SUMS

sudo mv /var/lib/corrobore "/var/lib/corrobore.before-restore"
sudo install -d -o corrobore -g corrobore -m 0750 /var/lib/corrobore
sudo tar --extract --gzip --file data.tar.gz \
  --directory /var/lib/corrobore
sudo chown -R corrobore:corrobore /var/lib/corrobore

sudo -u corrobore \
  /usr/local/bin/corrobore server validate-config --config /etc/corrobore/corrobore.toml
sudo systemctl start corrobore.service
corrobore server status --config /etc/corrobore/corrobore.toml
```

`validate-config` checks schema and paths without mutating storage. Startup then
validates `manifest.json`, required logs, checksums, record format, and storage
ownership before readiness.

Compose procedure:

```bash
docker compose down --volumes
docker compose create corrobore-http-server
tar --extract --gzip --file backup/corrobore-volumes.tar.gz \
  --directory backup
docker compose cp --archive backup/data/. corrobore-http-server:/data
docker compose cp --archive backup/graph-data/. corrobore-http-server:/graph-data
docker compose up --wait
```

The first command intentionally removes the old named volumes; run it only
after verifying the backup checksum and retaining a recoverable copy.

## Upgrade

1. Record the current executable or immutable image tag and configuration.
2. Query `GET /version` and save
   `storage_compatibility`, `active_storage_version`, and
   `active_record_format`.
3. Read the target release notes and verify that the active values appear in
   its supported storage versions and record formats.
4. Create and verify a consistent offline backup.
5. Run the candidate `corrobore server version` and
   `corrobore server validate-config` before replacing the service.
6. Stop cleanly, replace only the binary or immutable image tag, and start.
7. Require `GET /health/ready`, `GET /version`, and representative authenticated
   reads before declaring success.

Do not upgrade by moving a writable data directory between two running
processes. Never use the floating `latest` image tag when a rollback decision
must be reproducible.

## Rollback

A binary or image rollback is supported only when the previous release declares
the current `active_storage_version` and `active_record_format` as compatible
and the configuration uses fields that release understands.

If the target upgrade changed or migrated durable data, do not start the old
release on that directory. Stop the candidate, move its directory aside, and
restore the pre-upgrade archive into an empty restore target. Then reinstall
the recorded binary or immutable image tag and repeat readiness, version, and
representative-read checks.

Retain both the pre-upgrade backup and the failed candidate data until the
rollback is verified. A storage incompatibility exits with code `5`; recovery
or integrity failure exits with code `6`. Do not bypass either gate.

## Troubleshooting

| Symptom | Check | Action |
| --- | --- | --- |
| Configuration exits `2` | Field named in stderr | Run `validate-config --print-effective`; correct the highest-precedence source. |
| Startup exits `4` | Another process owns the graph directory | Stop the other process; do not delete lock metadata to bypass OS ownership. |
| Startup exits `5` | Unsupported storage version/format | Restore a compatible backup or install a compatible release. |
| Startup exits `6` | Manifest, log, or recovery integrity failure | Preserve the directory, inspect diagnostics, and restore a validated backup. |
| Status exits `8` | Listener, TLS trust, token, or readiness | Check supervisor logs and probe `/health/live` and `/health/ready`. |
| Status exits `9` | Remote compatibility payload | Compare the `/version` compatibility fields with the client release. |
| Readiness stays false | Recovery or lifecycle initialization | Follow structured logs; never route production traffic to the instance. |
| Shutdown exits `7` | Work exceeded the drain bound or flush failed | Preserve logs and storage; verify recovery before restarting. |

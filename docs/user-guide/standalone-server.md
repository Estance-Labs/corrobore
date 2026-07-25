# Standalone Server CLI

The `corrobore` executable provides one operator entry point for the standalone
server:

```console
corrobore server start
corrobore server validate-config
corrobore server status
corrobore server version
```

See [Deployment Modes](deployment-modes.md) for how this deployable process
relates to the HTTP contract and embedded engine.

`start` runs in the foreground. A process supervisor should own restarts and
daemonization.

The complete field/default/precedence matrix is in the
[standalone configuration reference](standalone-configuration.md). Deployment,
monitoring, backup, restore, upgrade, and rollback procedures are in the
[standalone operations guide](standalone-operations.md).

## Configuration sources and precedence

The server resolves configuration in this order:

```text
CLI arguments > environment variables > TOML file > defaults
```

Authentication defaults to `required`. Supply the bearer token inline or,
preferably, through a protected file. The explicit `local-insecure` mode omits
authentication only on a loopback bind. Use `corrobore server start --help` to
list every CLI override.

The following example is a complete starting point:

```toml
[server]
host = "127.0.0.1"
port = 8080
auth_mode = "required"
auth_token_file = "/run/secrets/corrobore-http-token"
data_directory = ".corrobore-runtime"
shutdown_timeout_ms = 5000

[storage]
mode = "persistent"
directory = ".corrobore-runtime/graph"
require_fsync = true
strict_recovery = true
max_hot_nodes = 16384
max_hot_relationships = 32768
max_warm_adjacency_entries = 65536

[logging]
directory = ".corrobore-runtime/logs"
level = "info"
format = "json"

[limits]
request_timeout_ms = 30000
max_body_bytes = 2097152
import_max_body_bytes = 33554432
opencti_sync_max_operations = 512
opencti_sync_max_replay_identities = 4096
rate_limit_per_second = 50
rate_limit_burst = 200

[interfaces]
enabled = ["http"]

[maintenance]
enabled = false
interval_ms = 60000

[operations]
endpoint_policy = "public"

[tls]
enabled = false
```

Start with that file:

```console
corrobore server start --config corrobore.toml
```

For secrets, prefer a file reference over TOML or command-line arguments:

```console
CORROBORE_HTTP_AUTH_TOKEN_FILE=/run/secrets/corrobore-http-token \
  corrobore server start --config corrobore.toml
```

The existing environment-only contract remains supported:

```console
CORROBORE_HTTP_AUTH_TOKEN=replace-with-a-secret \
CORROBORE_HTTP_HOST=127.0.0.1 \
CORROBORE_HTTP_PORT=8080 \
  corrobore server start
```

All variables documented in the [HTTP server guide](http-server.md#configuration)
can be used without a TOML file. The CLI also recognizes these operational
variables:

| Variable | Default | Purpose |
| --- | --- | --- |
| `CORROBORE_SERVER_INTERFACES` | `http` | Comma-separated enabled interfaces (`http`, optionally `web`). |
| `CORROBORE_LOG_LEVEL` | `info` | Tracing filter or log level; `RUST_LOG` remains a compatibility alias. |
| `CORROBORE_LOG_FORMAT` | `json` | Structured log format. Only `json` is currently supported. |
| `CORROBORE_MAINTENANCE_ENABLED` | `false` | Enables the maintenance policy reported to the server lifecycle. |
| `CORROBORE_MAINTENANCE_INTERVAL_MS` | `60000` | Maintenance interval. |
| `CORROBORE_OPERATIONAL_ENDPOINT_POLICY` | `public` | Operational endpoints are `public` or `authenticated`. Non-loopback binds require `authenticated`. |
| `CORROBORE_TLS_ENABLED` | `false` | Enables the HTTPS listener. |
| `CORROBORE_TLS_CERTIFICATE_FILE` | unset | TLS certificate path. |
| `CORROBORE_TLS_PRIVATE_KEY_FILE` | unset | TLS private-key path. |

Non-loopback binds require TLS, required bearer authentication, and
authenticated operational endpoints. Startup validates that certificate and
private-key files are readable, that the key matches the certificate, and that
the certificate is currently valid before opening the HTTPS listener. TLS and
secret files are reloaded on process restart; rotating either does not require
storage migration.

```console
corrobore server start \
  --host 0.0.0.0 \
  --auth-token-file /run/secrets/corrobore-http-token \
  --operational-endpoint-policy authenticated \
  --tls-enabled true \
  --tls-certificate-file /etc/corrobore/tls/server.crt \
  --tls-private-key-file /etc/corrobore/tls/server.key
```

Diagnostics identify only the configuration field that failed. Secret values
and private-key contents are excluded from effective configuration, errors,
logs, and metrics.

## Persistent directory ownership and recovery

Persistent mode gives one server process exclusive ownership of the configured
storage directory for its full lifetime. Before creating a manifest, rebuilding
catalog metadata, or exposing a writable store, the server locks a hidden
sibling file named `.<directory-name>.corrobore.lock`. For example,
`/srv/corrobore/graph` uses `/srv/corrobore/.graph.corrobore.lock`.

A second server targeting the same directory fails immediately. The lock is
released when the server state is dropped or the process terminates, including
an abrupt process termination. The lock file itself can remain on disk; its
non-secret process and package-version metadata is diagnostic only, and stale
metadata is replaced after the operating system confirms that no process still
owns the lock.

With `storage.strict_recovery = true`, startup validates the manifest and all
required append logs, then deterministically rebuilds derived catalog metadata
before readiness. Unsupported storage versions or record formats are rejected
before recovery. Corrupted or incomplete durable state is never opened as a
writable store.

## Graceful shutdown

The server installs `SIGINT` and `SIGTERM` handlers before opening its listener.
Either signal moves the process from `ready` to `draining`, rejects new
non-operational requests with HTTP 503 and code `SERVICE_DRAINING`, and lets
accepted work finish within `server.shutdown_timeout_ms`. When the bound
expires, remaining work is cancelled before persistent files are flushed.

Persistent files and directory metadata are synchronized before the
data-directory ownership lock is released. A clean drain exits with code `0`.
A forced shutdown or flush failure exits non-zero and remains observable in
stderr without including credentials.

## Validate without side effects

Use the same resolution and validation path without opening storage, creating
runtime directories, or binding listeners:

```console
corrobore server validate-config --config corrobore.toml
```

To inspect the resolved non-secret settings:

```console
corrobore server validate-config \
  --config corrobore.toml \
  --print-effective
```

Authentication values are always rendered as `<redacted>`. TOML diagnostics do
not echo source lines, so a malformed secret setting is not copied to stderr.

## Exit codes

| Code | Meaning |
| ---: | --- |
| `0` | Command completed successfully. |
| `2` | Configuration could not be read, parsed, resolved, or validated. |
| `3` | Configuration was valid, but a general server startup step failed. |
| `4` | The persistent storage directory is owned by another process. |
| `5` | The storage manifest declares an incompatible version or record format. |
| `6` | Persistent storage recovery or integrity validation failed. |
| `7` | Graceful shutdown exceeded its bound or the final durability flush failed. |
| `8` | `server status` could not reach a ready compatible endpoint within the configured timeout. |
| `9` | `server status` reached the endpoint but its storage compatibility contract is unsupported. |

Validation errors identify the affected field. General startup errors cover
conditions such as an invalid bind address, an occupied port, or a requested
listener that this release cannot provide. Storage errors include the affected
directory and an actionable reason without echoing authentication secrets.

## Version and compatibility

`corrobore server version` is independent of runtime configuration and reports
the package version, build target, and compile-time revision metadata:

```console
corrobore server version
```

Release archives and the production container distribute `corrobore` as the
standalone product entry point. The legacy `corrobore-http-server` target
remains a development compatibility binary and is not included in standalone
release archives.

## Native, container, and systemd distribution

Native release archives contain the `corrobore` executable, its SHA-256
checksum, and compile-time source revision. Validate an extracted artifact
without runtime configuration:

```console
corrobore server version
```

The production image runs the same foreground command:

```text
corrobore server start --config /etc/corrobore/corrobore.toml
```

It runs as uid/gid `65532`, declares `/data` as its persistent volume, uses
`SIGTERM`, and allows fifteen seconds for the configured drain. Authentication
and TLS files must be mounted read-only under `/run/secrets`; neither is stored
in the image. OCI labels expose the product version and exact source revision.

The minimal configuration is
`packaging/corrobore.production.toml`.
The matching foreground systemd example is
`packaging/systemd/corrobore.service`.
Install it with a dedicated `corrobore` user and group, create the writable
state/log directories, place the configuration under `/etc/corrobore`, then
use normal systemd supervision:

```console
sudo systemctl enable --now corrobore.service
systemctl status corrobore.service
```

The unit does not fork or use an internal daemon mode. `systemctl stop` sends
`SIGTERM` and leaves shutdown coordination to the standalone process.

Use the dedicated `packaging/systemd/corrobore.toml` with that unit. It places
writable state under `/var/lib/corrobore` and logs under `/var/log/corrobore`,
matching the unit's filesystem protections. Do not install the container
configuration unchanged on a systemd host because its `/data` paths are
container-specific.

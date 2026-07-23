# Standalone Server CLI

The `corrobore` executable provides one operator entry point for the standalone
server:

```console
corrobore server start
corrobore server validate-config
corrobore server version
```

`start` runs in the foreground. A process supervisor should own restarts and
daemonization.

## Configuration sources and precedence

The server resolves configuration in this order:

```text
CLI arguments > environment variables > TOML file > defaults
```

The authentication token has no default and must be supplied by one of those
sources. Use `corrobore server start --help` to list every CLI override.

The following example is a complete starting point:

```toml
[server]
host = "127.0.0.1"
port = 8080
auth_token = "replace-with-a-secret"
data_directory = ".corrobore-runtime"
shutdown_timeout_ms = 5000

[storage]
mode = "persistent"
directory = ".corrobore-runtime/graph"
require_fsync = true
strict_recovery = true

[logging]
directory = ".corrobore-runtime/logs"
level = "info"
format = "json"

[limits]
request_timeout_ms = 30000
max_body_bytes = 2097152
import_max_body_bytes = 33554432
rate_limit_per_second = 50
rate_limit_burst = 200

[interfaces]
enabled = ["http"]

[maintenance]
enabled = false
interval_ms = 60000

[tls]
enabled = false
```

Start with that file:

```console
corrobore server start --config corrobore.toml
```

For secrets, prefer environment variables over TOML or command-line arguments:

```console
CORROBORE_HTTP_AUTH_TOKEN=replace-with-a-secret \
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
| `CORROBORE_TLS_ENABLED` | `false` | Requests TLS configuration. |
| `CORROBORE_TLS_CERTIFICATE_FILE` | unset | TLS certificate path. |
| `CORROBORE_TLS_PRIVATE_KEY_FILE` | unset | TLS private-key path. |

TLS configuration is validated, but the TLS listener is intentionally not
available in this release. `server start` refuses TLS-enabled configuration
instead of silently serving plaintext.

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
| `3` | Configuration was valid, but server startup failed. |

Validation errors identify the affected field. Startup errors cover conditions
such as an invalid bind address, an occupied port, or a requested listener that
this release cannot provide.

## Version and compatibility

`corrobore server version` is independent of runtime configuration and reports
the package version, build target, and compile-time revision metadata:

```console
corrobore server version
```

The legacy `corrobore-http-server` executable remains available for
environment-only deployments. New automation should use
`corrobore server start`.

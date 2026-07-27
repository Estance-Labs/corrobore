# Standalone Configuration Reference

This page is the complete configuration reference for the unified `corrobore`
standalone executable. Validate production settings before starting the
listener:

```console
corrobore server validate-config --config /etc/corrobore/corrobore.toml
```

## Commands

| Command | Configuration required | Behavior |
| --- | --- | --- |
| `corrobore server start` | Yes | Validates configuration, opens storage, then serves in the foreground until `SIGINT` or `SIGTERM`. |
| `corrobore server validate-config` | Yes | Resolves and validates configuration without opening storage, creating runtime directories, or binding a port. |
| `corrobore server status` | Yes | Probes readiness and version compatibility through the configured HTTP or HTTPS endpoint with the configured authentication policy. |
| `corrobore server version` | No | Prints package version, build target, and compile-time source revision. |
| `corrobore server snapshot` | Offline storage | Creates a coherent local snapshot while holding exclusive ownership of the persistent storage root. |
| `corrobore server validate-snapshot` | Snapshot | Validates a local snapshot manifest, components, checksums, and optional encryption-key identity. |
| `corrobore server export-snapshot-s3` | Snapshot and S3 credentials | Uploads a validated snapshot to an S3-compatible object store. |
| `corrobore server restore` | Snapshot and empty target | Restores a validated snapshot into a new empty persistent storage root. |
| `corrobore server migrate` | Offline storage | Runs or resumes the supported previous-version storage migration. |
| `corrobore server rollback` | Offline storage | Rolls back the compatible manifest boundary after a completed migration. |
| `corrobore server rebuild-indexes` | Offline storage | Rebuilds derived indexes from canonical data. |
| `corrobore server cancel-rebuild` | Offline storage | Cancels an incomplete derived-index rebuild at its durable boundary. |

`validate-config --print-effective` prints only non-secret effective settings.
`status` exits `8` when the endpoint is unavailable and `9` when the remote
storage contract is incompatible. The other stable exit codes are documented
in the [CLI guide](standalone-server.md#exit-codes).

## Configuration precedence

Settings resolve deterministically:

```text
CLI arguments > environment variables > TOML file > defaults
```

A higher-precedence inline secret replaces a lower-precedence file source, and
a higher-precedence file source replaces a lower-precedence inline value.
Providing both forms at the same precedence is rejected.

## CLI options

`start`, `status`, and the configuration portion of `validate-config` share the
same overrides.

| Option | Purpose |
| --- | --- |
| `--config` | Read the TOML file at this path. |
| `--host` | Override the listener host. |
| `--port` | Override the listener port. |
| `--auth-mode` | Select `required` or loopback-only `local-insecure`. |
| `--auth-token` | Supply the primary bearer token inline; avoid this in production because process arguments can be observable. |
| `--auth-token-file` | Read the primary bearer token from a protected file. |
| `--admin-auth-token` | Supply the optional administrative bearer token inline. |
| `--admin-auth-token-file` | Read the administrative bearer token from a protected file. |
| `--operational-endpoint-policy` | Select `public` or `authenticated` operational endpoints. |
| `--data-dir` | Override the session/runtime data directory. |
| `--storage-mode` | Select `ephemeral` or `persistent` graph storage. |
| `--storage-dir` | Override the persistent graph root. |
| `--storage-require-fsync` | Enable or disable fsync-required persistent commits. |
| `--storage-strict-recovery` | Enable or disable strict recovery and catalog rebuild. |
| `--log-dir` | Override the structured log directory. |
| `--log-level` | Override the tracing filter or level. |
| `--log-format` | Select the structured log format; this release supports `json`. |
| `--query-timeout-ms` | Override the request/query timeout. |
| `--shutdown-timeout-ms` | Override the graceful-shutdown budget. |
| `--max-body-bytes` | Override the standard request-body limit. |
| `--import-max-body-bytes` | Override the import request-body limit. |
| `--opencti-sync-max-operations` | Override the maximum mutations accepted in one OpenCTI synchronization batch. |
| `--opencti-sync-max-replay-identities` | Override bounded replay-identity and dead-letter retention. |
| `--rate-limit-per-second` | Override the sustained protected-route rate. |
| `--rate-limit-burst` | Override the protected-route burst allowance. |
| `--opencti-rate-limit-per-second` | Override the sustained rate reserved for OpenCTI provider traffic. |
| `--opencti-rate-limit-burst` | Override the OpenCTI provider-traffic burst allowance. |
| `--probe-host` | Override the host used by the bounded `status` probe. |
| `--interfaces` | Supply a comma-separated set containing `http` and optionally `web`. |
| `--web-dir` | Override the production explorer asset directory. |
| `--maintenance-enabled` | Enable or disable lifecycle maintenance policy. |
| `--maintenance-interval-ms` | Override the maintenance interval. |
| `--tls-enabled` | Enable or disable HTTPS. |
| `--tls-certificate-file` | Override the PEM certificate-chain path. |
| `--tls-private-key-file` | Override the PEM private-key path. |
| `--print-effective` | With `validate-config`, print resolved non-secret settings. |

Run `corrobore server start --help` or
`corrobore server validate-config --help` for the generated CLI help.

### Offline storage command options

Offline storage commands require exclusive ownership of the persistent data
directory. The [database operations guide](database-operations.md) provides
complete procedures and validation steps.

| Option | Commands | Purpose |
| --- | --- | --- |
| `--storage-dir` | `snapshot`, `migrate`, `rollback`, `rebuild-indexes`, `cancel-rebuild` | Select the offline persistent storage root. |
| `--destination` | `snapshot` | Select the new local snapshot directory. |
| `--encryption-key-id` | `snapshot`, `validate-snapshot`, `restore` | Record or verify the external key-provider identity without storing key material. |
| `--retention-hook` | `snapshot` | Invoke an optional provider lifecycle or retention hook. |
| `--snapshot` | `validate-snapshot`, `export-snapshot-s3`, `restore` | Select the local snapshot artifact directory. |
| `--endpoint` | `export-snapshot-s3` | Set the S3 or MinIO endpoint. |
| `--bucket` | `export-snapshot-s3` | Set the destination bucket. |
| `--prefix` | `export-snapshot-s3` | Set the destination object prefix. |
| `--region` | `export-snapshot-s3` | Set the AWS signing region; the default is `us-east-1`. |
| `--target` | `restore` | Select the new empty restoration target. |
| `--from` | `migrate` | Set the source storage version; the current supported value is `V0`. |
| `--to` | `migrate` | Set the target storage version; the current supported value is `V1`. |

## TOML reference

Every accepted TOML field appears below. Unknown sections and fields are
rejected.

| Field | Environment variable | CLI option | Default |
| --- | --- | --- | --- |
| `server.host` | `CORROBORE_HTTP_HOST` | `--host` | `127.0.0.1` |
| `server.port` | `CORROBORE_HTTP_PORT` | `--port` | `8080` |
| `server.auth_mode` | `CORROBORE_HTTP_AUTH_MODE` | `--auth-mode` | `required` |
| `server.auth_token` | `CORROBORE_HTTP_AUTH_TOKEN` | `--auth-token` | unset; production should use a file |
| `server.auth_token_file` | `CORROBORE_HTTP_AUTH_TOKEN_FILE` | `--auth-token-file` | unset |
| `server.admin_auth_token` | `CORROBORE_HTTP_ADMIN_AUTH_TOKEN` | `--admin-auth-token` | unset |
| `server.admin_auth_token_file` | `CORROBORE_HTTP_ADMIN_AUTH_TOKEN_FILE` | `--admin-auth-token-file` | unset |
| `server.data_directory` | `CORROBORE_HTTP_SESSION_STORE_DIR` | `--data-dir` | `.corrobore-runtime` |
| `server.shutdown_timeout_ms` | `CORROBORE_HTTP_SHUTDOWN_TIMEOUT_MS` | `--shutdown-timeout-ms` | `5000` |
| `storage.mode` | `CORROBORE_STORAGE_MODE` | `--storage-mode` | `ephemeral` |
| `storage.directory` | `CORROBORE_STORAGE_DIR` | `--storage-dir` | unset; required in persistent mode |
| `storage.require_fsync` | `CORROBORE_STORAGE_REQUIRE_FSYNC` | `--storage-require-fsync` | `false` in ephemeral mode, `true` in persistent mode |
| `storage.strict_recovery` | `CORROBORE_STORAGE_STRICT_RECOVERY` | `--storage-strict-recovery` | `false` in ephemeral mode, `true` in persistent mode |
| `storage.max_hot_nodes` | `CORROBORE_STORAGE_MAX_HOT_NODES` | `--storage-max-hot-nodes` | `16384` |
| `storage.max_hot_relationships` | `CORROBORE_STORAGE_MAX_HOT_RELATIONSHIPS` | `--storage-max-hot-relationships` | `32768` |
| `storage.max_warm_adjacency_entries` | `CORROBORE_STORAGE_MAX_WARM_ADJACENCY_ENTRIES` | `--storage-max-warm-adjacency-entries` | `65536` |
| `logging.directory` | `CORROBORE_HTTP_LOG_DIR` | `--log-dir` | `<server.data_directory>/logs` |
| `logging.level` | `CORROBORE_LOG_LEVEL` | `--log-level` | `info`; `RUST_LOG` is a compatibility alias |
| `logging.format` | `CORROBORE_LOG_FORMAT` | `--log-format` | `json` |
| `limits.request_timeout_ms` | `CORROBORE_HTTP_REQUEST_TIMEOUT_MS` | `--query-timeout-ms` | `30000` |
| `limits.max_body_bytes` | `CORROBORE_HTTP_MAX_BODY_BYTES` | `--max-body-bytes` | `2097152` |
| `limits.import_max_body_bytes` | `CORROBORE_HTTP_IMPORT_MAX_BODY_BYTES` | `--import-max-body-bytes` | `33554432`; also bounds transactional writes |
| `limits.opencti_sync_max_operations` | `CORROBORE_OPENCTI_SYNC_MAX_OPERATIONS` | `--opencti-sync-max-operations` | `512`; also bounds transactional bulk writes |
| `limits.opencti_sync_max_replay_identities` | `CORROBORE_OPENCTI_SYNC_MAX_REPLAY_IDENTITIES` | `--opencti-sync-max-replay-identities` | `4096`; also bounds reconciliation state |
| — | `CORROBORE_OPENCTI_ELASTIC_FREE` | — | `false`; `true` requires `CORROBORE_STORAGE_MODE=persistent` and a routing policy file, and forbids `CORROBORE_OPENCTI_SHADOW_REFERENCE_ENDPOINT`. See [Elastic-free OpenCTI operations](opencti-elastic-free-operations.md) |
| — | `CORROBORE_OPENCTI_SHADOW_REFERENCE_ENDPOINT` | — | unset |
| — | `CORROBORE_OPENCTI_SHADOW_REFERENCE_VERSION` | — | `unconfigured` |
| — | `CORROBORE_OPENCTI_SHADOW_REFERENCE_AUTH_TOKEN` | — | unset; prefer file source |
| — | `CORROBORE_OPENCTI_SHADOW_REFERENCE_AUTH_TOKEN_FILE` | — | unset |
| — | `CORROBORE_OPENCTI_SHADOW_RELEASE` | — | package version |
| — | `CORROBORE_OPENCTI_SHADOW_SAMPLE_BASIS_POINTS` | — | `0` |
| — | `CORROBORE_OPENCTI_SHADOW_MAX_CONCURRENCY` | — | `4`; also bounds Corrobore-primary writes |
| — | `CORROBORE_OPENCTI_SHADOW_TIMEOUT_MS` | — | `2000`; also applies to canonical writes and reference projection |
| — | `CORROBORE_OPENCTI_SHADOW_MAX_REPORTS` | — | `10000` |
| — | `CORROBORE_OPENCTI_SHADOW_SAMPLING_POLICY_FILE` | — | unset |
| — | `CORROBORE_OPENCTI_SHADOW_BASELINE_FILE` | — | unset |
| — | `CORROBORE_OPENCTI_READ_ROUTING_POLICY_FILE` | — | unset; safe default is reference-only |
| — | `CORROBORE_OPENCTI_READ_ROUTING_MAX_AUDITS` | — | `10000` |
| `limits.rate_limit_per_second` | `CORROBORE_HTTP_RATE_LIMIT_PER_SECOND` | `--rate-limit-per-second` | `50` |
| `limits.rate_limit_burst` | `CORROBORE_HTTP_RATE_LIMIT_BURST` | `--rate-limit-burst` | `200` |
| `limits.opencti_rate_limit_per_second` | `CORROBORE_OPENCTI_RATE_LIMIT_PER_SECOND` | `--opencti-rate-limit-per-second` | `50` |
| `limits.opencti_rate_limit_burst` | `CORROBORE_OPENCTI_RATE_LIMIT_BURST` | `--opencti-rate-limit-burst` | `200` |
| `interfaces.enabled` | `CORROBORE_SERVER_INTERFACES` | `--interfaces` | `["http"]` |
| `interfaces.web_directory` | `CORROBORE_HTTP_WEB_DIR` | `--web-dir` | unset |
| `maintenance.enabled` | `CORROBORE_MAINTENANCE_ENABLED` | `--maintenance-enabled` | `false` |
| `maintenance.interval_ms` | `CORROBORE_MAINTENANCE_INTERVAL_MS` | `--maintenance-interval-ms` | `60000` |
| `operations.endpoint_policy` | `CORROBORE_OPERATIONAL_ENDPOINT_POLICY` | `--operational-endpoint-policy` | `public` |
| `tls.enabled` | `CORROBORE_TLS_ENABLED` | `--tls-enabled` | `false` |
| `tls.certificate_file` | `CORROBORE_TLS_CERTIFICATE_FILE` | `--tls-certificate-file` | unset |
| `tls.private_key_file` | `CORROBORE_TLS_PRIVATE_KEY_FILE` | `--tls-private-key-file` | unset |

`limits.query_timeout_ms` remains a TOML alias for
`limits.request_timeout_ms`.

## Environment-variable reference

The matrix above covers variables that map to the standalone TOML schema.
These additional variables are supported by the underlying HTTP runtime and
remain environment-only:

| Variable | Default | Purpose |
| --- | --- | --- |
| `CORROBORE_HTTP_SESSION_IDLE_TTL_MS` | `0` | Automatically stop inactive sessions after this many milliseconds; `0` disables expiry. |
| `CORROBORE_HTTP_LICENSE_PEM` | unset | Inline signed enterprise license; prefer the file form. |
| `CORROBORE_HTTP_LICENSE_PEM_FILE` | unset | Signed enterprise license file. |
| `CORROBORE_HTTP_LICENSE_PUBLIC_KEY_PEM` | unset | Inline Ed25519 license-verification key; prefer the file form. |
| `CORROBORE_HTTP_LICENSE_PUBLIC_KEY_PEM_FILE` | unset | Ed25519 license-verification key file. |
| `CORROBORE_HTTP_LICENSED_MODULES` | unset | Legacy compatibility fallback when no signed license is configured. |
| `CORROBORE_DOMAIN_PROVIDER_DIR` | unset | Trusted root containing optional native domain providers. |
| `CORROBORE_DOMAIN_PROVIDER_MANIFEST_FILE` | unset | Manifest pinning provider paths, hashes, policy, and capabilities. |
| `CORROBORE_MEMORY_WORKSPACE_ID` | `workspace--standalone-default` | Trusted workspace for high-level memory operations. |
| `CORROBORE_MEMORY_ACTOR_ID` | `actor--standalone-client` | Trusted actor attribution for high-level memory operations. |
| `CORROBORE_MEMORY_AGENT_ID` | unset | Optional trusted agent attribution for high-level memory operations. |
| `CORROBORE_MEMORY_SESSION_ID` | `session--standalone-api` | Trusted session attribution for high-level memory operations. |
| `CORROBORE_MEMORY_PERMISSIONS` | `read,write,trace,forget,consolidate` | Independently enabled high-level memory capabilities. |
| `CORROBORE_OPENCTI_ELASTIC_FREE` | `false` | Require final operation without the reference search provider. |
| `CORROBORE_OPENCTI_RATE_LIMIT_PER_SECOND` | `200` | Sustained rate reserved for authenticated OpenCTI provider traffic. |
| `CORROBORE_OPENCTI_RATE_LIMIT_BURST` | `1000` | Burst allowance reserved for authenticated OpenCTI provider traffic. |
| `CORROBORE_S3_ACCESS_KEY` | unset | Access-key identifier required by `export-snapshot-s3`. |
| `CORROBORE_S3_SECRET_KEY` | unset | Secret access key required by `export-snapshot-s3`; inject it through the process environment. |
| `CORROBORE_S3_SESSION_TOKEN` | unset | Optional temporary-credential session token used by `export-snapshot-s3`. |
| `CORROBORE_BUILD_REVISION` | `unknown` at compile time | Build-time source revision embedded by release automation; it is not a runtime override. |

The complete HTTP-specific behavior for licensing and domain providers remains
in the [HTTP server reference](http-server.md#configuration).

## Secret handling

Use protected files for bearer tokens, licenses, public verification material,
TLS certificates, and especially private keys. A production TOML should contain
only paths:

```toml
[server]
auth_mode = "required"
auth_token_file = "/etc/corrobore/secrets/http-token"

[tls]
enabled = true
certificate_file = "/etc/corrobore/tls/server.crt"
private_key_file = "/etc/corrobore/tls/server.key"
```

Do not commit token files, private keys, `.env`, or effective configuration
output. Non-loopback binds require TLS, required authentication, and
authenticated operational endpoints. TLS files are checked for readability,
validity, and key matching before a listener opens; token and TLS rotation takes
effect after a supervised restart.

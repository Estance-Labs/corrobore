# HTTP Server

`corrobore-http-server` is an Axum service exposing the runtime to agents and applications. It binds to `127.0.0.1:8080` by default.

This page is the transport and client contract. For the supported operator entry
point and deployment runbooks, start with
[Deployment Modes](deployment-modes.md).

Administrative database operations are available at
`POST /v1/admin/storage/snapshots`, `POST /v1/admin/storage/indexes/rebuild`, and
`GET /v1/admin/storage/operations`. They require the administrative bearer token;
see [Database operations](database-operations.md) for restore, migration, S3/MinIO,
rollback, cancellation and recovery procedures.

## Authentication and limits

Operational endpoints are public by default on loopback and can be protected
with `CORROBORE_OPERATIONAL_ENDPOINT_POLICY=authenticated`. Every `/v1/*`
route requires:

```http
Authorization: Bearer <CORROBORE_HTTP_AUTH_TOKEN>
```

Bearer values are compared in constant time. Protected routes share a global token-bucket rate limiter. Standard JSON routes and STIX import routes have separate body limits. Request tracing excludes headers so the token is not written to logs.

Every response includes `X-Request-Id` and `X-Correlation-Id`. A client may
provide a log-safe `X-Request-Id` of at most 128 characters; otherwise the
server generates a UUID. Structured request logs and JSON error envelopes carry
that same identifier.

Success responses generally use `{ "ok": true, "result": ... }`. Errors use:

```json
{ "ok": false, "correlation_id": "5bf2...", "error": { "code": "INVALID_REQUEST", "message": "..." } }
```

Application errors use the JSON envelope above. Transport middleware can reject a request before a handler runs: missing/invalid auth returns 401, rate limiting returns 429, oversized bodies return 413, and handler timeouts return 504 with code `REQUEST_TIMEOUT`.

## Configuration

| Variable | Default | Description |
| :--- | :--- | :--- |
| `CORROBORE_HTTP_AUTH_TOKEN` | required | Non-empty Bearer token for protected routes. |
| `CORROBORE_HTTP_AUTH_MODE` | `required` | Authentication policy: `required` or explicit loopback-only `local-insecure`. |
| `CORROBORE_HTTP_AUTH_TOKEN_FILE` | unset | Protected file containing the bearer token; mutually exclusive with `CORROBORE_HTTP_AUTH_TOKEN`. |
| `CORROBORE_HTTP_ADMIN_AUTH_TOKEN` | unset | Optional dedicated Bearer token for admin-only endpoints (for example `/v1/admin/license/status`). |
| `CORROBORE_HTTP_ADMIN_AUTH_TOKEN_FILE` | unset | Protected file containing the admin bearer token; mutually exclusive with `CORROBORE_HTTP_ADMIN_AUTH_TOKEN`. |
| `CORROBORE_MEMORY_WORKSPACE_ID` | `workspace--standalone-default` | Trusted workspace for high-level memory operations; clients cannot override it in JSON. |
| `CORROBORE_MEMORY_ACTOR_ID` | `actor--standalone-client` | Trusted authenticated actor attribution for high-level memory mutations and traces. |
| `CORROBORE_MEMORY_AGENT_ID` | unset | Optional trusted agent attribution for high-level memory operations. |
| `CORROBORE_MEMORY_SESSION_ID` | `session--standalone-api` | Trusted session attribution for high-level memory operations. |
| `CORROBORE_MEMORY_PERMISSIONS` | `read,write,trace,forget,consolidate` | Independently enabled high-level capabilities; omit a capability to deny it after bearer authentication. |
| `CORROBORE_HTTP_HOST` | `127.0.0.1` | Bind host. Set `0.0.0.0` deliberately for containers or remote access. |
| `CORROBORE_HTTP_PORT` | `8080` | Bind port. |
| `CORROBORE_HTTP_SESSION_STORE_DIR` | `.corrobore-runtime` | Durable session-state directory. |
| `CORROBORE_HTTP_LOG_DIR` | `<session store>/logs` | Structured JSONL log directory. |
| `CORROBORE_HTTP_REQUEST_TIMEOUT_MS` | `30000` | Query/import/export/validation timeout. |
| `CORROBORE_HTTP_SHUTDOWN_TIMEOUT_MS` | `5000` | Configured graceful-shutdown budget. |
| `CORROBORE_HTTP_SESSION_IDLE_TTL_MS` | `0` | Idle auto-stop TTL; `0` disables expiration. |
| `CORROBORE_HTTP_MAX_BODY_BYTES` | `2097152` | Standard protected-route body limit (2 MiB). |
| `CORROBORE_HTTP_IMPORT_MAX_BODY_BYTES` | `33554432` | STIX import and OpenCTI transactional-write body limit (32 MiB). |
| `CORROBORE_OPENCTI_SYNC_MAX_OPERATIONS` | `512` | Maximum mutations admitted in one OpenCTI synchronization or transactional-write batch. |
| `CORROBORE_OPENCTI_SYNC_MAX_REPLAY_IDENTITIES` | `4096` | Bounded replay identities, dead-letter diagnostics, and write-reconciliation records retained durably. |
| `CORROBORE_OPENCTI_SHADOW_REFERENCE_ENDPOINT` | unset | Fixed Knowledge Data Engine endpoint backed by the reference Elasticsearch/OpenSearch provider. |
| `CORROBORE_OPENCTI_SHADOW_REFERENCE_VERSION` | `unconfigured` | Explicit provider version retained in every comparison report. |
| `CORROBORE_OPENCTI_SHADOW_REFERENCE_AUTH_TOKEN` | unset | Optional inline reference-provider bearer token; prefer the file source. |
| `CORROBORE_OPENCTI_SHADOW_REFERENCE_AUTH_TOKEN_FILE` | unset | Protected file containing the reference-provider bearer token. |
| `CORROBORE_OPENCTI_SHADOW_RELEASE` | package version | Bounded Corrobore release label used by parity and latency metrics. |
| `CORROBORE_OPENCTI_SHADOW_SAMPLE_BASIS_POINTS` | `0` | Deterministic fallback sample rate from `0` through `10000`. |
| `CORROBORE_OPENCTI_SHADOW_MAX_CONCURRENCY` | `4` | Independent shadow and primary-write concurrency ceiling; excess work receives explicit backpressure. |
| `CORROBORE_OPENCTI_SHADOW_TIMEOUT_MS` | `2000` | Independent shadow, canonical write and reference-projection deadline. |
| `CORROBORE_OPENCTI_SHADOW_MAX_REPORTS` | `10000` | Bounded durable privacy-safe report retention. |
| `CORROBORE_OPENCTI_SHADOW_SAMPLING_POLICY_FILE` | unset | JSON rules selecting environment, operation, query class, entity, organization, tenant, cohort, and percentage. |
| `CORROBORE_OPENCTI_SHADOW_BASELINE_FILE` | unset | JSON list of exact divergence fingerprints with required owner and expiry. |
| `CORROBORE_OPENCTI_READ_ROUTING_POLICY_FILE` | unset | Validated progressive routing policy; unset defaults to reference-only. |
| `CORROBORE_OPENCTI_READ_ROUTING_MAX_AUDITS` | `10000` | Bounded durable provider-decision audit retention. |
| `CORROBORE_HTTP_RATE_LIMIT_PER_SECOND` | `50` | Sustained global protected-route rate. |
| `CORROBORE_HTTP_RATE_LIMIT_BURST` | `200` | Global burst allowance. |
| `CORROBORE_HTTP_WEB_DIR` | unset | Optional directory containing the production explorer build. Unset keeps API-only mode. |
| `CORROBORE_HTTP_LICENSE_PEM` | unset | Inline signed license PEM containing `client_uuid`, `client_email`, `modules`, `valid_until` (RFC3339), optional `tags`, and `signature`. |
| `CORROBORE_HTTP_LICENSE_PEM_FILE` | unset | Path to signed license PEM file (alternative to inline variable). |
| `CORROBORE_HTTP_LICENSE_PUBLIC_KEY_PEM` | unset | Inline Ed25519 public key PEM used to verify the license signature. |
| `CORROBORE_HTTP_LICENSE_PUBLIC_KEY_PEM_FILE` | unset | Path to Ed25519 public key PEM file (alternative to inline variable). |
| `CORROBORE_HTTP_LICENSED_MODULES` | unset | Compatibility fallback: comma-separated module claims used only when no PEM license is provided. |
| `CORROBORE_DOMAIN_PROVIDER_DIR` | unset | Trusted root containing native CTI, FIMI, and Crisis provider libraries. Must be configured with the manifest file. |
| `CORROBORE_DOMAIN_PROVIDER_MANIFEST_FILE` | unset | Strict JSON manifest pinning provider domains, relative paths, SHA-256 digests, required policy, and capabilities. Must be configured with the provider directory. |
| `CORROBORE_STORAGE_MODE` | `ephemeral` | Runtime graph storage mode (`ephemeral` or `persistent`). |
| `CORROBORE_STORAGE_DIR` | unset | Required when `CORROBORE_STORAGE_MODE=persistent`; graph storage root path. |
| `CORROBORE_STORAGE_REQUIRE_FSYNC` | `false` in `ephemeral`, `true` in `persistent` | Durability control for persistent writes. |
| `CORROBORE_STORAGE_STRICT_RECOVERY` | `false` in `ephemeral`, `true` in `persistent` | When enabled, validates all required append logs and rebuilds derived catalog metadata before readiness. |
| `CORROBORE_STORAGE_MAX_HOT_NODES` | `16384` | Maximum node payloads admitted into one persistent request projection. |
| `CORROBORE_STORAGE_MAX_HOT_RELATIONSHIPS` | `32768` | Maximum relationship payloads admitted into one persistent request projection. |
| `CORROBORE_STORAGE_MAX_WARM_ADJACENCY_ENTRIES` | `65536` | Maximum lightweight adjacency entries retained for one persistent request projection. |
| `CORROBORE_OPERATIONAL_ENDPOINT_POLICY` | `public` | `public` or `authenticated`; non-loopback binds require `authenticated`. |
| `CORROBORE_TLS_ENABLED` | `false` | Enables HTTPS. Non-loopback binds require TLS. |
| `CORROBORE_TLS_CERTIFICATE_FILE` | unset | PEM certificate chain loaded and validated at startup. |
| `CORROBORE_TLS_PRIVATE_KEY_FILE` | unset | PEM private key loaded and matched to the certificate at startup. |

TLS material and token files are re-read on restart, making process restart the
rotation boundary without any graph-data migration. Invalid, unreadable,
mismatched, expired, or not-yet-valid TLS material prevents startup. Effective
configuration, diagnostics, logs, and metrics never include token or private-key
contents.

The binary loads `.env`, supports `-v`/`-vv` verbosity, and honors `RUST_LOG` as the logging-filter override. Provider configuration is fail-fast: a missing required library, path escape, digest mismatch, incompatible ABI, invalid metadata, missing capability, creation failure, or unhealthy response prevents the HTTP listener from starting. See [the manifest example](../examples/domain-providers.json).

Persistent mode acquires an exclusive process-lifetime filesystem lock before
storage creation or recovery. Only one server can own a storage directory at a
time. Manifest incompatibility and unsafe recovery state prevent the listener
from becoming ready; see the [standalone server ownership and recovery
contract](standalone-server.md#persistent-directory-ownership-and-recovery).

## `GET /health/live`

Returns `200` whenever the HTTP event loop can answer. It deliberately does not
claim that storage or application dependencies are ready.

## `GET /health/ready`

Returns `200` with `ready: true` only after engine initialization and storage
recovery complete and while the lifecycle accepts requests. It returns `503`
before initialization and during draining, stopped, or failed states.

```json
{
  "status": "ready",
  "ready": true,
  "service": "corrobore-http-server",
  "lifecycle_state": "ready",
  "checks": {
    "engine_initialized": true,
    "storage_recovered": true,
    "accepting_requests": true
  }
}
```

## `GET /version`

Returns the crate version, source revision, build target, supported storage
versions and record formats, and the active persistent format when applicable.
The response is deterministic for a build and never includes configuration or
secrets.

## `GET /health` (deprecated)

Returns service name, crate version, lifecycle state, uptime, cumulative/recent idle-session expiration metrics, and durability diagnostics (mode controls, validated storage version and record format, WAL size/lag, checkpoint age, compaction backlog, recovery outcome). Compatibility fields are `null` in ephemeral mode. Health and metrics remain observable during draining; new non-operational requests receive `SERVICE_DRAINING`.

This compatibility endpoint includes `Deprecation: true` and a successor link
to `/health/ready`.

```json
{
  "status": "ok",
  "service": "corrobore-http-server",
  "version": "0.3.1",
  "lifecycle_state": "ready",
  "storage_mode": "ephemeral",
  "uptime_ms": 1200,
  "session_ttl_metrics": {
    "total_expired_sessions": 0,
    "expired_last_5m_sessions": 0
  },
  "domain_providers": {"configured": 3, "ready": 3},
  "durability": {
    "controls": {
      "require_fsync": false,
      "strict_recovery": false
    },
    "storage_version": null,
    "record_format": null,
    "wal_bytes": 0,
    "wal_lag_sequences": 0,
    "checkpoint_sequence": null,
    "checkpoint_age_seconds": null,
    "compaction_backlog_bytes": 0,
    "recovery": {
      "outcome": "ephemeral",
      "manifest_validated": false,
      "required_components_validated": false,
      "catalog_recovered": false,
      "adjacency_storage_recovered": false,
      "warning_count": 0,
      "derived_state_rebuilt": false
    }
  }
}
```

## `GET /metrics`

Returns Prometheus text exposition (`0.0.4`) for build, uptime, sessions,
storage, providers, lifecycle, readiness, active requests, shutdown counters,
and OpenCTI core-read request, P50/P95/P99 latency, page-in, and cache-hit
metrics grouped only by bounded query class. Storage index counts include the
payload-free `node_access` and `relationship_access` projections used to
authorize candidates before page-in.

## Bounded CLI status probe

`corrobore server status` loads the same host, port, and timeout configuration
as `server start`, then probes `/health/ready` and `/version`. Exit code `0`
means ready and compatible, `8` means unavailable or not ready, and `9` means
the operational or storage-compatibility contract is incompatible.

## `POST /v1/cypher/read`

Executes a forced read-only request.

```json
{
  "query": "MATCH (n:ThreatActor) RETURN n LIMIT 10",
  "params": {},
  "workspace_id": "workspace--demo",
  "session_id": "<started-session-uuid>",
  "budget_ref": "budget--interactive"
}
```

Only `query` is required. When a real started session id is supplied, the server transitions it through `working`, `processing`, and `idle` (or `degraded` on failure).

The response embeds the typed shared-runtime response, including `status`, `data`, mutation summary, validation errors, warnings, fix hints, budget usage, and audit references when present. `Rejected` and `ValidationFailed` are valid runtime results and can still arrive with HTTP 200; inspect the inner status.

Cypher parameters preserve homogeneous JSON arrays of strings, integers, decimals, or booleans as bounded typed lists (1 to 256 items); arrays are never flattened into JSON text. Mutation summaries distinguish `matched_rows`, `native_fields_changed`, and `property_fields_changed`, alongside created, updated, and deleted node/relationship counts. The reserved fields `confidence`, `status`, and `evidence_refs` address native graph metadata; when such a field is updated, any legacy generic property with the same name is removed and native read-back takes precedence.

## `POST /v1/cypher/write`

Executes a forced mutation request with the same body shape. Host runtime policy still controls whether mutations are permitted.

## `POST /v1/cypher/execute`

Compatibility endpoint accepting `mode: "read" | "write" | "validate" | "auto"`; default `auto` chooses by mutation keywords. Prefer explicit read/write routes for safety. Validate-only mode is affected by issue #228 and is not a mutation-safety boundary.

## `POST /v1/seed/search`

Resolves a natural-language objective into ranked graph seeds.

```json
{
  "objective": "infrastructure linked to the phishing campaign",
  "workspace_id": "workspace--demo",
  "domain_profile": "cti",
  "mode": "hybrid",
  "top_k": 5,
  "score_threshold": 0.2
}
```

Defaults are cross-domain, hybrid retrieval, `top_k=10`, and threshold `0.0`. Profiles are `cti`, `fimi`, `crisis`, or `cross_domain`; modes are `hybrid`, `full_text`, `semantic`, or `vector`. Each candidate includes `node_id`, `score`, and an explanation with rationale, source refs, and boundary notes. Expected 422 errors include `NO_SEED`, `AMBIGUOUS_SEED`, and `OVERBROAD_OBJECTIVE`.

Domain-scoped profiles (`cti`, `fimi`, `crisis`) are enterprise-gated:

- Build-time gate: profile requests return `FEATURE_NOT_AVAILABLE` if the matching enterprise feature is not compiled.
- Runtime license gate: profile requests return `LICENSE_MODULE_MISSING` if `CORROBORE_HTTP_LICENSED_MODULES` does not contain the requested module.
- Provider gate: profile requests return `DOMAIN_PROVIDER_NOT_READY` unless the matching provider is loaded and healthy.

## `POST /v1/memory/operations`

Executes the versioned, domain-neutral `remember`, `relate`, `recall`, `update`,
`forget`, `consolidate`, or `trace` contract. The request JSON is exactly the
serialized embedded `MemoryRequest`; workspace, actor, agent, session,
permissions, request identity, and correlation identity come from trusted
server configuration and middleware and are rejected if supplied in the
payload. Mutations require `idempotency_key` and return a durability-gated
receipt. See [High-level Memory Operations](memory-operations.md) for complete
semantics, limits, examples, errors, and compatibility rules.

## `POST /v1/domains/{domain}/validate`

Invokes `node.validate/1` through the common provider registry for `cti`, `fimi`, `crisis`, `medical`, and `research` after the gates that apply to the requested domain.

The enterprise domains `cti`, `fimi`, and `crisis` pass build, license, readiness, and capability gates. The MIT domains `medical` and `research` ship with the open-source runtime and pass only readiness and capability gates: they require neither an enterprise build feature nor a signed license claim, so `FEATURE_NOT_AVAILABLE` and `LICENSE_MODULE_MISSING` never apply to them. Every domain still fails closed when its provider is absent, unhealthy, or missing `node.validate`.

```json
{
  "request_id": "validation--123",
  "workspace_id": "workspace--demo",
  "snapshot_id": "snapshot--current",
  "payload": {"id": "node--123", "labels": ["ThreatActor"]}
}
```

The successful envelope preserves `request_id` and returns provider `status` (`accepted`, `rejected`, or `failed`), structured `issues`, and optional diagnostics. Stable gate errors are `INVALID_DOMAIN`, `FEATURE_NOT_AVAILABLE`, `LICENSE_MODULE_MISSING`, `DOMAIN_PROVIDER_NOT_READY`, and `DOMAIN_PROVIDER_CAPABILITY_MISSING`; invocation failures return `DOMAIN_PROVIDER_ERROR` and timeouts return `REQUEST_TIMEOUT`.

## `GET /v1/admin/domain-providers/status`

Uses the same dedicated admin Bearer boundary as the admin license route. It returns no paths, hashes, handles, or configuration secrets, only each loaded provider's `provider_id`, `provider_version`, `domain`, declared capabilities, and `ready` state.

## `GET /v1/license/status`

Returns the authenticated runtime view of enterprise licensing.

```json
{
  "ok": true,
  "result": {
    "source": "signed_pem",
    "client_uuid": "11111111-2222-4333-8444-555555555555",
    "client_email": "security@example.com",
    "valid_until": "2099-01-01T00:00:00+00:00",
    "is_nfr": true,
    "modules": ["cti", "crisis"]
  }
}
```

The runtime rejects a signed license when `valid_until` is expired. `is_nfr` is derived from the case-insensitive presence of the `nfr` tag in `tags`.

`source` is one of:

- `signed_pem`: modules and identity were loaded from a verified license PEM.
- `legacy_env`: modules were loaded from `CORROBORE_HTTP_LICENSED_MODULES` fallback.
- `none`: no active enterprise module claims.

## `GET /v1/admin/license/status`

Returns the same license summary as `/v1/license/status` but is protected by a secondary admin token configured in `CORROBORE_HTTP_ADMIN_AUTH_TOKEN`.

This endpoint is independent from the standard `/v1/*` middleware token. It validates:

- `Authorization: Bearer <CORROBORE_HTTP_ADMIN_AUTH_TOKEN>`

Error behavior:

- `401 AUTH_REQUIRED`: missing `Authorization` header.
- `401 AUTH_INVALID`: invalid admin token.
- `403 ADMIN_AUTH_NOT_CONFIGURED`: server has no `CORROBORE_HTTP_ADMIN_AUTH_TOKEN` configured.

## `POST /v1/admin/storage/snapshots`

Creates a consistent online canonical snapshot through the administrative
Bearer boundary. See [Database operations](database-operations.md) for request,
restore, retention, and object-store rules.

## `POST /v1/admin/storage/indexes/rebuild`

Rebuilds selected provider-owned indexes from canonical state through the
administrative Bearer boundary. Progress and terminal outcome are visible in
the database-operations status.

## `GET /v1/admin/storage/operations`

Returns bounded status for online snapshot, restore, migration, compaction and
index-rebuild operations without exposing credentials or record payloads.

## `POST /v1/import/stix`

Imports a STIX 2.1 bundle as one atomic typed graph mutation. Plain bundles
remain lossless and candidate-only. A STIX `confidence` value is normalized
from `0..=100` to native `0..=1` (for example, `50` becomes `0.5`), but it is
not evidence.

```json
{
  "bundle": {
    "type": "bundle",
    "objects": [{"type": "identity", "id": "identity--demo", "name": "Demo"}]
  },
  "workspace_id": "workspace--demo",
  "session_id": "session--import",
  "budget_ref": "budget--import"
}
```

The result keeps the compatibility counters `processed_objects`,
`applied_mutations`, `rejected_mutations`, and `errors`, and adds a canonical
`outcomes` list plus fixed-cardinality `metrics`. Each requested STIX ID is
classified as `created`, `updated`, `duplicate`, `rejected`,
`unresolved_reference`, or `failed`. Only `created` and `updated` count as
applied mutations.

The whole bundle is preflighted and committed atomically. Node-like records are
resolved before relationships regardless of input order, and relationship
endpoints may refer to either the same bundle or the existing canonical graph.
If any endpoint is missing, the relationship names the missing reference as
`unresolved_reference`, the remaining records are `rejected` with
`ATOMIC_IMPORT_ABORTED`, and nothing is committed. Conflicting payloads under
the same STIX ID fail with `CONFLICTING_STIX_ID` before mutation.

Only `source_ref` and `target_ref` are authoritative graph endpoints. Other
STIX references such as `created_by_ref`, `object_marking_refs`, report
`object_refs`, and future reference arrays remain losslessly available in
`opencti.raw` and in the adapter's typed properties; they may intentionally
refer to records outside the imported graph and are not silently discarded.

Extraction agents can add the optional versioned `evidence` envelope. Evidence
IDs are caller-owned and stable; every annotation key must be a STIX ID in the
same bundle. Only `candidate` may be requested. Workspace, session, actor,
permissions and export authority remain controlled by the authenticated runtime
boundary, never by fields inside the bundle.

```json
{
  "bundle": {
    "type": "bundle",
    "objects": [{
      "type": "threat-actor",
      "id": "threat-actor--demo",
      "name": "Grounded candidate"
    }]
  },
  "evidence": {
    "schema_version": "1.0",
    "records": [{
      "id": "evidence--report-p7-p2",
      "source_id": "document--report",
      "content_sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      "payload": "Exact supporting excerpt",
      "locator": {"type": "paragraph", "page": 7, "paragraph": 2}
    }],
    "annotations": {
      "threat-actor--demo": {
        "evidence_refs": ["evidence--report-p7-p2"],
        "confidence": 50,
        "status": "candidate"
      }
    }
  },
  "workspace_id": "workspace--demo",
  "session_id": "session--import",
  "budget_ref": "budget--import"
}
```

The complete raw STIX object remains available in `opencti.raw`, while scalar,
homogeneous string lists and nested values are stored as graph-native typed
properties. Missing evidence, conflicting evidence IDs, invalid locators,
out-of-range confidence and authoritative statuses fail before any state is
committed.

## `POST /v1/import/stix/file`

Multipart import for plain candidate-only bundles. `file` is required and its
filename must end in `.json` or `.stix`. Optional text parts are `workspace_id`,
`session_id`, and `budget_ref`. Use the JSON endpoint when attaching the
versioned evidence envelope.

```bash
curl -X POST http://127.0.0.1:8080/v1/import/stix/file \
  -H 'Authorization: Bearer change-me' \
  -F 'file=@bundle.stix;type=application/json' \
  -F 'workspace_id=workspace--demo'
```

## `POST /v1/opencti/sync/batches`

Accepts one ordered, bounded OpenCTI snapshot, catch-up, or steady-state batch.
Persistent WAL storage is required. The server maps the lossless records,
applies the contiguous non-retryable prefix in one canonical graph transaction,
and only then fsyncs the source checkpoint. Replaying a batch after a crash is
idempotent.

Each operation reports `applied`, `duplicate`, `retryable`,
`permanently_rejected`, or `quarantined`. A retryable sequence stops checkpoint
progress and applies backpressure to later operations. Permanent rejections and
conflicting replay identities are retained as bounded dead-letter diagnostics
in the durable checkpoint. When an expected digest is supplied, record,
property, identifier, relationship, access-policy, and projection checks must
all match before `shadow_reads_enabled` becomes true.

## `GET /v1/opencti/sync/status`

Returns the restored phase, acknowledged sequence, source high-water mark, lag,
queue depth, retry/rejection/quarantine counters, divergence status, and
shadow-read gate. The same lag, queue, retry, rejection, checkpoint, and gate
values are exported on `/metrics`.

## `POST /v1/opencti/writes`

Executes a create, update, delete, relationship/access-policy mutation, merge,
or ordered bulk through the versioned Knowledge Data Engine contract. Corrobore
is committed first and its response is authoritative. Elasticsearch/OpenSearch
is updated afterward through the durable ordered outbox; an outage retains lag
without losing or rejecting the accepted canonical write.

A non-empty `context.idempotency_key` is mandatory. Persistent Corrobore
acknowledgement occurs only after WAL intent, canonical records, adjacency,
payload-free audit and the applied marker are durable. Replays return the
original outcome. `expected_revision` protects updates and deletes from lost
writes; atomic and partial bulk policies retain deterministic per-item order.
See [OpenCTI transactional writes](../developer-guide/opencti-transactional-writes.md)
for the request example, recovery rules and configured bounds.

## `POST /v1/opencti/files`

Enqueues durable file-content extraction, or deletes existing file-content
projections, against the canonical store. The body is one of two variants:

- `enqueue` takes a `descriptor` carrying the canonical file identity,
  provenance, digest and access metadata for one immutable object-storage
  version. The response is `202` with a deterministic `job_id`; a descriptor
  already queued returns `result: "duplicate"` instead of `"enqueued"` rather
  than queueing the work twice.
- `delete` takes a non-empty `file_ids` array and removes those projections
  synchronously, answering `200` with `result: "deleted"`.

Responses carry no file content and no authorization metadata. The route
requires persistent canonical storage and returns `503` under an ephemeral
store. See [OpenCTI file content search](../developer-guide/opencti-file-content-search.md)
for extraction and search behavior.

## `GET /v1/opencti/writes/status`

Returns write counters, authority, outbox depth/lag/retries/quarantine,
reconstruction count, ordered projection records and WAL-bound audit receipts.
Original idempotency keys and tokens are excluded. During projection lag,
`read_your_writes` requests are served from Corrobore.

Operators use the admin-token routes below after an outage or rollback trigger.
See [OpenCTI transactional writes](../developer-guide/opencti-transactional-writes.md)
for the complete rollback runbook.

## `POST /v1/admin/opencti/projection/drain`

Retries pending entries in global sequence order and stops at the first
retryable reference failure. Exact canonical/reference outcomes are required
before an entry becomes delivered.

## `POST /v1/admin/opencti/reconstruction`

Returns every canonical OpenCTI record losslessly and deterministically with
the captured outbox high-water sequence for a clean reference rebuild.

## `POST /v1/admin/opencti/authority/suspend`

Immediately and durably suspends new mutations for one declared rollback
trigger.

## `POST /v1/admin/opencti/authority`

Assigns exclusive write authority only after the required reference-health,
replay-completion and parity-verification gates pass.

## `POST /v1/opencti/reconciliation`

Compares a bounded reference snapshot with persistent canonical data. `dry_run`
returns and persists the exact missing, extra, property, relationship,
permission, and stale-index plan without mutation. `repair` applies safe
targeted changes in one WAL transaction, rebuilds required projections, and
verifies parity. Unsafe category conflicts and unapproved deletions are
quarantined. See [OpenCTI merge and targeted reconciliation](../developer-guide/opencti-merge-reconciliation.md).

## `GET /v1/opencti/reconciliation/status`

Returns bounded payload-free reports plus retained, quarantined, and
parity-verified command counts. Reports survive restart; quarantined reports are
retained until operator action.

## `POST /v1/opencti/shadow/reads`

Forwards one supported typed Knowledge Data Engine read to the configured
Elasticsearch/OpenSearch reference endpoint and returns that response envelope
unchanged. When synchronization parity, deterministic sampling, and the
concurrency budget all permit it, the same request runs asynchronously against
Corrobore. Shadow success, failure, timeout, or shedding never delays or alters
the reference response.

The body contains `request` and non-sensitive `metadata`. A single correlation
ID links the request, both executions, durable report, and metrics. Persistent
storage is mandatory.

## `GET /v1/opencti/shadow/reports`

Returns newest-first privacy-safe reports, optionally filtered by
`query_class`, `release`, and a bounded `limit`. Reports contain provider
versions, both latencies, ID-set, significant-property, ordering, cursor,
aggregation, relationship, permission, error, and performance dimensions.
Record identities are SHA-256 evidence handles; property values and remote
error messages are never persisted.

`/metrics` exports comparison and equivalent totals, blocking security
divergences, and cumulative latency histograms using only `query_class`,
`release`, and the bounded `provider` dimension.

## `POST /v1/opencti/reads`

Executes one supported Knowledge Data Engine read through the progressive
routing policy. Modes are `reference_only`, `shadow`, `canary`, `graph_reads`,
and `primary_reads`. Canary rules use first-match semantics and can select the
environment, operation, query class, entity type, organization, tenant, cohort,
feature flag, and a deterministic percentage. Exactly one provider owns the
visible response; independently bounded shadow work can only create parity
evidence.

Session IDs bind pagination to one provider and index generation. A provider or
generation change fails explicitly. Synchronization, reference freshness,
availability, corruption, parity, security, error-rate, and P95 latency gates
open the durable circuit breaker and restore subsequent traffic to the
reference provider. If the reference is not fresh, routing fails closed.

## `GET /v1/opencti/routing/decisions`

Returns newest-first provider decisions or the decision for an exact
`correlation_id`. Evidence includes only query class, provider, policy version,
decision reason, timestamp, and correlation ID; access context and request
payload are never retained. `/metrics` exports provider decisions by bounded
query class and provider plus the circuit-breaker state.

## `POST /v1/opencti/routing/rollback`

Opens the durable operator circuit breaker in one authenticated call. New
eligible reads route to the validated reference provider without configuration
rewrites. Existing incompatible pagination sessions fail explicitly rather
than crossing provider or index generations.

## `POST /v1/stix/validate`

Validates either an explicit bundle (default) or current graph CTI nodes.

```json
{
  "source": "bundle",
  "bundle": {"type": "bundle", "objects": []},
  "workspace_id": "workspace--demo",
  "snapshot_id": "snapshot--current"
}
```

The result contains `source_mode`, `valid`, `issues`, `playbooks_applied`, optional `corrections_summary`, optional import `persistence`, and `errors`. Bundle playbooks cover missing `identity.name`, `malware.is_family`, and required temporal fields for indicators, reports, and observed data. Graph mode reports readiness issues and does not auto-mutate nodes.

Graph-native CTI validation has two explicit gates:

- Build-time gate: when the server is compiled without enterprise CTI support, `source=graph` returns a forbidden error.
- Runtime license gate: when enterprise CTI is compiled but `CORROBORE_HTTP_LICENSED_MODULES` does not contain `cti`, `source=graph` returns a forbidden error.
- Provider gate: graph mode requires a ready CTI provider exposing `node.validate/1`; availability does not depend on whether the graph is empty.

```json
{
  "ok": true,
  "result": {
    "source_mode": "bundle",
    "valid": false,
    "issues": [
      {
        "code": "STIX_IDENTITY_NAME_REQUIRED",
        "message": "identity object requires 'name'",
        "field": "name",
        "severity": "error",
        "node_id": "identity--abc"
      }
    ],
    "playbooks_applied": [
      {
        "id": "PLAYBOOK_FIX_IDENTITY_NAME",
        "description": "fill missing identity.name with placeholder",
        "node_id": "identity--abc"
      }
    ],
    "corrections_summary": {
      "total_corrections": 1,
      "by_field": {"name": 1},
      "by_strategy": {"playbook_default": 1},
      "by_playbook_id": {"PLAYBOOK_FIX_IDENTITY_NAME": 1}
    },
    "persistence": {
      "processed_objects": 1,
      "applied_mutations": 1,
      "rejected_mutations": 0,
      "errors": []
    },
    "errors": []
  }
}
```

`valid` is computed from issues found during the pass; it is not a post-fix revalidation. Corrected objects are imported when `playbooks_applied` is non-empty, so `valid: false` and non-null `persistence` can legitimately coexist. Revalidate if a post-correction verdict is required.

| Error code | HTTP | Meaning |
| :--- | :---: | :--- |
| `MISSING_BUNDLE` | 400 | `source=bundle` without a bundle. |
| `INVALID_STIX_BUNDLE` | 400 | The payload is not a STIX bundle object. |
| `INVALID_SOURCE_MODE` | 400 | Unknown source value. |
| `FEATURE_NOT_AVAILABLE` | 403 | `source=graph` requested when enterprise CTI support is not compiled in. |
| `LICENSE_MODULE_MISSING` | 403 | `source=graph` requested without a valid `cti` runtime license claim. |

## `GET /v1/export/stix`

Exports a raw STIX bundle (not the standard envelope). Query parameters:

| Parameter | Default |
| :--- | :--- |
| `snapshot_id` | `snapshot--current` |
| `transaction_id` | `transaction--http-export` |
| `exporter_version` | `corrobore-http-server-v0` |
| `mode` | `strict` (`permissive` is also accepted) |
| `profile` | `stix-mvp` |

The route exports only eligible CTI records. Imported OpenCTI objects and
relationships preserve their original STIX identity and fields; unrelated
memory and receipt nodes are excluded. Every exported record must carry native
confidence and retained evidence. Relationship endpoints always reference the
actual exported object identifiers.

Strict mode returns `EXPORT_PLAN_FAILED` (HTTP 400) with named readiness,
identity, evidence, provider-validation, or endpoint issue codes. Permissive
mode omits failing records and returns bounded machine-readable details in
`export_diagnostics.exclusions`. Retained evidence referenced by exported
objects is included in `x_corrobore_evidence`.

| Error code | HTTP | Meaning |
| :--- | :---: | :--- |
| `FEATURE_NOT_AVAILABLE` | 403 | Enterprise CTI support is not compiled in. |
| `LICENSE_MODULE_MISSING` | 403 | The runtime license does not enable `cti`. |
| `DOMAIN_PROVIDER_NOT_READY` | 503 | No loaded, healthy CTI provider is available. |
| `DOMAIN_PROVIDER_CAPABILITY_MISSING` | 503 | The CTI provider does not expose `node.validate/v1`. |
| `EXPORT_PLAN_FAILED` | 400 | Strict CTI readiness or identity checks rejected the export; the message contains named issue codes. |

## `POST /v1/sessions/start`

```json
{
  "workspace_id": "workspace--demo",
  "actor_id": "actor--agent-01",
  "actor_kind": "Agent",
  "metadata": {"source": "report.pdf"}
}
```

`actor_kind` defaults to `agent` and accepts `user`, `agent`, `orchestrator_agent`, `worker_agent`, `tool`, `system`, or `test_fixture` (hyphenated and compact aliases are also accepted for compound values). The response contains a server-generated UUID and `idle` status.

```json
{
  "ok": true,
  "result": {
    "session_id": "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx",
    "status": "idle"
  }
}
```

## Explorer read API

The initial 3D explorer uses three authenticated, read-only routes. They share
the standard request body and rate-limit policies even though the routes accept
only path and query parameters.

### `GET /v1/explorer/sessions`

Returns sessions in stable identifier order for the explorer's left rail.
Stopped sessions are excluded by default; pass `include_stopped=true` to include
them. Each record contains session, workspace and actor identity, actor kind,
status, and start/update times in epoch milliseconds.

### `GET /v1/explorer/sessions/{session_id}/timeline`

Returns the selected session's persisted snapshot/timeshot forest. Every node
contains a stable boundary id, `snapshot` or `timeshot` kind, optional parent and
transaction ids, an RFC 3339 timestamp, a label, and deterministically ordered
children. Snapshots are derived from authoritative graph-core snapshot records;
timeshots are read-only analysis boundaries and do not create graph branches.

### `GET /v1/explorer/sessions/{session_id}/graph`

Returns the bounded deterministic visualization projection. `boundary_kind`
defaults to `current`; `snapshot` and `timeshot` require `boundary_id`. Optional
budgets are `max_nodes`, `max_relationships`, `max_properties_per_record`,
`max_payload_bytes`, and `max_computation_units`.

Unknown sessions return `SESSION_NOT_FOUND`. Unknown, wrong-kind, and
cross-session boundaries all return the leak-safe `TEMPORAL_BOUNDARY_NOT_FOUND`.
Invalid selections return `INVALID_TEMPORAL_BOUNDARY`; invalid budgets return
`INVALID_VISUALIZATION_PROJECTION`.

## Explorer frontend split

The browser explorer is maintained in a dedicated repository:
`Estance-Labs/corrobore-web`.

This repository documents and validates the HTTP backend contract only. The
frontend consumes these backend routes:

- `GET /v1/explorer/sessions`
- `GET /v1/explorer/sessions/{session_id}/timeline`
- `GET /v1/explorer/sessions/{session_id}/graph`

### API stack with Docker Compose

The repository Compose stack runs the Rust standalone service only, persists
session and graph state in named volumes, and waits for authenticated
`GET /health/ready` over HTTPS to pass. Docker Compose v2
with support for `docker compose up --wait` is required.

From the repository root:

```bash
cp .env.sample .env
```

Edit `.env` and replace `CORROBORE_HTTP_AUTH_TOKEN=change-me` with a non-empty local
token. Generate or install the TLS certificate and private key referenced by
`CORROBORE_TLS_CERTIFICATE_SOURCE` and `CORROBORE_TLS_PRIVATE_KEY_SOURCE`. Do
not add surrounding whitespace and never commit `.env` or private keys.
Validate the resolved configuration before starting containers:

```bash
docker compose config
```

Then build, start, and wait for the service healthcheck:

```bash
docker compose up --build --wait
```

Connect to <https://localhost:8080>. The port is published on `127.0.0.1` by
default, so the stack is not exposed to the local network. Local self-signed
certificates require explicit client trust. Change `CORROBORE_HTTP_PORT` in
`.env` when port 8080 is unavailable, then use the matching localhost port.

Inspect status and follow logs with:

```bash
docker compose ps
docker compose logs -f corrobore-http-server
```

Restart or rebuild after local changes:

```bash
docker compose restart corrobore-http-server
docker compose up --build --wait
```

Changing `CORROBORE_HTTP_AUTH_TOKEN` requires the following command so the
server receives the new value:

```bash
docker compose up --force-recreate --wait
```

Stop the stack while preserving sessions and logs:

```bash
docker compose down
```

To remove all persisted sessions and logs, explicitly remove the named
volumes:

```bash
docker compose down --volumes
```

The external frontend repository owns browser-specific build, end-to-end, and
accessibility validation.

## `GET /v1/sessions/{session_id}/health`

Returns workspace, actor identity/kind, FSM status (`idle`, `working`, `processing`, `degraded`, `stopped`), start/update time, uptime, and optional `idle_ttl_expired` stop reason.

Unknown session ids return `SESSION_NOT_FOUND` with HTTP 404. Invalid transitions return `INVALID_STATUS_TRANSITION` with HTTP 400.

## `GET /v1/sessions/{session_id}/logs`

Reads structured entries for a known session. Query parameters:

- `limit`: 1–5000, default 500;
- `from_ms`, `to_ms`: inclusive epoch-millisecond bounds;
- `format`: `json` (default) or `ndjson`.

JSON output includes matched counts, stop reason, entries, the log path, and audit parity (`input_events`, `output_events`, missing/orphan event ids, `parity_ok`). NDJSON returns the matching raw log lines.

## `POST /v1/sessions/{session_id}/stop`

Persists the session in `stopped` state and returns its id, status, and update time.

The machine-readable definitions are in the [OpenAPI specification](../api/openapi.yaml).

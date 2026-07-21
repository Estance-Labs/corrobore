# TAXII Ingestion

`corrobore-ingest` polls one TAXII 2.1 collection, follows envelope pagination, imports fetched STIX objects through `POST /v1/import/stix`, and persists the collection cursor. This keeps connector logic outside the graph core and preserves the same validation and audit path as other HTTP imports.

This guide targets the current `0.1.x` runtime baseline.

## Deployment topology (multi-repo)

Corrobore uses a boundary-first model:

- the runtime (`corrobore-http-server`) is the core surface of this repository;
- connectors are integration surfaces that call the public HTTP API;
- integration assets can evolve in dedicated repositories without changing core runtime packaging.

In practice, this means ingestion remains an HTTP client of Corrobore, not an in-process extension of graph internals. The same import validations, auth controls, and audit trails apply whether data comes from manual imports or scheduled TAXII polling.

This repository still ships `corrobore-ingest` as the reference TAXII connector. Some integrations (for example XTM One assets) have moved to dedicated repositories; they should keep their own environment files and release cadence while targeting the same HTTP contracts.

## Configuration strategy

Use separate environment scopes per runtime component:

- Corrobore runtime env (server auth, limits, host/port) belongs to the runtime deployment.
- Connector env (TAXII source, polling, connector state) belongs to the connector deployment.

For local development from this workspace:

- `.env.sample` is intentionally runtime-first.
- Connector variables are documented here and can be stored in a connector-local env file (for example `.env.ingest.local`) instead of mixing them into the runtime `.env`.

This separation avoids coupling unrelated knobs, keeps secret handling scoped by component, and maps directly to multi-repo operations where runtime and connectors are released independently.

## Configuration

| Variable | Required | Default | Meaning |
| :--- | :---: | :--- | :--- |
| `CORROBORE_INGEST_TAXII_ROOT_URL` | yes | — | TAXII API root, without the collection path. |
| `CORROBORE_INGEST_TAXII_COLLECTION_ID` | yes | — | Collection to poll. |
| `CORROBORE_INGEST_CORROBORE_BASE_URL` | yes | — | Corrobore server base URL. |
| `CORROBORE_INGEST_CORROBORE_AUTH_TOKEN` | yes | — | Bearer token for Corrobore imports. |
| `CORROBORE_INGEST_TAXII_TOKEN` | no | — | TAXII Bearer token. |
| `CORROBORE_INGEST_TAXII_USERNAME` / `CORROBORE_INGEST_TAXII_PASSWORD` | no | — | TAXII Basic credentials; supply both. |
| `CORROBORE_INGEST_WORKSPACE_ID` | no | `workspace--ingest-taxii` | Workspace attached to imports. |
| `CORROBORE_INGEST_POLL_INTERVAL_MS` | no | `300000` | Delay between cycles. |
| `CORROBORE_INGEST_PAGE_LIMIT` | no | `100` | Requested TAXII page size. |
| `CORROBORE_INGEST_STATE_DIR` | no | `.corrobore-runtime/ingest` | Persisted cursor directory. |

Bearer and Basic TAXII credentials are mutually exclusive. No TAXII credentials means unauthenticated collection access.

## Before you run

- Ensure `corrobore-http-server` is reachable from the connector process.
- Ensure `CORROBORE_INGEST_CORROBORE_AUTH_TOKEN` matches the runtime Bearer token.
- Ensure `CORROBORE_INGEST_STATE_DIR` points to writable storage for cursor
	durability.
- Prefer a dedicated connector env file so ingestion credentials and polling
	settings are not co-mingled with runtime deployment variables.

## Run one cycle

Start `corrobore-http-server`, load connector variables, then:

```bash
set -a
source .env.ingest.local
set +a
```

Then run one deterministic cycle:

```bash
cargo run -p corrobore-ingest -- --once
```

Remove `--once` to poll until Ctrl+C:

```bash
cargo run -p corrobore-ingest --release
```

Use `--once` for deterministic CI/debug cycles and continuous mode for
production-like polling.

The connector sends `added_after` from the persisted cursor, follows `more`/`next` pagination, and advances the cursor only after Corrobore accepts the import. A failed import leaves the previous cursor in place, giving at-least-once delivery on retry. Empty TAXII envelopes do not call the import endpoint.

## Operational notes

- The connector caps a single cycle at 1,000 TAXII pages to prevent a malformed feed from looping forever.
- The cursor is isolated per collection id.
- Transient cycle failures are logged and retried on the next interval in loop mode.
- The Corrobore HTTP service and its auth token must be reachable from the connector process.

## Failure behavior summary

- Import failure: cursor is not advanced, so objects are retried later
	(at-least-once semantics).
- Empty TAXII page: no import request is emitted.
- Pagination loop anomalies: cycle hard-capped at 1,000 pages.

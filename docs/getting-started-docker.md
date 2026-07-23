# Getting Started with Docker

This guide covers running Corrobore as a container — no Rust toolchain required.

## Deployment modes

Corrobore supports three practical startup modes:

1. HTTP runtime only (default Compose mode).
2. HTTP runtime + TAXII connector (`corrobore-ingest`) through a Compose profile.
3. Embedded engine (in-process Rust library), which is not a Docker Compose service mode.

Use Docker for modes 1 and 2. Use the embedded mode from your Rust application
as documented in [Getting Started](getting-started.md#use-corrobore-in-process).

## Prerequisites

- [Docker](https://docs.docker.com/get-docker/) 24 or later
- `curl` and `jq` for the shell examples

## Quick start (ephemeral)

The fastest way to run Corrobore locally. Data is lost when the container stops.

```bash
docker run --rm \
  -e CORROBORE_HTTP_AUTH_TOKEN=change-me \
  -p 127.0.0.1:8080:8080 \
  ghcr.io/noetance-labs/corrobore:latest
```

Verify the server is up:

```bash
curl http://127.0.0.1:8080/health/ready
```

## Persistent setup with Docker Compose

Use the provided `docker-compose.yml` for a setup that survives restarts.

**1. Copy the sample environment file and set a token:**

```bash
cp .env.sample .env
# open .env and replace the placeholder token with a real secret
```

**2. Start the stack:**

```bash
docker compose up --build --wait
```

The `--wait` flag blocks until the health check passes. Session state and logs are persisted in the `corrobore-data` volume.

**3. Stop and remove containers (data is kept in the volume):**

```bash
docker compose down
```

To also remove persisted data:

```bash
docker compose down -v
```

## Environment variables

The most common variables to override:

| Variable | Default | Description |
| :--- | :--- | :--- |
| `CORROBORE_HTTP_AUTH_TOKEN` | **required** | Bearer token for all protected routes. |
| `CORROBORE_HTTP_PORT` | `8080` | Published port. |
| `CORROBORE_STORAGE_MODE` | `ephemeral` | Set to `persistent` to write the graph to disk. |
| `CORROBORE_STORAGE_DIR` | `/graph-data` in Compose | Graph directory when `persistent` mode is enabled. |
| `CORROBORE_INGEST_TAXII_ROOT_URL` | unset | Required when TAXII profile is enabled. |
| `CORROBORE_INGEST_TAXII_COLLECTION_ID` | unset | Required when TAXII profile is enabled. |
| `CORROBORE_INGEST_CORROBORE_BASE_URL` | `http://corrobore-http-server:8080` in Compose | Corrobore target URL for connector imports. |
| `CORROBORE_INGEST_CORROBORE_AUTH_TOKEN` | defaults to `CORROBORE_HTTP_AUTH_TOKEN` in Compose | Auth token used by the connector toward Corrobore. |

See the [HTTP Server reference](user-guide/http-server.md#configuration) for the full list.

## Compose startup examples

### Mode 1: HTTP only (default)

```bash
docker compose up --build --wait
```

### Mode 2: HTTP with persistent graph storage

```bash
CORROBORE_STORAGE_MODE=persistent \
docker compose up --build --wait
```

The Compose file mounts `corrobore-graph-data` at `/graph-data` by default.

### Mode 3: HTTP + TAXII connector

Set the required TAXII variables, then enable the `taxii` profile:

```bash
export CORROBORE_INGEST_TAXII_ROOT_URL=https://taxii.example.org/api/v1
export CORROBORE_INGEST_TAXII_COLLECTION_ID=collection-id

docker compose --profile taxii up --build --wait
```

The connector waits for the HTTP runtime health check before starting.

## First query

Once the container is running:

```bash
# Health check (no token required)
curl http://127.0.0.1:8080/health

# Write a node
curl -X POST http://127.0.0.1:8080/v1/cypher/write \
  -H 'Authorization: Bearer change-me' \
  -H 'Content-Type: application/json' \
  -d '{"query":"MERGE (n:Indicator {name: \"phishing.example\"}) RETURN n"}'

# Read it back
curl -X POST http://127.0.0.1:8080/v1/cypher/read \
  -H 'Authorization: Bearer change-me' \
  -H 'Content-Type: application/json' \
  -d '{"query":"MATCH (n:Indicator) RETURN n LIMIT 10"}'
```

## Persistent graph storage

To keep the graph between container restarts, set the storage mode and mount a volume:

```yaml title="docker-compose.override.yml"
services:
  corrobore-http-server:
    environment:
      CORROBORE_STORAGE_MODE: persistent
      CORROBORE_STORAGE_DIR: /graph-data
    volumes:
      - corrobore-graph:/graph-data

volumes:
  corrobore-graph:
```

```bash
docker compose -f docker-compose.yml -f docker-compose.override.yml up --build --wait
```

This override remains useful when you want to customize storage location or
volume naming beyond the default Compose contract.

## Session workflow

Sessions group writes and reads under a single audit trail.

```bash
# Open a session
SESSION_ID=$(curl -s -X POST http://127.0.0.1:8080/v1/sessions/start \
  -H 'Authorization: Bearer change-me' \
  -H 'Content-Type: application/json' \
  -d '{"workspace_id":"ws-1","actor_id":"actor-1","actor_kind":"agent"}' \
  | jq -r '.result.session_id')

# Write through the session
curl -X POST http://127.0.0.1:8080/v1/cypher/write \
  -H 'Authorization: Bearer change-me' \
  -H 'Content-Type: application/json' \
  -d "{\"query\":\"MERGE (n:Indicator {name: 'beacon.example'}) RETURN n\",\"session_id\":\"${SESSION_ID}\"}"

# Inspect audit logs
curl "http://127.0.0.1:8080/v1/sessions/${SESSION_ID}/logs?limit=50" \
  -H 'Authorization: Bearer change-me'

# Close the session
curl -X POST "http://127.0.0.1:8080/v1/sessions/${SESSION_ID}/stop" \
  -H 'Authorization: Bearer change-me'
```

## Next steps

- [HTTP Server reference](user-guide/http-server.md) — full route catalogue and configuration.
- [TAXII Ingestion](user-guide/ingestion.md) — connector variables and polling behavior.
- [Cypher support](user-guide/cypher.md) — which Cypher features are available.
- [Intelligence Domains](user-guide/domains.md) — built-in node schemas for CTI, FIMI, and Crisis.

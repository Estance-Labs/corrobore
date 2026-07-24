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

## Quick start

Generate a local TLS certificate and token file:

```bash
mkdir -p .corrobore-tls .corrobore-secrets
printf '%s\n' 'change-me' > .corrobore-secrets/http-token
openssl req -x509 -newkey rsa:2048 -sha256 -nodes \
  -keyout .corrobore-tls/server.key \
  -out .corrobore-tls/server.crt \
  -days 30 -subj '/CN=localhost' \
  -addext 'subjectAltName=DNS:localhost,IP:127.0.0.1'
docker run --rm --name corrobore \
  -p 127.0.0.1:8080:8080 \
  -v corrobore-data:/data \
  -v "$PWD/.corrobore-secrets/http-token:/run/secrets/corrobore-http-token:ro" \
  -v "$PWD/.corrobore-tls/server.crt:/run/secrets/tls.crt:ro" \
  -v "$PWD/.corrobore-tls/server.key:/run/secrets/tls.key:ro" \
  ghcr.io/noetance-labs/corrobore:latest
```

Verify the server is up:

```bash
curl --insecure https://127.0.0.1:8080/health/ready \
  -H 'Authorization: Bearer change-me'
```

## Persistent setup with Docker Compose

Use the provided `docker-compose.yml` for a setup that survives restarts.

**1. Copy the sample environment file and set a token:**

```bash
cp .env.sample .env
# Replace the token and generate the TLS files referenced by the sample.
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
| `CORROBORE_TLS_CERTIFICATE_SOURCE` | `.corrobore-tls/server.crt` | Host certificate mounted read-only into the container. |
| `CORROBORE_TLS_PRIVATE_KEY_SOURCE` | `.corrobore-tls/server.key` | Host private key mounted read-only into the container. |
| `CORROBORE_STORAGE_MODE` | `persistent` | Set to `ephemeral` only for disposable graph state. |
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
# Health check
curl --insecure https://127.0.0.1:8080/health \
  -H 'Authorization: Bearer change-me'

# Write a node
curl --insecure -X POST https://127.0.0.1:8080/v1/cypher/write \
  -H 'Authorization: Bearer change-me' \
  -H 'Content-Type: application/json' \
  -d '{"query":"MERGE (n:Indicator {name: \"phishing.example\"}) RETURN n"}'

# Read it back
curl --insecure -X POST https://127.0.0.1:8080/v1/cypher/read \
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
SESSION_ID=$(curl --insecure -s -X POST https://127.0.0.1:8080/v1/sessions/start \
  -H 'Authorization: Bearer change-me' \
  -H 'Content-Type: application/json' \
  -d '{"workspace_id":"ws-1","actor_id":"actor-1","actor_kind":"agent"}' \
  | jq -r '.result.session_id')

# Write through the session
curl --insecure -X POST https://127.0.0.1:8080/v1/cypher/write \
  -H 'Authorization: Bearer change-me' \
  -H 'Content-Type: application/json' \
  -d "{\"query\":\"MERGE (n:Indicator {name: 'beacon.example'}) RETURN n\",\"session_id\":\"${SESSION_ID}\"}"

# Inspect audit logs
curl --insecure "https://127.0.0.1:8080/v1/sessions/${SESSION_ID}/logs?limit=50" \
  -H 'Authorization: Bearer change-me'

# Close the session
curl --insecure -X POST "https://127.0.0.1:8080/v1/sessions/${SESSION_ID}/stop" \
  -H 'Authorization: Bearer change-me'
```

## Next steps

- [HTTP Server reference](user-guide/http-server.md) — full route catalogue and configuration.
- [TAXII Ingestion](user-guide/ingestion.md) — connector variables and polling behavior.
- [Cypher support](user-guide/cypher.md) — which Cypher features are available.
- [Intelligence Domains](user-guide/domains.md) — built-in node schemas for CTI, FIMI, and Crisis.

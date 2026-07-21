# Getting Started with Docker

This guide covers running Corrobore as a container — no Rust toolchain required.

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
curl http://127.0.0.1:8080/health
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
| `CORROBORE_STORAGE_DIR` | unset | Required when `persistent`; directory inside the container (mount a volume here). |

See the [HTTP Server reference](user-guide/http-server.md#configuration) for the full list.

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
- [Cypher support](user-guide/cypher.md) — which Cypher features are available.
- [Intelligence Domains](user-guide/domains.md) — built-in node schemas for CTI, FIMI, and Crisis.

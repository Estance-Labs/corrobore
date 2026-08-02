# Getting Started

This guide targets the current `0.3.x` release.

## Startup mode overview

Pick the mode that matches your deployment target:

| Mode | What runs | Best for | How to start |
|---|---|---|---|
| HTTP (ephemeral) | `corrobore-http-server` only | Local API development and quick checks | `docker compose up --build --wait` |
| HTTP (persistent) | `corrobore-http-server` + persistent graph volume | Durable local state across restarts | `CORROBORE_STORAGE_MODE=persistent docker compose up --build --wait` |
| HTTP + TAXII | `corrobore-http-server` + `corrobore-ingest` | Connector-driven ingestion workflows | `docker compose --profile taxii up --build --wait` |
| Embedded | In-process `corrobore_engine` library | Rust applications embedding Corrobore directly | See [Use Corrobore in process](#use-corrobore-in-process) |

The HTTP rows are service topologies: the supported standalone
`corrobore server start` process exposes the HTTP API. See
[Deployment Modes](user-guide/deployment-modes.md) for the distinction between
the interface and its operational host.

For Docker-specific examples and environment variables, see
[Getting Started with Docker](getting-started-docker.md).

## Install

### Docker (recommended)

No local installation required. Pull and run in a single command:

```bash
docker run -e CORROBORE_HTTP_AUTH_TOKEN=change-me -p 8080:8080 ghcr.io/estance-labs/corrobore:latest
```

### Prebuilt binary

Download the archive for your platform from the [latest release](https://github.com/Estance-Labs/corrobore/releases/latest):

| Platform          | Archive                             |
|-------------------|-------------------------------------|
| Linux x64         | `corrobore-linux-x86_64.tar.gz`         |
| Linux arm64       | `corrobore-linux-aarch64.tar.gz`        |
| macOS x64         | `corrobore-macos-x86_64.tar.gz`         |
| macOS arm64       | `corrobore-macos-aarch64.tar.gz`        |
| Windows x64       | `corrobore-windows-x86_64.zip`          |

Extract the archive and run the unified `corrobore` binary directly.

## Run the HTTP server

The only required setting is a non-empty Bearer token. The server binds to loopback (`127.0.0.1:8080`) by default.

```bash
CORROBORE_HTTP_AUTH_TOKEN=change-me ./corrobore server start
```

Check the public endpoints:

```bash
curl http://127.0.0.1:8080/health/live
curl http://127.0.0.1:8080/health/ready
curl http://127.0.0.1:8080/version
curl http://127.0.0.1:8080/metrics
```

Then run a protected query:

```bash
curl -X POST http://127.0.0.1:8080/v1/cypher/read \
  -H 'Authorization: Bearer change-me' \
  -H 'Content-Type: application/json' \
  -d '{"query":"MATCH (n) RETURN n LIMIT 10"}'
```

## First end-to-end workflow

Create a durable session:

```bash
SESSION_ID=$(curl -s -X POST http://127.0.0.1:8080/v1/sessions/start \
  -H 'Authorization: Bearer change-me' \
  -H 'Content-Type: application/json' \
  -d '{"workspace_id":"workspace--demo","actor_id":"actor--demo","actor_kind":"agent"}' \
  | jq -r '.result.session_id')
```

Write and read through that session:

```bash
curl -X POST http://127.0.0.1:8080/v1/cypher/write \
  -H 'Authorization: Bearer change-me' \
  -H 'Content-Type: application/json' \
  -d "{\"query\":\"MERGE (n:Indicator {name: 'demo.example'}) RETURN n\",\"session_id\":\"${SESSION_ID}\"}"

curl -X POST http://127.0.0.1:8080/v1/cypher/read \
  -H 'Authorization: Bearer change-me' \
  -H 'Content-Type: application/json' \
  -d "{\"query\":\"MATCH (n:Indicator) RETURN n LIMIT 10\",\"session_id\":\"${SESSION_ID}\"}"
```

Inspect audit parity and close the session:

```bash
curl "http://127.0.0.1:8080/v1/sessions/${SESSION_ID}/logs?limit=100" \
  -H 'Authorization: Bearer change-me'

curl -X POST "http://127.0.0.1:8080/v1/sessions/${SESSION_ID}/stop" \
  -H 'Authorization: Bearer change-me'
```

See the [Standalone Server CLI guide](user-guide/standalone-server.md) for TOML,
environment, and command-line configuration. The [HTTP Server
guide](user-guide/http-server.md) documents all routes and runtime settings.

## Use Corrobore in process

Add the workspace crate as a path or Git dependency, then use the facade:

```rust
use corrobore_engine::{CypherResponseData, CorroboreEngine};

let mut engine = CorroboreEngine::strict_default();
engine.write("CREATE (n:Indicator {name: 'observed-domain'})")?;
let response = engine.read("MATCH (n:Indicator) RETURN n")?;
assert!(matches!(response.data, CypherResponseData::Records(_)));
# Ok::<(), corrobore_engine::EngineError>(())
```

The [Embedded Engine guide](user-guide/embedded-engine.md) covers policies, parameters, seed search, and export.

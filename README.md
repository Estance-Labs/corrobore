# Corrobore

Corrobore is a Rust-native, Cypher-compatible graph runtime used as structured
working memory for intelligence agents.

It keeps entities, relationships, evidence, confidence, temporal metadata, and
session audit history outside model context while exposing bounded graph reads
and writes through embedded Rust and HTTP interfaces.

## Choose an interface

| Interface | Best for | Entry point |
| :--- | :--- | :--- |
| Embedded Rust | Applications that want an in-process engine | `docs/user-guide/embedded-engine.md` |
| Standalone server | Native service operation and layered configuration | `docs/user-guide/standalone-server.md` |
| HTTP API | Agent tools, services, and remote integrations | `docs/user-guide/http-server.md` |
| TAXII ingestion | Incremental STIX 2.1 collection polling through the public import boundary | `docs/user-guide/ingestion.md` |

## Current baseline

- Workspace version: `0.2.2`.
- Public API contract: `docs/api/openapi.yaml`.
- Runtime behavior documentation: `docs/user-guide/http-server.md`.
- Release notes: `docs/release-notes/`.

### Released capability snapshot

- Authenticated HTTP runtime with explicit read and write Cypher boundaries.
- Durable session lifecycle with status transitions and structured JSONL logs.
- Operational observability through distinct liveness/readiness, `/version`,
  `/metrics`, correlated requests, and session log export.
- Deterministic STIX import, validation, and export surfaces.
- Incremental TAXII 2.1 ingestion with persisted cursors.
- Bounded seed-search and working-set retrieval primitives for agent loops.

## Quick start (Docker Compose)

Run the complete local stack (HTTP API + explorer) from repository root:

```bash
cp .env.sample .env
# Replace CORROBORE_HTTP_AUTH_TOKEN=change-me with a local non-empty token.
mkdir -p .corrobore-tls
openssl req -x509 -newkey rsa:2048 -sha256 -nodes \
  -keyout .corrobore-tls/server.key \
  -out .corrobore-tls/server.crt \
  -days 30 -subj '/CN=localhost' \
  -addext 'basicConstraints=critical,CA:FALSE' \
  -addext 'keyUsage=critical,digitalSignature,keyEncipherment' \
  -addext 'extendedKeyUsage=serverAuth' \
  -addext 'subjectAltName=DNS:localhost,IP:127.0.0.1'
docker compose up --build --wait
```

The API is available at <https://localhost:8080>. The example certificate is
self-signed, so local clients must explicitly trust it or use `--insecure`.

For lifecycle commands, token handling, and security boundaries, see the
Docker section in `docs/user-guide/http-server.md`.

## Quick start (native HTTP runtime)

```bash
CORROBORE_HTTP_AUTH_TOKEN=change-me \
  cargo run -p corrobore-http-server --release --bin corrobore -- server start
```

Health checks:

```bash
curl http://127.0.0.1:8080/health/ready
curl http://127.0.0.1:8080/version
curl http://127.0.0.1:8080/metrics
cargo run -p corrobore-http-server --bin corrobore -- server status \
  --auth-token change-me
```

Protected read query:

```bash
curl -X POST http://127.0.0.1:8080/v1/cypher/read \
  -H 'Authorization: Bearer change-me' \
  -H 'Content-Type: application/json' \
  -d '{"query":"MATCH (n) RETURN n LIMIT 10"}'
```

## Build and validate

```bash
cargo build --workspace --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features --locked
node --test scripts/docs-contract-guard.test.mjs
node scripts/docs-contract-guard.mjs
```

The docs contract guard fails when HTTP routes or runtime env vars drift between
server code and public docs.

## Repository segmentation

Corrobore follows a public runtime plus integration model:

- this repository owns runtime behavior, API contracts, and user-facing docs;
- integration code consumes stable runtime boundaries, mainly through HTTP;
- candidate and forward-looking artifacts remain separate from implemented behavior.

Use `docs/` and code on `main` as the source of truth for shipped functionality.

## Documentation map

- Product docs landing: `docs/index.md`
- Getting started: `docs/getting-started.md`
- Embedded Rust usage: `docs/user-guide/embedded-engine.md`
- Standalone server CLI: `docs/user-guide/standalone-server.md`
- HTTP runtime and API behavior: `docs/user-guide/http-server.md`
- TAXII connector: `docs/user-guide/ingestion.md`
- Cypher subset: `docs/user-guide/cypher.md`
- Architecture: `docs/architecture.md`
- LLM operating guidance: `docs/for-llms.md`
- Interactive API reference: `docs/api/index.html`

## License

MIT. See `LICENSE`.

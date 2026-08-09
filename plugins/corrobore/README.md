# Corrobore Agent Plugin

This directory is the portable Corrobore package for
[Agent Plugins v1.0.0](https://agent-plugins.org/specification). It gives
compatible agents evidence-first operating guidance plus an MCP bridge to a
running Corrobore HTTP service.

## Package contents

```text
corrobore/
├── plugin.json
├── mcp.json
├── mcp-server/
│   ├── lib.mjs
│   └── server.mjs
├── LICENSE
├── README.md
└── skills/
    ├── corrobore/
    │   ├── SKILL.md
    │   └── references/
    └── opencti-intel-harvester/
        └── SKILL.md
```

`mcp.json` starts a zero-dependency Node.js process over standard input and
output. The process is a portable bridge to Corrobore's documented HTTP API; it
is not an embedded database and does not present the Rust engine itself as a
native MCP implementation.

## Requirements

- Node.js 20 or newer must be available as `node` on `PATH`.
- A Corrobore HTTP runtime must be reachable. The default base URL is
  `http://127.0.0.1:8080`.
- The Agent Plugin client must support portable stdio MCP servers.

The command uses `${PLUGIN_ROOT}` and has no operating-system-specific launcher,
package manager, or shell syntax. The same extracted package can therefore run
on macOS, Linux, and Windows when Node.js is available.

## Install

1. Download
   [`corrobore-agent-plugin.zip`](https://github.com/Estance-Labs/corrobore/releases/download/agent-plugin-v0.2.0/corrobore-agent-plugin.zip).
2. Extract the complete `corrobore/` directory into the plugin location used by
   your client. Keep `plugin.json`, `mcp.json`, `mcp-server/`, and `skills/`
   together.
3. Enable the plugin using the client's installation flow.
4. Start or connect to Corrobore, configure the MCP environment described below,
   and call `corrobore_ready` before protected tools.

Installation, permissions, and enablement are client-owned concerns outside the
portable specification. See the
[compatible clients](https://agent-plugins.org/compatible-clients) page for the
current client-specific instructions.

For source-based installation, use the `plugins/corrobore` directory from this
repository without copying files from outside that directory into the package.

## MCP configuration

The package itself contains no credential or environment-specific endpoint.
Configure these variables in the MCP process environment using the secret and
configuration controls owned by your client or launcher:

| Variable | Default | Boundary |
| :--- | :--- | :--- |
| `CORROBORE_MCP_BASE_URL` | `http://127.0.0.1:8080` | Corrobore HTTP base URL; only `http` and `https` are accepted, without URL credentials, query, or fragment. |
| `CORROBORE_MCP_AUTH_TOKEN` | unset | Optional bearer token. Never put it in `mcp.json`, prompts, logs, or version control. |
| `CORROBORE_MCP_AUTH_TOKEN_FILE` | unset | Alternative path to a client-managed bearer-token file, limited to 16 KiB. Do not set it together with the direct token. |
| `CORROBORE_MCP_TIMEOUT_MS` | `10000` | Per-request timeout from 50 to 60000 ms. |
| `CORROBORE_MCP_MAX_MESSAGE_BYTES` | `1048576` | Maximum MCP request and Corrobore response size from 256 bytes to 16 MiB. |

Use TLS for non-loopback or otherwise untrusted network paths. The MCP bridge
passes the configured bearer token only in the HTTP `Authorization` header and
never returns or logs it. Authentication and Corrobore policy remain the
authority boundary; loading the plugin does not grant read, write, trace,
forget, consolidate, STIX, or enterprise-domain permissions.

## MCP tools

| Tool | HTTP route | Effect |
| :--- | :--- | :--- |
| `corrobore_ready` | `GET /health/ready` | Readiness check. |
| `corrobore_remember` | `POST /v1/memory/operations` | Authorized memory create or identity-upsert. |
| `corrobore_relate` | `POST /v1/memory/operations` | Authorized relationship create or version. |
| `corrobore_recall` | `POST /v1/memory/operations` | Bounded working-set read. |
| `corrobore_update` | `POST /v1/memory/operations` | Authorized optimistic update. |
| `corrobore_forget` | `POST /v1/memory/operations` | Authorized expiry, tombstone, or application deletion semantics. |
| `corrobore_consolidate` | `POST /v1/memory/operations` | Policy-gated proposal or approved apply. |
| `corrobore_trace` | `POST /v1/memory/operations` | Bounded provenance and policy trace. |
| `corrobore_stix_import` | `POST /v1/import/stix` | STIX 2.1 import with optional retained evidence. |
| `corrobore_stix_validate` | `POST /v1/stix/validate` | Bundle or graph validation; supported playbooks may persist corrections. |
| `corrobore_stix_export` | `GET /v1/export/stix` | Deterministic CTI-scoped STIX projection; strict by default. |

The seven memory tools always add `contract_version: "v1"` and select the
operation named by the tool. Callers still own complete operation input,
explicit recall limits, mutation authority, and mutation idempotency keys. MCP
annotations are hints only and are never an authorization decision.

## Runtime behavior

The server implements MCP initialization, initialized notification, ping,
`tools/list`, and `tools/call` over newline-delimited UTF-8 JSON-RPC. Standard
output is reserved for protocol messages. Malformed or oversized input,
unsupported tools, request timeouts, network failures, oversized responses, and
non-success HTTP responses produce bounded protocol or tool errors without
printing credentials.

Start with `corrobore_ready`, use bounded read operations before authorized
writes, and preserve evidence, confidence, candidate status, strict export, and
audit boundaries. The plugin does not start Corrobore or authorize mutation.

## Validate

From the Corrobore repository root:

```bash
node --test scripts/agent-plugin-contract.test.mjs
node --test scripts/agent-plugin-mcp.test.mjs
```

The contracts check the closed Agent Plugins manifests, Agent Skills discovery,
package-local paths, documentation and release wiring, MCP lifecycle, complete
tool discovery, HTTP mapping, bearer isolation, timeouts, bounded failures, and
stdout purity.

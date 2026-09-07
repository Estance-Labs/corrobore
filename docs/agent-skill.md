# Agent Plugin

Corrobore Agent Plugin v0.2.0 is a portable
[Agent Plugins v1.0.0](https://agent-plugins.org/specification) package. It gives
compatible clients the same evidence-first skills plus an MCP bridge to a
running Corrobore HTTP service.

## Download

[Download Corrobore Agent Plugin v0.2.0](https://github.com/Estance-Labs/corrobore/releases/download/agent-plugin-v0.2.0/corrobore-agent-plugin.zip)

The versioned archive contains one self-contained `corrobore/` directory. Its
portable manifests are `plugins/corrobore/plugin.json` and
`plugins/corrobore/mcp.json` in the source repository.

[Browse the package source](https://github.com/Estance-Labs/corrobore/tree/main/plugins/corrobore)

## Install

1. Install Node.js 20 or newer and make `node` available on `PATH`.
2. Extract the complete `corrobore/` directory from the archive.
3. Place or import that directory using your client's Agent Plugin flow. Keep
   `plugin.json`, `mcp.json`, `mcp-server/`, `skills/`, and `references/`
   together.
4. Start or connect to a Corrobore HTTP service.
5. Configure any endpoint and bearer secret with the client-owned MCP controls,
   then run `corrobore_ready`.

The portable stdio command uses `node` and `${PLUGIN_ROOT}` without a shell or
platform-specific package manager. It works from the same extracted package on
macOS, Linux, and Windows when the runtime requirement is met.

Distribution, installation, permissions, environment injection, and enablement
are client-specific. Consult the official
[compatible clients](https://agent-plugins.org/compatible-clients) list for the
current setup instructions.

## MCP configuration and security

The bridge defaults to `http://127.0.0.1:8080`. A client or launcher can set:

- `CORROBORE_MCP_BASE_URL` for a different HTTP(S) service;
- `CORROBORE_MCP_AUTH_TOKEN` for a bearer token, or
  `CORROBORE_MCP_AUTH_TOKEN_FILE` for a client-managed token file;
- `CORROBORE_MCP_TIMEOUT_MS` for a bounded 50–60000 ms request timeout;
- `CORROBORE_MCP_MAX_MESSAGE_BYTES` for a bounded 256-byte–16-MiB protocol and
  response limit.

Do not set both token sources. Never add a token to `mcp.json`, a prompt, logs,
or version control. Use TLS beyond a trusted local path. The MCP tool
annotations describe effects but do not grant authority: Corrobore
authentication, workspace isolation, independent permissions, policy approval,
licensing, and durability gates remain authoritative.

This is a portable MCP-to-HTTP bridge. It does not embed the Rust engine, start
the Corrobore service, or claim that the underlying engine is itself a native
MCP server.

## Included MCP tools

The MCP surface is the `mcp` projection of the runtime's
[capability catalogue](user-guide/agentic-platform.md#one-capability-catalogue-several-adapters).
Each tool renders one catalogued capability; effect and authorization are
decided in the catalogue, so the tool annotations can never disagree with the
HTTP surface about whether an operation writes. Raw Cypher execution is
restricted to HTTP and is not a tool.

| Area | Tools | Capability | Public Corrobore route |
| :--- | :--- | :--- | :--- |
| Claim audit (current source) | `corrobore_claim_audit` | `claim.audit` | `GET /v1/claims/{id}/audit` |
| Runtime | `corrobore_ready` | `runtime.ready` | `GET /health/ready` |
| Memory writes | `corrobore_remember`, `corrobore_relate`, `corrobore_update`, `corrobore_forget` | `memory.remember`, `memory.relate`, `memory.update`, `memory.forget` | `POST /v1/memory/operations` |
| Memory reads | `corrobore_recall`, `corrobore_trace` | `memory.recall`, `memory.trace` | `POST /v1/memory/operations` |
| Memory policy | `corrobore_consolidate` | `memory.consolidate` (operator approval) | `POST /v1/memory/operations` |
| STIX | `corrobore_stix_import`, `corrobore_stix_validate`, `corrobore_stix_export` | `stix.import`, `stix.validate`, `stix.export` | `/v1/import/stix`, `/v1/stix/validate`, `/v1/export/stix` |

Each memory tool fixes `contract_version` to `v1` and maps to the operation in
its name. Callers supply the typed `input`, explicit recall budget, and any
required mutation idempotency key. `corrobore_consolidate` accepts the additive
`authority_policy` and `revoked_source_ids` fields: fused authority is capped by
the strongest justified source and a revocation recomputes without deleting.
Strict STIX export remains the default; permissive or forced export is used only
when explicitly requested and its diagnostics must be inspected.

`scripts/ws-h-capability-adapters.test.mjs` holds the bridge to the committed
`compatibility/capabilities/v1/catalogue.json`: the tool set must cover exactly
the capabilities exposed to `mcp`, each `readOnlyHint` must match the defined
effect, and the operator-approval capability must be advertised as destructive.

## Included skills

### Corrobore

The general skill treats Corrobore as bounded external structured memory. It
enforces read-before-write, evidence and confidence ownership, explicit
mutation authority, candidate status, strict export, and session cleanup.

[Read the Corrobore skill](https://github.com/Estance-Labs/corrobore/blob/main/plugins/corrobore/skills/corrobore/SKILL.md)

Its progressive references cover working memory, CTI, FIMI, report-to-STIX,
evidence-first validation, candidate ingestion, the claim audit, verdicts and
actionability with corrective routes, and campaign provenance without
attribution.

### OpenCTI Intelligence Harvester

The specialized skill extracts grounded CTI from supplied documents, uses
Corrobore as validation substrate, and returns exactly one deterministic STIX
2.1 bundle without inventing missing facts.

[Read the OpenCTI harvester](https://github.com/Estance-Labs/corrobore/blob/main/plugins/corrobore/skills/opencti-intel-harvester/SKILL.md)

## Version 0.2.0

This plugin release adds the root `mcp.json`, the zero-dependency stdio bridge,
11 documented tools, environment-owned authentication, bounded transport and
HTTP failures, and executable integration contracts. Version 0.1.0 remains the
skills-only historical package.

## Source and validation

- Package: `plugins/corrobore/`
- Plugin manifest: `plugins/corrobore/plugin.json`
- MCP manifest: `plugins/corrobore/mcp.json`
- Package contract: `scripts/agent-plugin-contract.test.mjs`
- MCP integration contract: `scripts/agent-plugin-mcp.test.mjs`
- Capability catalogue projection: `scripts/ws-h-capability-adapters.test.mjs`
- Skill guidance contracts: `scripts/ws-c-guidance.test.mjs`, `scripts/ws-f-guidance.test.mjs`, `scripts/agent-cti-contract.test.mjs`
- Specification: [agent-plugins.org/specification](https://agent-plugins.org/specification)

The repository contracts reject unknown portable fields, invalid skill
frontmatter, escaping links or symlinks, secret-bearing MCP configuration,
missing release wiring, incomplete tool discovery, incorrect HTTP mapping,
stdout pollution, and unbounded error behavior.

## Claim audit before verdicts

Both packaged skills require `GET /v1/claims/{id}/audit` before asserting a verdict.
The packaged `references/claim-audit.md` playbook covers the four questions,
mechanical versus semantic checks, missing provenance and reversible human
judgments. See the [audit guide and acceptance evidence](user-guide/claim-audit.md).

The current source package exposes this read as `corrobore_claim_audit` with
`claim_id`; the historical v0.2.0 archive does not include this additional tool.

## Verdicts, corrective routes, and collections

The packaged `references/verdicts-and-actionability.md` playbook tells an agent
how to report the six named confidence dimensions, independence clusters, the
separate actionability gate, and what could change a verdict (`falsifiers`,
`corrective_routes`, or the `no_recorded_falsifier` gap). The packaged
`references/campaign-provenance.md` playbook keeps narrative and campaign
membership as context, reports coordination signals as shared production
patterns rather than authorship, and attributes only through a supported
governed claim. See [Verdicts and Actionability](user-guide/verdicts-and-actionability.md)
and [Narratives, Campaigns, and Misleadingness](user-guide/narratives-and-campaigns.md).

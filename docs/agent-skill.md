# Agent Plugin

Corrobore is available as a portable
[Agent Plugins v1.0.0](https://agent-plugins.org/specification) package. It can be
loaded by compatible agent clients without rearranging the same skills for each
client.

## Download

[Download Corrobore Agent Plugin v0.1.0](https://github.com/Estance-Labs/corrobore/releases/download/agent-plugin-v0.1.0/corrobore-agent-plugin.zip)

The versioned archive contains one self-contained `corrobore/` directory. Its
portable manifest is `plugins/corrobore/plugin.json` in the source repository.

[Browse the package source](https://github.com/Estance-Labs/corrobore/tree/main/plugins/corrobore)

## Install

1. Extract the complete `corrobore/` directory from the archive.
2. Place or import that directory using the Agent Plugin installation flow of
   your client.
3. Keep `plugin.json`, `skills/`, and `references/` together.
4. Connect the agent to a running Corrobore HTTP service using client-owned
   secret handling for protected routes.

Distribution, installation, permissions, and enablement are intentionally
client-specific. Consult the official
[compatible clients](https://agent-plugins.org/compatible-clients) list for the
current setup instructions for ChatGPT and Codex, GitHub Copilot, VS Code,
Cursor, Kiro, and other conformant clients.

## Included skills

### Corrobore

The general skill treats Corrobore as bounded external structured memory. It
enforces read-before-write, evidence and confidence ownership, explicit
mutation authority, candidate status, strict export, and session cleanup.

[Read the Corrobore skill](https://github.com/Estance-Labs/corrobore/blob/main/plugins/corrobore/skills/corrobore/SKILL.md)

Its progressive references cover working memory, CTI, FIMI, report-to-STIX,
and evidence-first validation workflows.

### OpenCTI Intelligence Harvester

The specialized skill extracts grounded CTI from supplied documents, uses
Corrobore as validation substrate, and returns exactly one deterministic STIX
2.1 bundle without inventing missing facts.

[Read the OpenCTI harvester](https://github.com/Estance-Labs/corrobore/blob/main/plugins/corrobore/skills/opencti-intel-harvester/SKILL.md)

## Portable boundary

The plugin does not include `mcp.json`: this repository currently owns embedded
Rust and HTTP runtime surfaces, not a portable MCP server. The skills name the
real HTTP routes and do not imply that packaging the instructions starts the
runtime or authorizes writes.

## Source and validation

- Package: `plugins/corrobore/`
- Manifest: `plugins/corrobore/plugin.json`
- Contract: `scripts/agent-plugin-contract.test.mjs`
- Specification: [agent-plugins.org/specification](https://agent-plugins.org/specification)

The repository contract rejects unknown portable manifest fields, invalid skill
frontmatter, escaping links or symlinks, undeclared MCP configuration, and
missing release wiring.

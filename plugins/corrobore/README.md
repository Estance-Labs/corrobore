# Corrobore Agent Plugin

This directory is the portable Corrobore package for
[Agent Plugins v1.0.0](https://agent-plugins.org/specification). It gives compatible
agents operating guidance for evidence-backed structured memory and CTI report
processing.

## Package contents

```text
corrobore/
├── plugin.json
├── LICENSE
├── README.md
└── skills/
    ├── corrobore/
    │   ├── SKILL.md
    │   └── references/
    └── opencti-intel-harvester/
        └── SKILL.md
```

The package deliberately has no `mcp.json`. Corrobore currently exposes its
runtime through embedded Rust and HTTP interfaces in this repository; the
portable plugin does not claim that either interface is an MCP server.

## Install

1. Download
   [`corrobore-agent-plugin.zip`](https://github.com/Estance-Labs/corrobore/releases/download/agent-plugin-v0.1.0/corrobore-agent-plugin.zip).
2. Extract the complete `corrobore/` directory into the plugin location used by
   your client. Keep `plugin.json`, `skills/`, and their relative paths together.
3. Enable the plugin using the client's installation flow.
4. Start or connect to a Corrobore runtime before asking an agent to use the
   packaged workflows.

Installation, permissions, and enablement are client-owned concerns outside the
portable specification. See the
[compatible clients](https://agent-plugins.org/compatible-clients) page for the
current client-specific instructions.

For source-based installation, use the `plugins/corrobore` directory from this
repository without copying files from outside that directory into the package.

## Runtime boundary

The skills describe real Corrobore HTTP routes. Protected routes require a bearer
token supplied through the agent client's own secret handling. Never store a
token in this package, a prompt, or version control.

Start with `GET /health/ready`, use bounded read operations before authorized
writes, and preserve evidence, confidence, status, and audit boundaries. The
plugin does not start Corrobore and does not grant mutation authority by itself.

## Validate

From the Corrobore repository root:

```bash
node --test scripts/agent-plugin-contract.test.mjs
```

The contract checks the closed manifest schema, Agent Skills discovery and
frontmatter, package-local paths, documentation links, and release wiring.

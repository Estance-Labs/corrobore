# Domain Provider Runtime Contract

This page documents the public runtime contract exposed by the OSS server for
native domain providers. Private EE release and operations procedures are
maintained outside this repository.

## Scope

The OSS server supports:

1. Loading domain providers from a trusted directory and strict manifest.
2. Startup fail-closed validation of provider manifest, digest, ABI shape, and
	required capabilities.
3. Runtime gating by build feature and license claim before provider invocation.
4. Public diagnostics through stable health, metrics, and admin-status surfaces.

The server does not perform provider hot reload; provider and manifest changes
require process restart.

## Required configuration

Configure both variables together:

1. `CORROBORE_DOMAIN_PROVIDER_DIR`
2. `CORROBORE_DOMAIN_PROVIDER_MANIFEST_FILE`

If one is set without the other, startup fails. See the manifest shape example
in [docs/examples/domain-providers.json](../examples/domain-providers.json).

## Provider retrieval automation

Use [scripts/fetch-ee-domain-binaries.mjs](../../scripts/fetch-ee-domain-binaries.mjs)
to assemble runtime-ready provider libraries and manifest entries from EE release
archives.

The script performs these checks before writing output:

1. Extracts each provider archive for `cti`, `fimi`, and `crisis`.
2. Validates `release-manifest.json` (`schema_version`, domain, platform suffix,
	 provider version, and library name).
3. Validates `SHA256SUMS` and fails closed on digest mismatches.
4. Copies verified libraries into the configured output directory.
5. Writes a runtime `domain-providers.json` with strict `sha256` digests and
	 `node.validate/1` capability requirements.

Example:

```bash
node scripts/fetch-ee-domain-binaries.mjs \
	--version v0.1.0 \
	--platform linux-x64 \
	--output-dir overrides/domain-providers \
	--manifest-file overrides/domain-providers/domain-providers.json
```

For offline or test execution, set `--download-mode local` and provide
`--local-archive-dir` containing the expected release archives.

## Runtime and observability surfaces

Use these public endpoints to verify runtime state:

1. `GET /health`: includes `domain_providers.configured` and
	`domain_providers.ready`.
2. `GET /metrics`: exports `corrobore_domain_providers_configured` and
	`corrobore_domain_providers_ready`.
3. `GET /v1/admin/domain-providers/status`: returns provider identity,
	readiness, and capability summary (admin token required).
4. `GET /v1/admin/license/status`: returns the effective module-license view
	used by runtime gates (admin token required).

## Public gate outcomes

Before a provider call is attempted, handlers enforce build-feature and license
contracts. Stable API-level outcomes include:

| Code | Meaning |
| :--- | :--- |
| `FEATURE_NOT_AVAILABLE` | The running binary was built without the required enterprise module feature. |
| `LICENSE_MODULE_MISSING` | The running instance has no valid claim for the requested module. |
| `DOMAIN_PROVIDER_NOT_READY` | The provider is not available or did not pass startup/readiness checks. |
| `DOMAIN_PROVIDER_CAPABILITY_MISSING` | The provider is loaded but does not declare the required capability version. |
| `DOMAIN_PROVIDER_ERROR` | Provider invocation failed while handling a request. |
| `REQUEST_TIMEOUT` | Invocation exceeded the handler timeout budget. |

## Security boundaries

1. Keep provider libraries under the trusted directory root.
2. Keep manifest paths relative and pinned by digest.
3. Supply signed licenses only through runtime secret boundaries.
4. Never bake customer license material into container image layers.

## Private EE operations

EE release engineering, provenance/signing workflows, deployment promotion,
rollback playbooks, and incident procedures are maintained in the private
project-documentation repository and are intentionally not duplicated here.
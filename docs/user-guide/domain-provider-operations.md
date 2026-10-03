# Domain Provider Runtime Contract

This page documents the runtime contract the server exposes for native domain
providers.

## Scope

The server supports:

1. Loading domain providers from a trusted directory and strict manifest.
2. Startup fail-closed validation of provider manifest, digest, ABI shape, and
	required capabilities.
3. Runtime dispatch to the provider of a distributed domain (`medical`,
	`research`); the ABI-named `cti`, `fimi`, and `crisis` domains are not
	distributed with Corrobore and are rejected before any provider call.
4. Public diagnostics through stable health, metrics, and admin-status surfaces.

The server does not perform provider hot reload; provider and manifest changes
require process restart.

## Required configuration

Configure both variables together:

1. `CORROBORE_DOMAIN_PROVIDER_DIR`
2. `CORROBORE_DOMAIN_PROVIDER_MANIFEST_FILE`

If one is set without the other, startup fails. See the manifest shape example
in [docs/examples/domain-providers.json](../examples/domain-providers.json).

## Runtime and observability surfaces

Use these public endpoints to verify runtime state:

1. `GET /health`: includes `domain_providers.configured` and
	`domain_providers.ready`.
2. `GET /metrics`: exports `corrobore_domain_providers_configured` and
	`corrobore_domain_providers_ready`.
3. `GET /v1/admin/domain-providers/status`: returns provider identity,
	readiness, and capability summary (admin token required).

## Public gate outcomes

Before a provider call is attempted, handlers check that the domain is
distributed and its provider is ready. Stable API-level outcomes include:

| Code | Meaning |
| :--- | :--- |
| `FEATURE_NOT_AVAILABLE` | The requested domain (`cti`, `fimi`, or `crisis`) is not distributed with Corrobore. |
| `DOMAIN_PROVIDER_NOT_READY` | The provider is not available or did not pass startup/readiness checks. |
| `DOMAIN_PROVIDER_CAPABILITY_MISSING` | The provider is loaded but does not declare the required capability version. |
| `DOMAIN_PROVIDER_ERROR` | Provider invocation failed while handling a request. |
| `REQUEST_TIMEOUT` | Invocation exceeded the handler timeout budget. |

## Security boundaries

1. Keep provider libraries under the trusted directory root.
2. Keep manifest paths relative and pinned by digest.
3. Load only provider libraries whose provenance you trust: a provider is
	native code running inside the server process.

# Intelligence Domains

Corrobore keeps shared evidence primitives in-workspace and consumes domain logic through native binary providers.

This guide targets the current `0.3.x` runtime baseline.

## Distribution model

- In this workspace, `domain-common` provides shared evidence and epistemic primitives.
- The MIT `medical` and `research` packs ship in this workspace as `domain-medical-provider` and `domain-research-provider`, built against the provider ABI.
- The ABI also names the `cti`, `fimi`, and `crisis` domains so existing provider binaries keep loading. Corrobore distributes no implementation for them: no source, no binary, and no request is served for them (`FEATURE_NOT_AVAILABLE`).
- Every pack uses the same versioned provider ABI and the `node.validate/1` capability; domain-specific behavior stays inside the pack.

## Provider contract and deployment

The normative cross-repository C contract is `crates/domain-provider-abi/include/corrobore_domain_provider.h`. ABI v1 has one exported entrypoint returning a prefix-versioned table for metadata, create, invoke, health, destroy, and provider-owned buffer release. JSON request and response envelopes carry evolvable capability data without changing the binary table. ABI minor 2 adds optional `claim.verify/1`; hosts at minor 2 continue to accept providers built against supported minor 1.

A provider declaring `claim.verify/1` is registered as a host-side `Verifier`. Its declaration may include `"deterministic": true` or `false`; omission defaults to `false`, so older metadata remains advisory and compatible. The request payload contains the governed claim, active links, resolved observations and sources, evidence records, and the bitemporal `as_of` point. The response payload reports `pass`, `fail`, or `inconclusive`, plus rationale, limits, and consumed evidence. The provider only reports the result: the host owns record provenance and the deterministic-first precedence policy.

Set both `CORROBORE_DOMAIN_PROVIDER_DIR` and `CORROBORE_DOMAIN_PROVIDER_MANIFEST_FILE` to enable providers. The manifest uses relative library paths, lowercase SHA-256 digests, required/optional policy, and required capabilities; see [the production manifest shape](../examples/domain-providers.json). At startup the host confines canonical paths to the trusted root, verifies each digest, negotiates ABI v1, validates provider identity and limits, creates one instance, and requires a ready health response. Any failure for a required provider prevents the server from accepting traffic.

A distributed domain is gated by provider readiness and capability alone. Provider calls are serialized in ABI v1, bounded by declared request/response sizes, wrapped by the server request timeout, and correlated by `request_id`.

Use `POST /v1/domains/{domain}/validate` for generic `medical` or `research` validation; `cti`, `fimi`, and `crisis` are accepted names that return `FEATURE_NOT_AVAILABLE`. `GET /health` reports aggregate configured/ready counts; authenticated operators can inspect non-sensitive provider identity, version, capabilities, domain, and readiness through `GET /v1/admin/domain-providers/status`.

## Narrative and campaign primitives

Since Epic 0029 WS-G the core holds domain-neutral, immutable `Narrative` and `Campaign` collections, coordination signals stored as evidence, and an attribution gate that refuses coordination signals as the only support. Domain packs add meaning on top of these primitives through `node.validate/1` and `claim.verify/1`. See [Narratives, Campaigns, and Misleadingness](narratives-and-campaigns.md).

## Shared evidence and epistemic primitives

`domain-common` and `graph-core` provide evidence, confidence, claims, stances, belief states, hypotheses, and provenance-oriented metadata used across domains. Keep observations, claims, and hypotheses distinct; a semantic seed score or model inference is not evidence by itself.

## Function registry boundary

`function-registry` implements typed `namespace.symbol` registration, arity checking, type validation, and dispatch. Domain Rust functions exist today, but they are not generally exposed as callable Cypher built-ins. Do not generate syntax such as `crisis.score(...)` unless the host has explicitly registered and wired that function.

## Modeling guidance

- Keep evidence objects separate from inferred claims.
- Keep confidence as explicit metadata, not implicit truth.
- Keep cross-domain links attributable to source material.
- Prefer additive modeling (`MERGE` + updates) over destructive rewrites.

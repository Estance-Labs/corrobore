# Domain Provider Operations

This runbook covers production deployment of private CTI, FIMI, and Crisis native providers into the public Corrobore runtime. Provider source, signing credentials, customer licenses, and private image assembly remain outside this repository.

## Release contract

Each EE repository builds one platform-native dynamic library against the canonical `corrobore_domain_provider.h` from the exact core release. Its release pipeline must:

1. Run repository tests plus the shared ABI conformance suite.
2. Compile with panic/exception containment at every exported C function.
3. Strip according to the private release policy and preserve separate debug symbols.
4. Generate a lowercase SHA-256 digest for the final bytes copied into the image.
5. Sign and attest the provider artifact in the private supply-chain system.
6. Publish immutable artifacts keyed by core version, provider version, target OS, and architecture.

The private EE image starts from the matching immutable core image digest, copies all licensed provider libraries into a root such as `/opt/corrobore/providers`, copies a generated manifest to `/etc/corrobore/domain-providers.json`, and sets both provider environment variables at runtime. Never bake a customer license into either image layer.

## Manifest generation

Start from [the three-domain example](../examples/domain-providers.json). Replace filenames for the target platform and replace every sample digest with the digest of the exact packaged bytes:

```bash
shasum -a 256 /opt/corrobore/providers/libcorrobore_domain_cti.dylib
sha256sum /opt/corrobore/providers/libcorrobore_domain_cti.so
```

Use relative filenames only. Set `required: true` when the image promises that module. The manifest capability list is a deployment requirement, not discovery: provider metadata must declare every listed name/version or startup fails.

## Deployment procedure

1. Verify image signatures, SBOM, provenance, provider digests, and core/provider compatibility in the promotion environment.
2. Deploy with `CORROBORE_DOMAIN_PROVIDER_DIR` and `CORROBORE_DOMAIN_PROVIDER_MANIFEST_FILE` configured as a pair.
3. Supply the signed customer license at runtime through the normal secret boundary.
4. Require process startup success before adding an instance to service discovery.
5. Check `/health`: `domain_providers.configured` and `domain_providers.ready` must equal the number promised by the image.
6. Scrape `corrobore_domain_providers_configured` and `corrobore_domain_providers_ready`; alert when they differ or change unexpectedly.
7. Query `/v1/admin/domain-providers/status` with the admin token and verify domain, provider id, version, capabilities, and readiness.
8. Run one non-mutating validation canary per licensed domain through `/v1/domains/{domain}/validate`.

The server intentionally has no hot reload. Roll out a new image or restart the process after any provider or manifest change.

## Failure diagnosis

Startup diagnostics identify the stage without exposing paths through public HTTP:

| Diagnostic | Operator action |
| :--- | :--- |
| Manifest missing or invalid | Verify both environment variables, JSON schema `1`, unique domains, relative paths, lowercase 64-character hashes, and known fields. |
| Path outside trusted directory | Remove symlinks/path traversal and package the library directly under the trusted root. |
| SHA-256 mismatch | Stop rollout; compare artifact provenance and regenerate the manifest only from verified final bytes. |
| Missing entrypoint or incompatible ABI | Deploy a provider built against the matching core ABI major/minor and canonical header. |
| Invalid metadata or domain mismatch | Correct provider identity, non-zero limits/concurrency, schema version, and declared domain in the EE repository. |
| Required capability missing | Publish a provider implementing the manifest capability version or correct the image manifest promise. |
| Create/health failure | Inspect private provider logs/debug symbols; do not bypass readiness or mark the provider optional to complete rollout. |
| `FEATURE_NOT_AVAILABLE` | Use a core build containing the domain feature. |
| `LICENSE_MODULE_MISSING` | Install a valid signed license claim for that domain. |
| `DOMAIN_PROVIDER_NOT_READY` | Verify image contents, startup status, health aggregate, and admin provider status. |
| `DOMAIN_PROVIDER_ERROR` | Correlate server and provider diagnostics by `request_id`; check response schema, size limits, and provider status. |

## Rollback

Rollback is image-based. Restore the previous signed private image digest and its matching manifest as one unit, restart, then repeat health, metrics, admin-status, and per-domain canaries. Do not mix a previous core with newer providers unless that exact combination passed ABI conformance and promotion. Retain the failed image, manifest, digests, logs, and request ids for incident analysis.

## Migration from the CTI-only loader

The former `CORROBORE_CTI_PLUGIN_PATH` loaded a CTI library per call and expected `corrobore_cti_validate_node_json_v1` plus `corrobore_cti_free_string_v1`. These symbols and that variable are no longer supported.

To migrate:

1. Implement the single `corrobore_domain_provider_get_api_v1` entrypoint and all six mandatory v1 functions in the CTI EE repository.
2. Return CTI identity, operational limits, and `node.validate/1` from metadata; accept the generic invocation envelope and preserve `request_id`.
3. Add the CTI library to the common manifest with its final SHA-256 digest.
4. Replace `CORROBORE_CTI_PLUGIN_PATH` with the provider directory/manifest variable pair.
5. Build the private image with CTI, FIMI, and Crisis artifacts appropriate to the licensed distribution; do not copy them into the OSS image.
6. Validate startup fail-closed behavior and the generic route before removing the previous deployment.
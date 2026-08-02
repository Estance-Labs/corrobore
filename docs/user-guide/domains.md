# Intelligence Domains

Corrobore keeps shared evidence primitives in-workspace and consumes enterprise domain logic through dedicated binary providers.

This guide targets the current `0.3.x` runtime baseline.

## Distribution model

- In this workspace, `domain-common` provides shared evidence and epistemic primitives.
- Enterprise domain implementations (`cti`, `fimi`, `crisis`) are externalized to dedicated EE repositories.
- The public core image contains the ABI host but no EE implementation or source.
- The private EE image adds all licensed native libraries and a deployment manifest without replacing the core server binary.
- CTI, FIMI, and Crisis use the same versioned provider ABI and the `node.validate/1` capability; domain-specific behavior remains inside each EE repository.

## Provider contract and deployment

The normative cross-repository C contract is `crates/domain-provider-abi/include/corrobore_domain_provider.h`. ABI v1 has one exported entrypoint returning a prefix-versioned table for metadata, create, invoke, health, destroy, and provider-owned buffer release. JSON request and response envelopes carry evolvable capability data without changing the binary table.

Set both `CORROBORE_DOMAIN_PROVIDER_DIR` and `CORROBORE_DOMAIN_PROVIDER_MANIFEST_FILE` to enable providers. The manifest uses relative library paths, lowercase SHA-256 digests, required/optional policy, and required capabilities; see [the production manifest shape](../examples/domain-providers.json). At startup the host confines canonical paths to the trusted root, verifies each digest, negotiates ABI v1, validates provider identity and limits, creates one instance, and requires a ready health response. Any failure for a required provider prevents the server from accepting traffic.

Build availability, signed license claims, and provider readiness are independent gates. A licensed module is not usable unless its matching build feature and healthy provider are also present. The build and license gates apply to the enterprise domains only; the MIT `medical` and `research` packs ship with the open-source runtime and are gated by provider readiness and capability alone. Provider calls are serialized in ABI v1, bounded by declared request/response sizes, wrapped by the server request timeout, and correlated by `request_id`.

Use `POST /v1/domains/{domain}/validate` for generic `cti`, `fimi`, `crisis`, `medical`, or `research` validation. `GET /health` reports aggregate configured/ready counts; authenticated operators can inspect non-sensitive provider identity, version, capabilities, domain, and readiness through `GET /v1/admin/domain-providers/status`.

## CTI model surface

The CTI provider contract validates the following node labels and relationship vocabulary:

- Nodes: `ThreatActor`, `Malware`, `Indicator`, `Tool`, `Campaign`, `Infrastructure`, `Vulnerability`, `Identity`, `Location`, `Report`.
- Relationships: `Indicates`, `Uses`, `Targets`, `AttributedTo`, `CommunicatesWith`, `RelatedTo`.
- Validation: CTI-to-STIX readiness rules are applied through the shared provider registry by graph-mode STIX validation and the generic domain route.

## FIMI model surface

The EE FIMI provider models foreign information manipulation and interference.

- Nodes: `Actor`, `Narrative`, `Claim`, `Account`, `Outlet`, `Campaign`, `CoordinationCluster`.
- Relationships: `Amplifies`, `CoordinatesWith`, `OriginatesFrom`, `Targets`, `Repeats`, `Contradicts`.
- Typical use: connect claims, accounts, narratives, amplification, and coordination while preserving evidence and confidence.

## Crisis model surface

The EE crisis provider models crisis events, locations, needs, organizations, and observations.

- Nodes: `CrisisEvent`, `Location`, `HumanitarianNeed`, `Organization`, `Observation`.
- Relationships: `OccursAt`, `Impacts`, `ReportedBy`, `Needs`, `EscalatesTo`.
- Domain-specific Rust functions are implemented in EE repositories and are not distributed as source in this workspace. Their deployment boundary is the same ABI and manifest used by CTI and FIMI.

## Shared evidence and epistemic primitives

`domain-common` and `graph-core` provide evidence, confidence, claims, stances, belief states, hypotheses, and provenance-oriented metadata used across domains. Keep observations, claims, and hypotheses distinct; a semantic seed score or model inference is not evidence by itself.

## Function registry boundary

`function-registry` implements typed `namespace.symbol` registration, arity checking, type validation, and dispatch. Domain Rust functions exist today, but they are not generally exposed as callable Cypher built-ins. Do not generate syntax such as `crisis.score(...)` unless the host has explicitly registered and wired that function.

## Modeling guidance

- Keep evidence objects separate from inferred claims.
- Keep confidence as explicit metadata, not implicit truth.
- Keep cross-domain links attributable to source material.
- Prefer additive modeling (`MERGE` + updates) over destructive rewrites.

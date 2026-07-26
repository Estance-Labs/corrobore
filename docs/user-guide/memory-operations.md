# High-level Memory Operations

Corrobore exposes a domain-neutral memory contract for ordinary SDK, SaaS, MCP,
embedded, and standalone clients. The default contract is not a query language:
clients choose one of seven operations and never need to know graph labels,
storage records, or Cypher templates.

The version-one journey is **remember → relate → recall → update → trace → forget**.
`consolidate` is a separate policy-gated maintenance operation.
Advanced Cypher remains available through its explicit embedded and HTTP
interfaces and has a separate authorization boundary.

## Boundaries

Embedded Rust callers use `CorroboreEngine::execute_memory` with a
`MemoryServiceContext` and a `MemoryRequest`. Standalone clients send the same
serialized `MemoryRequest` to `POST /v1/memory/operations`.

The operation payload cannot contain workspace, actor, agent, session,
permissions, request identity, or correlation identity. An authenticated host
constructs `MemoryServiceContext` from trusted runtime state. The standalone
server uses `CORROBORE_MEMORY_*` deployment settings after bearer
authentication. Unknown payload fields are rejected.

Every request carries `contract_version: "v1"`. Mutations also require a
non-empty `idempotency_key`. Replaying the same key and byte-equivalent request
returns the original response with `receipt.replayed: true`; reusing the key for
different input returns `IDEMPOTENCY_CONFLICT`. A successful mutation is
returned only after the configured graph persistence transition and durability
gate complete. Failure returns `DURABILITY_FAILED` and does not publish the
candidate graph.

## Records and relationships

A memory has an engine ID, optional application identity key,
application-defined kind and schema version, text and/or structured properties,
source references, confidence, validity and recorded time, optional expiry,
lifecycle, tags, and a monotonic version. No domain provider is required.

A relationship is a versioned record, not an anonymous edge. It owns its ID,
optional application identity, endpoints, kind, structured properties,
provenance, confidence, temporal metadata, expiry, lifecycle, and version.

## Operations

| Operation | Semantics |
| :--- | :--- |
| `remember` | Create a memory or upsert the workspace-local record carrying the same application identity key. Provenance is appended rather than silently replaced. |
| `relate` | Create or version an evidence-bearing relationship between two visible memories. Both endpoints must belong to the trusted workspace. |
| `recall` | Resolve explicit and lexical objective seeds, traverse only the authorized neighborhood, and return a bounded working set with reasons, paths, completeness, outcomes, usage, and an opaque workspace-bound page token. |
| `update` | Apply an optimistic, auditable memory or relationship patch as a new version. Evidence additions preserve prior provenance. |
| `forget` | Expire, tombstone, or apply application deletion semantics so ordinary recall no longer returns the memory. |
| `consolidate` | Produce a non-destructive proposal, or apply a matching approved proposal while retaining originals and explicit disagreements. |
| `trace` | Explain memory/relationship versions, recall selection paths, evidence, actor/agent/session attribution, mutation correlation, and policy decisions. |

## Example

```json
{
  "contract_version": "v1",
  "idempotency_key": "notes:42:create",
  "operation": "remember",
  "input": {
    "identity_key": "note-42",
    "kind": "observation",
    "schema_version": "1",
    "content": {
      "format": "text_and_properties",
      "value": {
        "text": "The deployment decision was approved.",
        "properties": {"project": "atlas"}
      }
    },
    "provenance": [
      {
        "source_id": "meeting--2026-07-26",
        "locator": "minutes#decision-3",
        "observed_at": "2026-07-26T09:30:00Z"
      }
    ],
    "confidence": 0.9,
    "valid_from": "2026-07-26T09:30:00Z",
    "valid_until": null,
    "expires_at": null,
    "tags": ["decision"]
  }
}
```

The response includes the committed record and a stable receipt:

```json
{
  "contract_version": "v1",
  "result": {
    "operation": "remember",
    "result": {
      "record": {"id": "node--1", "version": 1},
      "receipt": {
        "committed_id": "node--1",
        "committed_version": 1,
        "audit_correlation_id": "request-42",
        "replayed": false
      }
    }
  }
}
```

The abbreviated record above omits fields only for readability; the actual
response returns the complete typed record.

## Recall limits and bounded outcomes

`recall` requires a non-empty objective plus positive limits for items,
traversal depth, serialized payload bytes, deterministic expansion cost,
timeout, and supernode degree. Contract maxima are 10,000 items, depth 16,
16 MiB payload, 1,000,000 cost units, and 60 seconds.

Each returned item includes `selection_reasons`; returned relationships retain
their evidence. `usage` reports items, depth, payload bytes, cost, and elapsed
time. `completeness` distinguishes complete and truncated results and can carry
bounded outcomes such as `supernode_blocked`, `cost_budget_exhausted`,
`payload_budget_exhausted`, `timeout`, or
`semantic_provider_unavailable`. The stable typed outcome enum also reserves
`partial_page_in`, `cancelled`, and `overloaded` for storage and runtime gates;
these conditions never trigger unbounded retry or expansion. Missing optional
semantic providers therefore do not trigger an unbounded fallback. Invalid
tokens from another workspace produce the same not-found surface as hidden
records.

## Permissions and errors

`read`, `write`, `trace`, `forget`, and `consolidate` permissions are resolved
and enforced independently. Stable version-one error codes are:

- `INVALID_REQUEST`, `INVALID_BUDGET`, `PERMISSION_DENIED`, and `NOT_FOUND`;
- `VERSION_CONFLICT`, `IDEMPOTENCY_CONFLICT`, and
  `IDEMPOTENCY_KEY_REQUIRED`;
- `BUDGET_EXCEEDED`, `CANCELLED`, `OVERLOADED`, and
  `SEMANTIC_PROVIDER_UNAVAILABLE`;
- `DURABILITY_FAILED`, `POLICY_APPROVAL_REQUIRED`, and `INTERNAL`.

Errors, identifiers, counts, paths, page tokens, and trace output are filtered
inside the trusted workspace boundary. A hidden target returns `NOT_FOUND`
rather than revealing that another workspace owns it.

## Forgetting and erasure

Application forgetting and regulatory erasure are intentionally different:

- expiry keeps versioned evidence but removes the memory after its application
  retention time;
- tombstone and application deletion remove the memory from ordinary retrieval
  while retaining only the version and audit information permitted by the
  runtime policy;
- regulatory erasure across replicas, backups, exports, caches, and tenant keys
  is a privileged SaaS/control-plane workflow and is not represented by this
  data-plane operation.

## Consolidation safety

Proposal mode changes no memory. Approved apply must repeat the exact bounded
candidate set and canonical identity, name its approval policy, and preserve
disagreements. The engine versions non-canonical originals as `superseded` and
adds traceable `superseded_by` relationships. It does not silently destroy
original evidence. Requests for destructive consolidation are rejected with
`POLICY_APPROVAL_REQUIRED`.

## Compatibility and evolution

`v1` is the compatibility key for both Rust and JSON schemas. Within `v1`, new
optional response fields and new bounded outcome strings may be added; required
request fields, existing enum meanings, error codes, receipt semantics, and
workspace isolation do not change. A breaking request shape, enum meaning, or
authorization/idempotency behavior requires a new contract version and a
parallel transport route or discriminator before removal of the old version.

Rust callers should construct public types rather than matching storage
properties. JSON callers should reject unknown operation names but tolerate
unknown optional response fields. The shared machine-readable fixtures live in
`compatibility/memory/v1/conformance.json` and are exercised for both embedded
and standalone adapters.

See also [HTTP Server](http-server.md), [Embedded Engine](embedded-engine.md),
and [Cypher Support](cypher.md).

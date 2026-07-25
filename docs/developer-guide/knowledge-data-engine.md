# Knowledge Data Engine contract

`corrobore-engine` exposes a backend-neutral, versioned
`KnowledgeDataEngine` contract alongside the existing Cypher API. The contract
is the boundary for product adapters: application call sites construct typed
operations and never import a storage client.

The OpenCTI compatibility adapter belongs outside this repository's generic
graph core. It may select either route through configuration:

```json
{"provider": "embedded_corrobore"}
```

```json
{
  "provider": "remote_reference",
  "endpoint": "https://reference.example.com/v1/knowledge-data"
}
```

Both routes use `KnowledgeDataRequest` and
`KnowledgeDataResponseEnvelope`. `execute_remote_contract` performs a canonical
JSON round trip over those exact types; HTTP, RPC, queues, or an embedded caller
must not invent different error or context semantics.

## Versioning and compatibility

Contract version `1.0` follows these rules:

- a major version change is breaking;
- a provider accepts a client minor version less than or equal to its own;
- every capability declares the version in which it appeared;
- `deprecated_after` is exclusive: callers at or beyond that version may no
  longer rely on the capability;
- unsupported capabilities are returned as `UNSUPPORTED_CAPABILITY`; a provider
  must never silently degrade an operation.

Initialization returns the contract, engine and schema versions, every supported
or unsupported capability, readiness blockers, and recovery state. Required
capabilities are negotiated before the application starts.

## Request context

Every operation carries the same provider-neutral context:

- request and correlation IDs;
- an optional idempotency key;
- an absolute deadline and cancellation ID;
- subject, organization, marking, tenant, role and extension access facts;
- eventual, read-your-writes or snapshot consistency.

The context deliberately contains no HTTP headers, status codes, RPC metadata,
or backend-specific request objects. Deadlines and cancellations are rejected
before provider dispatch with stable error codes.

## Operation surface

The version 1 operation set is:

| Lifecycle | Reads | Graph | Writes | Durability |
| --- | --- | --- | --- | --- |
| initialize, health, migrate | get-by-id, list, paginate, search, count, aggregate | neighbors, traverse, subgraph | create, update, delete, bulk, merge | snapshot, restore, rebuild-indexes |

The current Corrobore provider implements initialization, health, fundamental
point/list/count/cursor reads, and bounded neighbors/traversal/subgraph
operations. Persistent hosts prepare these operations through compact
identifier, label, property, temporal, and adjacency indexes before hydrating
payloads. The exact semantics are documented in
[OpenCTI core reads](opencti-core-reads.md). The other typed operations are
declared unsupported with their delivery issue:

- search and advanced reads: #46 and #47;
- typed transactional writes: #50;
- merge and reconciliation: #51;
- migration, snapshot, restore, and index maintenance: #52.

This preserves the explicit capability boundary while later issues deliver the
remaining query and mutation surface.

## Pagination integrity

Pagination tokens are opaque `kde1` envelopes. They contain a token version,
the ordered record cursor, the number already returned, the provider schema
version, and SHA-256 fingerprints of both the normalized list query and the
declared result snapshot. The payload is authenticated with HMAC-SHA-256 and a
provider key of at least 256 bits.

A changed byte returns `INVALID_PAGINATION_TOKEN`. Reusing a valid token with
another query, schema, or token version returns
`INCOMPATIBLE_PAGINATION_TOKEN`. Records are ordered by stable identifier and a
token resumes strictly after its cursor.

## Stable errors

Embedded and serialized remote execution share the same taxonomy:

`INVALID_REQUEST`, `INCOMPATIBLE_CONTRACT_VERSION`,
`UNSUPPORTED_CAPABILITY`, `NOT_FOUND`, `CONFLICT`, `UNAUTHORIZED`,
`DEADLINE_EXCEEDED`, `CANCELLED`, `INVALID_PAGINATION_TOKEN`,
`INCOMPATIBLE_PAGINATION_TOKEN`, `STALE_PAGINATION_TOKEN`,
`UNBOUNDED_OPERATION`, `QUERY_BUDGET_EXCEEDED`,
`SUPERNODE_EXPANSION_BLOCKED`, `BACKEND_UNAVAILABLE`, `SCHEMA_INCOMPATIBLE`,
and `INTERNAL`.

Provider diagnostics must remain safe to return across a remote boundary and
must not leak backend response bodies or transport internals.

## OpenCTI mapping and conformance

`compatibility/opencti/7.260722.0/knowledge-data-engine-mapping.json` maps all 32
logical operations captured by issue #38. File extraction and engine-private
transport tuning are explicitly outside the portable contract; every other
operation maps to a typed version 1 operation.

`run_conformance_cases` accepts the same case corpus for any execution closure.
The test suite runs lifecycle success and stable unsupported errors through
both the embedded provider and the serialized remote path, and checks exact
result and error equivalence. A reference provider can reuse the same cases
without depending on Corrobore internals.

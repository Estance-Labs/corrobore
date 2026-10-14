# High-level Memory Operations

Corrobore exposes a domain-neutral memory contract for ordinary SDK, SaaS, MCP,
embedded, and standalone clients. The default contract is not a query language:
clients choose one of a small set of operations and never need to know graph
labels, storage records, or Cypher templates.

The version-one journey is **remember → relate → recall → update → trace → forget**.
`consolidate` is a separate policy-gated maintenance operation, and `get` reads
current records back by application identity key or engine id.
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
Returned records also carry two read-only fields derived at read time:
`superseded_by` (the canonical memory after an approved consolidation, else
`null`) and `recallable` (whether ordinary recall currently considers the record:
active lifecycle, not past `expires_at`, inside its validity window). Returned
relationships carry `recallable` with the same meaning for recall traversal.

Records and relationships also expose evidence statistics derived from their
provenance (read-only, never accepted as input):

| Field | Meaning |
| :--- | :--- |
| `evidence_count` | Number of distinct `provenance[].source_id`. |
| `distinct_observation_days` | Number of distinct UTC calendar dates over each entry's effective observation time. |
| `first_observed_at`, `last_observed_at` | Earliest and latest effective observation time, RFC 3339 UTC; `null` without provenance. |

An entry's effective observation time is its `observed_at`, or the time the
engine first appended the entry when the source gave none. The engine keeps
that append time per entry; records written before this field existed fall back
to their `recorded_at`.

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
| `consolidate` | Produce a non-destructive proposal, or apply a matching approved proposal while retaining originals and explicit disagreements. The fused memory retains back-pointers to every origin and an authority capped by the strongest justified source, never raised by repetition. |
| `trace` | Explain memory/relationship versions, recall selection paths, evidence, actor/agent/session attribution, mutation correlation, and policy decisions. |
| `reinforce` | Deposit a usage reward (or penalty) on a memory or relationship's decaying heat, as a new auditable version. |
| `get` | Read current memories and relationships by application identity key or engine id. Expired and superseded records are returned with their lifecycle; unresolved values are listed in `missing`. |

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

## Heat: reinforcement, decay, and cold expiry

Every memory and relationship carries a heat value that follows the pheromone
law `τ(t+1) = λ·τ(t) + reward − penalty` with a wall-clock half-life. The engine
stores the heat and the time of its last deposit and decays it lazily at read
time: `heat(now) = heat(t) · 0.5^((now − t) / half_life)`.

- Every provenance entry newly appended by `remember`, `relate`, or
  `update.patch.add_provenance` deposits the evidence reward, decayed from that
  entry's `observed_at` (or now when absent; future times never amplify). Replaying
  history in any order therefore yields the same heat as live writes. Re-sending
  an entry that is already present deposits nothing.
- `reinforce` deposits an explicit amount, for example after a client used the
  memory in an answer. It is a mutation: it needs the `write` permission and an
  `idempotency_key`, creates a new version, and returns the record (operation
  `reinforce`) or relationship (operation `reinforce_relationship`) with a
  receipt.

```json
{
  "contract_version": "v1",
  "idempotency_key": "usage:answer-981:node--3",
  "operation": "reinforce",
  "input": {
    "target": {"kind": "memory", "id": "node--3"},
    "amount": 1.0,
    "reason": "used in an answer"
  }
}
```

`amount` is a non-zero finite number in `[-1000, 1000]`; a negative amount is a
penalty and heat never goes below zero. `reason` is optional.

Records and relationships expose `heat` (decayed to the response time) and
`heat_updated_at` (the last deposit; `null` for records written before heat
existed, whose heat is derived from their provenance with the same law).
Memories also expose `durable`.

Cold expiry is opt-in. When the server's cold threshold is greater than zero, a
memory that is not `durable` and whose current heat is below the threshold is
treated as expired by `recall`: it is not a seed, lexical match, or neighbour.
Its lifecycle is unchanged; `get` still returns it with `recallable: false`, and
a deposit or `durable: true` makes it recallable again. Set `durable` on
`remember` (absent keeps the stored flag on an upsert, `false` on creation) or
through `update.patch.durable`. Relationships are never cold-expired.

Recall ranks objective matches by match count, then by current heat, then by
engine id. Because every heat decays at the same rate, this order does not drift
with wall-clock time; the reported `score` is unchanged.

| Standalone variable | Default | Meaning |
| :--- | :--- | :--- |
| `CORROBORE_MEMORY_HEAT_HALF_LIFE_DAYS` | `21` | Half-life of heat, in days (positive, at most 36500). |
| `CORROBORE_MEMORY_HEAT_EVIDENCE_REWARD` | `1.0` | Reward per newly appended provenance entry (non-negative). |
| `CORROBORE_MEMORY_HEAT_COLD_THRESHOLD` | `0` | Cold-expiry threshold; `0` disables cold expiry. |

Embedded hosts pass the same policy with
`CorroboreEngine::builder().memory_heat_policy(MemoryHeatPolicy::new(..)?)`.

## Lookup by identity key or id

`get` is a read: it needs the `read` permission and no idempotency key. Its
input accepts any combination of four optional arrays, with between 1 and 100
non-blank values in total:

```json
{
  "contract_version": "v1",
  "operation": "get",
  "input": {
    "identity_keys": ["person:alice", "person:unknown"],
    "ids": ["node--7"],
    "relationship_identity_keys": ["alice-knows-bob"],
    "relationship_ids": []
  }
}
```

```json
{
  "contract_version": "v1",
  "result": {
    "operation": "get",
    "result": {
      "records": [
        {"id": "node--3", "identity_key": "person:alice", "lifecycle": "active", "recallable": true, "superseded_by": null, "version": 4},
        {"id": "node--7", "identity_key": "person:al", "lifecycle": "superseded", "recallable": false, "superseded_by": "node--3", "version": 2}
      ],
      "relationships": [
        {"id": "rel--1", "identity_key": "alice-knows-bob", "lifecycle": "active", "recallable": true, "version": 1}
      ],
      "missing": [{"field": "identity_keys", "value": "person:unknown"}]
    }
  }
}
```

Records and relationships are returned in request order (identity keys before
ids), each at most once, as complete typed records (abbreviated above). Expired
and superseded records stay visible with their `lifecycle`, so a client can
follow `superseded_by` to the canonical memory. Tombstoned and application-deleted
records, unknown values, and records of another workspace all appear in
`missing` with the field they were requested in; the response never reveals
that another workspace owns a value. A lookup indexes the trusted workspace
projection once, so its cost does not grow with the number of requested keys.

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
and enforced independently. `recall` and `get` require `read`; `reinforce` requires `write`. Stable version-one error codes are:

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

A consolidation retains a fusion lineage: every atomic origin, the sources each
one cites, and the authority it contributed. Authority is
`min(asserted confidence, granted authority)` per cited source under the
optional `authority_policy` (a registered WS-D source authority policy named by
`version`, `authority_domain`, and `predicate_class`), and the fused authority
is the maximum over active origins, never a sum. Remembering the same weak
observation repeatedly yields the authority of remembering it once. Without a
policy the fusion carries back-pointers and no justified authority; a named
policy that is not registered fails the operation. `revoked_source_ids`
withdraws sources and recomputes the interpretation while every origin stays
enumerated and every original memory survives. Both fields are additive to `v1`,
omitted when unused, and part of the proposal identity, so a revocation is its
own governed decision that an earlier approval never covers. See
[Agentic Platform Foundations](agentic-platform.md#memory-fusion-back-pointers-and-the-authority-cap).

### Merging evidence into the canonical memory

`merge_provenance: true` (default `false`, omitted when false) makes an approved
apply also merge the originals into the canonical memory, in the same commit and
receipt:

- every original's provenance is appended to the canonical memory,
  deduplicated (canonical entries first, then originals in id order), with each
  entry's recorded time, so evidence statistics stay exact;
- the originals' current heat is added to the canonical memory's, tags are
  unioned, and the canonical memory becomes `durable` if any original is;
- each active relationship of a non-canonical original is re-pointed to the
  canonical memory. When the canonical memory already has an equivalent
  relationship (same kind, same direction, same other endpoint), the original's
  provenance and heat merge into it; otherwise an equivalent relationship is
  created. The original relationship becomes `superseded` with lineage to its
  replacement, and its identity key moves to a replacement that has none.
  Relationships between consolidated memories (which would become self-loops)
  are left untouched.

The result reports `merged_provenance_count` and `repointed_relationships`
(`original_id`, `replacement_id`, `merged_into_existing`); a proposal reports
what would merge without writing (`replacement_id` is `null` where a new
relationship would be created). Canonical content and confidence are not
modified. `merge_provenance` is part of the proposal identity, so an approval
for a plain consolidation never covers a merging one.

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

See also [Agent Memory Lifecycle](agent-memory-lifecycle.md),
[HTTP Server](http-server.md), [Embedded Engine](embedded-engine.md), and
[Cypher Support](cypher.md).

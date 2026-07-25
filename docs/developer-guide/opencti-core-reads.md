# OpenCTI core reads and graph operations

Issue #44 implements the fundamental OpenCTI read surface on the versioned
Knowledge Data Engine contract. It uses the pinned OpenCTI `7.260722.0` model
and the issue #38 parity corpus as the compatibility reference.

## Supported reads

The embedded Corrobore provider supports:

- point reads by graph ID, OpenCTI internal or standard ID, historical STIX ID,
  alias, deduplication ID, or external-reference ID;
- bounded lists and counts by entity type with conjunctive equality,
  inequality, existence, and scalar or temporal range filters;
- stable multi-key ordering with canonical ID as the final tie-breaker;
- HMAC-protected cursor pagination bound to the normalized query, schema,
  result snapshot, last ordered record, and original result limit;
- incoming, outgoing, or bidirectional neighbors;
- breadth-first traversal and subgraph projection with relationship-type,
  neighbor-type and neighbor-property filters.

Responses use the canonical OpenCTI ID and raw OpenCTI body. Graph responses
also include seed-first path provenance containing the canonical node and
relationship revisions used by the result.

Full-text search, nested boolean planning, aggregations, and production traffic
routing remain outside this issue.

## Access semantics

Authorization is evaluated during candidate selection, before values enter a
response or payload cache. System-role callers may read every record. Other
callers are checked against the mapped OpenCTI marking, organization, tenant,
authorized-member, creator, owner, authority and sharing-policy metadata. A
direct lookup of an inaccessible existing record is indistinguishable from a
missing identifier; collection and graph reads omit inaccessible nodes and
relationships without exposing their identifiers, topology or properties.

Pagination tokens and resident caches are bound to the normalized access
policy. Every graph path requires access to all nodes and relationships. See
[OpenCTI query authorization](opencti-query-authorization.md) for the complete
decision, invalidation, audit and shadow-enforcement contract.

Unknown, empty and tombstoned indexed lookups return provider-neutral empty
results without scanning or paging unrelated payloads.

## Persistent access paths

The canonical store persists compact metadata with each atomic node mutation:

- identifier to current node;
- label to current node;
- canonical scalar property value to current node;
- canonical temporal value to current node;
- payload-free node and relationship access policy documents;
- incoming and outgoing typed adjacency.

Updates replace stale metadata entries and tombstones remove them. On startup,
these indexes are recovered before any graph payload is hydrated. The typed
Knowledge Data preparation boundary maps each operation to the smallest
projection: point reads use the identifier index, filtered reads intersect
label/property/temporal indexes, and graph reads expand persistent adjacency
from the requested seeds.

## Deterministic bounds and errors

Traversal depth is limited to `1..=8`. Every graph request must declare non-zero
result, expansion, and supernode bounds. Exceeding an expansion budget returns
`QUERY_BUDGET_EXCEEDED`; an unguarded high-degree expansion returns
`SUPERNODE_EXPANSION_BLOCKED`; missing mandatory limits return
`UNBOUNDED_OPERATION`.

Pagination never silently continues across a changed result set. A valid token
whose declared snapshot has changed returns `STALE_PAGINATION_TOKEN`, preventing
duplicates and omissions.

## Validation and observability

`opencti_core_reads` validates exact corpus IDs, properties, direction, type,
count, ordering, access filtering, pagination, graph provenance, and safety
limits. `opencti_authorization` validates the OpenCTI policy and
non-inference contract. `opencti_read_indexes` and `opencti_access_pushdown`
validate cold indexed reads, bounded persistent adjacency, access selection
before page-in, policy cache invalidation, and unknown/tombstoned behavior.

Prometheus exposes request count, P50/P95/P99 latency, records examined,
payload page-ins, and cache hits with `query_class` as the only label:

- `corrobore_opencti_core_reads_total`;
- `corrobore_opencti_core_read_latency_ms`;
- `corrobore_opencti_core_read_records_examined_total`;
- `corrobore_opencti_core_read_page_ins_total`;
- `corrobore_opencti_core_read_cache_hits_total`.

The shadow comparison surface from issue #43 remains the migration gate. Core
reads are not routed to production by this implementation.

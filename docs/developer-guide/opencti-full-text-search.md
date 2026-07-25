# OpenCTI full-text search

Issue #46 implements the `read.full-text` compatibility subset behind the
backend-neutral Knowledge Data Engine `search` operation. It does not accept
Elasticsearch/OpenSearch Query DSL, arbitrary analyzers, aggregations or file
content. Those boundaries remain assigned to issues #47 and #48.

## Request contract

`SearchRequest.expression` accepts either a string or an object:

```json
{
  "text": "documentation domain",
  "mode": "term",
  "fields": ["name", "description"],
  "types": ["indicator"],
  "filters": [
    {"field": "pattern_type", "value": "stix"}
  ],
  "cursor": null
}
```

The supported modes are `term`, `phrase`, `prefix`, and `fuzzy`.
`fuzziness` is restricted to edit distance 1 or 2. A fuzzy query may set
`prefix: true`. Fields, types and exact filters are conjunctive restrictions;
all text terms are conjunctive as well.

Unknown expression keys are rejected. The `content` field is rejected because
file extraction and file-content search belong to #48.

## Analysis and ranking

The index uses Tantivy's default Unicode-aware tokenization and lowercase
normalization. It deliberately applies no language-specific stemming, synonym
expansion or locale-dependent collation. Every textual `opencti.field.*`
property except `content` is indexed. Objects and relationships share the same
abstraction but keep their record class, OpenCTI kind, canonical identifier and
revision.

Ranking uses Tantivy relevance scores with field boosts:

- `name`: 3.0
- `aliases` and `x_opencti_aliases`: 2.0
- every other searchable field: 1.0

Scores sort descending. Equal scores always sort by canonical identifier
ascending, which makes page boundaries deterministic.

The annotated relevance fixture is
`compatibility/opencti/7.260722.0/full-text-relevance.json`. Acceptance requires
MRR@10 of at least 0.90.

## Authorization and cursors

Each indexed document stores only compact `opencti.access` metadata alongside
search fields. The shared OpenCTI policy evaluates candidates before totals and
pages are created. Denied identifiers and payload values are absent from the
result.

Continuation cursors are HMAC-authenticated and bind:

- the normalized query;
- the canonical index generation;
- the policy version and a pseudonymous policy fingerprint;
- the final score and canonical-ID ordering key.

A cryptographically random cursor key is generated once per persistent store,
saved with owner-only permissions, and reused across restarts.

A mutation, rebuild, query change or policy change therefore rejects an old
cursor as incompatible instead of continuing from an unsafe boundary.

## Lifecycle and consistency

Canonical mutations invalidate the published generation before the WAL-backed
graph commit, then synchronously rebuild the derived generation. A failed
derived rebuild cannot make stale results appear ready: the canonical commit
remains acknowledged, the invalidation marker remains durable, and the next
search retries reconstruction from canonical data. The generation fingerprint
is deterministic over sorted canonical IDs, revisions, fields and access
metadata, so replaying the same state is a no-op.

Rebuild writes to `search/full-text-v1/staging`, commits checkpoint progress,
and publishes by directory rename only after every canonical current record is
indexed. An interrupted staging generation reports `building`; it never
becomes queryable. Missing or corrupt Tantivy metadata reports
`rebuild_required` and the standalone store reconstructs the index from
canonical node and relationship logs.

The declared guarantee is read-your-writes for an acknowledged canonical
mutation and snapshot-stable cursor paging within one unchanged index
generation.

## Resource gates

The standalone store configures a 50 MB Tantivy writer budget and a hard bound
of 100,000 matched candidates before authorization. Queries exceeding the
candidate budget fail explicitly rather than returning a partial total.

The reproducible release-profile benchmark uses the PRD small profile of
100,000 objects plus 500,000 relationships, 20 warmup queries and 60 measured
phrase queries. The recorded run reached 1.729 ms P95 against the strict
6.1608 ms parity ceiling derived from the OpenSearch 3.7.0 reference. It used a
1,813,954,560-byte resident working set (2,019,147,776-byte maximum RSS) and a
21,154,143-byte index. Exact inputs and measurements are stored in
`compatibility/opencti/7.260722.0/full-text-benchmark-results.json`.

Reproduce it with:

```bash
cargo run --release -p opencti-search \
  --example small_profile_benchmark --locked
```

The benchmark removes its temporary index automatically on both success and
failure.

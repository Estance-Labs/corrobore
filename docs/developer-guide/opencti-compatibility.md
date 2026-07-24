# OpenCTI compatibility inventory

This document is the Phase 0 compatibility baseline for replacing the
Elasticsearch/OpenSearch dependency used by OpenCTI. It is an inventory and a
parity contract, not a provider implementation.

## Pinned reference

The inventory is reproducible against:

- OpenCTI `7.260722.0`, commit
  `e41adc1c3fd98a849602db33dbe550f689fe6d83`;
- Elasticsearch `8.19.18`;
- OpenSearch `3.7.0`.

The exact repositories, images and source locations are in
`compatibility/opencti/7.260722.0/source-lock.json`. The OpenCTI versions of
Elasticsearch and OpenSearch come from its development Compose and Dockerfile,
not from an inferred compatibility range.

## What exists in Corrobore today

The current code already provides useful foundations:

- `graph-core` owns typed nodes, relationships, versions, bitemporal facts,
  snapshots and a bounded working-set model;
- `graph-storage` contains append-only records, catalog rebuild, atomic mutation,
  WAL, checkpoint, recovery and compaction primitives;
- the embedded engine and HTTP server expose policy-checked Cypher reads and
  writes;
- the HTTP server can load and persist a graph through an `EnginePersistence`
  adapter and exposes a STIX import endpoint.

The gap to OpenCTI parity remains material:

- the public engine surface is Cypher-oriented and does not implement the
  `KnowledgeDataEngine` operations from the PRD;
- the current STIX importer maps a small property subset and maps unsupported
  object types to `Identity`;
- the persistent standalone server uses the WAL-backed, record-level paged
  store; OpenCTI-specific lookup/query semantics remain separate adapter work;
- there is no embedded full-text index, OpenCTI security policy adapter, file
  extraction worker, aggregation planner or OpenCTI routing/shadow provider.

These are delivery inputs for later issues. Issue #38 deliberately records the
target surface without implementing it.

## Machine-readable bundle

The bundle under `compatibility/opencti/7.260722.0/` contains:

| File | Purpose |
| --- | --- |
| `source-lock.json` | Exact upstream and engine versions |
| `operations.json` | Logical operation classes and their PRD, criticality, security, ordering, lifecycle and contract metadata |
| `knowledge-data-engine-mapping.json` | Exhaustive mapping from the 32 logical operations to the versioned provider contract or an explicit unsupported boundary |
| `catalogue.json` | 612 production callsites from 183 OpenCTI source files |
| `parity-corpus.json` | Fully synthetic objects, relations, access controls, files and lifecycle fixtures |
| `reference-results.json` | Canonical expected IDs, properties, ordering, cursors, aggregations, authorization and errors |
| `decisions.json` | Accepted Phase 0 architecture decisions |
| `benchmark-profiles.json` | Dataset, hardware, warmup and measurement protocol |
| `benchmark-results.json` | Elasticsearch/OpenSearch small and medium reference measurements |

Test and fixture directories are excluded from callsite discovery. The scanner
includes:

1. calls to helpers imported from OpenCTI's `database/engine` and
   `database/file-search` modules;
2. direct client calls in those modules, including casted Elasticsearch and
   OpenSearch clients.

Every discovered symbol must belong to exactly one operation. A new imported
helper or direct client method therefore fails generation as **unmapped**, while
a changed line, deletion or classification fails verification as missing or
stale. Each operation also names its downstream delivery issue: #39 for the
provider lifecycle, #44 for core reads, #46 for full-text, #47 for advanced
queries, #48 for files, #50 for transactional writes, #51 for merge/reconciliation
and #52 for migrations and maintenance.

## Verify or update the inventory

Check out the pinned OpenCTI commit, then run:

```bash
node --test scripts/opencti-compatibility.test.mjs
node scripts/opencti-compatibility.mjs verify \
  --source /path/to/opencti/opencti-platform/opencti-graphql/src
```

When a reviewed upstream change intentionally alters the surface, regenerate
and inspect the diff:

```bash
node scripts/opencti-compatibility.mjs generate \
  --source /path/to/opencti/opencti-platform/opencti-graphql/src
```

Generation refuses unclassified symbols. Update `operations.json` with the PRD
mapping and contracts before generating again. CI independently checks out the
exact commit and runs both commands, so a hand-edited catalogue or changed
corpus hash cannot pass.

## Corpus and capture rules

The corpus is fully synthetic and uses only:

- `example.com`, `example.net` and `example.org`;
- IPv4 documentation ranges from RFC 5737;
- IPv6 `2001:db8::/32`;
- deterministic synthetic identifiers and timestamps.

The validator recursively rejects likely non-example email addresses, public IP
addresses, bearer tokens, API keys and secret-bearing fields. No production
capture may be committed directly. A future capture tool must normalize a real
request into this synthetic schema or keep the data outside the repository.

Canonical JSON orders object keys but preserves arrays. Arrays encode result
order, bucket order, pagination and mutation sequence and must never be sorted
as a formatting step.

## Accepted boundaries

The full rationale is machine-readable in `decisions.json`. The main decisions
are:

- keep the provider boundary in OpenCTI and the CTI mapping outside
  `graph-core`;
- capture at the logical provider boundary;
- require read-your-writes and transaction-versioned, snapshot-consistent
  pagination;
- put Tantivy behind a Corrobore-owned full-text abstraction;
- isolate file extraction in a worker while Corrobore owns the index;
- use one portable snapshot artifact for local and S3/MinIO storage;
- implement only the observed aggregation subset: count, cardinality, terms,
  date histogram, sum, filter, nested and reverse-nested;
- use the PRD small and medium volume profiles and defer large/distributed
  validation.

## Reference benchmark

The benchmark ran one engine at a time in the same Podman VM: 7 ARM64 vCPUs,
5,743,632,384 bytes of memory, no swap, XFS-backed overlay storage and a 2 GiB
JVM heap. Each index used one shard, zero replicas and disabled refresh during
initial ingestion. Twenty warmup iterations preceded sixty measured iterations
of each workload at concurrency one.

The workloads are get by ID, a marking/tenant/type filtered list, phrase
full-text search, search-after deep pagination, terms aggregation and date
histogram. The aggregate latency values below combine those six equally
weighted workloads.

| Engine | Profile | Documents | Ingest docs/s | P50 ms | P95 ms | P99 ms | Query ops/s | CPU | Heap | Disk |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Elasticsearch 8.19.18 | small | 600,000 | 46,812.735 | 4.107 | 5.063 | 5.736 | 247.641 | 16.706% | 1,477,616,160 B | 55,399,040 B |
| Elasticsearch 8.19.18 | medium | 6,000,000 | 65,598.335 | 3.175 | 3.977 | 4.228 | 317.176 | 16.740% | 792,184,832 B | 524,844,463 B |
| OpenSearch 3.7.0 | small | 600,000 | 42,048.865 | 4.110 | 5.084 | 5.672 | 250.822 | 15.826% | 826,802,176 B | 54,714,118 B |
| OpenSearch 3.7.0 | medium | 6,000,000 | 67,005.110 | 3.170 | 3.792 | 4.262 | 325.277 | 23.105% | 754,974,720 B | 882,200,664 B |

The medium latency being lower than small is not treated as a scaling claim:
these are single runs after ingestion, JIT compilation and warmup. The raw
per-workload figures, timestamps and deterministic dataset-manifest hashes are
kept in `benchmark-results.json`.

Reproduce the full four-cell matrix with:

```bash
scripts/opencti-reference-benchmark.sh all
```

The script also accepts a profile and engine for a focused rerun, for example:

```bash
scripts/opencti-reference-benchmark.sh small opensearch
```

The medium matrix indexes twelve million documents across both engines. It is
intentionally not part of pull-request CI; its committed results and
methodology are validated there, while explicit benchmark reruns remain a
controlled release activity.

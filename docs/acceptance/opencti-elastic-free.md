# Elastic-free OpenCTI acceptance

Status: in progress for issue #54. This matrix is a release gate, not a claim
that the distribution is certified before its exact-image workflow is green.

The supported target is OpenCTI `7.260722.0` from the Estance fork at commit
`cba9785b6b32093cfa645a1bacc9243c0d771260` (based on upstream commit
`e41adc1c3fd98a849602db33dbe550f689fe6d83`), Corrobore `0.2.2` or newer,
the certified `small` profile, and the conditional `medium` single-node
profile. Elasticsearch and OpenSearch
are absent from the shipped Compose model and `ELASTICSEARCH__*` variables are
rejected by the native provider.

The pinned provider retries Corrobore write-backpressure responses only for
canonical mutations carrying an idempotency key. Retries reuse the exact
payload and remain bounded by `CORROBORE__TIMEOUT_MS`; other POST failures stay
fail-closed.

## Required matrix

| Suite | Required evidence | Blocking gate |
| --- | --- | --- |
| `functional` | Pinned OpenCTI entity and relationship workflows | Exact normalized results |
| `dashboard` | Dashboard counts, facets and time series | Exact access-aware buckets |
| `export` | STIX and supported report exports | Stable complete output |
| `traversal` | Neighborhood, path and bounded subgraph queries | No omitted or unauthorized edge |
| `search` | Full text, phrase, fuzzy and filtered ranking | Accepted relevance corpus |
| `aggregation` | Count, cardinality, terms, histogram, sum and nested plans | Exact buckets and order |
| `file-content` | Supported PDF, office, text and HTML fixtures | Authorized results only |
| `bulk` | Atomic and partial bulk ingestion | Stable per-item outcomes |
| `merge` | Merge, deduplication, relationship movement and provenance | No lost reference or weakened marking |
| `concurrent-write` | Conflicts, idempotency and bounded backpressure | One durable outcome per key |
| `durability` | Forced kill between WAL boundaries and restart | No acknowledged mutation lost |
| `security` | Content, IDs, topology, counts, order, cursors and timing corpus | Zero disclosure divergence |
| `migration` | Import, catch-up, parity, shadow, canary, primary reads/writes and rollback | Every monotonic gate recorded |
| `operations` | Snapshot/restore, upgrade/rollback and index rebuild | Clean-instance recovery passes |
| `performance-small` | Read P95, writes, startup, CPU, memory, disk, snapshot and restore | All declared small thresholds pass |
| `performance-medium` | Reference results at 1M/5M are published; Corrobore measurements remain required | Conditional until the published Corrobore gate passes |

Run repository evidence with:

```bash
scripts/opencti-elastic-free-acceptance.sh contracts
scripts/opencti-elastic-free-acceptance.sh local
```

The exact deployed stack is the pinned upstream runtime gate. It builds the
Estance fork from its immutable commit, starts the complete distribution,
checks that OpenCTI initialized with `DATABASE_ENGINE=corrobore`, rejects any
`ELASTICSEARCH__*` runtime variable, and executes an authenticated GraphQL
administrator query through the native provider:

```bash
OPENCTI_CORROBORE_ENV_FILE=packaging/opencti-elastic-free/.env \
  scripts/opencti-elastic-free-acceptance.sh stack
```

`all` runs the contract, mapped Corrobore matrix, and exact runtime layers.
The CI gate runs the same layers; a provider-only unit test or a mocked OpenCTI
process does not satisfy certification.

## Resource comparison

The harness publishes `resource-evidence.json` with `service_count`,
`mandatory_configuration`, and normalized `memory_bytes`, plus per-container
CPU and disk evidence. Certification compares that artifact with the official
OpenCTI Docker model pinned at `99a52e27504318303f1adffc278c87c8e150ffc9`:
21 services and 40 declared configuration values. The conservative memory
baseline is `1,477,616,160` bytes, the measured Elasticsearch process alone in
the small reference profile; the complete reference stack necessarily uses
more. The target stack has seven
services: OpenCTI, its ingestion worker, Corrobore, the isolated file worker,
Redis, RabbitMQ, and MinIO.

## Completion conditions

Issue #54 remains open until the exact image build, stack, pinned runtime matrix,
forced-kill recovery, snapshot restoration, upgrade, rollback, index rebuild,
small gate, and explicit conditional medium gate are green in
`.github/workflows/opencti-elastic-free.yml`. The closing PR must reference
`Closes #54`; epic #37 closes only after that PR is merged and documentation is
updated with immutable workflow and release evidence.

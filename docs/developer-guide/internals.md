# Engine Internals

## Stable boundaries

- `graph-core` owns graph semantics and orchestration, not network connectors or model calls.
- `storage-api` prevents core types from depending on a concrete persistence backend.
- `shared-runtime` is the common policy, budget, request, response, and audit boundary for embedded and HTTP callers.
- `corrobore-http-server` adapts JSON/HTTP into typed runtime requests and owns transport security.
- `corrobore-ingest` is an external connector that uses the public import route.
- Domain and exporter crates depend on stable facades rather than private modules.

Runtime contracts are owned by this repository. Integration assets may live in
dedicated repositories, but they are expected to consume the same public
boundaries instead of depending on graph internals.

## Query pipeline

`cypher-parser` produces a typed AST and classifies the request. `cypher-planner` creates a deterministic logical plan. `cypher-executor` applies the plan to `Graph` under `ExecutionPolicy`. `shared-runtime::CypherGateway` adds request mode, mutation permission, validation, budget, and audit contracts around that pipeline.

The parser implements the subset listed in [Cypher Support](../user-guide/cypher.md). Unsupported clauses fail explicitly rather than being approximated.

## Graph and storage

`Graph` provides request-scoped node, relationship, traversal, temporal,
snapshot, claim, and semantic-seed operations. In persistent standalone mode,
`CanonicalEngineStore` is the source of truth: WAL intent and record-level
append logs are committed before acknowledgement, while catalog, label/type and
adjacency state are rebuildable projections. Startup restores metadata and
checkpoint/WAL state without hydrating graph payloads. The public engine asks
the file-backed pager for a bounded projection before each request and commits
only changed record versions afterward. Ephemeral and embedded callers can
continue using an in-memory `Graph`.

The persistent path deliberately separates:

- canonical node and relationship versions in append-only logs;
- durable transaction intent, applied markers and periodic checkpoints;
- compact catalog and adjacency projections;
- request-scoped hot payloads and warm adjacency metadata;
- cold records addressed by cataloged file offsets.

Checkpointing is periodic so a small mutation does not rewrite the full
catalog. Recovery loads the last safe checkpoint and replays newer committed
transactions idempotently. Compaction operates behind that checkpoint boundary
and never rewrites active node or relationship payload references.

### Deterministic simulation

`crates/graph-storage/tests/deterministic_simulation.rs` tests the persistent
store as a whole. Each universe starts from a seed and draws a sequence of
operations from a xorshift generator: writes on `main`, writes interrupted at
each `MutationCrashStage`, branch creation and branch writes, merges (with or
without deleting the source), history cleanup, reopening the store and
truncating the last history line. After every step it checks four oracles:

- the records visible on `main` and on each branch match an in-memory model;
- the commit history stays continuous: every parent exists and generations grow;
- the view of the `main` head equals what the canonical catalog holds;
- a write interrupted by a crash is either fully visible or absent after reopen.

A failure reports its seed and step; rerunning that seed replays the same
universe. Add new persistence operations to the simulator together with the
model change that describes their expected effect.

### Vector index selection

Vector search (`canonical_engine_store/vector_search.rs`) goes through the
`VectorIndex` trait. `CanonicalStoreOptions::vector_index`, a
`VectorIndexPolicy`, picks the implementation each time an index is built or
moved forward to a commit:

- `ExactVectorIndex` compares the query with every vector. It answers every
  metric and is the default below `hnsw_min_vectors` (10,000) vectors, where a
  scan costs less than building a graph.
- `HnswVectorIndex` (`vector_search/hnsw.rs`) is a Hierarchical Navigable Small
  World graph, implemented in the crate without a new dependency, with
  `m = 16`, `ef_construction = 128` and `ef_search = 128` by default. A graph
  is built for one metric. A query with another metric is answered by an exact
  scan of the same vectors, and the store keeps one graph per metric that has
  been queried. A query asking for at least as many hits as there are vectors,
  as a label-filtered search does, is also answered exactly.

`auto` mode switches to HNSW once an index reaches the threshold and back to
exact below it. `exact` and `hnsw` force one implementation. The server reads
the policy from `CORROBORE_STORAGE_VECTOR_INDEX` and
`CORROBORE_STORAGE_VECTOR_HNSW_MIN_VECTORS`.

The graph is deterministic. A vector's layer comes from a hash of its record
id and a fixed seed, and the store inserts vectors in id order. Every heap and
sort breaks ties by insertion slot, so the same operations build the same graph
and return the same hits. A removal leaves a tombstone, which still routes
searches but is never returned. A replaced vector is tombstoned and inserted
again. Once tombstones outnumber the live vectors, the graph is rebuilt from
the live vectors. Hits are rescored with the exact similarity, so HNSW and
exact scores are comparable.

`crates/graph-storage/tests/hnsw_vector_index.rs` measures recall@10 against
the exact index on 5,000 seeded 64-dimension vectors and requires at least
0.95 for cosine and L2. `cargo bench -p graph-storage --bench vector_search`
compares the query latency of both implementations.

## Working-set subsystem

The subsystem is split by responsibility:

- `working_set` defines the bounded data model;
- `working_set_manager` owns lifecycle and tracking;
- `working_set_expansion`, `expansion_budget`, `graph_pager`, and loading profiles handle bounded loading;
- `semantic_seed` resolves objectives into candidate starting nodes;
- `working_set_telemetry` records retrieval decisions and outcomes;
- `pheromone_trace` and `anti_pheromone` maintain positive and negative task-scoped navigation fields;
- `bandit_controller` defines explicit actions, context, reward, and controller interfaces;
- `working_set_benchmark` runs comparable policies on reproducible workloads.

Safety budgets and supernode guards remain deterministic even when a learned controller is attached.

## HTTP runtime

`AppState` owns the gateway, session runtime, configuration, and process start time. Protected routes share constant-time Bearer authentication, a global token-bucket limiter, and request-body limits. STIX import routes have a separate larger limit. Query work runs in blocking tasks behind a request timeout. Session state is persisted, transitions through a small FSM, and expires only when an opt-in idle TTL is configured.

Structured tracing omits request headers. Cypher audit input and output events include a shared audit event id, and session-log reads calculate parity between both sides.

## Native domain-provider runtime

`domain-provider-abi` owns the language-neutral ABI constants, C-compatible layouts, and JSON schemas. Its canonical C header is the cross-repository contract every pack implements, including the in-workspace `medical` and `research` providers. The exported table is prefix-versioned: the host reads the fixed header, rejects incompatible major/minor or undersized tables, and only then reads function pointers. No Rust allocation or dynamic Rust type crosses this boundary.

`domain_providers::manifest` parses a strict, unknown-field-denying deployment manifest. `domain_providers::registry` canonicalizes the trusted root and each library, rejects path escape, verifies SHA-256 before `dlopen`, negotiates ABI and capabilities, validates metadata identity and operational limits, creates one instance, and runs health before `AppState` is returned. Required-provider failures abort startup. Optional entries may be absent, but present optional libraries must still validate fully.

The registry owns each `Library` for longer than its handle, destroys handles before unload, and releases every provider output through the same table's `free_buffer`. ABI v1 serializes calls per provider with a mutex even when metadata declares thread safety; request and response sizes are bounded by provider metadata, and HTTP handlers add an outer timeout. Providers must contain language panics/exceptions because unwinding across C is forbidden.

Capability envelopes are JSON and independently versioned. The host dispatches `node.validate/1` after checking, in order, that the domain is distributed, that its provider is loaded, and that it declares the capability. ABI minor 2 also recognizes `claim.verify/1`: startup wraps each declaration as a `Verifier`, derives determinism from provider metadata with a non-deterministic default, and leaves record provenance and verdict precedence in `graph-core`. ABI minor 1 providers remain loadable and simply register no domain verifier. Unknown capability declarations remain visible for forward-compatible status reporting, but dispatch fails closed. Every response must preserve schema version and `request_id`. See [Domain Provider Runtime Contract](../user-guide/domain-provider-operations.md) for the public runtime boundary and diagnostic surfaces.

## Architecture records

The `project-documents/adr/` directory is the source for design decisions. Indexed `project-documents/feature-*` directories contain both completed and candidate work. Do not infer implementation status from an artifact alone; confirm exported types, runtime wiring, and tests.

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

## Enterprise domain-provider runtime

`domain-provider-abi` owns the language-neutral ABI constants, C-compatible layouts, and JSON schemas. Its canonical C header is the cross-repository contract consumed by CTI, FIMI, and Crisis implementations. The exported table is prefix-versioned: the host reads the fixed header, rejects incompatible major/minor or undersized tables, and only then reads function pointers. No Rust allocation or dynamic Rust type crosses this boundary.

`enterprise::manifest` parses a strict, unknown-field-denying deployment manifest. `enterprise::registry` canonicalizes the trusted root and each library, rejects path escape, verifies SHA-256 before `dlopen`, negotiates ABI and capabilities, validates metadata identity and operational limits, creates one instance, and runs health before `AppState` is returned. Required-provider failures abort startup. Optional entries may be absent, but present optional libraries must still validate fully.

The registry owns each `Library` for longer than its handle, destroys handles before unload, and releases every provider output through the same table's `free_buffer`. ABI v1 serializes calls per provider with a mutex even when metadata declares thread safety; request and response sizes are bounded by provider metadata, and HTTP handlers add an outer timeout. Providers must contain language panics/exceptions because unwinding across C is forbidden.

Capability envelopes are JSON and independently versioned. The host dispatches `node.validate/1` after checking, in order, build feature, license claim, loaded domain, and declared capability. ABI minor 2 also recognizes `claim.verify/1`: startup wraps each declaration as a `Verifier`, derives determinism from provider metadata with a non-deterministic default, and leaves record provenance and verdict precedence in `graph-core`. ABI minor 1 providers remain loadable and simply register no domain verifier. Unknown capability declarations remain visible for forward-compatible status reporting, but dispatch fails closed. Every response must preserve schema version and `request_id`. See [Domain Provider Runtime Contract](../user-guide/domain-provider-operations.md) for the public runtime boundary and diagnostic surfaces.

## Architecture records

The `project-documents/adr/` directory is the source for design decisions. Indexed `project-documents/feature-*` directories contain both completed and candidate work. Do not infer implementation status from an artifact alone; confirm exported types, runtime wiring, and tests.

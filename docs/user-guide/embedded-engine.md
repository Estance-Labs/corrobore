# Embedded Engine

`corrobore-engine` embeds Corrobore in a Rust process without running an HTTP service. It uses the same `shared-runtime` policy, budget, validation, and executor path as the server.

See [Deployment Modes](deployment-modes.md) to compare this in-process model
with the HTTP and standalone service layers.

This guide targets the current `0.1.x` runtime baseline.

## Basic use

```rust
use corrobore_engine::{CypherResponseData, CorroboreEngine};

let mut engine = CorroboreEngine::strict_default();
engine.write("MERGE (n:Campaign {name: 'acme phishing'})")?;

let response = engine.read("MATCH (n:Campaign) RETURN n")?;
assert!(matches!(response.data, CypherResponseData::Records(_)));
# Ok::<(), corrobore_engine::EngineError>(())
```

`execute` auto-detects reads and mutations. `read` and `write` force the request mode. Each also has a `*_with_params` variant accepting `HashMap<String, String>`.

Use explicit `read` or `write` in production paths where accidental mode
switching would be risky.

## Configure identity and policy

```rust
use corrobore_engine::CorroboreEngine;

let mut engine = CorroboreEngine::builder()
    .workspace_id("workspace--case-42")
    .session_id("session--analyst-7")
    .budget_ref("budget--interactive")
    .read_only(true)
    .build()?;

let response = engine.write("CREATE (n:Indicator {value: 'blocked'})")?;
assert_eq!(format!("{:?}", response.status), "Rejected");
# Ok::<(), corrobore_engine::EngineError>(())
```

Advanced callers can provide `RuntimePolicy`, `RuntimeBudget`, and `ExecutionPolicy` values through the builder. A read-only engine returns a rejected runtime response for mutations; it does not silently execute them.

Set `workspace_id`, `session_id`, and `budget_ref` consistently so embedded
audit and policy behavior stays aligned with HTTP workflows.

## Semantic seed search

Resolve a natural-language objective into ranked graph nodes before expanding a working set:

```rust
let seeds = engine.seed_search("phishing campaign", 5)?;
for candidate in seeds.seed_candidates() {
    println!("{} {}", candidate.node_id(), candidate.score());
}
# Ok::<(), corrobore_engine::EngineError>(())
```

`seed_search` uses the cross-domain profile and hybrid retrieval. `seed_search_with_request` accepts a fully typed `SemanticSeedQueryRequest` for domain, retrieval mode, threshold, workspace, and `top_k` control.

## Deterministic STIX export

```rust
use corrobore_engine::{StixExportOptions, CorroboreEngine};

let engine = CorroboreEngine::strict_default();
let bundle = engine.export_stix_bundle(&StixExportOptions::default())?;
println!("{}", bundle.objects.len());
# Ok::<(), corrobore_engine::EngineError>(())
```

`StixExportOptions` records a logical snapshot id, transaction id, exporter version, profile, and strict/permissive mode. The export is built from the current in-memory graph; this is not historical snapshot rollback.

## Current boundary

The facade exposes an immutable `graph()` view. A
`CorroboreKnowledgeDataProvider` can borrow the same engine to serve the
versioned Knowledge Data Engine contract without duplicating state. This keeps
the existing Cypher methods compatible while product adapters migrate to typed
initialize, health, read, graph, write, snapshot and maintenance operations.

The provider currently implements the lifecycle and bounded graph reads
documented in the [Knowledge Data Engine contract](../developer-guide/knowledge-data-engine.md).
Other typed operations fail explicitly as unsupported until their delivery
issues implement them.

The facade does not wire Python bindings or validate-only execution. Issue #228
tracks the shared-runtime validate-only defect, so the facade intentionally
omits a `validate()` method.

For authenticated transport features, session log retrieval, or admin license
status endpoints, use `corrobore-http-server`.

# Ingestion

`corrobore-ingest` polls one TAXII 2.1 collection, follows envelope pagination,
and persists the collection cursor. The simple TAXII path remains available
through `POST /v1/import/stix`. Consistent OpenCTI snapshots and ordered
catch-up streams use `POST /v1/opencti/sync/batches`, which commits a bounded
batch as one WAL transaction and acknowledges its source checkpoint afterward.

This guide describes the current runtime contract. Check `GET /version` for the
deployed release before relying on version-specific behavior.

## Candidate ingestion (WS-C)

Document and model extraction use a candidate-first contract. OCR, extraction
and re-extraction remain external to `graph-core`; the runtime retains proposals,
checks the supplied constraints and records explicit review decisions. Existing
TAXII/STIX import is a separate structured-data integration surface, not a way
for agents to bypass this extraction workflow.

The loop is **submit, read the failing constraint, re-extract that field,
resubmit**:

1. Submit `id`, `extraction_run_id`, `actor`, exact `raw_payload` and `constraints`
   through `POST /v1/import/candidates`. The default tier is `Shadow`;
   `Hypothesis` is also accepted. Submission creates no canonical node or edge.
2. Read `validation.valid` and every entry in `validation.failures` from the
   response or `GET /v1/import/candidates/{id}`. HTTP success means the proposal
   was retained, not that it passed validation. Feedback preserves the precise
   RFC 6901 `field`, complete `constraint`, observed value and repeated-failure
   information.
3. Revisit only the implicated source span. Re-extract the failing field while
   retaining unaffected assertions and their evidence. Constraints cover JSON
   syntax, required fields, value types, cardinality, temporal order and allowed
   predicates. They are supplied by the caller; this is not an automatic semantic
   verifier or a general entity-existence check.
4. Resubmit through `POST /v1/import/candidates/{id}/repairs` with a new ID and
   `caused_by` containing the failing constraint IDs. The original raw payload,
   extraction runs, inherited constraints and predecessor link survive. Exact
   retries are idempotent; changed raw content requires a new version ID.
5. Keep unresolved candidates in their noncanonical tier. After source review,
   an authorized `POST /v1/import/candidates/{id}/promote` supplies an actor,
   reason and explicit reviewed node or relationship. Failed constraints block
   promotion atomically; passing constraints never promotes automatically.

A property named `status: candidate` on a graph node is not a `Shadow` tier.
Generic promotion does not infer arbitrary raw-JSON-to-domain mappings. Reviewers
must supply a faithful record with the required domain metadata, including each
relationship's own evidence and confidence.

See the [HTTP payloads and errors](http-server.md#post-v1importcandidates) and the
[agent's executable submission/repair recipe](https://github.com/Estance-Labs/corrobore/blob/main/plugins/corrobore/skills/corrobore/references/candidate-ingestion.md).
The portable MCP adapter does not yet expose these candidate routes; use an
authorized HTTP client rather than substituting generic memory or STIX writes.

### Evidence-backed reconciliation

`EntityMention` retains an observation, UTF-8 offsets, surface form and contextual
features independently from canonical `Entity`. Candidate entity references and
similarity scores are only hints. `ReconciliationRecord` retains `Merge`,
`Distinct` or `Abstain`, contextual citations pinned to source versions, rationale
and an actor or versioned verifier. The core records externally reviewed decisions;
it does not implement an automatic alias or transliteration classifier.

Applying a supported merge is explicit. It groups mentions in the governed
projection while retaining original member records and observation links.
An attributed undo restores separation, preserving the judgment and undo record.
Later dependent decisions block undo with a typed error naming the dependent
record; there is no silent cascade. See [HTTP inspection and undo](http-server.md#get-v1reconciliationsid)
and the [Cypher projection](cypher.md#reconciliation-decisions-ws-c).

### Quality measurements

The existing `/metrics` handler exports extraction accuracy, repair success,
false-repair rate, per-predicted-outcome reconciliation accuracy and abstain rate.
Reference evaluations are separate from constraint validity. Reviewed counts
expose coverage, unknown ratios are `NaN`, and an observed system that never
abstains reports zero abstention. Definitions and denominators are in
[ingestion quality metrics](http-server.md#ingestion-quality-ws-c).

### WS-C acceptance evidence

The release gate is [epic #192](https://github.com/Estance-Labs/corrobore/issues/192),
assembled by [issue #203](https://github.com/Estance-Labs/corrobore/issues/203).
The matrix includes transport tests in the HTTP crate because `graph-core`
does not depend on the HTTP server. Tests exercise the retained-record contract;
the mini reference set does not certify an external extraction model.

| Epic criterion | Executable evidence |
| --- | --- |
| Spike B: schema, temporal and entity-field violations blocked before canonical promotion | [`spike_b_schema_temporal_and_entity_violations_never_reach_canonical`](https://github.com/Estance-Labs/corrobore/blob/main/crates/graph-core/tests/epic_0029_ws_c_acceptance.rs), covering both Shadow and Hypothesis; [candidate repair rules](https://github.com/Estance-Labs/corrobore/blob/main/crates/graph-core/tests/candidate_repair.rs). |
| Spike D: homonyms distinct, supported aliases/transliterations merged, ambiguous pairs abstain; similarity alone rejected | [`spike_d_aliases_transliteration_homonyms_and_abstention_use_evidence`](https://github.com/Estance-Labs/corrobore/blob/main/crates/graph-core/tests/epic_0029_ws_c_acceptance.rs), using the [mini reference set](https://github.com/Estance-Labs/corrobore/blob/main/crates/graph-core/tests/fixtures/reconciliation_aliases.json). |
| HTTP inspection/undo retains both records, dependent reversal refused | [HTTP restart and dependency tests](https://github.com/Estance-Labs/corrobore/blob/main/crates/corrobore-http-server/tests/reversible_merges.rs), [exact link restoration and ledger integrity](https://github.com/Estance-Labs/corrobore/blob/main/crates/graph-core/tests/reversible_merges.rs), and the WS-C restart scenario. |
| Repair success and false repair separate from extraction accuracy; abstain visible | [WS-C over-repair scenario](https://github.com/Estance-Labs/corrobore/blob/main/crates/graph-core/tests/epic_0029_ws_c_acceptance.rs), [unknown/partial labels and never-abstaining cases](https://github.com/Estance-Labs/corrobore/blob/main/crates/graph-core/tests/ingestion_metrics.rs), [Prometheus persistence tests](https://github.com/Estance-Labs/corrobore/blob/main/crates/corrobore-http-server/tests/ingestion_metrics.rs). |
| Exact field feedback enables targeted re-extraction and retained lineage | `published_agent_examples_reextract_only_the_failing_field_and_preserve_lineage` in the WS-C suite executes the packaged guide's JSON examples; [guidance contracts](https://github.com/Estance-Labs/corrobore/blob/main/scripts/ws-c-guidance.test.mjs) reject direct mutation recipes and broken candidate routing. |
| WS-A, WS-B and WS-D compatibility | The existing [WS-A](https://github.com/Estance-Labs/corrobore/blob/main/crates/graph-core/tests/epic_0029_ws_a_acceptance.rs), [WS-B](https://github.com/Estance-Labs/corrobore/blob/main/crates/graph-core/tests/epic_0029_ws_b_acceptance.rs) and [WS-D](https://github.com/Estance-Labs/corrobore/blob/main/crates/graph-core/tests/epic_0029_ws_d_acceptance.rs) suites run unchanged in workspace validation. |

The existing HTTP native-array/relationship-metadata test keeps its executable
API assertions. Its old assertions requiring a direct-write snippet in the agent
skill are replaced by the candidate guide's executable examples and guidance
contracts; the WS-A/WS-B/WS-D suites themselves are unchanged.

Run the combined acceptance checks:

```bash
cargo test -p graph-core --test epic_0029_ws_c_acceptance
cargo test -p corrobore-http-server --test reversible_merges --test ingestion_metrics --test candidate_ingestion
node --test scripts/ws-c-guidance.test.mjs
cargo test --workspace --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```

## STIX confidence boundary

STIX objects and STIX import annotations use `0..=100`. Corrobore normalizes
them into native `0..=1` graph metadata: `90` is stored as native 0.9. Cypher
and memory operations already use the native scale and must not receive the
unconverted STIX value.

Annotations are keyed by STIX ID, including Relationship SRO IDs. A relationship
needs its own evidence references and confidence; endpoint annotations do not
transfer to it.

## Deployment topology (multi-repo)

Corrobore uses a boundary-first model:

- the runtime (`corrobore-http-server`) is the core surface of this repository;
- connectors are integration surfaces that call the public HTTP API;
- integration assets can evolve in dedicated repositories without changing core runtime packaging.

In practice, this means ingestion remains an HTTP client of Corrobore, not an in-process extension of graph internals. The same import validations, auth controls, and audit trails apply whether data comes from manual imports or scheduled TAXII polling.

This repository still ships `corrobore-ingest` as the reference TAXII connector. Some integrations (for example XTM One assets) have moved to dedicated repositories; they should keep their own environment files and release cadence while targeting the same HTTP contracts.

## Configuration strategy

Use separate environment scopes per runtime component:

- Corrobore runtime env (server auth, limits, host/port) belongs to the runtime deployment.
- Connector env (TAXII source, polling, connector state) belongs to the connector deployment.

For local development from this workspace:

- `.env.sample` is intentionally runtime-first.
- Connector variables are documented here and can be stored in a connector-local env file (for example `.env.ingest.local`) instead of mixing them into the runtime `.env`.

This separation avoids coupling unrelated knobs, keeps secret handling scoped by component, and maps directly to multi-repo operations where runtime and connectors are released independently.

## Configuration

| Variable | Required | Default | Meaning |
| :--- | :---: | :--- | :--- |
| `CORROBORE_INGEST_TAXII_ROOT_URL` | yes | — | TAXII API root, without the collection path. |
| `CORROBORE_INGEST_TAXII_COLLECTION_ID` | yes | — | Collection to poll. |
| `CORROBORE_INGEST_CORROBORE_BASE_URL` | yes | — | Corrobore server base URL. |
| `CORROBORE_INGEST_CORROBORE_AUTH_TOKEN` | yes | — | Bearer token for Corrobore imports. |
| `CORROBORE_INGEST_TAXII_TOKEN` | no | — | TAXII Bearer token. |
| `CORROBORE_INGEST_TAXII_USERNAME` / `CORROBORE_INGEST_TAXII_PASSWORD` | no | — | TAXII Basic credentials; supply both. |
| `CORROBORE_INGEST_WORKSPACE_ID` | no | `workspace--ingest-taxii` | Workspace attached to imports. |
| `CORROBORE_INGEST_POLL_INTERVAL_MS` | no | `300000` | Delay between cycles. |
| `CORROBORE_INGEST_PAGE_LIMIT` | no | `100` | Requested TAXII page size. |
| `CORROBORE_INGEST_STATE_DIR` | no | `.corrobore-runtime/ingest` | Persisted cursor directory. |

Bearer and Basic TAXII credentials are mutually exclusive. No TAXII credentials means unauthenticated collection access.

## Before you run

- Ensure `corrobore-http-server` is reachable from the connector process.
- Ensure `CORROBORE_INGEST_CORROBORE_AUTH_TOKEN` matches the runtime Bearer token.
- Ensure `CORROBORE_INGEST_STATE_DIR` points to writable storage for cursor
	durability.
- Prefer a dedicated connector env file so ingestion credentials and polling
	settings are not co-mingled with runtime deployment variables.

## Run one cycle

Start `corrobore-http-server`, load connector variables, then:

```bash
set -a
source .env.ingest.local
set +a
```

Then run one deterministic cycle:

```bash
cargo run -p corrobore-ingest -- --once
```

Remove `--once` to poll until Ctrl+C:

```bash
cargo run -p corrobore-ingest --release
```

Use `--once` for deterministic CI/debug cycles and continuous mode for
production-like polling.

The connector sends `added_after` from the persisted cursor, follows `more`/`next` pagination, and advances the cursor only after Corrobore accepts the import. A failed import leaves the previous cursor in place, giving at-least-once delivery on retry. Empty TAXII envelopes do not call the import endpoint.

## Operational notes

- The connector caps a single cycle at 1,000 TAXII pages to prevent a malformed feed from looping forever.
- The cursor is isolated per collection id.
- Transient cycle failures are logged and retried on the next interval in loop mode.
- The Corrobore HTTP service and its auth token must be reachable from the connector process.
- Capture the OpenCTI high-water mark before exporting a consistent snapshot,
  then replay source mutations after that boundary in monotonic sequence order.
- Persist the response checkpoint before fetching the next batch. A lost
  response can be replayed safely; the server reports acknowledged records as
  `duplicate`.
- Do not enable shadow reads until the server status reports zero divergence and
  `shadow_reads_enabled: true`.

## Failure behavior summary

- Import failure: cursor is not advanced, so objects are retried later
	(at-least-once semantics).
- Empty TAXII page: no import request is emitted.
- Pagination loop anomalies: cycle hard-capped at 1,000 pages.

# Cypher Support

Corrobore implements a bounded, agent-oriented subset of Cypher. Queries are parsed, classified as read/mutation/mixed, planned deterministically, and executed under host policy and runtime budgets.

This guide describes the current runtime contract. Check `GET /version` for the
deployed release before relying on version-specific behavior.

## Supported clauses

| Clause | Parsed and executed | Notes |
| :--- | :---: | :--- |
| `MATCH`, `OPTIONAL MATCH` | yes | Node and relationship patterns with labels and properties. |
| `WHERE` | yes | Nested `AND`/`OR`, comparisons, `IN`/`NOT IN`, and `IS NULL`/`IS NOT NULL`. |
| `WITH`, `RETURN` | yes | Intermediate and final projection. |
| `DISTINCT` | yes | Projection deduplication. |
| `COUNT`, `SUM`, `AVG`, `MIN`, `MAX` | yes | Aggregate-only projections; missing numeric values are ignored. |
| `ORDER BY`, `SKIP`, `LIMIT` | yes | Multi-key deterministic result shaping and bounds. |
| `CREATE` | yes | Create nodes and relationships. |
| `MERGE` | yes | Match-or-create/upsert behavior. |
| `SET`, `REMOVE` | yes | Property mutation. |
| `DELETE` | yes | Tombstone matched records. |

Explicitly rejected: `DETACH DELETE`, `LOAD CSV`, `UNWIND`, `FOREACH`, `CALL APOC`, and `CALL DBMS`.

## Reads

```cypher
MATCH (c:Case {id: "case-123"})-[:MENTIONS]->(n:Narrative)
WHERE n.confidence >= 0.7
RETURN DISTINCT n
ORDER BY n.confidence DESC, n.name ASC
LIMIT 20
```

Membership accepts scalar and homogeneous list properties:

```cypher
MATCH (n:Indicator)
WHERE (n.score >= 10 AND n.tags IN ['c2', 'malware'])
   OR n.name = 'priority'
RETURN n.name, n.score
ORDER BY n.score DESC, n.name ASC
```

Aggregate and non-aggregate projections cannot be mixed without a grouping
contract. The current bounded subset therefore expects aggregate-only returns:

```cypher
MATCH (n:Indicator)
RETURN COUNT(n), SUM(n.score), AVG(n.score), MIN(n.score), MAX(n.score)
```

Use `/v1/cypher/read` or `CorroboreEngine::read` when the request must not mutate the graph. The runtime rejects a mutation sent through a read-only request.

## Writes

```cypher
MATCH (a:ThreatActor {name: "APT28"})
MATCH (e:EvidenceSpan {id: "span--123"})
MERGE (a)-[r:USES]->(m:Malware {name: "X-Agent"})
SET r.confidence = 0.82,
    r.evidence_refs = [e.id],
    r.status = "candidate"
RETURN a, r, m
```

Use `/v1/cypher/write` or `CorroboreEngine::write`. A write route does not override a host-level mutation prohibition; read-only deployments return a rejected response.

### Native confidence scale

Cypher uses the native 0..=1 scale for node and relationship confidence.
Use 0.9 for 90% STIX confidence. Values such as `90` are rejected with
conversion guidance; only the STIX import adapter accepts and normalizes the
`0..=100` scale.

Confidence and retained evidence belong to each assertion. A relationship does
not inherit either field from its source or target node.

## Epistemic projection (Epic 0029)

Governed evidence records live beside the graph in the epistemic stores:
sources, observations, claims with their evidence links, verification records,
verdicts, and state transitions. They are not graph nodes, so ordinary
`MATCH` does not see them. `Graph::epistemic_projection()` renders them as a
read-only graph in the epistemic vocabulary that any read query can traverse:

| Label(s) | Record | Key properties |
| :--- | :--- | :--- |
| `Source` | `Source` version | `source_id`, `source_version`, `source_uri`, `source_type`, `source_artifact_sha256`, `source_derived_from_legacy` |
| `Observation` | `Observation` | `observation_id`, `observation_source`, `observation_selector`, `observation_payload`, `observation_modality` |
| `Evidence` | `EvidenceRecord` | `evidence_id`, `evidence_source_ref`, `evidence_source`, `evidence_observation` |
| `Claim` | `Claim` | `claim_id`, `claim_status`, `claim_statement`, `proposition_*`, `verdict_state`, `verdict_lifecycle_projection`, `verdict_id`, `verification_coverage*` |
| `Verdict`, `Assessment` | `Verdict` | `verdict_id`, `verdict_claim`, `verdict_state`, `verdict_policy_version`, `verdict_valid_from`, `verdict_transaction_time`, `verdict_dimension_*`, `verdict_explanation`, `verdict_uncertainty_kind` |
| `VerificationRecord`, `Assessment` | `VerificationRecord` | `verification_id`, `verification_claim`, `verification_verifier_id`, `verification_verifier_version`, `verification_deterministic`, `verification_result`, `verification_coverage_class`, `verification_coverage_current` |
| `StateTransition`, `Decision` | `StateTransition` | `transition_id`, `transition_claim`, `transition_from_state`, `transition_to_state`, `transition_trigger` |

Relationships follow the vocabulary: `REPORTS` (source to observation), the
evidence-link kinds `SUPPORTS`, `REFUTES`, `CONTRADICTS`, `SUPERSEDES`,
`CONTEXT_FOR`, `DUPLICATES`, `DERIVED_FROM`, `DEPENDS_ON` (link source to
claim, carrying `evidence_link_*` properties), `ASSESSES` (verdict and
verification record to claim), and `DECIDES` (state transition to claim).

```cypher
MATCH (c:Claim) RETURN c.claim_id, c.verdict_state, c.claim_status ORDER BY c.claim_id ASC
MATCH (c:Claim) RETURN c.claim_id, c.verification_coverage, c.verification_coverage_unchecked
MATCH (o:Observation)-[:SUPPORTS]->(c:Claim) RETURN c.claim_id, o.observation_payload
MATCH (t:StateTransition) RETURN t.transition_claim, t.transition_from_state, t.transition_to_state
```

Dimension properties such as `verdict_dimension_evidence_sufficiency`,
`verdict_dimension_source_independence`, `verdict_dimension_contradiction_load` and
`verdict_dimension_actionability` are present only when known; absent is not zero.
`verdict_explanation` is a JSON object containing `dimensions`, `clusters`,
`uncertainty_kind`, authority provenance, actionability and hypotheses. Each
cluster includes member indices/references, dependency reasons and directional
weights. `verdict_hypothesis_set` remains the separate ordered JSON string
introduced by WS-D item 5.

```cypher
MATCH (v:Verdict) RETURN v.verdict_claim, v.verdict_dimension_evidence_sufficiency, v.verdict_dimension_actionability
MATCH (v:Verdict) RETURN v.verdict_id, v.verdict_uncertainty_kind, v.verdict_explanation
```

The uncertainty token is `ignorance`, `ambiguity`, `unresolved_conflict` or
`staleness`. If no cause is classified, the standalone token is absent and the
JSON payload contains null. This does not authorize action or export.

Node identifiers in the projection are generated; record identifiers are
properties. The projection is read-only: verdicts are computed by the engine
(`resolve_claim_verdict`) and no write clause reaches the epistemic stores.
`verdict_state` is the computed epistemic state; `claim_status` is the
lifecycle status the ADR-0016 projection table derives from it.

Verification coverage is a current, derived view rather than another stored
report. `verification_coverage` lists `mechanically_checked`,
`semantically_judged`, `unchecked`, or `failing` entries. The companion
`verification_coverage_mechanical`, `verification_coverage_semantic`, and
`verification_coverage_failing` properties name each verifier as
`<id>@<version>`; `verification_coverage_target` says whether the check
covered a structured proposition or a text-only statement. A failing entry
keeps `verification_deterministic` on its `VerificationRecord`, so callers
can distinguish an authoritative mechanical failure from an advisory semantic
failure. Older verification records remain queryable and carry
`verification_coverage_current = false` when a newer verifier version or run
has replaced them in the current view.

## Parameters and modes

HTTP requests accept a `params` JSON object alongside `query`. Each `$name` placeholder is resolved into a typed value at the position where it appears, so a parameter is never assembled into the query text and cannot contribute syntax.

JSON scalar types are preserved end to end:

| JSON value | Bound as | Usable where |
| :--- | :--- | :--- |
| string | text | property values, comparisons |
| integer number | integer | comparisons, `SKIP`, `LIMIT` |
| fractional number | decimal (lossless source text) | comparisons |
| boolean | boolean | comparisons |
| `null` | null | comparisons, `IS NULL` checks |

Arrays and objects have no scalar equivalent in the supported subset and are rejected with `UNSUPPORTED_PARAMETER_TYPE`.

Because types are preserved, a placeholder must match its position: `LIMIT $n` requires an integer, and binding the string `"10"` there is a rejected request rather than a query that silently returns the wrong rows. An undeclared placeholder is also rejected rather than dropped.

`POST /v1/cypher/execute` accepts `mode` values `read`, `write`, `validate`, or `auto`. `auto` detects mutation keywords. Validate-only mode currently has a known defect tracked in issue #228 and must not be relied on for mutation safety; use the explicit read route or a read-only policy.

## Runtime budgets

Every request runs under a budget. Most dimensions are enforced *while the query
runs* rather than measured afterwards, so an expensive query is stopped instead
of merely reported.

| Dimension | Default | Enforced |
| :--- | ---: | :--- |
| `max_query_length` | 8192 | before execution |
| `max_parameter_count` | 128 | before execution |
| `max_loaded_records` | 50000 | during matching, as rows are materialized |
| `max_returned_records` | 10000 | when the projection is built |
| `max_mutation_count` | 5000 | before any write is applied |
| `max_execution_time_ms` | 20000 | sampled during matching |
| `max_payload_bytes` | 4 MiB | after execution |

Two consequences are worth knowing:

- **A rejected mutation has changed nothing.** The mutation bound is checked
  against a projected upper bound once matching is complete and before the first
  write, so exceeding it can never leave the graph partially modified. Because
  the projection is an upper bound, a query close to the ceiling may be refused
  even though it would have stayed just under it.
- **`max_loaded_records` counts matched rows, not returned rows.** Matching
  materializes rows before `SKIP`/`LIMIT` applies, so a broad pattern with a
  small `LIMIT` is still charged for everything it matched. Narrow the pattern
  rather than relying on `LIMIT` to stay within the bound.

The execution-time bound is deliberately lower than
`CORROBORE_HTTP_REQUEST_TIMEOUT_MS` (20s against 30s by default) so a runaway
query stops itself before the transport gives up on it.

Exceeding a bound returns `QUERY_BUDGET_EXCEEDED` naming the dimension, the limit
and the value reached.

## Safety guidance

- Bound result sets and avoid broad traversals.
- Read before writing and prefer `MERGE` when identity may already exist.
- Treat runtime validation and policy failures as actionable results.
- Do not assume Neo4j extensions, APOC, procedures, CSV loading, or full openCypher compatibility.
- Use [semantic seed search](http-server.md#post-v1seedsearch) when an objective is known but graph ids are not.

## Canonical references

- [HTTP Server](http-server.md)
- [For LLM Agents](../for-llms.md)
- [OpenAPI specification](../api/openapi.yaml)

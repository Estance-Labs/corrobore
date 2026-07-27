# Cypher Support

Corrobore implements a bounded, agent-oriented subset of Cypher. Queries are parsed, classified as read/mutation/mixed, planned deterministically, and executed under host policy and runtime budgets.

This guide targets the current `0.1.x` runtime baseline.

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

# Cypher Support

Corrobore implements a bounded, agent-oriented subset of Cypher. Queries are parsed, classified as read/mutation/mixed, planned deterministically, and executed under host policy and runtime budgets.

This guide targets the current `0.1.x` runtime baseline.

## Supported clauses

| Clause | Parsed and executed | Notes |
| :--- | :---: | :--- |
| `MATCH`, `OPTIONAL MATCH` | yes | Node and relationship patterns with labels and properties. |
| `WHERE` | yes | Comparison and filtering expressions supported by the parser. |
| `WITH`, `RETURN` | yes | Intermediate and final projection. |
| `DISTINCT` | yes | Projection deduplication. |
| `COUNT`, `SUM`, `AVG`, `MIN`, `MAX` | yes | Aggregation. |
| `ORDER BY`, `SKIP`, `LIMIT` | yes | Deterministic result shaping and bounds. |
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
ORDER BY n.confidence DESC
LIMIT 20
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

HTTP requests accept a `params` JSON object alongside `query`. The shared runtime binds string-valued parameters outside quoted literals and escapes them as Cypher string literals; an undeclared placeholder rejects the request. The HTTP adapter validates JSON values before conversion.

`POST /v1/cypher/execute` accepts `mode` values `read`, `write`, `validate`, or `auto`. `auto` detects mutation keywords. Validate-only mode currently has a known defect tracked in issue #228 and must not be relied on for mutation safety; use the explicit read route or a read-only policy.

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

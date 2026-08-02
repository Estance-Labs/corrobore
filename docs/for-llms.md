# For LLM Agents

Corrobore is your external, structured working memory for intelligence work. Use it to store and retrieve entities, relationships, evidence, confidence, time, versions, and audit history instead of carrying a growing graph as JSON in context.

This guide targets the current `0.1.x` runtime baseline.

```text
You                                      Corrobore
read and reason                     ->   store and validate structured state
propose evidence-backed changes     ->   enforce policy and record mutations
ask focused graph questions         <-   return the bounded result you need
```

## Operating loop

1. Check `/health` when tool connectivity is uncertain.
2. Search for seed nodes from the task objective when ids are unknown.
3. Read the smallest subgraph that can answer the question.
4. Inspect source material and distinguish observation from inference.
5. `MERGE` new entities and relationships with evidence, confidence, and status.
6. Read back the changed subgraph.
7. Validate STIX before export and leave uncertain claims for human review.

## Request discipline

- Prefer explicit mode routes:
  - `POST /v1/cypher/read` for reads.
  - `POST /v1/cypher/write` for mutations.
- Use `session_id` when a session was started; this keeps audit traces and
  lifecycle state coherent.
- Treat runtime `Rejected` and `ValidationFailed` statuses as expected control
  signals, not transport failures.
- Keep query scope bounded (`LIMIT`, narrow patterns, explicit objective).

## Find a starting point

```http
POST /v1/seed/search
Authorization: Bearer <token>
Content-Type: application/json

{
  "objective": "find infrastructure linked to the phishing campaign",
  "domain_profile": "cti",
  "mode": "hybrid",
  "top_k": 5,
  "score_threshold": 0.2
}
```

Use the returned `node_id`, score, rationale, source references, and boundary notes to choose a defensible seed. Do not treat ranking as proof.

## Read before writing

```cypher
MATCH (c:Campaign {id: "campaign--42"})-[:USES]->(i:Infrastructure)
RETURN c, i
LIMIT 50
```

Query narrowly and include a bound. If no supported answer exists, report the gap rather than inventing one.

## Write evidence-backed intelligence

```cypher
MATCH (a:ThreatActor {name: "APT28"})
MATCH (m:Malware {name: "X-Agent"})
MATCH (e:EvidenceSpan {id: "span--123"})
MERGE (a)-[r:USES]->(m)
SET r.confidence = 0.82,
    r.evidence_refs = [e.id],
    r.status = "candidate"
RETURN r
```

- Prefer `MERGE` for identities that may already exist.
- Attach evidence and confidence to important assertions.
- Mark inference as `candidate`; do not silently promote it to fact.
- Use domain vocabulary from [Intelligence Domains](user-guide/domains.md).
- Never use unsupported clauses such as `DETACH DELETE`, `LOAD CSV`, `UNWIND`, `FOREACH`, `CALL APOC`, or `CALL DBMS`.

## Respect host boundaries

The host decides whether a tool is read-only or permits mutations. Use `/v1/cypher/read` for reads and `/v1/cypher/write` only when the task authorizes a graph change. A rejected mutation is a policy result, not a reason to bypass the gateway.

Named sessions make actions inspectable. When a workflow starts a session, pass its `session_id` on Cypher and import calls, inspect its health, and stop it when finished. Session logs expose input/output audit parity and can be filtered by time.

Do not bypass policy outcomes by switching route shape or query wording. If the
runtime rejects a mutation, return the rejection reason and request explicit
authorization.

## Validate and export

`POST /v1/stix/validate` accepts either an explicit bundle or current graph CTI nodes. For explicit bundles, built-in playbooks may correct supported missing fields; corrected objects are imported whenever at least one playbook runs. `valid` reports whether the validation pass found an error, not a second verdict after fixes. Inspect `issues`, `playbooks_applied`, `corrections_summary`, and `persistence`, then revalidate when you need a post-correction verdict.

Use `GET /v1/export/stix` for deterministic, CTI-scoped STIX projection after validation. The route fails closed unless the licensed CTI provider is ready. Strict mode returns named readiness failures; permissive mode returns bounded `export_diagnostics.exclusions`. An operator may explicitly pass `force=true` to include otherwise eligible records rejected only by semantic validation (for example, the confidence threshold); validation still runs and each bypassed finding remains in the diagnostics. Force does not bypass lifecycle, identity, evidence-integrity, endpoint, provider, or license gates. Preserve the returned object identities and `x_corrobore_evidence_refs` instead of inventing replacements. Logical export metadata identifies the snapshot and transaction, but the current HTTP export does not roll the graph back in time.

## Recover safely

- Fix invalid ids, modes, domain profiles, arity, or syntax from the returned error.
- Narrow objectives that return `OVERBROAD_OBJECTIVE`.
- Ask for disambiguation when seed search returns `AMBIGUOUS_SEED`.
- Stop or ask a human when evidence is missing, contradictory, or outside the authorized scope.
- Do not repeat the same rejected mutation or weaken evidence requirements.

## Canonical references

See [Cypher Support](user-guide/cypher.md), [HTTP Server](user-guide/http-server.md), and the [OpenAPI contract](api/openapi.yaml).

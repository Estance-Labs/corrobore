# For LLM Agents

Corrobore is your external, structured working memory for intelligence work. Use it to store and retrieve entities, relationships, evidence, confidence, time, versions, and audit history instead of carrying a growing graph as JSON in context.

This guide describes the current runtime contract. Check `GET /version` when a
workflow depends on a particular deployed release.

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
6. Read back entities and run an explicit relationship coverage pass.
7. Finish every authorized write before promotion.
8. Promote eligible nodes and relationships, then attempt strict export.
9. If a late write is necessary, repeat read-back, validation, and promotion.
10. Leave uncertain claims for human review.

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

### Confidence boundary

| Surface | Accepted scale | Example |
| :--- | :---: | :--- |
| Native Cypher and memory operations | `0..=1` | `0.9` means 90% |
| STIX objects and STIX import annotations | `0..=100` | `90` is stored as native 0.9 |

Never copy a STIX value such as `90` directly into Cypher. Corrobore rejects it
rather than guessing which boundary the caller intended.

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
- Every relationship assertion owns its own evidence and confidence; endpoint
  metadata does not transfer to the relationship.
- Mark inference as `candidate`; do not silently promote it to fact.
- Use domain vocabulary from [Intelligence Domains](user-guide/domains.md).
- Never use unsupported clauses such as `DETACH DELETE`, `LOAD CSV`, `UNWIND`, `FOREACH`, `CALL APOC`, or `CALL DBMS`.

## Respect host boundaries

The host decides whether a tool is read-only or permits mutations. Use `/v1/cypher/read` for reads and `/v1/cypher/write` only when the task authorizes a graph change. A rejected mutation is a policy result, not a reason to bypass the gateway.

Named sessions make actions inspectable. When a workflow starts a session, pass its `session_id` on Cypher and import calls, inspect its health, and stop it when finished. Session logs expose input/output audit parity and can be filtered by time.

Do not bypass policy outcomes by switching route shape or query wording. If the
runtime rejects a mutation, return the rejection reason and request explicit
authorization.

## CTI relationship coverage

For STIX 2.1 Indicator modeling, keep these assertions distinct:

- `Indicator -> Observed Data: based-on` states which observation supports the
  Indicator.
- `Indicator -> CTI domain object (SDO): indicates` states what threat object,
  such as Malware, the Indicator detects or characterizes.

Each is a Relationship SRO with its own STIX ID, retained evidence annotation,
confidence, and candidate lifecycle status. After importing, compare the
expected relationship inventory from the source with the graph. Do not fabricate
a relationship merely to improve coverage; report an unsupported bridge as a
gap.

```cypher
MATCH (source)-[r]->(target)
WHERE r.confidence IS NULL OR r.evidence_refs IS NULL
RETURN r
LIMIT 100
```

```cypher
MATCH (source)-[r]->(target)
WHERE r.status = "candidate"
RETURN r
LIMIT 100
```

## Validate and export

`POST /v1/stix/validate` accepts either an explicit bundle or current graph CTI nodes. For explicit bundles, built-in playbooks may correct supported missing fields; corrected objects are imported whenever at least one playbook runs. `valid` reports whether the validation pass found an error, not a second verdict after fixes. Inspect `issues`, `playbooks_applied`, `corrections_summary`, and `persistence`, then revalidate when you need a post-correction verdict.

Use `GET /v1/export/stix` for deterministic, CTI-scoped STIX projection after validation. The route is read-only and strict is the default correctness gate. Late writes remain candidate and require a new readiness and promotion pass before another strict attempt. Permissive is only for an explicit caller request for a diagnostic partial bundle. `force=true` is an explicit operator decision and never an automatic LLM fallback. Validation still runs and each bypassed semantic finding remains in diagnostics; force does not bypass lifecycle, identity, evidence-integrity, endpoint, provider, or license gates. Preserve the returned object identities and `x_corrobore_evidence_refs` instead of inventing replacements. Logical export metadata identifies the snapshot and transaction, but the current HTTP export does not roll the graph back in time.

## Recover safely

- Fix invalid ids, modes, domain profiles, arity, or syntax from the returned error.
- Narrow objectives that return `OVERBROAD_OBJECTIVE`.
- Ask for disambiguation when seed search returns `AMBIGUOUS_SEED`.
- Stop or ask a human when evidence is missing, contradictory, or outside the authorized scope.
- Do not repeat the same rejected mutation or weaken evidence requirements.

## Canonical references

See [Cypher Support](user-guide/cypher.md), [HTTP Server](user-guide/http-server.md), and the [OpenAPI contract](api/openapi.yaml).

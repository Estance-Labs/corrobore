---
name: corrobore
description: >-
  Use Corrobore as external structured working memory for CTI, FIMI, crisis, and
  cross-domain investigations. Query focused subgraphs, resolve semantic seeds,
  and write evidence-backed intelligence through the bounded Cypher interface.
license: MIT
---

# Corrobore

Corrobore stores entities, relationships, evidence, confidence, time, and audit history outside your context window.

## Report-to-STIX boundary

Corrobore accepts already-extracted structured candidates plus caller-owned evidence;
PDF parsing, OCR, and LLM extraction happen outside Corrobore. Evidence ingestion retains
source digests and bounded locators. Generic graph memory remains application-owned and is
not automatically CTI. Licensed CTI validation uses the native domain provider, while the
OpenCTI provider and OpenCTI endpoints are separate integration surfaces. See the
[release-gating report-to-STIX acceptance](../../acceptance/report-to-stix.md) for the
executable correction and export flow.

## Workflow

1. Check service health when connectivity is uncertain.
2. Search semantic seeds when the task provides an objective but no graph id.
3. Read the smallest bounded subgraph that answers the question.
4. Compare graph state with source evidence.
5. Write only authorized, evidence-backed changes.
6. Read back changes and validate before export.
7. Stop named sessions when the workflow ends.

## HTTP mapping

- `GET /health/live`: liveness only.
- `GET /health/ready`: engine, storage-recovery, and lifecycle readiness.
- `GET /version`: build and storage-format compatibility.
- `GET /metrics`: Prometheus metrics.
- `POST /v1/seed/search`: ranked seed candidates with explanations.
- `POST /v1/cypher/read`: read-only Cypher.
- `POST /v1/cypher/write`: mutation Cypher.
- `POST /v1/cypher/execute`: compatibility route with explicit or automatic mode.
- `POST /v1/import/stix` and `/v1/import/stix/file`: STIX import.
- `POST /v1/stix/validate`: native STIX validation and supported corrections.
- `GET /v1/export/stix`: deterministic STIX projection; `force=true` is an explicit audited override for semantic validation only.
- `POST /v1/sessions/start`, `GET /v1/sessions/{session_id}/health`, `GET /v1/sessions/{session_id}/logs`, `POST /v1/sessions/{session_id}/stop`: durable session lifecycle and audit.

Protected routes require `Authorization: Bearer <token>`.

## Tool boundary

When Corrobore is exposed as agent tools, preserve the transport boundary:

- health, metrics, seed search, Cypher reads, export, session health, and session logs are read operations;
- Cypher writes, STIX import, validation with correction persistence, session start, and session stop can change durable or graph state;
- never route a mutation through a read tool or retry a policy rejection through a broader endpoint.

### Seed search

```json
{
  "objective": "infrastructure linked to the phishing campaign",
  "domain_profile": "cti",
  "mode": "hybrid",
  "top_k": 5,
  "score_threshold": 0.2
}
```

Use candidate scores and explanations to choose where to inspect. Ranking is not evidence and does not authorize a write.

## Cypher rules

Supported reads include `MATCH`, `OPTIONAL MATCH`, `WHERE`, `WITH`, `RETURN`, `DISTINCT`, aggregations, `ORDER BY`, `SKIP`, and `LIMIT`. Supported mutations include `CREATE`, `MERGE`, `SET`, `REMOVE`, and `DELETE` when host policy allows them.

Never emit `DETACH DELETE`, `LOAD CSV`, `UNWIND`, `FOREACH`, `CALL APOC`, or `CALL DBMS`.

```cypher
MATCH (a:ThreatActor {name: "APT28"})
MATCH (e:EvidenceSpan {id: "span--123"})
MERGE (a)-[r:USES]->(m:Malware {name: "X-Agent"})
SET r.confidence = 0.82,
    r.evidence_refs = [e.id],
    r.status = "candidate"
RETURN r
```

### Diagnostic reads

```cypher
MATCH (n)
WHERE NOT (n)--()
RETURN n
LIMIT 50
```

```cypher
MATCH ()-[r]->()
WHERE r.confidence < 0.6
RETURN r
ORDER BY r.confidence ASC
LIMIT 100
```

## Domain vocabulary

### CTI

- Nodes: `ThreatActor`, `Malware`, `Indicator`, `Tool`, `Campaign`, `Infrastructure`, `Vulnerability`, `Identity`, `Location`, `Report`.
- Relationships: `Indicates`, `Uses`, `Targets`, `AttributedTo`, `CommunicatesWith`, `RelatedTo`.

### FIMI

- Nodes: `Actor`, `Narrative`, `Claim`, `Account`, `Outlet`, `Campaign`, `CoordinationCluster`.
- Relationships: `Amplifies`, `CoordinatesWith`, `OriginatesFrom`, `Targets`, `Repeats`, `Contradicts`.

### Crisis

- Nodes: `CrisisEvent`, `Location`, `HumanitarianNeed`, `Organization`, `Observation`.
- Relationships: `OccursAt`, `Impacts`, `ReportedBy`, `Needs`, `EscalatesTo`.

Domain Rust functions are not automatically callable from Cypher. Do not invent `namespace.symbol(...)` syntax unless the host says the function is registered and wired.

## PDF or report to STIX playbook

1. Split the source into stable evidence spans.
2. Extract candidate entities and relations with span ids and initial confidence.
3. Start a named Corrobore session.
4. Read or search for existing identities before merging candidates.
5. Materialize pass A with idempotent `MERGE` statements.
6. Query orphans, low-confidence links, contradictions, and missing bridges.
7. Re-read only implicated source spans and write delta corrections.
8. Validate the candidate STIX bundle or graph CTI nodes.
9. Review issues, playbooks, correction summary, and persistence. Revalidate after corrections when a post-fix verdict is required.
10. Export deterministic STIX and stop the session.

Example pass-A record:

```json
{
  "entity": {
    "type": "ThreatActor",
    "name": "APT-X",
    "evidence_ref": "span--p12-l03-09",
    "confidence": 0.72,
    "status": "candidate"
  }
}
```

Quality gates: source grounding, referential integrity, low orphan rate, evidence on key claims, honest confidence, bounded queries, stable export metadata, and no fabricated bridge entities.

## Trust rules

- Read before writing and prefer `MERGE` for identities.
- Attach evidence and confidence to important claims.
- Keep inferred items in `candidate` status.
- Bound queries and avoid broad traversal.
- Treat seed ranking as navigation guidance, not proof.
- Do not fabricate missing facts or bypass a policy rejection.
- If evidence is contradictory or incomplete, report uncertainty and ask for review.
- A validation response may persist corrections while `valid` remains false, because `valid` reflects issues found during that pass. Inspect every result field and revalidate when needed.

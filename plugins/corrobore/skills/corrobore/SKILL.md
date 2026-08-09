---
name: corrobore
description: Use Corrobore as external structured working memory for CTI, FIMI, crisis, and cross-domain investigations with focused reads and evidence-backed writes.
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
[release-gating report-to-STIX acceptance](https://docs.corrobore.org/acceptance/report-to-stix/) for the
executable correction and export flow.

## Workflow

1. Check service health when connectivity is uncertain.
2. Search semantic seeds when the task provides an objective but no graph id.
3. Read the smallest bounded subgraph that answers the question.
4. Compare graph state with source evidence.
5. Write only authorized, evidence-backed entities and relationships.
6. Read back changes and audit expected relationship coverage.
7. Complete every authorized write before promotion.
8. Promote eligible nodes and relationships, then attempt strict export.
9. Repeat readiness and promotion after any late write.
10. Stop named sessions when the workflow ends.

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

## Confidence boundary

| Surface | Accepted scale | Example |
| :--- | :---: | :--- |
| Native Cypher and memory operations | `0..=1` | `0.9` means 90% |
| STIX objects and STIX import annotations | `0..=100` | `90` is stored as native 0.9 |

Do not copy `90` from STIX into `SET r.confidence = 90`. Cypher rejects it;
write `0.9` at the native boundary.

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

Every relationship assertion owns its own evidence and confidence. Evidence or
confidence on either endpoint does not make the relationship export-ready.

### Diagnostic reads

```cypher
MATCH (n)
WHERE NOT (n)--()
RETURN n
LIMIT 50
```

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

## Domain vocabulary

### CTI

- Nodes: `ThreatActor`, `Malware`, `Indicator`, `Tool`, `Campaign`, `Infrastructure`, `Vulnerability`, `Identity`, `Location`, `Report`.
- Relationships: `Indicates`, `Uses`, `Targets`, `AttributedTo`, `CommunicatesWith`, `RelatedTo`.

For STIX relationship coverage, distinguish these source-backed assertions:

- `Indicator -> Observed Data: based-on` links the Indicator to the observation
  that supports it.
- `Indicator -> CTI domain object (SDO): indicates` links the Indicator to what
  it detects or characterizes, for example Malware.

Each Relationship SRO needs its own STIX ID and import annotation with evidence,
confidence, and candidate status. Compare the extracted relationship inventory
with the imported graph. Do not fabricate missing bridges to improve a score;
return the unsupported relationship as a gap.

### FIMI

- Nodes: `Actor`, `Narrative`, `Claim`, `Account`, `Outlet`, `Campaign`, `CoordinationCluster`.
- Relationships: `Amplifies`, `CoordinatesWith`, `OriginatesFrom`, `Targets`, `Repeats`, `Contradicts`.

### Crisis

- Nodes: `CrisisEvent`, `Location`, `HumanitarianNeed`, `Organization`, `Observation`.
- Relationships: `OccursAt`, `Impacts`, `ReportedBy`, `Needs`, `EscalatesTo`.

Domain Rust functions are not automatically callable from Cypher. Do not invent `namespace.symbol(...)` syntax unless the host says the function is registered and wired.

## PDF or report to STIX playbook

For reusable task prompts, load only the reference that matches the current job:

- [Working memory](references/working-memory.md)
- [CTI investigation](references/cti-investigation.md)
- [FIMI investigation](references/fimi-investigation.md)
- [STIX from unstructured data](references/stix-from-unstructured.md)
- [Evidence-first validation](references/evidence-first-validation.md)

1. Split the source into stable evidence spans.
2. Extract candidate entities and relations with span ids and initial confidence.
3. Start a named Corrobore session.
4. Read or search for existing identities before merging candidates.
5. Materialize all entities and Relationship SROs with idempotent writes.
6. Query orphans, missing relationship metadata, contradictions, and expected
   `based-on` / `indicates` coverage.
7. Re-read only implicated source spans and write delta corrections.
8. Read back the final graph; late writes remain candidate and require a new
   readiness and promotion pass.
9. Validate, review issues and persistence, and revalidate after corrections
   when a post-fix verdict is required.
10. Promote eligible nodes and relationships, attempt strict export, and stop
    the session.

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

## Export decisions

- Strict is the default correctness gate.
- Permissive is allowed only for an explicit caller request for a diagnostic
  partial bundle; inspect every exclusion.
- `force=true` is an explicit operator decision and never an automatic LLM
  fallback.
- `GET /v1/export/stix` is read-only. It never promotes candidates. If any
  authorized write occurs after promotion, run a new readiness and promotion
  pass before retrying strict export.

## Trust rules

- Read before writing and prefer `MERGE` for identities.
- Attach evidence and confidence to important claims.
- Keep inferred items in `candidate` status.
- Bound queries and avoid broad traversal.
- Treat seed ranking as navigation guidance, not proof.
- Do not fabricate missing facts or bypass a policy rejection.
- If evidence is contradictory or incomplete, report uncertainty and ask for review.
- A validation response may persist corrections while `valid` remains false, because `valid` reflects issues found during that pass. Inspect every result field and revalidate when needed.

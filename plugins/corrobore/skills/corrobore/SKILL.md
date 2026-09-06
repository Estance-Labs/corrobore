---
name: corrobore
description: Use Corrobore as external structured working memory for CTI, FIMI, crisis, and cross-domain investigations with focused reads and evidence-backed candidate ingestion.
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
5. Follow [candidate ingestion and targeted repair](references/candidate-ingestion.md): submit, read the failing constraint, re-extract that field, resubmit.
6. Keep raw candidate versions in Shadow or Hypothesis with extraction runs and repair lineage.
7. Review source grounding and explicitly promote eligible candidates within the task's authorization.
8. Read back the reviewed records and audit relationship coverage before strict export.
9. Route later extraction corrections through a new candidate workflow.
10. Stop named sessions when the workflow ends.

## HTTP mapping

- `GET /health/live`: liveness only.
- `GET /health/ready`: engine, storage-recovery, and lifecycle readiness.
- `GET /version`: build and storage-format compatibility.
- `GET /metrics`: Prometheus metrics.
- `POST /v1/seed/search`: ranked seed candidates with explanations.
- `POST /v1/cypher/read`: read-only Cypher.
- `POST /v1/import/candidates`: retain raw extraction candidates and return constraint feedback.
- `GET /v1/import/candidates/{id}`: inspect raw versions, feedback and promotion receipts.
- `POST /v1/import/candidates/{id}/repairs`: resubmit a targeted repair with predecessor lineage.
- `POST /v1/import/candidates/{id}/promote`: explicit reviewed promotion.
- `GET /v1/reconciliations/{id}` and `POST /v1/reconciliations/{id}/undo`: inspect or reverse an evidence-cited merge.
- `POST /v1/stix/validate`: native STIX validation and supported corrections.
- `GET /v1/export/stix`: deterministic STIX projection; `force=true` is an explicit audited override for semantic validation only.
- `POST /v1/sessions/start`, `GET /v1/sessions/{session_id}/health`, `GET /v1/sessions/{session_id}/logs`, `POST /v1/sessions/{session_id}/stop`: durable session lifecycle and audit.

Protected routes require `Authorization: Bearer <token>`.

## Confidence boundary

| Surface | Accepted scale | Example |
| :--- | :---: | :--- |
| Native Cypher and memory operations | `0..=1` | `0.9` means 90% |
| STIX objects and STIX import annotations | `0..=100` | `90` is stored as native 0.9 |

Normalize STIX confidence `90` to `0.9` when mapping reviewed content to native
graph metadata. Preserve the original raw candidate payload and its scale.
Evidence and confidence remain owned by each assertion.

## Tool boundary

When Corrobore is exposed as agent tools, preserve the transport boundary:

- health, metrics, seed search, Cypher reads, export, session health, and session logs are read operations;
- candidate submission, repair, reviewed promotion, reconciliation application/undo, validation with correction persistence, session start, and session stop change durable state;
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

Use bounded `MATCH`, `OPTIONAL MATCH`, `WHERE`, `WITH`, `RETURN`, aggregations,
ordering and limits for investigation. Candidate status on an ordinary graph
record does not implement candidate-tier isolation. Use the candidate API for
extracted assertions; never bypass its validation through a graph mutation.

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

1. Split the source into stable evidence spans and extract candidate assertions externally.
2. Start a named session; search and inspect existing identities.
3. Follow the [candidate loop](references/candidate-ingestion.md) for entities and relationships.
4. Re-extract only fields identified by constraint feedback; retain all raw versions.
5. Review provenance and explicitly promote eligible candidates with complete domain metadata.
6. Query orphans, relationship-owned evidence, contradictions and expected `based-on` / `indicates` coverage.
7. Keep unresolved assertions outside canonical state and ambiguous identities unmerged.
8. Validate the supported STIX projection, inspect each result, and revalidate after authorized corrections.
9. Attempt strict export and stop the session. Never fabricate a bridge to make export succeed.

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

- Read before submitting candidates; reconcile identity only with contextual evidence.
- Attach evidence and confidence to important claims.
- Keep unreviewed extraction in Shadow or Hypothesis; a `candidate` property alone is insufficient.
- Bound queries and avoid broad traversal.
- Treat seed ranking as navigation guidance, not proof.
- Do not fabricate missing facts or bypass a policy rejection.
- If evidence is contradictory or incomplete, report uncertainty and ask for review.
- A validation response may persist corrections while `valid` remains false, because `valid` reflects issues found during that pass. Inspect every result field and revalidate when needed.

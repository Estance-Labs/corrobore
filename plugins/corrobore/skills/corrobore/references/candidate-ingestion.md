# Candidate ingestion and targeted repair

Use this contract for extraction from documents or observations. Extraction,
OCR and model calls remain outside the core. Read relevant graph context first,
then **submit, read the failing constraint, re-extract that field, resubmit**.
Do not send extraction output directly to canonical graph mutation or STIX
import routes. A property named `status: candidate` is not a shadow tier.

## Submit

Send `POST /v1/import/candidates` with a unique version ID, extraction run,
attributed actor, exact raw payload string and the caller's constraint contract.
Use `Shadow` (the default) or `Hypothesis`; neither creates canonical records.
Keep the original payload even if it is malformed. The constraints below are a
small example; supply the schema, temporal and domain constraints needed for the
actual source, rather than assuming the engine infers them.

```json
{
  "id": "candidate--report-1-v1",
  "extraction_run_id": "run--report-1",
  "actor": "extractor--1",
  "tier": "Shadow",
  "raw_payload": "{\"name\":42,\"evidence_ref\":\"span--p1-l3\",\"confidence\":0.72}",
  "constraints": [
    {"id":"entity-name-type","field":"/name","rule":{"kind":"type","expected":"string"}}
  ]
}
```

Read the returned `validation` and, when resuming, use
`GET /v1/import/candidates/{id}`. A successful HTTP submission retains the
candidate even when `validation.valid` is false. Inspect each failure's `field`
(RFC 6901 pointer), `constraint.id`, complete `constraint.rule`, `observed`,
`present`, and `repeated`. In this example `/name` contains a number instead of a
string. Revisit the implicated source span and re-extract `/name` alone. Keep
unaffected evidence, confidence and other fields intact.

## Resubmit the repair

Send `POST /v1/import/candidates/{id}/repairs` using the preceding candidate's ID
in the URL, a new candidate ID, the corrected raw payload, and the failing rule
IDs in `caused_by`. The tier and constraints are inherited; do not replace them
with weaker rules or submit an unrelated candidate to hide the repair lineage.

For the preceding example, the URL ends in `candidate--report-1-v1/repairs`:

```json
{
  "id": "candidate--report-1-v2",
  "extraction_run_id": "run--report-1-reextract-name",
  "actor": "extractor--1",
  "raw_payload": "{\"name\":\"Example Organization\",\"evidence_ref\":\"span--p1-l3\",\"confidence\":0.72}",
  "caused_by": ["entity-name-type"]
}
```

Read the new feedback. A repeated failure is not permission to relax the
contract. Continue only while source evidence supports another targeted repair
and the task budget permits it; otherwise retain the unresolved candidate and
report the failing field and evidence gap. Exact retries are idempotent; a new
raw version requires a new ID. Neither successful repair nor constraint validity
promotes the candidate.

## Reviewed promotion

After source review and within the user's authorization, use
`POST /v1/import/candidates/{id}/promote` with the reviewer's `actor`, a nonblank
`reason` and the explicit reviewed node or relationship input. The reviewed
record must faithfully represent the candidate and carry the required domain
metadata. For relationships, use existing endpoint IDs and independently backed
relationship evidence; endpoint evidence does not transfer automatically.

The generic API does not infer a mapping from arbitrary raw JSON to graph fields
or automatically construct a complete STIX object. Follow the host's supported
reviewed-record schema. Failed constraints block promotion atomically. The raw
versions, extraction runs, repair causes and promotion receipt remain auditable.
A later correction is a new candidate workflow, not an unreviewed graph delta.

## Mention identity and uncertainty

Keep observation-bound `EntityMention` records distinct from canonical `Entity`
records. Candidate entity IDs and name/embedding scores are hints, not proof.
A reconciliation requires contextual evidence and an attributed `Merge`,
`Distinct` or `Abstain` judgment. Homonyms stay distinct; ambiguous pairs abstain.
The core records reviewed judgments; it does not run an identity classifier.

Use `POST /v1/reconciliations` to retain a judgment on existing mentions,
`GET /v1/reconciliations/{id}` to inspect it, and an explicitly authorized
`POST /v1/reconciliations/{id}/merge` to apply a supported merge. Undo through
`POST /v1/reconciliations/{id}/undo` names the merge, actor, time and reason.
A `DEPENDENT_RECONCILIATION` conflict names the blocking later decision; do not
cascade silently. Original evidence and both records survive undo. Mention
creation itself requires the host's engine ingestion capability; do not invent
an HTTP mention-creation endpoint.

## Transport and measurement

Use the host's authorized HTTP client with bearer authentication. The bundled
portable MCP adapter does not yet expose candidate or reconciliation tools;
do not invent tool names or substitute its generic memory/STIX tools. If the
host lacks the required HTTP capability, retain proposals outside canonical
state and report that capability gap.

Read `/metrics` for extraction accuracy, repair success, false-repair rate,
per-outcome reconciliation accuracy, evaluation coverage and abstain rate.
Constraint validity is not semantic correctness: accuracy requires independent
reference evaluations, and missing labels produce `NaN`. An observed engine
that never abstains reports zero abstention, which warrants examination.

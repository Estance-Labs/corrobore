# Read the claim audit before asserting a verdict

Before asserting a verdict, call `GET /v1/claims/{id}/audit` with the configured
Bearer token. In the current source plugin, use `corrobore_claim_audit` with
`claim_id` to perform that GET. Use the exact governed claim identifier; a graph node identifier,
search score, extraction confidence, or candidate validation is not a verdict.
This read does not re-run a resolver, verifier, extraction, or reconciliation.
Do not invoke a write or recomputation merely to answer an audit question.

If the read fails, the claim is unknown, or `current_verdict` is null, do not
assert a verdict. Report the unavailable or unresolved state. Never substitute
an inferred conclusion for a missing machine record. An HTTP success alone is
not evidence of support: inspect the returned fields before reporting a result.

## Four questions from one response

- **Why this verdict?** Read `current_verdict`, `explanation.dimensions`, stored
  clusters and `link_membership`. Follow the exact evidence links to the retained
  observations and source versions. Quote observations verbatim; keep paraphrase
  and inference explicit. Do not collapse the dimensions into a new score or
  treat several links in one cluster as independent corroboration.
- **What contradicts it?** Read `contradictions` and
  `verification_disagreements`, including refuting observations and failing
  checks. Preserve contradictions in the answer even when support is strong.
- **What changed?** Read `state_transitions`, `verdict_history`, candidate repair
  predecessors, reconciliations, merge undos and `analyst_decisions`. Keep the
  machine history separate from human judgments; a reversal leaves both records.
- **What has not been checked?** Inspect `coverage` and `unverified_steps` per
  claim, including dependencies. Do not borrow a related claim's coverage for the
  root claim. Missing retained provenance is a gap, never a completed check.

## Verification is a separate dimension

Use the canonical coverage classes returned by the API:

| Class | How to report it |
| --- | --- |
| `mechanically_checked` | A deterministic verifier ran; also inspect `result`, since an inconclusive check is not a pass. |
| `semantically_judged` | An advisory semantic assessment; it does not establish mechanical verification. |
| `unchecked` | No verifier record; strong apparent support does not fill this gap. |
| `failing` | A failed check; `deterministic` identifies mechanical versus advisory failure. |

Cite the `verifier_id`, `verifier_version`, result, retained inputs, rationale and
limits. A passing check establishes only its checked scope. Do not call the whole
claim mechanically verified merely because one dependency or one limited check
passed. Report the stored verdict and the mechanical/semantic gaps separately.

## Human disagreement

Within the task's authorization, append an attributed judgment through
`POST /v1/claims/{id}/decisions`. The actor is caller-attributed, not an identity
attested by this API. Never edit an observation, verification or stored verdict
in order to express disagreement. An annotation uses `{ "kind": "annotation",
"text": "..." }` as its action. An override records a human judgment:

```json
{
  "id": "review-example-1",
  "actor": "analyst-example",
  "recorded_at": "2026-09-06T18:00:00Z",
  "action": {
    "kind": "override",
    "judgment": "Needs further investigation",
    "rationale": "The retained sources disagree on the date"
  }
}
```

Use a unique decision ID for each actual decision; the example IDs are illustrative.
Keep the entire payload unchanged when retrying an uncertain write. A successful
response acknowledges `decision_id`; re-read the audit to inspect the stored
record. If that refresh fails, report that the write succeeded but its audit view
is unavailable; do not submit a second judgment with a new ID.

To withdraw a human decision, append a new reversal targeting its ID:

```json
{
  "id": "review-example-2",
  "actor": "analyst-example",
  "recorded_at": "2026-09-06T19:00:00Z",
  "action": {
    "kind": "reversal",
    "decision_id": "review-example-1",
    "rationale": "Withdrawn after reviewing the original source"
  }
}
```

A human override never changes the machine verdict. Re-read after any authorized
mutation before reporting the resulting state. For offline review, retain the
`x_corrobore_audit_archive` extension in STIX/FIMI exports; never strip the lineage
and claim that the remaining projection proves the same audit.

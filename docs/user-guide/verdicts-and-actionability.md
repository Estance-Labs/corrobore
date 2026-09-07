# Verdicts, Confidence Dimensions, and Actionability

A claim in Corrobore carries two separate answers. The **verdict** says what the
retained evidence supports. The **actionability assessment** says whether the
claim may be acted on or exported. Neither is a model score, and neither is
the legacy scalar `confidence` property. This guide explains how to read both
from Cypher, the claim audit, and exports.

This is the user-facing view of the Epic 0029 corroboration engine (WS-D).
Internals and acceptance evidence are in
[Architecture](../architecture.md#epistemic-stores-epic-0029).

## What a verdict is

A verdict is computed by the engine from active evidence links and
deterministic verification records. No Cypher, HTTP, or memory route writes a
verdict. The `verdict_state` is one of `Supported`, `Refuted`, `Mixed`,
`Contested`, `Unknown`, `InsufficientEvidence`, or `Superseded`; the lifecycle
`claim_status` is projected from it.

Three rules hold across every policy version:

- a deterministic verifier failure outranks any semantic score or amount of
  apparent support;
- `Supported`, `Refuted`, and `Mixed` require evidence that reaches a stored
  `Observation` bound to a `Source`; otherwise the claim abstains as
  `InsufficientEvidence` and records a reachability gap;
- a policy or structure change appends a new verdict snapshot and never
  rewrites history.

## The six named dimensions

The current policy (`ws-d-cluster-v1`) computes six dimensions independently.
Each is present only when the inputs that define it exist; **absent is not
zero**.

| Dimension | Meaning |
| :--- | :--- |
| `evidence_sufficiency` | Combined support contribution across independence clusters. Absent without eligible support inputs. |
| `source_authority` | Maximum resolved authority among distinct signal sources under the selected authority policy. Repetition cannot raise it. |
| `source_independence` | `k / (k + 1)` for `k` supporting clusters with known provenance. Separate clusters mean no recorded dependency, not proven independence. |
| `temporal_validity` | One when any stamped signal is active, zero when all stamped signals are outside validity, absent without stamped signals. |
| `contradiction_load` | Refuting mass divided by total directional mass. A deterministic failure forces it to one. |
| `verifier_strength` | One for authoritative deterministic conclusive coverage, zero for advisory or inconclusive input only, absent without verification records. |

These are bounded, deterministic indicators. They are not calibrated
probabilities, and they are never collapsed back into one number for any
policy decision.

## Independence clusters

Evidence links are grouped into dependency components before aggregation, so
ten copies of one source contribute once. Grouping reasons include shared
source identity or ancestry, publisher, upstream citations, artifact identity,
extraction run or model pipeline, and retained campaign coordination signals
(see [Narratives and Campaigns](narratives-and-campaigns.md#coordination-signals-and-independence)).
Links with no assignable provenance get an explicit unknown-independence
singleton rather than a free independent vote.

The cluster contribution rule is versioned and bounded: duplicates inside one
cluster add at most one percentage point, and distinct clusters combine as
`1 - product(1 - contribution)`. The retained `verdict_explanation` records
every cluster's members, dependency reasons, best strength, authority, and
resulting contribution.

## Source authority

Authority is a registered, versioned policy binding a source identity, an
authority domain, and a predicate class to a bounded weight. Callers select
the exact version and scope; there is no latest-version fallback and no
classification inferred from claim text. A source with no binding has absent
authority, even when trust inputs exist. Authority never creates support and
never overrides a deterministic failure.

## Hypothesis sets

Each verdict retains a ranked `verdict_hypothesis_set`: the anchor claim and
its direct `Contradicts` or `Supersedes` neighbours, each with its own state,
dimensions, clusters, and authority provenance, including the losing
alternatives. Supported and mixed alternatives rank first; within a group the
score is `evidence_sufficiency * (1 - contradiction_load)`. One independent
high-authority source can therefore outrank many dependent copies.

## Actionability is a separate gate

`verdict_actionability` reports whether the claim may be acted on. The default
`actionability-v1` policy requires:

- a current, grounded deterministic pass;
- at least two positively weighted supporting clusters with known provenance
  (one for claim types that do not require corroboration);
- `contradiction_load` at most `0.25`;
- `temporal_validity` equal to one.

Every unmet condition is retained in the assessment, not only the first. A
missing required dimension makes the gate abstain (property absent); explicit
blocked and allowed decisions project to `0` and `1`. Strong apparent support
never replaces a grounded deterministic check.

Exports read this gate. Strict STIX or FIMI export names the actionability
blockers of every claim targeting a selected record; permissive export excludes
the record. `force=true` cannot bypass permission. See
[Exporters](exporters.md).

High-impact claims carry one more requirement before publication: a live
corrective route on an independent channel and a recorded falsifier. That
publish gate is described in
[Agentic Platform Foundations](agentic-platform.md#corrective-routes-falsifiers-and-the-publish-gate).

## Uncertainty explanation

`verdict_uncertainty_kind` classifies the primary cause of doubt:

| Token | Cause |
| :--- | :--- |
| `unresolved_conflict` | Active support with refutation, or a mixed verdict. |
| `staleness` | Expired evidence with zero temporal validity. |
| `ambiguity` | Several positively scored supported or mixed hypotheses. |
| `ignorance` | An unknown verdict with nothing to weigh. |

An absent token means none of these causes was detected. It does not mean
certainty, and it does not grant permission to act.

## Read the verdict

From Cypher, through the read-only epistemic projection:

```cypher
MATCH (v:Verdict)
RETURN v.verdict_claim, v.verdict_state,
       v.verdict_dimension_evidence_sufficiency,
       v.verdict_dimension_source_independence,
       v.verdict_dimension_contradiction_load,
       v.verdict_dimension_actionability,
       v.verdict_uncertainty_kind
ORDER BY v.verdict_claim ASC
LIMIT 100
```

From HTTP, `GET /v1/claims/{id}/audit` returns the stored verdict, the
explanation, the dimensions, the cluster membership, the history, and the
verification coverage in one read. Agents must read it before asserting a
verdict; see [Claim Audit](claim-audit.md).

## What agents must not do

- Do not average, sum, or otherwise combine the dimensions into a new score.
- Do not treat several links in one cluster as independent corroboration.
- Do not read the legacy scalar `confidence` as a verdict; it is a display
  projection that no engine policy consults.
- Do not act on a `Supported` claim whose actionability is blocked or absent.
- Do not call a claim verified because a related claim has coverage.

## Canonical references

- [Claim Audit](claim-audit.md)
- [Cypher Support](cypher.md#epistemic-projection-epic-0029)
- [Agentic Platform Foundations](agentic-platform.md)
- [Architecture](../architecture.md#epistemic-stores-epic-0029)

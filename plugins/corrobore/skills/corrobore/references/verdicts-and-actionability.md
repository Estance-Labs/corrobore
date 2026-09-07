# Report verdicts, dimensions, and what could change them

Load this reference when a task asks whether a claim is true, supported,
reliable, or ready to act on. Read the [claim audit](claim-audit.md) first; this
reference tells you how to report what the audit returns.

## Two answers, never one score

A governed claim carries a **verdict** (`Supported`, `Refuted`, `Mixed`,
`Contested`, `Unknown`, `InsufficientEvidence`, `Superseded`) and a separate
**actionability** assessment. Report both. A `Supported` claim with blocked or
absent actionability may not be acted on or exported. The legacy scalar
`confidence` property is a display projection that no engine policy reads; do
not quote it as a verdict.

## The six dimensions

Report each dimension as returned in `explanation.dimensions`. Absent means the
inputs that define it do not exist; it is not zero and not a failure.

| Dimension | Say |
| :--- | :--- |
| `evidence_sufficiency` | how much independent support the clusters contribute |
| `source_authority` | the strongest justified authority among distinct sources |
| `source_independence` | how many supporting clusters have known provenance |
| `temporal_validity` | whether any stamped signal is still active |
| `contradiction_load` | the share of refuting mass; `1` after a deterministic failure |
| `verifier_strength` | whether an authoritative deterministic check passed |

Never average, sum, or rank these into a new number. Never present several
evidence links in one independence cluster as independent corroboration: a
cluster is one source, whatever its member count.

## Independence and authority

`explanation.clusters` lists every cluster with its members, dependency reasons,
and directional weights. Separate clusters mean no recorded dependency, not
proven independence; say so. `source_authority` comes from a registered,
versioned policy scoped to an authority domain and predicate class. A source
with no binding has absent authority. Authority never creates support.

## Hypotheses and uncertainty

`verdict_hypothesis_set` ranks the claim against its direct `Contradicts` and
`Supersedes` neighbours, including the losing ones. Report the alternatives
with their states. `uncertainty_kind` names the primary cause of doubt:
`unresolved_conflict`, `staleness`, `ambiguity`, or `ignorance`. An absent kind
is not certainty and not permission.

## The actionability gate

Report `actionability` and its retained blockers verbatim. The default policy
requires a grounded deterministic pass, at least two supporting clusters with
known provenance, `contradiction_load` at most `0.25`, and active temporal
validity. Strong apparent support never replaces the deterministic check. When
the gate is blocked or absent, say the claim is not actionable and name the
blockers; do not soften the finding.

## What could change the verdict

The audit's `falsifiers` say what evidence would move the claim to which state,
and `corrective_routes` say through which channels (`authoritative_api`,
`primary_document`, `sensor`, `signature_check`, `independent_retrieval`,
`human_review`) with their liveness. A route on the channel that produced the
claim is self-consistent and is not a correction. When `unverified_steps`
contains `no_recorded_falsifier`, report that nothing recorded could change the
verdict; treat it as a finding about the investigation, not a formality.

High-impact claims additionally need a live independent corrective route and a
recorded falsifier before publication. Do not publish a high-impact finding
whose publish gate names `corrective_route_missing`,
`corrective_route_not_live`, `self_consistent_routes_only`, or
`falsifier_missing`.

## Report template

State, in this order and without merging them:

1. the stored verdict state and the policy version;
2. each present dimension with its value, and each absent one as absent;
3. the clusters that support and refute, with their reasons;
4. the actionability decision and its blockers;
5. the ranked alternatives and the uncertainty kind;
6. what has not been checked, and what could change the verdict.

If any part of the audit is unavailable, report that part as unavailable.

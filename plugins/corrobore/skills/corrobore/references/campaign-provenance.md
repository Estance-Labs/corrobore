# Campaign provenance without attribution

Load this reference for FIMI, influence-operation, or coordination tasks. It
covers the domain-neutral `Narrative` and `Campaign` collections held by the
Corrobore core, coordination signals, the attribution gate, and how to report a
misleadingness assessment beside a factual verdict.

## Collections are context

`Narrative` and `Campaign` are immutable governed collections. They hold typed
members (governed claims, content sources, actor and infrastructure references),
neutral themes, and a bitemporal stamp; a campaign also lists the narratives it
collects. Read them from the epistemic projection:

```cypher
MATCH (c:Campaign)-[m:HAS_MEMBER]->(x)
RETURN c.campaign_id, c.campaign_themes, m.membership_role, labels(x), x.claim_id, x.record_id
ORDER BY c.campaign_id ASC
LIMIT 200
```

`membership_role` is `claim`, `content`, `actor`, `infrastructure`, or
`narrative`. Membership supports no claim and attributes no actor. Never write
"the campaign's actor" from a membership edge; write "an actor referenced by the
collection". A changed collection is a new record under a new identity, never an
edit.

The projected `Campaign` label is the neutral epistemic collection, not the
canonical CTI `Campaign` node; the two are separate records.

## Coordination signals are provenance, not authorship

The core detects five production-side signals across the claims of one
collection, each needing at least two records from two distinct sources:
repeated prompt artifact, generation-style fingerprint, cross-content
redundancy, shared infrastructure, and collection co-membership. Signals are
stored as evidence annotations with their exact records, sources, measurement,
and threshold.

Report a signal as "these records share a production pattern (signal, group,
measurement)". Never report it as "produced by" or "operated by". Retained
signals join the affected evidence links into one independence cluster, so the
supported claims behind them count once; co-membership alone never collapses
independence, because grouping is curation.

## Attribute only through a supported claim

The engine refuses an attribution that rests on coordination signals alone
(`CoordinationSignalsOnly`) or on a claim it does not hold `Supported`
(`CorroborationNotSupported`). Before you attribute:

1. name the governed claims the attribution rests on;
2. read each claim's [audit](claim-audit.md) and confirm the stored verdict is
   `Supported` and its [actionability](verdicts-and-actionability.md) is not
   blocked;
3. record the attribution as an ordinary governed claim with its own evidence,
   within the task's authorization, using the candidate workflow;
4. keep the coordination signals as context beside it, marked as not asserted.

If no such claim exists, report the coordination finding and state that
attribution is not supported.

## Misleadingness beside the factual verdict

The FIMI pack assesses six mechanisms: unsupported inference, exaggeration,
omission, framing or context shift, emotional arousal, and communicative intent.
An assessment records the reader interpretation beside the evidence-warranted
one and derives a band from that gap and the mechanisms that explain it. It is
marked `not_a_factual_determination` and its `claim.verify/1` result is always
`inconclusive`, so it cannot move a verdict.

Report two findings: the factual verdict of each claim, and the misleadingness
band of the piece with its mechanisms and cited records. "Claims mostly
supported" and "highly misleading" can both be true of the same content. Never
raise or lower one because of the other.

## Exports

FIMI exports carry `campaign_lineage` (collections referencing a record, the
matched role, themes, stamp, narratives, and signals marked
`attribution: "not_asserted"`) and `misleadingness` (the pack's recorded
assessments) as separate additive fields. Preserve both when you hand a document
on; do not fold either into the verdict.

# Narratives, Campaigns, and Misleadingness

Corrobore answers whether a claim is supported. For an influence operation that
question is not enough on its own: a piece can be accurate sentence by
sentence and still mislead, and content can share a production pattern without
anyone knowing who produced it. Epic 0029 WS-G adds the structure for those
questions to the core while keeping every answer in its own field.

This guide is the user-facing view. Internals and acceptance evidence are in
[Architecture](../architecture.md#fimi-misleadingness-and-campaign-provenance).

## Where each piece lives

| Concern | Repository | Surface |
| :--- | :--- | :--- |
| Neutral `Narrative` and `Campaign` collections, coordination signals, attribution gate | `corrobore` core | `Graph` API, epistemic projection, FIMI exporter |
| Misleadingness mechanisms, band rule, validators | `corrobore-domain-fimi` pack | `node.validate/1`, advisory `claim.verify/1` |
| Mechanism accuracy, grounding, factual-drift measurement | `corrobore-benchmarks` | benchmark suites |

The core carries structure and provenance. The pack carries meaning. The core
contains no FIMI vocabulary, which a repository test enforces.

## Neutral collections

`Narrative` and `Campaign` are immutable governed records in the epistemic
stores, created through `Graph::create_narrative` and `Graph::create_campaign`.
Each retains:

- typed membership: governed claims, content sources, and opaque actor and
  infrastructure node references;
- neutral themes;
- a validated bitemporal stamp.

A campaign additionally lists the narrative identities it collects. Repeating
an identical record is idempotent; changing content under the same identity is
an immutable-record conflict, so a revised collection is appended under a new
identity and the earlier record stays intact.

**Membership is context, never a judgment.** Placing a claim in a narrative does
not support it, placing an actor in a campaign does not attribute it, and
creating either record changes no claim, verdict, or canonical node. Actor and
infrastructure members are references; their payload need not be loaded in a
bounded graph view.

## Read collections from Cypher

The read-only epistemic projection renders the collections as nodes with
namespaced properties and `HAS_MEMBER` relationships:

| Label | Key properties |
| :--- | :--- |
| `Narrative` | `narrative_id`, `narrative_themes`, `narrative_membership` (JSON), `narrative_stamp` (JSON) |
| `Campaign` | `campaign_id`, `campaign_themes`, `campaign_membership`, `campaign_stamp`, `campaign_narratives` |
| `RecordReference` | `record_kind`, `record_id` for actor and infrastructure members |

Every `HAS_MEMBER` relationship carries `membership_role`: `claim`, `content`,
`actor`, `infrastructure`, or `narrative`. These edges have no claim-support
semantics.

```cypher
MATCH (c:Campaign)-[m:HAS_MEMBER]->(x)
RETURN c.campaign_id, m.membership_role, labels(x), x.claim_id, x.record_id
ORDER BY c.campaign_id ASC
LIMIT 200
```

The projection label `Campaign` here is the neutral epistemic collection. A
canonical `Campaign` node created through Cypher in the CTI vocabulary is a
different record and does not appear in the projection.

## Coordination signals and independence

`detect_campaign_signals` reads the whole claim set of one collection,
including the claims of every narrative a campaign collects, and
`Graph::record_campaign_signals` retains each finding as one content-addressed
annotation in the evidence store. Five signals exist, and each needs at least
two records from two distinct sources:

- repeated prompt artifact;
- generation-style fingerprint;
- cross-content redundancy;
- shared infrastructure;
- collection co-membership.

Every finding carries its exact records, resolved sources, measurement,
threshold, and the attribution of each instrument that reported it. Recording
a signal changes nothing factual and replays idempotently.

Retained signals feed [source independence](verdicts-and-actionability.md#independence-clusters):
links whose records share a signal group join one dependency cluster, with the
signal kept as the reason. Two limits apply. Co-membership never collapses
independence, because thematic grouping is curation, and letting it deflate
support would let an analyst weaken the claims they collect. And a signal is
provenance, never authorship.

## Attribution is never derived

`Graph::assess_campaign_attribution` checks whether an attribution request is
admissible and records nothing:

| Refusal | Cause |
| :--- | :--- |
| `CoordinationSignalsOnly` | The request cites coordination signals and no supported claim. |
| `CorroborationNotSupported` | A cited claim is not `Supported` at the assessment point. |

An admissible request names the supported claims an attribution may rest on.
The caller then asserts the attribution as an ordinary governed claim with its
own evidence. A fingerprint says content shares a production pattern; it never
says who produced it.

## Misleadingness in the FIMI pack

The `corrobore-domain-fimi` pack models six mechanisms: unsupported inference,
exaggeration, omission, framing or context shift, emotional arousal, and
communicative intent. Five are mechanically checkable from cited records;
communicative intent needs an explicit analyst attestation. An assessment
records the reader interpretation beside the evidence-warranted one, and a band
is a function of that gap and the number of distinct mechanisms, so it cannot
rise without a mechanism that explains it.

A misleadingness assessment is never a factual determination. The pack's
`claim.verify/1` implementation reports `inconclusive` only, which the core
keeps advisory with no verdict weight. "Claims mostly supported" and "highly
misleading" can therefore be true of the same piece, and neither field can move
the other.

## FIMI export fields

`export-fimi` records gain two additive fields beside the WS-A lineage:

- `campaign_lineage`: every collection referencing the record and the role that
  matched (`claim`, `content`, `actor`, `infrastructure`), with its themes,
  valid-from stamp, collected narratives, and retained coordination signals,
  each marked `attribution: "not_asserted"`.
- `misleadingness`: the assessments the pack recorded as evidence under the
  `fimi_misleadingness` payload key, each marked
  `not_a_factual_determination`. The exporter carries what the pack wrote and
  derives no band.

Verdict state and confidence band stay in the claim's lineage entry; band,
mechanisms, and interpretation gap stay in the assessment. Both fields are
omitted when empty, so a graph without these records exports the bytes it did
before. See [Exporters](exporters.md#fimi).

## Guidance for agents

- Use collections to organise an investigation, not to assert support or
  responsibility.
- Report a coordination signal as "shares a production pattern with", never as
  "was produced by".
- Attribute only through a governed claim that the engine holds `Supported`,
  and read its [audit](claim-audit.md) first.
- Report the misleadingness band and the factual verdict as two findings.

## Canonical references

- [Verdicts and Actionability](verdicts-and-actionability.md)
- [Intelligence Domains](domains.md#fimi-model-surface)
- [Exporters](exporters.md)
- [Architecture](../architecture.md#neutral-narrative-and-campaign-records)

# Agentic Platform Foundations

Epic 0029 WS-H adds the mechanics an agent platform needs around the evidence
graph without letting any agent runtime object into it: one capability
catalogue behind every protocol, write authorization decided outside the
prompt, provenance for query answers, corrective routes that say what could
change a verdict, an authority cap on memory consolidation, and investigation
artifacts bound to live records.

This guide says what each mechanism does, where it is reachable today, and
what an agent or operator should do with it. Internals and acceptance
evidence are in [Architecture](../architecture.md#one-capability-catalogue-several-protocol-adapters).

## Where each mechanism is reachable

| Mechanism | Embedded Rust | HTTP | MCP plugin |
| :--- | :---: | :---: | :---: |
| Capability catalogue | `shared_runtime::CapabilityCatalogue` | routes are the `http` projection | tools are the `mcp` projection |
| Agent write policy and run budgets | `CypherGateway::execute_for_agent_run` | not exposed | not exposed |
| Why-provenance | `ExecutionResult.why_provenance` | not in the Cypher response | not exposed |
| Corrective routes, falsifiers, publish gate | `Graph` API | `falsifiers`, `corrective_routes` in `GET /v1/claims/{id}/audit` | `corrobore_claim_audit` |
| Memory fusion back-pointers and authority cap | `ConsolidateRequest` | `consolidate` in `POST /v1/memory/operations` | `corrobore_consolidate` |
| Investigation artifacts | `Graph` API | not exposed | not exposed |

"Not exposed" means the runtime holds the record and the guarantee, and no
public route renders it yet. Do not infer the mechanism from a route that does
not exist.

## One capability catalogue, several adapters

`CapabilityCatalogue::v1` defines each capability once: identity, effect on
state (`read` or `mutation`), the authorization a caller must hold (`session`,
`mutation_permission`, or `operator_approval`), and the runtime operation it
dispatches to. There is no route, tool name, or protocol version in the
definition. Adapters render the catalogue into their own surface and must cover
exactly what it exposes to them.

| Capability | Effect | Authorization | Exposed to |
| :--- | :--- | :--- | :--- |
| `claim.audit` | read | session | all adapters |
| `cypher.read` | read | session | `http` only |
| `cypher.write` | mutation | mutation permission | `http` only |
| `memory.recall`, `memory.trace` | read | session | all adapters |
| `memory.remember`, `memory.relate`, `memory.update`, `memory.forget` | mutation | mutation permission | all adapters |
| `memory.consolidate` | mutation | operator approval | all adapters |
| `runtime.ready` | read | session | all adapters |
| `stix.export` | read | session | all adapters |
| `stix.import`, `stix.validate` | mutation | mutation permission | all adapters |

Raw Cypher execution is restricted to HTTP, so it is not an agent tool. The
committed artifact is `compatibility/capabilities/v1/catalogue.json`; a Rust
contract test compares it to the code, and `scripts/ws-h-capability-adapters.test.mjs`
holds the MCP bridge to it: the tool set must match the `mcp` projection, each
tool's `readOnlyHint` must match the defined effect, and an operator-approval
capability must be advertised as destructive. The convention that turns
`memory.recall` into `corrobore_recall` lives in that adapter contract, not in
the definition. See [Agent Plugin](../agent-skill.md#included-mcp-tools).

## Agent write policy, run budgets, and the mutation chain

`authorize_agent_write` decides whether an agent run may write. Its signature is
the guarantee: it never receives the query text or the prompt. It reads only
trusted context the gateway resolved before the request arrived: the tool, the
data domain, the write permission, the egress targets, the approval grant, and
the run budget. Every unmet condition is retained, so an operator sees the
whole list:

| Denial | Cause |
| :--- | :--- |
| `tool_not_allowed` | The tool is not in the policy. |
| `data_domain_not_allowed` | The data domain is not in the policy. |
| `write_not_permitted` | The policy grants no write. |
| `egress_denied` | An outbound target is outside the egress policy. |
| `approval_missing` | The policy requires an approval the request does not carry. |
| `budget_exhausted` | The run has spent one of its four budget dimensions. |

`CypherGateway::execute_for_agent_run` applies the decision before parsing. A
query that declares itself read-only is not taken at its word: when the policy
grants no write, any write-shaped query is refused too, so an injected
instruction is inert. The refusal is an ordinary rejected response carrying
`WRITE_PERMISSION_REQUIRED` and never reaches the executor, so a denied write
leaves no partial effect.

A run budget has four dimensions checked in a fixed order: tokens, cost in
micro-units, tool calls, wall-clock milliseconds. `RunBudget::strict_default`
is 200 000 tokens, 5 000 000 micro-units, 64 tool calls, and 600 000 ms. The
first exhausted dimension is reported, so the same measurement always names the
same cause.

Every accepted mutation is appended to a `MutationAuditChain` naming the run,
the tool call, the signing service identity, the actor, the tool, and the data
domain. Each entry commits to its predecessor's digest; `verify` names an
edited, removed, or reordered entry, and `head_digest` is the value an external
witness records. A refused decision cannot be appended.

Runtime objects (agent definitions, sessions, runs, tool calls) live in the
control plane. The graph carries only an opaque `RuntimeRef` that nothing reads
structure into, per ADR-0019.

## Why-provenance for query results

`ExecutionResult.why_provenance` explains what a read answer was computed
from. The planner declares the read set in a `ProvenancePlan`; the executor
records what it actually bound, so the result is a measurement against a
declaration. Two kinds stay in separate fields:

- `computational` is causal: this query bound that node and read that
  property. It says nothing about whether the answer is true.
- `semantic_support` is evidential: for every claim the answer read, the
  supporting and refuting links the graph retains with the claim's projected
  verdict state.

`WhyProvenance::is_computational_only` is true for an ordinary structural
query that read no claim. One aggregate row is one causal read set, the union
of the rows that fed it. A mutation carries no read set; its provenance is the
mutation record.

The field is available on the executor result in embedded use. The
shared-runtime `CypherResponse`, and therefore `POST /v1/cypher/read`, does not
carry it yet. Exporters add a PROV-O reading beside the Corrobore lineage:
each `x_corrobore_lineage` entry gains a `prov` object where an observation
`prov:wasDerivedFrom` its source and a claim `prov:wasGeneratedBy` its retained
verdict, which `prov:used` the linked observations. A claim without a stored
verdict gets no synthesized activity. See [Exporters](exporters.md#stix-21).

## Corrective routes, falsifiers, and the publish gate

A supported claim is not a closed claim. Two stores record how Corrobore could
change its mind:

- **Corrective routes**, per claim type, are channels for re-checking a claim
  against the world: `authoritative_api`, `primary_document`, `sensor`,
  `signature_check`, `independent_retrieval`, or `human_review`, each with a
  liveness (`live`, `unavailable`, `retired`).
- **Falsifiers** record, per claim, what evidence would change its state, which
  state it would move to, and which routes could produce that evidence. A
  falsifier must name a state other than `Supported`, because evidence that
  would confirm a claim is corroboration, not a falsifier.

`rank_corrective_routes` delegates eligibility to the Epic 0020 next-best-evidence
ranking, so budget, policy, and source-risk limits keep one implementation, and
adds one rule of its own: a route whose channel produced the claim is
`self_consistent` and sorts after every `independent` route, whatever its
expected value. Asking the pipeline that produced a claim to agree with itself
is not a correction. Producing channels are derived from the extraction lineage
of the claim's evidence.

`Graph::evaluate_publish_gate` composes the WS-D actionability decision and adds
two conditions for a **high-impact** claim: a live route on a channel that did
not produce the claim, and a recorded falsifier. Blockers are retained
separately:

| Blocker | Meaning |
| :--- | :--- |
| `not_actionable` | The [actionability gate](verdicts-and-actionability.md#actionability-is-a-separate-gate) is blocked; its own blockers are carried alongside. |
| `corrective_route_missing` | No route is registered for the claim type. |
| `corrective_route_not_live` | Routes exist but none is live. |
| `self_consistent_routes_only` | Every live route runs on a channel that produced the claim. |
| `falsifier_missing` | Nothing recorded could change the verdict. |

A standard-impact claim keeps the WS-D gate alone. The claim audit carries the
recorded `falsifiers` and the `corrective_routes` they name, and reports a
`no_recorded_falsifier` gap in `unverified_steps` when nothing could change the
verdict. Agents should surface that gap as part of "what has not been
checked". See [Claim Audit](claim-audit.md).

## Memory fusion back-pointers and the authority cap

A `consolidate` operation retains a fusion lineage: every atomic origin that
produced the fused memory, with the sources each one cites, ordered by memory
identity. Authority is capped, never summed. Each origin contributes
`min(asserted confidence, granted authority)` for each source it cites, resolved
against a registered WS-D source authority policy, and the fused authority is
the **maximum** over active origins. Remembering a weak observation five times
yields the authority of remembering it once. A source with no binding
contributes nothing, and a fusion evaluated without a policy carries
back-pointers and no justified authority.

Two additive fields enter the `memory/v1` consolidate input:

```json
{
  "contract_version": "v1",
  "operation": "consolidate",
  "idempotency_key": "consolidate--incident-42--rev-2",
  "input": {
    "mode": { "mode": "propose" },
    "memory_ids": ["mem--a", "mem--b", "mem--c"],
    "canonical_id": null,
    "reason": "Withdraw the retracted wire report from the fused timeline.",
    "preserve_disagreements": true,
    "authority_policy": {
      "version": "authority-v3",
      "authority_domain": "cti",
      "predicate_class": "infrastructure-ownership"
    },
    "revoked_source_ids": ["source--wire-2026-09-01"]
  }
}
```

`revoked_source_ids` withdraws sources and recomputes the interpretation;
every origin stays enumerated and every original memory survives. An origin is
inactive only when nothing it cites remains. Both fields enter the proposal
identity, so a revocation is its own governed decision and an earlier approval
never covers a different set of live sources. A named policy that is not
registered fails the operation closed. See
[High-level Memory Operations](memory-operations.md#consolidation-safety).

## Investigation artifacts bound to live records

An artifact is a view, not a fact. `Graph::create_artifact` retains a
timeline, evidence map, claim matrix, campaign graph, or analyst brief as a
version lineage whose versions bind governed records **by identity**: claims,
evidence, narratives, and campaigns. The type has a title, analyst
annotations, and bindings, and no field for a factual statement, so an
artifact cannot become a stale second copy of the evidence. An artifact that
binds nothing is refused.

- `Graph::regenerate_artifact` appends a version whose bindings are what the
  records say now, carrying every annotation forward; earlier versions are
  never rewritten.
- `Graph::annotate_artifact` appends a note the same way.
- `Graph::set_artifact_publication` is a permissions decision, not an
  epistemic one: it reads no verdict and changes no claim. A brief about a
  refuted claim publishes like any other, and a supported one may stay a draft.

Restoration refuses an artifact naming a record the snapshot does not carry,
so an artifact cannot outlive its bindings. Graphs without artifacts keep their
existing snapshot bytes.

## Guidance for agents

- Call the capability you need through the adapter you have; do not look for
  a route or tool the catalogue does not project to your adapter.
- Treat `WRITE_PERMISSION_REQUIRED` and a budget refusal as governed outcomes.
  Report them; do not rephrase the query or switch route shape.
- When you report a query answer, say whether you are showing what the query
  touched or what the evidence supports; they are different fields.
- When you assert a verdict, report what could change it. A
  `no_recorded_falsifier` gap is a finding, not a formality.
- When consolidating memories, name the authority policy and list revoked
  sources explicitly; never expect repetition to raise authority.
- Publishing an artifact does not make its bound claims true, and a supported
  claim does not publish an artifact.

## Canonical references

- [Verdicts and Actionability](verdicts-and-actionability.md)
- [Claim Audit](claim-audit.md)
- [High-level Memory Operations](memory-operations.md)
- [Agent Plugin](../agent-skill.md)
- [Architecture](../architecture.md#investigation-artifacts-bound-to-live-records)

# Natural-Language Queries

Corrobore can be asked questions in French or English through an optional,
compile-only adapter: the `corrobore-nlq` crate. A compiler (a template engine
today, a model tomorrow) turns a request into one typed action in the
versioned `nlq/v1` envelope; the envelope is validated against a trust
boundary the caller sets, canonicalized through the real Cypher, `INVESTIGATE`
and `memory/v1` parsers, and only then handed to an adapter that may execute
it. Nothing in this crate executes anything, and the runtime carries no
machine-learning dependency.

This is item 1 of the multilingual small-language-model programme
([corrobore#82](https://github.com/Estance-Labs/corrobore/issues/82)). The
decision behind it is ADR-0020 in `project-documents`; model bake-off,
fine-tuning and packaging are later items.

## Why compile-only

A model's output is untrusted text. The envelope gives it a shape a machine
can check, and the check refuses everything that would let text acquire
authority:

- **Exactly one action** per envelope. A stray key on an action is refused as
  an unknown field, so a second action cannot hide inside the first.
- **No trusted context.** `workspace_id`, `session_id`, `actor_id`,
  `agent_id`, `permissions`, `request_id`, `correlation_id`, `budget_ref` and
  `idempotency_key` have no field in the envelope and are refused wherever they
  appear, at any depth. The runtime decides authorization from context it
  resolved itself.
- **Evidence is caller-supplied.** Every evidence reference the envelope uses,
  and every provenance source a memory operation cites, must be in the allow
  list the caller passed; anything else is `invented_evidence`.
- **Writes need permission.** A `cypher_write_proposal` or a writing memory
  operation is refused when the caller did not allow writes; a `cypher_read`
  that mutates is `read_emits_write`.
- **Reads are bounded.** A `cypher_read` needs a `LIMIT` or an aggregate-only
  projection.

Compile and execute are separate operations. A host inspects the validated,
canonical action, applies its own authorization, and submits it through the
adapter it already uses (HTTP, MCP, embedded). The [capability catalogue](../architecture.md#one-capability-catalogue-several-protocol-adapters)
and the [agent write policy](../architecture.md#agent-write-policy-run-budgets-and-the-mutation-chain)
still apply; a compiled action gains no path they do not project.

## The envelope

```json
{
  "schema_version": "nlq/v1",
  "language": "fr",
  "action": { "kind": "cypher_read", "query": "MATCH (n:Indicator) RETURN n LIMIT 50" },
  "bounded": { "limit": 50 },
  "evidence_refs": [],
  "reason_code": "compiled"
}
```

| `action.kind` | Payload | Canonical form after validation |
| :--- | :--- | :--- |
| `memory_operation` | `operation` (`remember`, `relate`, `recall`, `update`, `forget`, `consolidate`, `trace`) and its `input` | `memory/v1 <operation> <canonical JSON of the typed request>` |
| `cypher_read` | `query` | the parser's normalized query text |
| `cypher_write_proposal` | `query` | the parser's normalized query text; never executed by the crate |
| `investigation` | `statement` | `InvestigationQuery::to_canonical_string()` |
| `clarification_required` | `question`, `options` | `clarification_required: <question>` |
| `abstain` | `reason` | `abstain: <reason>` |
| `unsupported` | `reason` | `unsupported: <reason>` |

The last three are terminal: the exchange stops with a typed answer rather
than a guess. Rejections carry one of thirteen stable codes (`malformed`,
`unknown_field`, `unsupported_schema_version`, `trusted_context_supplied`,
`invented_evidence`, `unbounded`, `read_emits_write`, `unsupported_cypher`,
`action_kind_mismatch`, `write_not_allowed`, `unsupported_investigation`,
`invalid_memory_input`, `unknown_memory_operation`).

The committed contract is `compatibility/nlq/v1/action-envelope.schema.json`
with replayable `fixtures.json`; a Rust contract test keeps both in step with
the code.

## Use it from Rust

```rust,ignore
use corrobore_nlq::{NlqCompiler, NlqRequest, TemplateCompiler, TrustBoundary, validate};

let request = NlqRequest::new("Quels acteurs de menace utilisent le malware \"X-Agent\" ?")
    .with_evidence_refs(["span--1".to_owned()])
    .allow_writes(false);
let envelope = TemplateCompiler.compile(&request);
let validated = validate(&serde_json::to_value(&envelope)?, &request.boundary())?;
assert_eq!(
    validated.canonical(),
    "MATCH (a:ThreatActor)-[r:USES]->(m:Malware) WHERE m.name = 'X-Agent' RETURN a.name LIMIT 50",
);
// Now decide whether to run it, and run it through the adapter you already use.
```

A model backend implements `NlqCompiler` and returns an `Envelope`; its output
goes through the same `validate` call. A model that emits JSON directly is
validated by passing that JSON to `validate`.

## The template compiler

`TemplateCompiler` is the deterministic baseline and the fallback when no
model is configured. It understands the recurring shapes in both languages and
refuses everything else with a typed result:

| Request shape (French / English) | Action |
| :--- | :--- |
| `Quels acteurs de menace utilisent le malware "X" ?` / `Which threat actors use the malware "X"?` | `cypher_read`: `MATCH (a:ThreatActor)-[r:USES]->(m:Malware) WHERE m.name = 'X' RETURN a.name LIMIT 50` |
| `Quelles campagnes ciblent l'identité "Y" ?` / `Which campaigns target the identity "Y"?` | `cypher_read` over `TARGETS` |
| `Liste les indicateurs` / `List the indicators`; `Montre les 20 premiers acteurs de menace` / `Show the first 20 threat actors` | `cypher_read`: `MATCH (n:Label) RETURN n LIMIT n` |
| `Combien de campagnes ?` / `How many campaigns?` | `cypher_read`: `MATCH (n:Campaign) RETURN count(n)` |
| `Enquête sur l'attribution de la campagne c` / `Investigate the attribution of campaign c` | `investigation`: `INVESTIGATE attribution OF Campaign("c") RETURN assessment, counter_evidence, unknowns, next_best_evidence` |
| `Que sais-tu de l'infrastructure d'APT28 ?` / `What do you know about the infrastructure of APT28?` | `memory_operation` `recall` with a language-neutral objective (`infrastructure APT28`) |
| `Retiens que "…" (source span--7)` / `Remember that "…" (source span--7)` | `memory_operation` `remember` when writes are allowed and `span--7` was supplied; `clarification_required` without a source; `abstain` when the source was not supplied or writes are not allowed |
| `Supprime tous les …` / `Delete all …` | `unsupported`, even with write permission |
| `Marque … comme validé` under a read-only task | `abstain` |
| `Parle-moi de X` / `Tell me about X` | `clarification_required` with the three readings |
| Anything else | `unsupported` |

Labels are recognised in both languages (`acteurs de menace` / `threat
actors`, `logiciels malveillants` / `malwares`, `indicateurs`, `campagnes`,
`identités`, `infrastructures`, `vulnérabilités`, `outils`, `rapports` and
their English forms), including code-switching such as `Liste les threat
actors`. User literals keep their case and are never translated; quotes win
over the last word. The language tag is detected from cue words or taken from
the request.

## Dataset and evaluation

`corrobore_nlq::dataset::generate(seed)` derives a pilot corpus from semantic
records: 15 template families, 16 records each, one French and one English
surface per record sharing a canonical expected outcome and their literals
verbatim. Five families are adversarial or abstention cases (destructive
requests, prompt injection inside a request, fabricated evidence, ambiguity,
off-topic and unsupported intents), which is more than a quarter of the
corpus. The manifest records generator version, license, seed, families and a
SHA-256 content hash. `split(corpus, seed)` partitions by template family
(60/20/20 by count, ordered by a seeded hash); because each family draws
entities from its own pool, entity sets are disjoint across splits too, and a
record's two languages always land together.

`corrobore_nlq::evaluation::evaluate(compiler, examples)` validates every
envelope exactly as a host would and reports, per language and in aggregate:
schema validity, canonical accuracy, action-class accuracy, abstention
accuracy, invented evidence, writes under read-only tasks, and unsupported
requests recompiled into an action, plus the agreement rate between the two
surfaces of each record.

```bash
cargo run -p corrobore-nlq --example nlq_dataset -- 42 report
```

Recorded baseline for this item (seed 42, 480 examples, 240 per language):
the template compiler scores 1.0 on schema validity, canonical accuracy,
action-class accuracy and abstention accuracy in both languages, with zero
invented evidence, zero read-only writes and zero recompiled unsupported
requests, and a pair agreement rate of 1.0. That is a floor for a model, not a
ceiling: the templates only cover the shapes they define.

## Boundaries

- No model ships with Corrobore; the crate is the contract, the baseline and
  the harness. Model selection and fine-tuning are later items of corrobore#82.
- The template compiler covers a small set of shapes and says so; it does not
  paraphrase, resolve co-reference or reason.
- The corpus is a pilot for the contract and the harness, not the release
  dataset; its size follows the learning curves the later items measure.
- French and English are the release languages; other tags travel in the
  envelope but are not evaluated.

## Verification

`crates/corrobore-nlq/tests/envelope_contract.rs` (trust boundary,
canonicalization, closed envelope), `template_compiler_contract.rs` (paired
French and English requests, literals, refusals),
`dataset_and_evaluation_contract.rs` (determinism, pairing, adversarial share,
leakage-safe splits, per-language metrics, a misbehaving compiler caught) and
`compatibility_artifacts.rs` (fixtures replay, schema in step with the code).
`scripts/nlq-contract.test.mjs` checks the committed artifacts and this
page's wiring.

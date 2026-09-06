# Synthetic red-team engine adapter

The `graph-core` example `red_team_adapter` executes the versioned synthetic red-team workload through source/observation ingestion, bitemporal claim links, evidence risk detection, independence aggregation, and the current verdict resolver. It accepts one JSON request on standard input and emits one JSON response. Errors terminate with a nonzero exit code instead of becoming successful attack rejections.

```sh
cargo build -p graph-core --example red_team_adapter --locked
cargo test -p graph-core --example red_team_adapter --locked
```

The canonical synthetic corpus is in `Estance-Labs/project-documents`, at `datasets/red-team/v1/corpus.json`. Corpus publication and adapter tests alone do not establish completion of issue #213: the benchmark runner must execute all eight families, volume sweeps, feedback-driven adaptive rounds, and paired utility/attack gates.

## Request v1

Supply `schemaVersion: "corrobore-red-team-request-v1"`, `mode`, `asOf`, `authorityPolicy`, `documents`, and `queries`. The authority policy carries `version` and `trustedSourceIds`. Documents and queries use the corpus fields. The runner expands attack templates into distinct document identities before invocation. Never pass the corpus `gold` object: unknown request fields are rejected, including evaluator labels.

Each request starts a fresh graph. The runner owns cumulative adaptive history and passes the complete document set for that round. Limits are 128 documents, 16 queries, and 4 MB of input. These limits accommodate the published 100-document attack budget plus primary records.

Assertions are structured synthetic facts: subject, predicate, and JSON value. Equal values support a matching query; unequal values refute it. This adapter does not claim natural-language extraction or language-model evaluation. Claimed authority titles and camouflage context do not confer authority on the malicious assertion. Only the externally configured source policy grants source weight. Temporal validity is represented by actual engine bitemporal links; expired evidence remains retained but cannot contribute to a current verdict.

Modes:

- `defended`: run the engine risk detector and immune response before resolving. Batches cover every pair and triple within the engine's 64-record assessment limit, including cross-batch duplication and temporal bursts. Source authority and evidence-risk weighting retain their normal engine semantics.
- `unprotected`: skip risk detection while retaining the same configured authority and temporal policy. This supplies the measured clean utility reference; it is not the deliberately unsafe volume-voting control.
- `quarantine-all`: explicitly place every evidence record in quarantine and refuse all source authority. This deliberately excessive control should lose benign utility. It is a test configuration, not a change to the production verdict resolver.

## Response v1

The response identifies `corrobore-red-team-response-v1`, `engine: "graph-core"`, and the aggregation policy version. `verdicts` maps query identifiers to actual engine states. `quarantinedCount` counts distinct quarantined evidence records; `tierTransitions` and per-query `audits` retain the engine explanations. The benchmark should record the executable build revision and canonical corpus revision/checksum alongside these measurements.

Expected answers remain harness-owned. A correct result requires both attack resistance and benign utility; an absent, errored, mixed, or unknown answer must not be silently counted as a correct benign verdict.

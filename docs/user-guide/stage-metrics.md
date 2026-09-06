# Pipeline stage metrics

The `corrobore-stage-metrics-v1` contract separates seven instrumented pipeline
stages. It complements existing quality KPIs; it does not derive stage results
from end-to-end F1, a claim verdict or a graph snapshot.

## Count meanings

Each completed measurement reports its own `inputs`, `outputs` and `failures`.
Inputs count submitted work items, outputs count produced artifacts, and failures
count input work items that failed processing. Fan-out is allowed: outputs may
exceed inputs. Failures cannot exceed inputs. A valid abstention or a verifier
returning `fail` is a completed output, not a processing failure. Correctness,
recall and judge accuracy need independent reference evaluation and are not
implied by these counters.

| Stage | Input unit | Output unit |
| --- | --- | --- |
| `extraction` | documents | candidate_assertions |
| `entity_resolution` | mention_comparisons | identity_decisions |
| `retrieval` | retrieval_requests | evidence_items |
| `subgraph_construction` | seed_sets | subgraphs |
| `evidence_sufficiency` | claim_evidence_sets | sufficiency_assessments |
| `verifier` | verification_requests | verification_records |
| `verdict` | claims | stored_verdicts |

A measurement is a disjoint completed batch. Use a stable measurement ID for
retries, and new IDs for distinct batches. Do not submit successive cumulative
snapshots as new batches: that would double-count prior work. `producer` identifies
the instrumentor/configuration version, not an attested identity. Count a stage
only after its actual operation completes; do not fill missing stages with guessed
counts or mirror one stage's results into another.

## Instrument the engine

A host records completed stages through
`CorroboreEngine::record_pipeline_stage(run_id, measurement)`, where
`StageMeasurement::new(id, stage, producer, inputs, outputs, failures)` validates
attribution and count bounds. The engine emits `PipelineStageReport` from these
observations. External extraction, retrieval and judge integrations instrument
their own stage boundaries; the schema does not pretend those operations run
inside Corrobore. No automatic instrumentation is implied for unmodified callers.

The authenticated HTTP bridge accepts the same typed measurement at
`POST /v1/metrics/stages/{run_id}`:

```json
{
  "schema_version": "corrobore-stage-metrics-v1",
  "measurement_id": "retrieval-batch-1",
  "stage": "retrieval",
  "producer": "retriever-config-v1",
  "inputs": 10,
  "outputs": 27,
  "failures": 2
}
```

GET on the same path returns the direct JSON report. Every response contains all
seven stages in the order above. Each row carries its units, distinct producers,
measurement count and counters. An unmeasured stage has zero measurements, no
producers and **null** counts. An observed empty batch has measured zero counts;
neither case establishes processing success over a nonempty sample.

## Reliability and scope

The run/stage/measurement identity is immutable: exact retries succeed without
incrementing counts, conflicting retries fail. Individual and aggregate counters
are bounded to 9,007,199,254,740,991 for lossless JavaScript consumption. Invalid
input, overflow and capacity failures leave the existing report unchanged.
Schema versions and stage names are closed enums; incompatible versions are
rejected rather than compared as if their meanings were identical.

This is bounded **per-engine telemetry**, like read metrics, not persisted claim
history. The default registry retains at most 256 runs and 4,096 measurements per
run, rejecting further records without evicting earlier measurements. Embedded
registries may use explicit positive bounds. Archive reports in benchmark results
before ending the engine lifetime; a restart has no prior run reports and returns
404, never a successful empty report. Telemetry recording does not mutate graph
evidence, verification records or claim verdicts.

## Benchmark report consumption

The `corrobore-benchmarks` product input accepts an optional `stageMetrics` object
containing the exact engine response. Product JSON adds a separate `stageMetrics`
map, and Markdown appends a seven-stage table without altering existing KPIs.
Legacy inputs remain readable and their stages are explicitly unmeasured.

Comparisons require the v1 schema, all seven stage names and the same units.
Producer/configuration changes remain visible rather than hiding the comparison.
A seeded retrieval processing failure changes only retrieval's counters and
failure-rate delta. Output volume is diagnostic, not a correctness score; release
quality gates also need the subsequent oracle, reference and adversarial suites.

The shared fixture `crates/graph-core/tests/fixtures/pipeline-stage-metrics-v1.json`
is checked against both registry serialization and the HTTP engine response.
`pipeline_stage_metrics` tests cover all stages, missing data, retries, bounds,
versions, authentication and unchanged graph state. The benchmark counterpart
[#38](https://github.com/Estance-Labs/corrobore-benchmarks/issues/38) exercises the
normal product report command with those exact fixture bytes.

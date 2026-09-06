# Evidence presence, reachability and residuals

`graph_core::measure_evidence_access` evaluates one verifier input trace against
one graph snapshot and bitemporal point. It returns the serializable
`corrobore-evidence-access-v1` report without modifying the graph or verdict.

Supply the actual `VerificationInputs` recorded by the verifier, the corresponding
`VerdictAsOf`, independent expected evidence IDs, and the IDs actually retrieved
for that verification. Do not construct the trace from every link in the graph:
that would hide retrieval failures. The host must retain the report alongside
its run ID and graph snapshot; this function does not persist telemetry or add
an HTTP endpoint. Existing verifiers are not automatically instrumented.

| Series | Numerator | Denominator |
| --- | --- | --- |
| `presence_rate` | Expected evidence records present in the graph | Unique expected IDs |
| `reachability_rate` | Expected records retrieved through the connected examined links | Unique expected IDs |
| `residual_evidence_rate` | Retrieved records with no active claim explanation | Unique retrieved IDs |

Empty denominators produce `null`, not zero or perfect coverage. IDs are
deduplicated and output lists sorted. Missing expected records are valid missing
reference evidence; unknown retrieved records or invalid examined links are
input errors. The report retains both denominators, the reference subsets,
residual IDs, input trace and bitemporal point.

Reachability follows only the active links named in the trace, backwards from
the verified claim through claim-to-claim links to evidence or observations.
A record bound to a linked observation is reachable through that observation.
An observation explicitly named in `VerificationInputs.observation_ids` is also
a direct recorded read, so its retrieved evidence is reachable without a claim
link. Such a read can still be residual if no active claim explains the evidence.
A disconnected examined link is rejected. A globally present or globally linked
record cannot become reachable without a recorded path and membership in the retrieved
set. This measures the evidence reachable through the recorded input path; it
cannot attest whether a remote model actually used every supplied item.

Residuals use active links across all claims in the supplied graph scope.
Support, refutation, contradiction and contextual links all explain why evidence
belongs to a claim; explanation is not agreement or a correct verdict. An
observation link also explains its bound evidence. Unretrieved expected evidence
is a retrieval gap, never a residual. For tenant/workspace-specific evaluation,
supply the corresponding authorized graph snapshot rather than unrelated claims.

## Drive further investigation

Nonempty `present_but_unreachable` emits an `ExpandRelation` proposal. Missing
reference evidence and residual evidence emit separate `SearchCorpus` proposals
with stable reason codes and exact evidence IDs. A residual proposal should
search for missing claims, conflicting accounts or sub-narratives; the retrieval
host can pass new material to its extraction pipeline.

Call `proposal.candidate(id, score, constraints)` to feed the existing
`rank_next_best_evidence` planner. The host supplies calibrated utility/cost
terms and its actual policy and remaining budget. A denied or exhausted candidate
cannot be selected. A selected external proposal still goes through the host's
execution boundary. No network request is performed by metric calculation.

## Reproduce the fixture

Run `cargo test -p graph-core --test evidence_access_metrics`. The versioned
fixture `crates/graph-core/tests/fixtures/evidence-access-v1.json` contains two
expected records, both present, but only one reached. A second retrieved record
has no claim explanation: presence is 1.0, reachability 0.5 and residual rate 0.5.
The contract test compares all series, evidence subsets and proposals with the
actual graph evaluation. Additional tests cover absent evidence, contradiction,
missing denominators, duplicate IDs, disconnected paths and budget enforcement.

These independently annotated coverage metrics complement [stage processing
counters](stage-metrics.md) and the retrieved-versus-oracle quality comparison.
They are not substitutes for verifier correctness or end-to-end benchmark F1.

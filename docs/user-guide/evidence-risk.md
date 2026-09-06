# Evidence-risk diagnostics and quarantine

The `ws-e-risk-v1` policy detects seven independently reported risk signals in
stored evidence. These are dependency and contamination heuristics, not proof
that an assertion is false or that a publisher acted maliciously.

| Signal | Required inputs and trigger |
| --- | --- |
| Lexical duplication | Stored payloads, at least four distinct alphanumeric tokens each; lowercase token-set Jaccard at least 0.9 |
| Semantic duplication | Nonzero embeddings in the same qualified model/version and dimension; cosine at least 0.98 |
| Shared infrastructure | An exact common attributed infrastructure identity |
| Shared upstream citation | An exact common canonical citation identity |
| Temporal burst | At least three distinct source identities publishing within 60 seconds |
| Generation fingerprint | An exact common attributed watermark or generation fingerprint |
| Embedding geometry anomaly | At least three embeddings in one model space; norm divided by peer median outside [0.05, 20] |

The norm diagnostic covers scale anomalies; it is not a general detector of
semantic manipulation or every possible geometric anomaly. Missing features
stay unmeasured and do not fire. Embedding spaces are never compared across
model versions. A generic model name is not a generation fingerprint, and a
broad hosting provider or ASN is not an appropriate infrastructure identity.
The host is responsible for supplying precise, attributed measurements.

## Assess stored evidence

`detect_evidence_risks(&graph, &claim_id, &features)` is read-only. Each
`EvidenceRiskFeatures::new(evidence_id, attribution)` starts with unknown
optional metadata; lexical text always comes from the existing evidence record.
Provide embeddings/model identity, infrastructure references, upstream citations,
UTC publication seconds and fingerprints when measured. Each record must be
bound to the assessed claim through an evidence or observation link.

An assessment accepts at most 64 distinct evidence records, 4096 coordinates per
embedding, finite coordinates bounded in magnitude by 1,000,000, 64 references
per metadata category, 1024 bytes per identity and 1 MB per evidence payload.
Invalid or inconsistent metadata fails the batch. These bounds limit pairwise
work; hosts should form meaningful claim-specific batches rather than treating
an arbitrary entire corpus as a temporal-burst cohort.

Every finding retains a stable content-derived group ID, its signal, exact
implicated evidence IDs, measured trigger/threshold and attribution. Input order
does not determine the result. Connected detections of the same signal are
coalesced into one group, with the original pair/window measurements retained
as structured witnesses; a dense copied corpus does not receive a separate
annotation for every pair. Thresholds are explicit versioned policy choices,
not universally calibrated estimates of attack probability.

## Apply the immune response

`Graph::apply_evidence_risks(claim_id, features, stamp, actor, tiers, immune)`
validates active claim bindings at the supplied bitemporal point and stages all
changes before committing them. Use an open-ended assessment stamp. A benign
batch leaves the graph unchanged. Exact retries do not duplicate annotations or
create no-op quarantine transitions.

Risk findings become append-only, content-addressed receipts in the evidence
store. Original evidence records retain references to these shared receipts.
Payload, source content and verifier records are unchanged. The existing
independence graph joins implicated records with explicit `EvidenceRisk`
reasons; the full affected dependency component is quarantined through
`ImmuneResponder` and `GraphTierRegistry`, including existing members outside
the detector batch. The original detector IDs remain distinct from the complete
quarantined component, so propagation is not misreported as a direct detection.

A suspect component receives one multiplier of 0.5, regardless of how many risk
signals it contains. The multiplier reduces each directional cluster
contribution once. The source-independence dimension uses the sum of these
component multipliers, `effective_components / (effective_components + 1)`,
for components whose provenance is known. Original authority and strength inputs
remain visible, and unmeasured dimensions remain absent. The penalty follows
the evidence when another claim uses it; assessments not yet known at an
historical bitemporal point do not alter that resolution.

These inputs remain below deterministic verification in ADR-0017 precedence.
A deterministic failure still refutes the claim even when other evidence has
high confidence. Risk is not itself a deterministic verifier result.

## Audit and persistence

Each shared receipt retains the finding, assessment stamp, quarantined
component IDs, immune responses and tier transitions including their actor and
reason. The full proof is stored once per assessment rather than copied into
every member of a large component. Native snapshots reject modified receipt
content and missing references. Native snapshots retain these receipts, and identical evidence
re-ingestion preserves them. Claim audit views include exact observation-bound
evidence and the transitive records named by risk annotations. Scoped audit
archives preserve that provenance without importing unrelated claims.

The runtime owns its tier registry and immune responder separately; persist and
restore those existing serializable objects alongside the graph when resuming
runtime tier state. Native graph export retains the historical response and
transition proof, not an implicit replacement for those runtime registries.
Use the tier registry's explicit reviewed promotion workflow to release records;
release does not erase historical risk findings or claim they never existed.
No automatic extraction, embedding generation, external call or periodic
assessment is added: the host invokes assessment at its evidence boundary.

Run `cargo test -p graph-core --test evidence_risk --test cluster_aggregation`.
Fixtures cover each signal, missing features, a fully measured benign cohort,
atomic rejection, retry, group quarantine, bounded proof storage for a large existing component, receipt
integrity, historical independence, cross-claim
reuse, scoped/native audit restoration and deterministic-verifier precedence.

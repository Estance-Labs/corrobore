# OpenCTI progressive read routing

Issue #49 turns the parity evidence from shadow reads into a reversible
production routing boundary. The default remains `reference_only`; Corrobore
cannot receive visible traffic until an operator supplies a validated policy.

## Decision order

For every typed read, the router:

1. classifies the operation and query class;
2. evaluates environment, operation, query class, entity, organization,
   tenant, cohort, feature flag, and deterministic-percentage selectors;
3. applies synchronization, freshness, availability, corruption, security,
   parity, error-rate, and latency gates;
4. enforces an existing provider/index-generation session binding;
5. persists the privacy-safe decision before provider execution.

The visible response always comes from exactly one primary provider. Optional
shadow execution is concurrency-limited and independently timed out; it can
only update the existing parity report stream.

## Modes

- `reference_only`: Elasticsearch/OpenSearch serves reads.
- `shadow`: the reference remains visible and Corrobore is compared.
- `canary`: matching deterministic cohorts use Corrobore.
- `graph_reads`: neighbors, traversals, and subgraphs use Corrobore; other
  reads stay on the reference.
- `primary_reads`: every supported read uses Corrobore while the reference is
  kept synchronized and shadowed.

Use [the complete policy example](../examples/opencti-read-routing.json) as the
starting point. A changed `policy_version` intentionally discards old sticky
bindings and circuit state rather than applying them to a semantically
different policy.

## Automatic rollback

Security divergence, corruption, unavailability, parity breach, excessive
error rate, excessive P95 latency, or a closed synchronization gate prevents
new Corrobore routing. The first cause opens a durable circuit breaker. If the
reference is fresh, subsequent eligible traffic immediately returns to it. If
it is not fresh, the request fails closed. Existing pagination sessions never
cross provider or index generations.

Operators can trigger the same bounded action with
`POST /v1/opencti/routing/rollback`. The durable state and audits live at
`runtime/opencti-read-routing.json` under the persistent storage root.

## Soak and promotion

Canary and full-read promotion requires the configured request volume, error
rate, and P95 threshold, zero parity breaches, and exactly zero security
divergences. Continue shadow validation throughout both stages. Provider
decisions and circuit state are available through `/metrics`; correlated,
payload-free explanations are available through
`GET /v1/opencti/routing/decisions`.

## Operator procedure

1. Confirm synchronization reports zero lag, empty queues, and parity enabled.
2. Run `shadow` for the evidence window.
3. Enable a narrow, feature-flagged `canary` rule.
4. Verify the soak gates before increasing the percentage.
5. Move graph-native operations to `graph_reads`, then supported reads to
   `primary_reads`.
6. On any gate breach, verify that the circuit is open, the reference is fresh,
   and new decisions select `reference` before investigating redacted reports.

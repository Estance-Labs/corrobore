# Standalone Server Acceptance Matrix

This matrix is the final automated acceptance gate for
[Epic #13](https://github.com/Noetance-Labs/corrobore/issues/13). It validates
the independently merged standalone-server slices as one product, using the
real unified executable and the actual production container image.

## Scope

The native and container runs execute the same public HTTP contract from
`scripts/standalone-acceptance.sh`. The client is `curl`, an external process
that links no Corrobore library. Both deployment adapters must pass:

- authenticated TLS liveness, readiness, version, Cypher write, and Cypher read;
- an unauthenticated rejection;
- a persistent write followed by a clean stop, restart, and read;
- exclusive persistent-directory ownership;
- bounded `SIGTERM`, successful exit, and durability flush;
- response and structured-log correlation identifiers;
- absence of the configured secret from preserved diagnostics.

The native adapter additionally proves configuration precedence
(`CLI > environment > TOML > defaults`) and invalid configuration failure.
Focused Rust contracts cover lifecycle transitions that cannot be observed
reliably by racing an external probe against a fast local initialization.

Bolt, clustering, high availability, replication, and performance benchmarking
remain outside this acceptance matrix.

## Epic acceptance traceability

| Epic #13 acceptance criterion | Automated evidence |
| --- | --- |
| Corrobore starts as an **independent foreground process**. | `standalone-acceptance.sh native` launches the unified executable and waits with a bounded external readiness probe. |
| A client connects through **HTTP without embedding** a Corrobore library. | The shared native/container scenario uses `curl` for `/health/live`, `/health/ready`, `/version`, `/v1/cypher/write`, and `/v1/cypher/read`. |
| Configured **data survives a clean server restart**. | `standalone-acceptance.sh native` and `standalone-acceptance.sh container` write, stop cleanly, restart on the same persistent root/volume, and read the record back. |
| Only **one server process** owns a persistent directory. | Both harness adapters start a competing instance and require exit code `4`; `persistent_ownership_contract` additionally covers independent handles and real processes. |
| **Invalid configuration** prevents startup with an actionable error. | The native harness requires exit code `2` and `configuration error:`; `cli_configuration_contract` covers precedence, validation, redaction, and listener absence. |
| **Readiness** follows storage recovery and initialization and becomes false while draining. | `lifecycle_contract` validates the initialization and draining state machine; `operational_contract` validates the external payload and storage metadata. |
| **SIGTERM** drains work and flushes persistent state within the configured bound. | Both harness adapters require bounded clean exit and successful persistent restart; `lifecycle_contract` covers active-work draining, forced timeout, and flush behavior. |
| Structured logs contain request **correlation identifiers** without secrets. | The shared scenario checks `X-Request-Id` and the JSONL log, rejects the configured token in artifacts, and is reinforced by `correlation_logging_contract` and `tls_security_contract`. |
| The server runs from a **native binary and a container image**. | The CI workflow builds `target/release/corrobore` and the repository `Dockerfile`, then runs the common harness against both. |
| **Embedded mode** works without server dependencies. | CI runs `cargo test -p corrobore-engine --locked`, records `cargo tree -p corrobore-engine`, and runs the dependency assertion in `engine_boundary_contract`. |
| Server behavior has **integration and acceptance tests**. | The common harness, this traceability contract, the complete workspace tests, and focused `cli_configuration_contract`, `persistent_ownership_contract`, `lifecycle_contract`, `correlation_logging_contract`, `tls_security_contract`, and `engine_boundary_contract` suites form the gate. |

## Issue #24 acceptance traceability

| Issue #24 criterion | Evidence |
| --- | --- |
| Every Epic #13 criterion is traceable. | The table above is enforced by `scripts/standalone-acceptance-contract.test.mjs`. |
| Native and container scenarios use the same public contract. | One `exercise_public_http_contract` function is called by both adapters. |
| Persistent data survives in both deployment forms. | Each adapter executes the initial and restarted phases against the same root or volume. |
| Concurrent ownership fails safely. | Both adapters require the documented ownership-conflict exit code. |
| Readiness is not early. | Deterministic lifecycle and operational Rust contracts run in the workspace suite. |
| Shutdown drains and flushes within bounds. | The harness timeout defaults to 30 seconds; lifecycle contracts cover active work and forced shutdown. |
| Correlated logs exclude secrets. | Headers, JSONL records, stderr, stdout, and container logs are preserved and scanned. |
| Embedded mode excludes server dependencies. | Dedicated CI commands build, test, inspect, and assert the engine boundary. |
| CI is reliable and actionable. | Job and probe timeouts are bounded; failure artifacts include responses, headers, process output, container output, configuration diagnostics, and structured logs. |

## Execution

Native:

```console
cargo build --release --locked -p corrobore-http-server --bin corrobore
CORROBORE_ACCEPTANCE_BINARY=target/release/corrobore \
  CORROBORE_ACCEPTANCE_ARTIFACT_DIR=acceptance-artifacts/native \
  scripts/standalone-acceptance.sh native
```

Container:

```console
docker build --tag corrobore-acceptance:local .
IMAGE=corrobore-acceptance:local \
  CORROBORE_ACCEPTANCE_ARTIFACT_DIR=acceptance-artifacts/container \
  scripts/standalone-acceptance.sh container
```

Set `CORROBORE_ACCEPTANCE_TIMEOUT_SECONDS` to a positive integer to change the
default 30-second bound. Diagnostics are always written under
`CORROBORE_ACCEPTANCE_ARTIFACT_DIR`; the workflow uploads that directory on
failure for 14 days.

## Release gate

No manual release gates remain. A version tag may be released only after this
workflow and the repository's Rust, security, documentation, and dependency
checks are green on the accepted commit.

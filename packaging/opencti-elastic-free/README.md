# Elastic-free OpenCTI distribution

This directory is the source-locked single-node distribution for OpenCTI
`7.260722.0` with `DATABASE_ENGINE=corrobore`.

- `compose.yml` defines the seven supported services and no Elasticsearch or OpenSearch service.
- `compose.migration.yml` temporarily connects an existing reference through an explicit secret during reversible migration.
- `Dockerfile.opencti` checks out the pinned Estance fork commit and validates its native Corrobore provider before building OpenCTI.
- `compatibility.json` is the machine-readable support boundary and records both the Estance provider commit and its upstream base.
- `profiles/` contains the certified small and conditional medium resource envelopes.

See [the operations guide](../../docs/user-guide/opencti-elastic-free-operations.md)
and [acceptance matrix](../../docs/acceptance/opencti-elastic-free.md). Runtime
secrets, `.env`, and durable migration state are deliberately ignored.

# Corrobore fuzz targets

Coverage-guided fuzzing for Corrobore's untrusted-input boundaries, built with
[`cargo-fuzz`](https://github.com/rust-fuzz/cargo-fuzz) / libFuzzer.

## Why this crate is isolated

libFuzzer requires a **nightly** toolchain, which conflicts with the
workspace's pinned stable toolchain (`rust-toolchain.toml`). To keep
`cargo build --workspace` and CI on stable, this crate declares its own empty
`[workspace]` table so it is **excluded from the main workspace** and is never
compiled by stable builds. It is run manually, on demand.

## Targets

| Target          | Boundary fuzzed                                                        |
| --------------- | --------------------------------------------------------------------- |
| `parse_query`   | `cypher_parser::parse_query` — the agent-facing query surface.        |
| `decode_record` | `graph_storage::decode_persisted_record_envelope` — the on-disk record page-in path. |

> Note: the audit asked for a "STIX import" target, but no STIX *import* path
> exists (`export-stix` only exports). The record decode path is the actual
> untrusted-bytes → typed-record ingestion boundary, so it is fuzzed instead.

## Running

```sh
# One-time: install the tool and a nightly toolchain.
cargo install cargo-fuzz
rustup toolchain install nightly

# Run a target (Ctrl-C to stop; findings land in fuzz/artifacts/).
cargo +nightly fuzz run parse_query
cargo +nightly fuzz run decode_record

# Time-boxed run (e.g. 60s), useful for local smoke checks:
cargo +nightly fuzz run parse_query -- -max_total_time=60
```

Reproduce a crash artifact:

```sh
cargo +nightly fuzz run parse_query fuzz/artifacts/parse_query/<crash-input>
```

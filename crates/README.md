# Crate layout

This directory contains the implementation crates for Corrobore.

Only crates listed in the root `Cargo.toml` workspace are compiled. Future crates may have planning directories before they become active Rust crates.

## Active crates

### graph-core

Core in-memory graph primitives.

Responsibilities:

- IDs
- nodes
- relationships
- properties
- record status
- confidence
- temporal metadata
- transaction metadata placeholders
- in-memory graph API

## Domain crates

### domain-common

Shared domain abstractions used by CTI, FIMI, crisis, and future intelligence domains.

The provider ABI names the `cti`, `fimi`, and `crisis` domains, but Corrobore distributes no implementation for them: no source crate in this workspace and no provider binary.

## Future infrastructure crates

Future crates may include:

- cypher-parser
- cypher-planner
- cypher-executor
- function-registry
- shared-runtime
- storage-api
- export-stix
- export-fimi
- audit-log
- snapshot-manager

These directories should be created only when their epic or ADR requires implementation work.

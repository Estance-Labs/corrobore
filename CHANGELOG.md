# Changelog

All notable changes to Corrobore are documented here. This root file is a pointer to
the detailed, per-release notes maintained under
[`docs/release-notes/`](docs/release-notes/).

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

Changes on `main` after `v0.3.3` that have not yet been tagged in a release.

### Added

- **Graph commit history (#317).** In persistent mode every applied write on
  `main` is an addressable graph commit (`gc` + 40 hex digits) with its parents,
  generation, transaction id and the record versions it published, kept in an
  append-only, checksummed `history/graph_history.log`. Stores that predate the
  history start it with a genesis commit; a commit lost between the WAL apply
  and the history append is republished from the transaction journals on the
  next open, and a torn final line is truncated. New routes
  `GET /v1/graph/commits` and `GET /v1/graph/commits/{commit_id}`. Snapshots
  and restores carry the history.

- **Commit-pinned reads (#318).** `POST /v1/cypher/read` and `/execute` accept
  `commit` or `branch` and run on a read-only projection of that commit;
  Cypher responses carry `graph_commit_id`. In persistent mode the explorer
  graph now reads a commit: the boundary's transaction, the newest commit at a
  timeshot's time, an explicit `commit`, or the `main` head. Previously it
  projected whatever graph the last request had paged in, and echoed the
  boundary without applying it. New routes `GET /v1/graph/records` (typed
  keyset pages pinned to a commit) and `GET /v1/graph/nodes/{node_id}`
  (relationship groups with neighbours).

- **Persisted branches (#319).** `GET/POST /v1/graph/branches` and
  `DELETE /v1/graph/branches/{*name}` create branches at any branch head or
  retained commit and delete merged ones (or any with `force`). Cypher writes
  with `branch` publish branch commits whose record versions live in
  `history/branch_payloads.log`, which the canonical catalog never indexes;
  `main` is unchanged until a merge. A branch write from a head that moved
  meanwhile answers `409 GRAPH_COMMIT_PRECONDITION_FAILED`. Every writer
  allocates graph ids above the highest id published on any branch.

- **Graph diff, merge base and change feed (#320).** `GET /v1/graph/diff`
  lists added, changed and removed records between two commits or branch
  heads, with totals, cursor paging and, on request, both versions and their
  property changes. `GET /v1/graph/merge-base` returns the best common
  ancestor. `GET /v1/graph/changes` is an at-least-once cursor feed of the
  commits published on a branch; a cursor no longer retained answers
  `410 CHANGE_FEED_GAP`.

- **Three-way branch merge (#321).** `POST /v1/graph/merge` merges a branch
  into `main` or another branch from the merge base: `already_up_to_date`,
  `fast_forward` or `merged`, with a `dry_run` preview, an optional
  `expected_target_head` and `delete_source`. Conflicts (`concurrent_update`,
  `update_delete`, `add_add`, `dangling_relationship`) are never resolved
  automatically and answer `409 MERGE_CONFLICT` with the full report. Merges
  into `main` go through the canonical WAL and keep the source head as a
  second parent, also when the commit is republished from the journals.

- **Optimistic write preconditions (#322).** Cypher writes accept the
  `If-Graph-Commit` header or `expected_commit` and commit only if the target
  branch (`main` or a branch head) is still at that commit; otherwise
  `409 GRAPH_COMMIT_PRECONDITION_FAILED`, with nothing written. Merges into
  `main` check the target head inside the WAL transaction, so a write that
  lands between the merge plan and its commit is refused, not overwritten.

- **Schema registry (#323).** `GET /v1/graph/schema` describes the records
  visible at a commit or branch head: labels and relationship types with
  stable ids (persisted in `history/schema_registry.log`, never reused),
  counts, property keys with their value types, and the label pairs each
  relationship type connects.

- **History retention (#324).** `corrobore server cleanup` and
  `POST /v1/admin/storage/cleanup` keep, on every branch, the newest commits,
  commits younger than `keep_seconds`, and recent merged work. The first commit
  past the window becomes a baseline that keeps its id and full view. The pass
  rewrites branch payloads into a new generation holding only referenced
  payloads, and compacts the WAL and journals behind the latest checkpoint;
  segment compaction was previously never called. Pruned commits answer
  `404`, and feed cursors on them `410 CHANGE_FEED_GAP`.

- **Parse-only Cypher validation (#336).** `POST /v1/cypher/validate` parses
  a query and returns its kind (`read`, `mutation`, `mixed`), clauses and
  diagnostics without executing anything, so editors can validate while the
  user types; unbound parameters and partially structured queries are
  warnings.

- **Incremental full-text maintenance (#325).** A canonical write now
  updates the published tantivy generation in place with the documents it
  changed. Before, it invalidated the index and the next search rebuilt every
  document. The index stays marked invalid during the transaction and is
  cleared only once the delta is published, so a crash, an index already
  being rebuilt, or a property field the index schema does not know yet still
  fall back to the full rebuild.

- **Vector index and hybrid retrieval (#326).** `POST /v1/graph/search/vector`
  ranks nodes by cosine or L2 similarity of an embedding property at a commit,
  through an exact index kept per commit and moved forward with later node
  changes, behind a `VectorIndex` trait. `POST /v1/graph/search/hybrid` fuses
  BM25 full text, vector similarity and graph proximity with Reciprocal Rank
  Fusion, reporting each signal's rank. The `vector` and `hybrid` modes of
  `/v1/seed/search` still need an embedding provider for their text
  objective and are unchanged.

- **Object storage and open columnar export (#327).** A new
  `storage-interchange` crate adopts the `object_store` crate (pinned 0.14.2)
  for `s3://`, `file://` and `memory://` stores behind the content-plane
  `ObjectStore` and snapshot `SnapshotArtifactStore` interfaces. `corrobore
  server import-snapshot` downloads and validates a snapshot from such a store
  for `restore`, which closes the S3 restore gap. It also adds Apache Parquet
  export of a commit (Arrow and Parquet pinned 60.1.0): one file per label and
  relationship type, with typed `prop_<key>` columns, through
  `corrobore server export-parquet` and `POST /v1/admin/storage/export/parquet`.

- **Deterministic simulation of the persistence layer (#328).**
  `crates/graph-storage/tests/deterministic_simulation.rs` drives seeded
  universes of main writes, crashes at every mutation stage, branch writes,
  merges, history cleanup, reopens and torn history tails, and checks each step
  against an in-memory model: visible records, history continuity, the `main`
  head view against the canonical catalog, and atomicity after a crash. A
  failing seed replays exactly.

- **Content plane served over HTTP (#341).** `POST /v1/content` places
  content by the ADR-0021 storage policy: text and JSON up to
  `CORROBORE_HTTP_CONTENT_INLINE_MAX_BYTES` (16 KiB) come back inline in the
  handle, while anything else is stored once, content-addressed, and gets a
  `content--<sha256>` id. `GET /v1/content/{content_id}` serves it with its
  media type, a digest `ETag` and single `Range` requests (`206`/`416`),
  verifying the digest on every read. The store is
  `CORROBORE_HTTP_CONTENT_STORE_URL` (`s3://` with the `CORROBORE_S3_*`
  variables, `file://`, `memory://`), else `<storage>/content`, else memory.
  `UrlObjectStore` now shuts its runtime down in the background when dropped,
  so async hosts can own one.

- **Embedding provider for seed search (#343).** `CORROBORE_EMBEDDING_PROVIDER`
  selects `hashing` (local, deterministic feature hashing of words and
  character trigrams, no model) or `http` (an OpenAI-compatible
  `/v1/embeddings` endpoint, such as a local Ollama, TEI or vLLM server).
  `POST /v1/embeddings` returns its vectors so clients store comparable node
  embeddings. With a provider and persistent storage, the `vector` and
  `semantic` seed modes rank nodes by cosine similarity at the head of `main`,
  and `hybrid` fuses that ranking with full text by reciprocal rank fusion.
  Seed results now say how they were resolved (`retrieval`, `commit_id`); the
  lexical resolver still answers, with a boundary note, when no provider is
  configured, in ephemeral mode, or when no node carries an embedding.

- **Revert one change and review approvals (#348).** `POST /v1/graph/revert`
  puts one node or relationship of a branch back to its version at the merge
  base with `main`, as one branch commit guarded by `expected_head`: changed
  records get their base version, added ones are tombstoned, deleted ones come
  back. A revert that would leave a relationship without an endpoint is
  refused (`409 GRAPH_REVERT_BLOCKED`). `POST /v1/graph/reviews` records an
  `approved` or `changes_requested` decision on the branch's exact current
  head, and `GET /v1/graph/reviews` lists them, marking those on the current
  head; reviews live in the append-only, checksummed `history/reviews.log`.

### Removed

- **Breaking:** the enterprise edition and its license gate leave Corrobore,
  which now ships under MIT only (#306). The `enterprise-cti`,
  `enterprise-cti-binary`, `enterprise-fimi`, and `enterprise-crisis` Cargo
  features, signed-license verification, and the `GET /v1/license/status` and
  `GET /v1/admin/license/status` routes are gone. Startup now fails when a
  `CORROBORE_HTTP_LICENSE_PEM[_FILE]`,
  `CORROBORE_HTTP_LICENSE_PUBLIC_KEY_PEM[_FILE]`, or
  `CORROBORE_HTTP_LICENSED_MODULES` variable is still set, instead of ignoring
  it. The `cti`, `fimi`, and `crisis` domains, their seed profiles,
  graph-native STIX validation, and `GET /v1/export/stix` return
  `403 FEATURE_NOT_AVAILABLE`, which was already the open-source build's
  response, and the `LICENSE_MODULE_MISSING` code is no longer emitted. The
  native domain-provider host and its ABI are unchanged: existing provider
  binaries keep loading, and the MIT `medical` and `research` packs are
  unaffected. The EE provider retrieval script, its workflow, and the
  report-to-STIX licensed acceptance gate are removed along with the edition
  contract CI job.

### Fixed

- History cleanup kept the commits each live branch can reach but could prune
  the commit where that branch split from `main`, so its next merge found no
  merge base. Cleanup now keeps the whole merge-base closure of every live
  branch (#328, found by the simulator).
- A three-way merge reported a conflict when both branches deleted the same
  record; that agreement now merges cleanly (#328, found by the simulator).
- Workspace compatibility after the `hmac` 0.13 and `fs4` 1.1 upgrades:
  direct SHA-2 consumers now share `digest` 0.11 with HMAC, hexadecimal digest
  rendering and key initialization use the current APIs, and file-worker
  locking uses the renamed `fs4` operation. CI now rejects future direct
  HMAC/SHA-2 digest-release drift (#175).

### Documentation

- Public documentation, the packaged agent skills, and the MCP bridge now cover
  every Epic 0029 workstream. New user guides describe verdicts, confidence
  dimensions and the actionability gate (WS-D), narratives, campaigns,
  coordination signals and misleadingness export fields (WS-G), and the agentic
  platform foundations (WS-H: capability catalogue, agent write policy and
  budgets, why-provenance, corrective routes and the publish gate, memory
  fusion authority cap, investigation artifacts). The Cypher, claim audit,
  HTTP, memory operations, exporter, domain and LLM guides gain the matching
  sections; the OpenAPI contract documents the audit's `falsifiers`,
  `corrective_routes` and `evidence_risk_assessments` fields and the
  consolidate `authority_policy` and `revoked_source_ids` fields. The Corrobore
  skill gains verdict and campaign-provenance references, and the
  `corrobore_consolidate` MCP schema exposes the fusion fields, held by the
  capability-adapter contract.

### Added

- Opt-in SQL frontend for the standalone server (epic #90, item #259): the
  `sql-frontend` crate compiles a documented PostgreSQL-flavoured subset
  (`SELECT` with relationship joins, aggregates, ordering, pagination and
  parameters; `INSERT`, `UPDATE`, `DELETE`; transactions; `information_schema`)
  into the shared structural query AST, and a PostgreSQL wire-protocol v3
  listener serves it with cleartext password authentication against the
  Corrobore bearer token, `SSLRequest` over the shared `[tls]` material, the
  simple and extended query protocols, typed row descriptions and standard
  SQLSTATE errors. The engine gains `execute_prepared_request` and a
  `prepare_graph_for_ast` persistence hook so a frontend that hands over an
  AST takes the same policy, budget and durability path as Cypher text.
  Enabled with `interfaces.enabled = ["http", "sql"]` and a `[sql]` section
  (`port`, `max_connections`) or the matching environment variables and CLI
  flags. Off by default; Cypher, HTTP and embedded behaviour are unchanged.
- `corrobore-nlq` (epic #82, item #260): the versioned `nlq/v1` action
  envelope with a validator that refuses trusted runtime context at any depth,
  invented evidence, unbounded or mutating reads and writes the caller did not
  allow, and canonicalizes actions through the real Cypher, `INVESTIGATE` and
  `memory/v1` parsers; a compile-only `NlqCompiler` trait with a deterministic
  bilingual `TemplateCompiler` baseline (French and English); a reproducible
  paired dataset generator with adversarial families and leakage-safe splits;
  and a parser-backed evaluation harness reporting per-language metrics. The
  contract is committed under `compatibility/nlq/v1` with replayable fixtures.
  No model ships and the runtime gains no machine-learning dependency.
- Opt-in Bolt listener for the standalone server (epic #12, item #258):
  Bolt 4.4 and 5.0 to 5.8 negotiation, PackStream v1, `HELLO`/`LOGON`
  authentication against the Corrobore bearer token, `RUN`/`PULL`/`DISCARD`
  with bounded server-side buffering and `has_more` streaming, statement-group
  transactions over the per-request atomic mutation model, `ROUTE` for
  `neo4j://` clients, stable failure codes, and TLS through the shared `[tls]`
  material. Enabled with `interfaces.enabled = ["http", "bolt"]` and a `[bolt]`
  section (`port`, `max_connections`) or the matching environment variables and
  CLI flags. Off by default; HTTP and embedded behaviour are unchanged.
- Typed record values: `ExecutionRecord` and `CypherRecord` carry a `values`
  twin of the string `fields`, and `ExecutionResult` and `CypherResponse`
  carry the ordered `columns`, so protocol adapters encode integers, floats,
  booleans, lists, nodes and relationships without guessing from text. Neither
  is serialized: the HTTP JSON contract is byte-identical.

- WS-H agentic platform foundations: corrective routes and falsifier records
  with a high-impact publish gate composed onto the WS-D actionability
  decision; memory fusion back-pointers with an authority cap where repetition
  cannot raise authority and revocation recomputes without deleting; planned
  why-provenance for query results, kept separate from semantic support, with a
  PROV-O reading in the exporters; agent write authorization decided outside
  the prompt with four-dimension run budgets and a hash-chained mutation log;
  one capability catalogue projected by every protocol adapter with no protocol
  shape in the definition; and investigation artifacts bound to governed
  records by identity, regenerable while preserving analyst annotations, with
  publication independent of truth. No agent runtime object enters the evidence
  graph (#222, #223, #224, #225, #226, #227).

- WS-G FIMI misleadingness and campaign provenance: domain-neutral `Narrative`
  and `Campaign` collections, collection-scoped coordination signals stored as
  evidence with explainable independence-cluster effects, an attribution gate
  that refuses coordination signals as their only support, and additive FIMI
  export fields carrying campaign lineage and pack assessments. A misleadingness
  band and a factual verdict travel in separate fields and neither can move the
  other; a curation signal never collapses independence; exports stay
  byte-identical for graphs without these records. Vocabulary and validators
  live in `corrobore-domain-fimi`, and the measured mechanism, grounding and
  factual-drift metrics live in `corrobore-benchmarks` (#217, #218, #219, #220,
  #221).

- WS-E evidence-risk diagnostics cover seven attributed signals, preserve
  quarantine and dependency provenance on evidence, and reduce component
  contribution and source-independence weights without changing deterministic
  verifier precedence (#212).

- WS-E evidence access metrics separate graph presence, recorded verifier-path
  reachability and unexplained retrieved evidence, with reproducible fixtures
  and policy/budget-aware retrieval proposals (#211).

- WS-E v1 stage instrumentation with explicit input/output units, failed-input
  counts, producer attribution, idempotent measurements and bounded per-engine
  reports. The authenticated stage API emits all seven stages, preserving null
  counters for unmeasured work; benchmark reports consume the schema additively
  (#209).

- WS-F claim audit: deterministic retained provenance, explicit verification
  coverage, stored dimensions/clusters and history; separate attributed,
  reversible human decisions; scoped offline audit archives in STIX/FIMI and
  native memory restoration. Agent skills require an audit before asserting a
  verdict, with a read-only `corrobore_claim_audit` MCP tool in the source plugin.
  The acceptance matrix links HTTP, export and UI evidence
  (#194, #204–#208).

- WS-C candidate-first ingestion: immutable raw versions, targeted constraint
  feedback and repair lineage, evidence-cited mention reconciliation with
  reversible merges, and independently evaluated repair/reconciliation metrics.
  Agent and plugin recipes now use submit, inspect feedback, re-extract the
  failing field and resubmit before explicit reviewed promotion. The WS-C
  acceptance gate covers Spikes B/D and links HTTP undo, metrics and unchanged
  WS-A/WS-B/WS-D compatibility evidence (#192, #197–#203).

- WS-D verdict uncertainty explanations with retained cluster membership,
  directional weights, dimensions, authority provenance and ranked alternatives.
  Projection and FIMI/STIX claim lineage expose the payload additively, including
  relationship claims. The WS-D acceptance suite covers epic #178 and preserves
  WS-A/WS-B compatibility; ungoverned export bytes remain unchanged (#185).

- `graph-core`: optional structured `ClaimProposition` beside the claim text
  statement (subject, predicate, entity or literal object, polarity, modality,
  valid-time scope, extraction version), with reference validation against the
  claim target context and an additive `proposition_*` property projection.
  Claims serialized before this change deserialize unchanged (Epic 0029 WS-A,
  #154).
- `graph-core`: immutable `Source` record and `SourceStore` (URI, type,
  publisher, authority domain, acquisition time, artifact SHA-256, signature,
  parent source). Sources have no update path: a changed artifact hash creates
  a superseding version and records a `source.content_drift` finding. Legacy
  evidence records lift into sources idempotently through
  `EvidenceRecordStore::lift_sources`, and `EvidenceRecord` gains an optional
  `source_id`. `ValidationTarget::Source` is added (Epic 0029 WS-A, #148).
- `graph-core`: immutable `Observation` record and `ObservationStore` bound to
  a registered `Source` (selector, verbatim payload, modality, observation
  time, payload SHA-256). Observations have no update path; a correction is a
  superseding observation. `EvidenceLocator` gains `CharacterSpan` and
  `RecordPath`, `EvidenceAttachmentTarget` gains `Observation`, and
  `EvidenceRecord` gains an optional `observation_id` filled by the idempotent
  `EvidenceRecordStore::lift_observations`. The legacy source lift no longer
  copies a record's `observed_at` into the source acquisition time (Epic 0029
  WS-A, #149).
- `graph-core`: `ClaimLink` becomes the evidence link of ADR-0016. `ClaimLinkKind`
  gains `ContextFor`, `Duplicates`, `DerivedFrom`, `DependsOn`;
  `ClaimLinkSource` gains `Observation`; links carry optional `strength`,
  `authority`, `independence_cluster`, and `BitemporalStamp` fields;
  `ClaimStore::attach_link` and `register_observation` are added;
  `EpistemicRelationKind` gains the four aligned kinds with canonical
  relationship types; `EpistemicExplanationKind` gains `ContextLink`,
  `DuplicateLink`, `DerivationLink`, `DependencyLink`. Links serialized before
  this change deserialize unchanged (Epic 0029 WS-A, #150).
- `graph-core`: append-only `VerificationRecord`, `Verdict`, and
  `StateTransition` records with their stores; `VerdictState` (`Supported`,
  `Refuted`, `Mixed`, `Contested`, `Unknown`, `InsufficientEvidence`,
  `Superseded`) and the ADR-0016 projection `project_verdict_state` onto the
  lifecycle `ClaimStatus`; `resolve_claim_verdict` computes a verdict from
  active evidence links and deterministic verification records with the
  minimal WS-A policy, appends verdict and transition on change, and applies
  the projected lifecycle status when the transition matrix allows it;
  deterministic as-of replay through `VerdictAsOf`; `ClaimStore` gains
  `links_active_at`, `close_link_validity`, and `apply_verdict_projection`
  (Epic 0029 WS-A, #151).
- `graph-core`: ADR-0016 reachability gate. `Supported`, `Refuted`, and `Mixed`
  verdicts require an active signal whose source resolves to an `Observation`
  bound to a `Source`; otherwise `resolve_claim_verdict` yields
  `InsufficientEvidence` and records a `ReachabilityGap` (finding
  `claim.verdict.unreachable_evidence`). `resolve_claim_verdict` now takes
  `ResolutionInputs` (verification, evidence, observation, and source stores).
  Governed records reject in-place changes with the typed
  `GraphError::ImmutableRecordConflict`. `validate_claim_reachability` flags
  `Supported` or `Validated` lifecycle claims without an observation path
  (`claim.lifecycle.without_observation_path`) without mutating them.
  `domain-common` gains `DomainValidationIssue::from_validation_record` and
  `DomainValidationResult::from_validation_records` (Epic 0029 WS-A, #152).
- `graph-core`: `EpistemicStores` bundles sources, observations, claims,
  verification records, and verdicts on `Graph`, in `GraphPersistenceSnapshot`
  (skipped when empty), and in a deterministic serialization of `ClaimStore`.
  `Graph::epistemic_projection()` renders every governed record as a read-only
  graph in the epistemic vocabulary for Cypher reads. `graph-storage` persists
  the stores in the `epistemic-records-v1.json` sidecar. STIX exports add
  `x_corrobore_lineage` and FIMI exports add `lineage` additively. The
  `compatibility/epistemic/v1` fixtures guard pre-WS-A payloads, and
  `epic_0029_ws_a_acceptance` closes workstream WS-A (Epic 0029 WS-A, #153).
- `graph-core`: verifier framework, per ADR-0017. The `Verifier` trait reports
  a `VerificationOutcome` (result, rationale, limits, evidence consumed) over a
  read-only `VerificationRequest` built from a claim, its active evidence
  links, the observations they resolve to, and the sources behind them.
  `VerifierRegistry` owns provenance: it mints the `VerificationRecord`
  identifier, stamp, and `deterministic` flag from the registration, keyed by
  identifier and version so versions coexist and earlier records stay
  reproducible. `VerificationContext` bundles the read-only stores
  (Epic 0029 WS-B, #163).
- `graph-core`: deterministic `verifier.identifier-syntax` and
  `verifier.content-hash` implementations. Identifier syntax covers public
  digest, UUID, RFC3339, domain, IP, URL, STIX, and CVE formats while recording
  that no external registry or semantic claim was checked. Content hashes
  recompute SHA-256 over exact observation and evidence payload bytes and
  report recorded and computed digests on drift. Both verifiers are versioned,
  domain-neutral, and emit their limits on every result (Epic 0029 WS-B, #164).
- `graph-core`: deterministic `verifier.temporal-ordering`,
  `verifier.arithmetic-consistency`, `verifier.graph-consistency`, and
  `verifier.schema-constraint` implementations. Propositions gain additive,
  typed arithmetic declarations for bounds, units, and aggregate parts;
  verification contexts can expose an immutable graph and an optional schema
  provider. `domain-common` supplies the schema registry and keeps required
  properties and type assertions owned by installed packs. Missing schemas
  remain explicitly inconclusive (Epic 0029 WS-B, #165).
- `graph-core`: deterministic-first verdict precedence. Resolution selects the
  newest conclusive record per verifier, gives deterministic results authority,
  keeps non-deterministic results advisory regardless of claim confidence,
  and records opposing deterministic/advisory results as append-only typed
  findings. Verification observation inputs now satisfy the existing verdict
  reachability gate; a deterministic failure can demote a previously
  `Validated` claim to `Contradicted` or `Disputed` instead of leaving a trusted
  lifecycle status behind (Epic 0029 WS-B, #166).
- `domain-provider-abi` and `corrobore-http-server`: additive ABI v1.2
  `claim.verify/1` capability with governed request/result payloads and an
  optional provider determinism declaration that defaults to advisory. The
  host registers capability adapters in `VerifierRegistry`, keeps provenance
  and precedence in the core, accepts existing ABI v1.1 providers unchanged,
  and rejects dispatch of unknown capabilities (Epic 0029 WS-B, #167).
- `graph-core`, Cypher projection, and STIX/FIMI exporters: derived
  `VerificationCoverage` selects the current record per verifier without
  rewriting history, distinguishes mechanical, semantic, unchecked, and
  failing claim coverage, and carries verifier id/version through projections
  and additive lineage. The `epic_0029_ws_b_acceptance` suite closes every
  deterministic-first workstream gate (Epic 0029 WS-B, #168).

## Releases

- **[v0.3.3]** — see [docs/release-notes/v0.3.3.md](docs/release-notes/v0.3.3.md).
- **[v0.3.2]** — see [docs/release-notes/v0.3.2.md](docs/release-notes/v0.3.2.md).
- **[v0.3.1]** — see [docs/release-notes/v0.3.1.md](docs/release-notes/v0.3.1.md).
- **[v0.3.0]** — see [docs/release-notes/v0.3.0.md](docs/release-notes/v0.3.0.md).
- **v0.2.2** — see [docs/release-notes/v0.2.2.md](docs/release-notes/v0.2.2.md).
- **v0.2.1** — see [docs/release-notes/v0.2.1.md](docs/release-notes/v0.2.1.md).
- **v0.2.0** — see [docs/release-notes/v0.2.0.md](docs/release-notes/v0.2.0.md).
- **v0.1.0** — see [docs/release-notes/v0.1.0.md](docs/release-notes/v0.1.0.md).
- **v0.0.1** — see [docs/release-notes/v0.0.1.md](docs/release-notes/v0.0.1.md) (historical foundation milestone).

[Unreleased]: https://github.com/Estance-Labs/corrobore/compare/v0.3.3...HEAD
[v0.3.3]: https://github.com/Estance-Labs/corrobore/compare/v0.3.2...v0.3.3
[v0.3.2]: https://github.com/Estance-Labs/corrobore/compare/v0.3.1...v0.3.2
[v0.3.1]: https://github.com/Estance-Labs/corrobore/compare/v0.3.0...v0.3.1
[v0.3.0]: https://github.com/Estance-Labs/corrobore/compare/v0.2.2...v0.3.0

# Changelog

All notable changes to Corrobore are documented here. This root file is a pointer to
the detailed, per-release notes maintained under
[`docs/release-notes/`](docs/release-notes/).

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

Changes on `main` after `v0.3.3` that have not yet been tagged in a release.

### Fixed

- Workspace compatibility after the `hmac` 0.13 and `fs4` 1.1 upgrades:
  direct SHA-2 consumers now share `digest` 0.11 with HMAC, hexadecimal digest
  rendering and key initialization use the current APIs, and file-worker
  locking uses the renamed `fs4` operation. CI now rejects future direct
  HMAC/SHA-2 digest-release drift (#175).

### Added

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

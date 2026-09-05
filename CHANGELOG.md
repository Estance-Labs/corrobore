# Changelog

All notable changes to Corrobore are documented here. This root file is a pointer to
the detailed, per-release notes maintained under
[`docs/release-notes/`](docs/release-notes/).

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

Changes on `main` after `v0.3.3` that have not yet been tagged in a release.

### Added

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

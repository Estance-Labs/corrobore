# Release Notes Process

This page defines the lightweight release-notes policy for the public docs.

## Scope

- Keep one release note page per released version under `docs/release-notes/`.
- Keep `CHANGELOG.md` as a pointer index to those versioned pages.
- Keep docs focused on behavior available on `main`; use release notes for
  version-specific deltas.

## Required updates per release

When tagging a release version `vX.Y.Z`:

1. Add `docs/release-notes/vX.Y.Z.md`.
2. Add the new version entry under **Release Notes** in `mkdocs.yml`.
3. Add the new version entry in `CHANGELOG.md` under **Releases**.
4. Move the `[Unreleased]` compare base in `CHANGELOG.md` to `vX.Y.Z`.
5. Update the workspace, public version examples, and OpenAPI `info.version`.

## Recommended structure for each release note

- Title: `# vX.Y.Z - <short qualifier>`
- One paragraph describing the release intent.
- `## Highlights` with user-visible behavior changes.
- `## Contracts` for API/config/security or operational guarantees.
- `## Upgrade notes` for migration or rollout caveats.
- `## Known boundaries` for capabilities deliberately outside the release.
- `## Validation and provenance` for release gates and publication evidence.

## Validation

Before merging release-note updates:

```bash
node --test scripts/docs-contract-guard.test.mjs
node --test scripts/release-notes-contract.test.mjs
node scripts/docs-contract-guard.mjs
node scripts/release-notes-contract.mjs X.Y.Z PREVIOUS.X.Y
python -m mkdocs build --strict
```

If `mkdocs` is installed in shell path, `mkdocs build --strict` is equivalent.

## Stage-level release qualification

Structural documentation checks above remain read-only checks of version metadata
and historical pages. They do not qualify a new release. Before publishing new or
modified versioned notes, run the same pinned-provider command used by release
workflows:

```bash
node scripts/release-quality.mjs \
  --provider /path/to/pinned/corrobore-benchmarks \
  --bundle /path/to/qualification/release-notes-publication.json \
  --candidate "$CANDIDATE_REVISION" \
  --baseline "$APPROVED_BASELINE_REVISION" \
  --release-ref "docs:$CANDIDATE_REVISION" \
  --release-path release-notes-publication \
  --output-dir /path/to/quality-decision
```

The provider checkout must match `scripts/release-quality-provider.json`, including
its repository, commit and runtime-file hashes. The wrapper executes a snapshot
of verified bytes. Candidate, approved quality baseline and release reference
are supplied independently of the evidence bundle; there is no aggregate-F1 or
missing-evidence fallback. Exit code zero is required before publication.

The documentation workflow compares versioned notes with the most recently
verified Pages publication, identified by its successful publication step.
Comparing only with the preceding push would let an unpublished note escape the
gate after a later ordinary documentation edit. Missing API access, incomplete
history or an unavailable published commit block this scope check. Ordinary
changes with no unpublished versioned-note differences do not require a new
release-quality measurement. Historical read-only checks continue to work.

When qualification is needed, the shared action requires repository variables
`WS_E_BASELINE_REVISION` and `WS_E_EVIDENCE_REVISION`, and a read-only
`QUALITY_EVIDENCE_TOKEN` for the benchmark provider/evidence repository. The
provider revision comes from the committed pin; the evidence revision is checked
out separately and must contain `qualification/release-notes-publication.json`.
The published documentation baseline used for change detection is distinct from
the approved measured baseline used to assess engine quality.

These hooks do not enable or dispatch GitHub Actions. A local test result or
workflow definition is not evidence that a hosted release was published.

The adoption check is retained in
[`core-hooks-validation-v1.json`](https://github.com/Estance-Labs/corrobore/blob/main/artifacts/release-quality/core-hooks-validation-v1.json).
It executes the pinned provider through the core command: four complete fixture
bundles pass, a verifier regression fails in each publication family, and each of
the seven isolated stage regressions fails for release-note publication despite
an improved aggregate score. These are test doubles, not production qualification
or proof of a hosted publication. The provider's
[`release-quality-gate.test.mjs`](https://github.com/Estance-Labs/corrobore-benchmarks/blob/ec91162b21883843bee5772cddc2f4aa89582b73/scripts/release-quality-gate.test.mjs)
reproduces the complete and seeded bundles and verifies that failed decisions never
reach the publication callback.

CI retains `decision.json`, `report.md` and `report.html` after a failed gate and
shows the Markdown report in the job summary. The original private evidence bundle
is not uploaded by this action. A missing report remains a blocking failure, not
a passing measurement.

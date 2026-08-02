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

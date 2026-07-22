# AGENTS

This file defines execution rules for coding agents in this repository.

Canonical reference: `.github/copilot-instructions.md`.
If wording diverges between files, `.github/copilot-instructions.md` is authoritative.

## Mandatory workflow

```text
one issue = one branch = one PR
```

Work must start from the GitHub issue for the feature.
Read the issue before coding, keep scope aligned with it, and use it as the acceptance reference.

Issue completion gate:

- An issue is not complete when its PR is only opened.
- The PR for the issue must be validated, merged into `main`, and `main` must be synced locally before starting the next issue.

```text
Phase 1: write tests first
Phase 2: write function stubs and implementation comments
Phase 3: implement the feature
Phase 4: validate all tests and fix regressions until green, then write/create the PR with the issue auto-close reference, validate and merge that PR to main, sync local main, and only then start the next issue from the epic
```

## Non-negotiable rules

- Build for durability and reliability, not short-term convenience.
- Do not apply temporary hotfixes, bypasses, or patch-only shortcuts.
- After each phase, run relevant tests.
- Any failing test is treated as a regression introduced by current work and must be fixed before continuing.
- Open a PR only after full Phase 4 validation passes.
- In Phase 4, include a closing reference to the issue in the PR body (for example: `Closes #123`).
- Do not start work on the next issue while the current issue PR is still open or unmerged.

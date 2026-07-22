# Copilot Instructions

These instructions are the canonical agent workflow rules for this repository.

## Workflow identity

```text
one issue = one branch = one PR
```

Feature development must always start from the GitHub issue.
Before writing code or tests, read the issue, scope the work to that issue, and keep implementation aligned with its acceptance intent.

Issue completion gate:

- An issue is not complete when its PR is only opened.
- The PR for the issue must be validated, merged into `main`, and `main` must be synced locally before the next issue starts.

## Required implementation phases

Every feature issue must be executed with the same 4-phase order:

```text
Phase 1: write tests first
Phase 2: write function stubs and comments describing intent, expected behavior, validation targets, and implementation direction
Phase 3: implement the feature
Phase 4: run full validation and fix regressions until tests are green, then write/create the PR with the issue auto-close reference, validate and merge that PR to main, sync local main, and only then start the next issue from the epic
```

## Quality and reliability policy

- Never choose the easiest short-term path when it weakens long-term reliability.
- Never use temporary hotfixes or patch-style bypasses during feature development.
- Code changes are expected and allowed; tests are the regression contract.
- Each phase must validate tests before progressing.
- If tests fail during a phase, treat failures as side effects of current changes and fix them before moving forward.
- A PR is allowed only after Phase 4 with a green test suite.
- During Phase 4, include the closing keyword and issue reference in the PR description (for example: `Closes #123`).
- Do not begin the next issue while the current issue PR is still open or unmerged.

## Engineering intent for stubs and tests

- Stub comments must explain what is being validated and how implementation will satisfy it.
- Tests should state intent and expected behavior clearly before implementation details.

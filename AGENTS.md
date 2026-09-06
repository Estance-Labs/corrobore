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
- Before starting the next issue, verify on GitHub that no PR for the previous issue branch remains open.
- Once the PR is merged, verify that its merge commit is present in the synchronized `main` and that the issue worktree is clean, unlocked, and no longer in use, then remove that worktree with a non-forced Git worktree removal and prune stale worktree metadata. For squash or rebase merges, do not require the original branch head to be an ancestor of `main`.
- If a PR was intentionally closed without merging, remove its clean worktree only after confirming that the issue branch is abandoned and no longer contains work that must be retained.
- Never force-remove a worktree or discard local changes. If the PR is still open, the merge is absent from `main`, or the worktree is dirty, keep it and report the blocker.

```text
Phase 1: write tests first
Phase 2: write function stubs and implementation comments
Phase 3: implement the feature
Phase 4: validate all tests and fix regressions until green, then write/create the PR with the issue auto-close reference, validate and merge that PR to main, sync local main, verify the previous issue has no open PR, safely remove its clean and integrated worktree, and only then start the next issue from the epic
```

## Autonomous delivery authorization

A user request to deliver an issue, epic, or a sequence of epics authorizes the
agent to create the scoped PRs and merge them after validation, without requesting
an additional human confirmation for each merge. Continue through the authorized
scope after each completed delivery; do not infer authorization for unrelated work.
An explicit user instruction to pause, keep a PR in draft, or require human review
still applies.

Before merging, review the diff against the issue acceptance criteria, fix review
findings, run the required validation, and verify that required GitHub checks pass
for the current PR head. The agent may perform the code review; human approval is
not an additional repository doctrine gate. Honor GitHub rules and any required
reviewers: never bypass branch protections or use an administrator override. If
an external requirement blocks the merge, report the blocker and stop progression
to the next issue.

After each merge, perform these steps in order:

1. Verify on GitHub that the PR is merged and record its merge commit. A queued
   merge or an enabled auto-merge is not a successful merge.
2. Fetch the remote and fast-forward the clean local `main` checkout to
   `origin/main`. If local changes, divergence, or active use prevent a safe update,
   preserve the checkout and report the blocker; never reset or discard work.
3. Verify that the merge commit is included in synchronized `main` and that no PR
   for the completed issue branch remains open. For squash or rebase merges,
   verify the resulting merge commit, not ancestry of the original branch head.
4. Check that the completed issue worktree is clean (including untracked files),
   unlocked, and no longer used by another agent, task, or process. Leave it from
   the current session, remove it with non-forced `git worktree remove`, then
   prune stale metadata. Preserve dirty, locked, or still-used worktrees and
   report why cleanup is deferred; retry when they become eligible.
5. Only after merge and main synchronization are verified, create the next issue
   branch and isolated worktree from the updated local `main`. Eligible completed
   worktrees must be cleaned before proceeding; a worktree still in active use
   must be preserved.

## Non-negotiable rules

- Build for durability and reliability, not short-term convenience.
- Do not apply temporary hotfixes, bypasses, or patch-only shortcuts.
- After each phase, run relevant tests.
- Any failing test is treated as a regression introduced by current work and must be fixed before continuing.
- Open a PR only after full Phase 4 validation passes.
- In Phase 4, include a closing reference to the issue in the PR body (for example: `Closes #123`).
- Do not start work on the next issue while the current issue PR is still open or unmerged.
- Do not retain completed issue worktrees after their cleanup gate passes; preserve those still in active use until they become eligible.

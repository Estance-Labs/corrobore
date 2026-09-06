# Contributing to Corrobore

Thanks for your interest in improving Corrobore. This guide describes how work is
organized and the workflow every change is expected to follow.

## ⚠️ Disclaimer — Contributing with AI assistance

If you intend to contribute using an AI coding assistant (Copilot, Cursor, Claude, etc.), **please read this first**.

AI-generated code must be reviewed with the same rigour as any human-authored change, or higher, because hallucinations and subtle logic errors are common and not always obvious at a glance.

Every AI-assisted PR requires a thorough code review, which an agent may perform
within user-authorized delivery. A separate human approval is required only when
explicitly requested by the user or enforced by GitHub rules. The following human
review estimates illustrate the expected care, not a mandatory human approval gate:

### Cost of a code review

**Time estimate.** A thorough review of a medium-sized PR—approximately 200–400 changed lines—typically requires around **1.5 to 3 hours** when the reviewer must understand the surrounding context, trace the affected logic, run relevant tests, and provide actionable feedback.

Current French freelance benchmarks place experienced software developers at approximately **€550–€650 per day**, with highly experienced or specialized engineers commonly reaching **€700–€800 per day**. Using a market-equivalent reviewer rate of approximately **€70–€80 per hour**, a thorough review therefore represents around **€105–€240 of senior engineering time per PR**. For highly specialized or consultancy-priced reviewers, the cost may reach **€300 per PR**.

**Token cost equivalence.** For reference, processing the same diff with a frontier model such as [Fable](https://fableai.com/) (≈ \$3–5 / 1M input tokens) for a 10 000-token review context costs roughly **\$0.03–0.05**. Human review costs roughly **3 000×–10 000× more** than running the model, which is why the quality bar for what you submit matters.

> **What this means for you:** before opening a PR, re-read every line of AI output, run all local validation gates, and make sure the change is coherent and intentional. Do not submit raw AI output and expect maintainers to clean it up.

## Workflow: one issue = one branch = one PR

All feature work starts from a GitHub issue and flows through a single branch and
a single pull request:

```text
one issue = one branch = one PR
```

- Read the issue first and keep your change scoped to it.
- Branch from an up-to-date `main` (for example `feat/<issue>-<slug>`).
- Open a pull request that references the issue with a closing keyword in the
  body (for example `Closes #123`).
- An issue is only complete once its PR is validated, merged into `main`, and
  `main` is synced locally. Do not start the next issue before that.

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

## Implementation phases

Each feature issue is executed in the same order:

1. **Tests first** — write tests that state the intent and expected behavior.
2. **Stubs** — add function stubs and comments describing intent, validation
   targets, and implementation direction.
3. **Implement** — make the feature satisfy the tests.
4. **Validate & deliver** — run the full validation suite, fix regressions until
   green, then open the PR with the closing issue reference, merge it autonomously
   once its required checks pass, sync local `main`, and clean the eligible worktree
   before starting the next issue.

## Quality policy

- Build for durability and reliability, not short-term convenience.
- Do not use temporary hotfixes or patch-style bypasses.
- Tests are the regression contract: a failing test is treated as a regression
  introduced by the current work and must be fixed before moving on.
- Only open a PR after Phase 4 passes with a green test suite.

## Local validation

Run these before pushing; they mirror the CI gates:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --locked
node --test scripts/docs-contract-guard.test.mjs
node scripts/docs-contract-guard.mjs
node --test scripts/open-source-readiness.test.mjs
gitleaks git --redact --no-banner .
cargo deny check advisories bans licenses sources
```

Requires the pinned toolchain declared in `rust-toolchain.toml`.

When runtime HTTP routes or configuration variables change, update both
`docs/api/openapi.yaml` and `docs/user-guide/http-server.md` in the same PR.
The documentation contract guard enforces route and environment-variable parity
with `corrobore-http-server` sources.

### Fast feedback profiles

Use these profiles during development to avoid running the full workspace suite
on every iteration:

```bash
# Fast: tests only for the crate you are actively changing
cargo test -p <crate-name> --tests --locked

# Intermediate: all workspace test targets (without benches/examples)
cargo test --workspace --tests --locked

# Complete: CI-equivalent local run before opening/updating a PR
cargo test --workspace --all-targets --locked
```

Optional (faster Rust test runner):

```bash
cargo install cargo-nextest --locked

# Fast/intermediate equivalents with nextest
cargo nextest run -p <crate-name> --tests
cargo nextest run --workspace --tests
```

Use `cargo test` as the reference command for final parity checks unless the CI
workflow explicitly switches to `cargo nextest`.

### OSS and enterprise edition contracts

`corrobore-http-server` supports two validation paths for enterprise-gated modules:

```bash
# OSS contract: compile without enterprise defaults and verify gated behavior.
cargo test -p corrobore-http-server --no-default-features --locked stix_validate_contract_graph_source
cargo test -p corrobore-http-server --no-default-features --locked seed_search_contract_rejects_

# Enterprise contract: default build with enterprise feature enabled.
cargo test -p corrobore-http-server --locked stix_validate_contract_graph_source
cargo test -p corrobore-http-server --locked seed_search_contract_rejects_
```

Runtime license claims are configured through `CORROBORE_HTTP_LICENSED_MODULES`
(comma-separated, for example `cti,crisis`).

### CI duration monitoring

The repository includes a scheduled workflow that tracks the `rust-ci.yml`
runtime budget using p95 over recent successful runs.

Run the same check locally:

```bash
export GITHUB_TOKEN="$(gh auth token)"
export GITHUB_REPOSITORY="Estance-Labs/corrobore"
export P95_MAX_MINUTES="30"
node scripts/ci-duration-guard.mjs --workflow rust-ci.yml --branch main --max-runs 30
```

If p95 exceeds the configured threshold, the command exits with a non-zero code
to signal a duration regression.

## Reporting security issues

Please do not open public issues for vulnerabilities. Follow the process in
[SECURITY.md](SECURITY.md) instead.

## License

By contributing, you agree that your contributions are licensed under the
project's [MIT License](LICENSE).

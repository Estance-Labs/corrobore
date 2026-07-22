# Contributing

## ⚠️ Disclaimer — Contributing with AI assistance

If you intend to contribute using an AI coding assistant (Copilot, Cursor, Claude, etc.), **please read this first**.

AI-generated code must be reviewed with the same rigour as any human-authored change, or higher, because hallucinations and subtle logic errors are common and not always obvious at a glance.

Every AI-assisted PR will be subject to a full senior-level code review. To calibrate what that means in practice:

### Cost of a code review

**Time estimate.** A thorough review of a medium-sized PR—approximately 200–400 changed lines—typically requires around **1.5 to 3 hours** when the reviewer must understand the surrounding context, trace the affected logic, run relevant tests, and provide actionable feedback.

Current French freelance benchmarks place experienced software developers at approximately **€550–€650 per day**, with highly experienced or specialized engineers commonly reaching **€700–€800 per day**. Using a market-equivalent reviewer rate of approximately **€70–€80 per hour**, a thorough review therefore represents around **€105–€240 of senior engineering time per PR**. For highly specialized or consultancy-priced reviewers, the cost may reach **€300 per PR**.

**Token cost equivalence.** For reference, processing the same diff with a frontier model such as [Fable](https://fableai.com/) (≈ \$3–5 / 1M input tokens) for a 10 000-token review context costs roughly **\$0.03–0.05**. Human review costs roughly **3 000×–10 000× more** than running the model, which is why the quality bar for what you submit matters.

> **What this means for you:** before opening a PR, re-read every line of AI output, run all local validation gates, and make sure the change is coherent and intentional. Do not submit raw AI output and expect maintainers to clean it up.

---

## Support the project

If you find Corrobore useful and want to contribute without writing code, you can **buy the maintainer a coffee** or support the project financially:

[![Sponsor on GitHub](https://img.shields.io/badge/Sponsor-%E2%9D%A4-ea4aaa?logo=github)](https://github.com/sponsors/AreDee-Bangs)

GitHub Sponsors is the crowdfunding platform built directly into GitHub. Recurring or one-time contributions help cover infrastructure, tooling, and maintainer time.

---

The canonical repository rules are in `CONTRIBUTING.md`.

## Required workflow

```text
one issue = one branch = one PR
```

Read the GitHub issue before editing and use it as the acceptance contract.

1. Write tests first.
2. Add function stubs and comments describing intent, behavior, and validation.
3. Implement the change.
4. Run full validation, fix every regression, create a PR containing `Closes #<issue>`, merge it, and sync local `main` before starting another issue.

An open PR is not completion. Do not use temporary bypasses or weaken tests to make a change pass.

## Local validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features --locked
node --test scripts/docs-contract-guard.test.mjs
node scripts/docs-contract-guard.mjs
```

Build public documentation with:

```bash
python -m pip install mkdocs-material
mkdocs build --strict
```

If `mkdocs` is not on your shell path, run `python -m mkdocs build --strict`.

Run focused crate tests during each phase, then the full workspace gates before opening the PR. The Rust end-to-end golden datasets live in crate integration tests; the old standalone Python golden-dataset command is no longer the validation entry point.

## Public API discipline

- Import stable types through each crate's `lib.rs` facade.
- Keep external transports and model calls outside `graph-core`.
- Update the OpenAPI contract and HTTP documentation whenever routes or payloads change.
- Keep docs contract parity green: runtime routes and env vars must stay aligned
	between `crates/corrobore-http-server/src/{app,config}.rs`,
	`docs/api/openapi.yaml`, and `docs/user-guide/http-server.md`.
- Update public docs when a configuration variable, supported clause, crate boundary, or operational default changes.

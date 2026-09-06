# Claim audit and human decisions

The claim audit answers four questions from retained records: why this verdict,
what contradicts it, what changed, and what has not been checked. Agents must read
it before asserting a verdict. The audit does not run a resolver or verifier and
does not manufacture missing provenance.

## Read the stored state

```bash
curl -H "Authorization: Bearer $CORROBORE_HTTP_AUTH_TOKEN" \
  "http://127.0.0.1:8080/v1/claims/claim-example/audit"
```

The response is a direct JSON object, not an `ok`/`result` envelope. Unknown claims
return 404; missing or invalid authentication returns 401. A known claim may have
`current_verdict: null` and `explanation: null`: report it as unresolved, rather
than deriving a conclusion from its links. See the
[HTTP contract](http-server.md#get-v1claimsidaudit) and
[OpenAPI specification](../api/openapi.yaml).

| Question | Stored response fields |
| --- | --- |
| Why? | `current_verdict`, `explanation.dimensions`, clusters, `link_membership`, `evidence_links`, observations and source versions |
| Contradictions? | `contradictions`, refuting observations, `verification_disagreements`, failing verification records |
| Changes? | `verdict_history`, `state_transitions`, claim decisions, candidate repair lineage, reconciliations, promotions, merge undos, human decisions |
| Unchecked? | `coverage` for each claim and `unverified_steps`; absent provenance remains explicit |

`mechanically_checked` describes a deterministic check, `semantically_judged`
an advisory assessment, `unchecked` the absence of records, and `failing` a failed
check. Read `deterministic`, `result`, verifier identity/version, inputs and limits
with each entry. Inconclusive is not passing; a passing limited check does not
prove every aspect of a claim. Related claims retain their own coverage. Strong
support, source authority or repeated evidence does not substitute for a check.

The stored confidence dimensions and cluster membership explain evidence weight.
They remain separate from verification coverage. Audit reads select exact
provenance associations, never all records sharing an extraction run. Producers
can retain explicit candidate/reconciliation associations through
`Graph::link_claim_audit_record`; absent bindings are reported as gaps.

## Record a human judgment

`POST /v1/claims/{id}/decisions` accepts an ID, caller-attributed actor, timestamp
and an annotation, override or reversal action. These append-only records appear
under `analyst_decisions`. They never replace machine evidence, verification
records or the stored verdict. Reversal targets an earlier human decision and
retains both records. Exact retries are idempotent; reuse the same ID and payload
when the write outcome is uncertain. See the
[decision API](http-server.md#post-v1claimsiddecisions) for request examples.

The packaged [agent audit playbook](../agent-skill.md#claim-audit-before-verdicts)
explains the required read-before-assertion behavior and reversible writes.

## Analyst view and offline review

In `corrobore-ui`, choose **Claim audit** in the rail or open `/?claim=<claim-id>`.
The four questions, dimensions, evidence groups, verifier attribution and human
ledger appear in one view. Strong support without mechanical coverage is visually
separate from passing deterministic verification. Override controls append through
the decision API and refresh the ledger.

For another instance or an offline review, preserve the STIX/FIMI
`x_corrobore_audit_archive` extension and restore it with
`Graph::from_exported_audit_bundle`. Scoped archives preserve the selected claims
and provenance closure; native `export_memory_json` / `from_memory_json` retain the
full memory snapshot. The importer checks consistency, not authenticity or source
truth. See [offline export and restore](exporters.md#offline-claim-audits-and-re-import-ws-f).

## WS-F acceptance evidence

Epic [#194](https://github.com/Estance-Labs/corrobore/issues/194) is checked by the
following executable evidence. Paths are repository-relative; no browser fixture
is presented as proof of a deployed service.

| Epic criterion | Evidence |
| --- | --- |
| Four questions from one API read, no pipeline rerun | `crates/corrobore-http-server/tests/epic_0029_ws_f_acceptance.rs`: seeded observations, contradictions, history, repair/reconciliation lineage and gaps; compares the entire persistence snapshot before/after the GET. |
| Overrides retained and reversible | The same HTTP acceptance test appends annotation, override and reversal, checks idempotent receipts, retains all three records, and repeats the audit after restart. `analyst_decisions` tests cover rejection paths and immutable records. |
| Export/re-import reproduces audit | The HTTP acceptance restores scoped and native memory exports; `crates/graph-core/tests/claim_audit_archive.rs` checks unrelated-record exclusion and tampering; `crates/export-stix/tests/epistemic_lineage.rs` and `crates/export-fimi/src/lib.rs::tests::fimi_archive_restores_sources_verdict_dimensions_and_human_decisions` exercise actual domain export bundles. |
| Mechanical, semantic, unchecked and failing coverage per claim | The HTTP acceptance asserts all four canonical serialized classes with verifier attribution; its OpenAPI assertion compares enum tokens to Rust serialization. |
| Cluster membership and WS-D dimensions visible | The HTTP acceptance checks retained explanation and stable link-to-cluster membership against the seeded machine result; `crates/graph-core/tests/claim_audit_path.rs` checks repeated reads and exact stored dimensions. |
| No override edits observations, verifications or verdict | The HTTP acceptance compares every machine audit field after all three human writes; `crates/graph-core/tests/analyst_decisions.rs` checks full machine-store equality. |
| UI and agent precondition | UI [PR #2](https://github.com/Estance-Labs/corrobore-ui/pull/2) adds seeded desktop/mobile and override/reversal E2E; [PR #4](https://github.com/Estance-Labs/corrobore-ui/pull/4) corrects successful semantic coverage. `scripts/ws-f-guidance.test.mjs` checks both packaged skills reach the audit playbook. |

Run the local release gate:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
node --test scripts/*.test.mjs
node scripts/docs-contract-guard.mjs
mkdocs build --strict
```

The separate UI gate is lint, typecheck, unit tests, production build and all
Playwright scenarios, as documented in that repository. GitHub Actions availability
is distinct from these executable acceptance results and from publication.

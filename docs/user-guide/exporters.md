# Export and STIX Validation

Corrobore builds deterministic interchange documents from an explicit export plan. Identical graph state, metadata, selection, and ordering produce stable output.

This guide describes the current runtime contract. Check `GET /version` for the
deployed release before relying on version-specific behavior.

## STIX 2.1

`export-stix` maps the supported CTI subset into a STIX bundle. Use either:

- `CorroboreEngine::export_stix_bundle` in process;
- `GET /v1/export/stix` over HTTP.

The plan records logical `snapshot_id`, `transaction_id`, exporter version, profile, and strict/permissive mode. It projects the current graph; these metadata do not perform historical rollback.

The `stix-mvp` profile is CTI-scoped. It selects supported imported OpenCTI
families and documented graph-native CTI labels; domain-neutral memories,
receipts, and unrelated graph records are not retyped as STIX identities.
Imported objects and relationships keep their original STIX `id`, `type`,
standard fields, supported custom fields, and relationship semantics. Native
confidence is projected to the STIX 0-100 scale, while
`x_corrobore_evidence_refs` points to retained records in the bundle-level
`x_corrobore_evidence` array.

Generated identifiers are limited to graph-native records with a documented
supported CTI label and no valid `stix_id` or `external_id`. They use the
exported STIX type plus a deterministic hash of the native record identifier.
Unknown labels and unsupported OpenCTI families never receive a generated
identity.

Strict mode rejects the export with named findings when an eligible CTI record
is not export-ready, lacks retained evidence, has a governed claim with blocked or missing actionability, has malformed
canonical identity, fails provider validation, or references an excluded
endpoint. Permissive mode omits those records and returns at most 256
machine-readable entries in `export_diagnostics.exclusions`; it never creates a
fallback STIX identity for an unsupported record.

Exceptional operator-driven exports can set `StixExportOptions::force` in the
embedded API or pass `force=true` to the HTTP route. Force keeps semantic
missing-evidence and provider findings as bounded deterministic
diagnostics while including the otherwise eligible record. It does not bypass
actionability, export status, unsupported profile selection, malformed canonical identities,
dangling evidence references, excluded relationship endpoints, licensing, or
provider readiness. The default is `false`.

Scalar confidence is display metadata; raising it cannot grant export permission.
For governed claims, refusals name the actionability blockers. Legacy scalar
findings from older CTI providers remain export diagnostics; the separate public
validation endpoint keeps its existing behavior.

### Agent export choreography

Strict is the default correctness gate. Complete authorized writes first, read
back both nodes and relationships, audit native evidence and confidence, and
then promote eligible records. Late writes remain candidate and require a new
readiness and promotion pass.

Permissive is only for an explicit caller request for a diagnostic partial
bundle. `force=true` is an explicit operator decision and never an automatic LLM
fallback.

`GET /v1/export/stix` is read-only and never promotes graph records. A client
may repair lifecycle bookkeeping and retry once through separately authorized
write operations, but export itself does not promote or mutate state.

The HTTP route is fail-closed: it requires enterprise CTI support, a valid
`cti` license claim, and a ready provider exposing `node.validate/v1`. Missing
license, provider readiness, and provider capability have distinct error codes.

## FIMI

`export-fimi::export_fimi_json_document` produces a deterministic FIMI document from a `Graph` and `DeterministicExportPlan`. The exporter is a Rust library surface and has no dedicated HTTP route.

## Native STIX validation

`POST /v1/stix/validate` supports two sources:

| Source | Behavior |
| :--- | :--- |
| `bundle` | Validate explicit STIX objects, apply supported playbooks, and import the corrected objects when at least one playbook was applied. |
| `graph` | Run CTI readiness rules over current graph nodes; corrections remain an explicit agent operation. |

Built-in bundle playbooks can supply required `identity.name`, `malware.is_family`, and selected missing temporal fields. Temporal substitutions use processing UTC and add machine-readable `x_corrobore_corrections` entries. The response reports issues, applied playbooks, a correction summary, optional import persistence, and operational errors.

### Built-in playbooks

| Playbook | Trigger | Correction |
| :--- | :--- | :--- |
| `PLAYBOOK_FIX_IDENTITY_NAME` | `identity.name` missing | Set `name` to `Unknown Identity`. |
| `PLAYBOOK_FIX_MALWARE_IS_FAMILY` | `malware.is_family` missing | Set `is_family` to `false`. |
| `PLAYBOOK_FIX_INDICATOR_VALID_FROM_PROCESSING_UTC` | `indicator.valid_from` missing | Use processing UTC and append a description note. |
| `PLAYBOOK_FIX_REPORT_PUBLISHED_PROCESSING_UTC` | `report.published` missing | Use processing UTC and append a description note. |
| `PLAYBOOK_FIX_OBSERVED_DATA_FIRST_OBSERVED_PROCESSING_UTC` | `observed-data.first_observed` missing | Use processing UTC and append a description note. |
| `PLAYBOOK_FIX_OBSERVED_DATA_LAST_OBSERVED_PROCESSING_UTC` | `observed-data.last_observed` missing | Use processing UTC and append a description note. |

Temporal corrections also attach a structured record:

```json
"x_corrobore_corrections": [
  {
    "field": "valid_from",
    "strategy": "processing_utc_default",
    "value": "2026-07-12T12:34:56Z",
    "reason": "missing required temporal field 'valid_from'",
    "playbook_id": "PLAYBOOK_FIX_INDICATOR_VALID_FROM_PROCESSING_UTC"
  }
]
```

### Read the result precisely

| Field | Meaning |
| :--- | :--- |
| `valid` | No error-severity issue was observed during this validation pass. It is not a separate post-correction revalidation verdict. |
| `issues` | Errors and warnings found during the pass. |
| `playbooks_applied` | Corrections selected and applied in memory. |
| `corrections_summary` | Counts by field, strategy, and playbook; `null` when no structured correction exists. |
| `persistence` | Import statistics when at least one playbook ran; `null` otherwise. |
| `errors` | Operational errors reported in a successful validation envelope. |

A response can therefore contain `valid: false`, non-empty `playbooks_applied`, and non-null `persistence` at the same time. Consumers must inspect all three fields rather than treating `valid` as an after-fix verdict.

Common request errors are `MISSING_BUNDLE`, `INVALID_STIX_BUNDLE`, and `INVALID_SOURCE_MODE` (HTTP 400).

Validation is native Rust. No Python process or validator sidecar is required.

## Recommended gate

1. Import or write graph data.
2. Validate the explicit bundle or graph projection.
3. Review errors, warnings, corrections, and persistence.
4. Revalidate corrected data when a post-correction verdict is required.
5. Fix unresolved evidence or domain issues through an authorized mutation.
6. Export with stable metadata.

See [HTTP Server](http-server.md#post-v1stixvalidate) for payloads and status behavior.

## Canonical references

- [HTTP Server](http-server.md)
- [OpenAPI specification](../api/openapi.yaml)
- [For LLM Agents](../for-llms.md)

## Offline claim audits and re-import (WS-F)

STIX and FIMI documents include `x_corrobore_audit_archive` when an emitted record
has a governed claim. The extension uses schema `corrobore-claim-audit-v1` and
contains selected `claim_ids`, their complete `audits`, and a version-preserving
`snapshot` of the records needed to reconstruct those audits. The snapshot is the
source of truth; the included views make the trace readable without running
Corrobore and are checked against the restored records on import.

The archive preserves verbatim observations, source versions, evidence links and
stored dependency clusters, verification coverage and records, verdict dimensions,
state transitions, repair predecessors, reconciliation dependencies and reversals,
and the separate append-only human decision ledger. A human override and its
withdrawal both survive re-import, distinct from machine verdicts.

Selection starts from records actually emitted by the exporter. Existing profile,
lifecycle and actionability gates still apply. Related claim and reconciliation
records needed to explain a selected claim are retained as provenance; unrelated
claims and canonical records are excluded. Reconciliation dependencies are retained
without inventing direct claim associations. Original claim-link ledger indices
remain stable even when unrelated links are removed, including during later
resolution and appends. Candidate tier sequence numbers are local to the selected
registry; immutable candidate, repair and promotion contents are preserved.

The embedded Rust API restores an export's audit into a new graph:

```rust,ignore
let bundle: serde_json::Value = serde_json::from_str(&exported_json)?;
let restored = graph_core::Graph::from_exported_audit_bundle(&bundle)?;
let audit = restored.claim_audit_path(&claim_id)?;
```

`Graph::export_claim_audit_archive(&claim_ids)` and
`Graph::from_claim_audit_archive(&archive)` expose the same scoped archive directly.
Unsupported schemas, invalid references, duplicate roots and disagreement between
stored records and included audit views are rejected. This is a consistency check,
not a signature or an attestation of the exporter. An invalid retained audit causes
JSON serialization to fail; it is never silently omitted from a successful export.

The archive restores the selected audit surface. For a complete operational memory
copy, use `Graph::export_memory_json()` and `Graph::from_memory_json(&json)`: these
transport the full native persistence snapshot, including all governed stores and
policy registrations. They use the existing snapshot restoration validation.
Neither import API replaces an existing instance implicitly; each returns a new
`Graph` for the embedding application to persist through its normal atomic write
boundary.

The extension is absent when no exported claim needs an audit archive. Graphs with
no governed records retain their previous STIX/FIMI bytes. Native memory export
uses the existing snapshot serialization, preserving the ungoverned byte format.

For cluster member lookups on a restored archive, use
`ClaimStore::claim_link_at_index(index)`. Positions in the compact `claim_links()`
slice are local; the stored ledger indices can contain gaps after selection.

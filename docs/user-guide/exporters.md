# Export and STIX Validation

Corrobore builds deterministic interchange documents from an explicit export plan. Identical graph state, metadata, selection, and ordering produce stable output.

This guide targets the current `0.1.x` runtime baseline.

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
is not export-ready, lacks confidence or retained evidence, has malformed
canonical identity, fails provider validation, or references an excluded
endpoint. Permissive mode omits those records and returns at most 256
machine-readable entries in `export_diagnostics.exclusions`; it never creates a
fallback STIX identity for an unsupported record.

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

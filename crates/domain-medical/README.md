# domain-medical

MIT-licensed MEDICAL domain pack for clinical and biomedical evidence over
Corrobore agentic memory.

The pack supplies node and relationship types, fail-closed validation,
deterministic built-ins, and a reproducible evidence-table exporter. It ships as
an optional workspace crate; an installation without it keeps every base memory
capability.

## Boundaries

- Not a medical device and not a clinical decision support system. No surface
  returns a treatment recommendation or output intended for direct patient care.
- No node type represents an identifiable patient or research participant.
  `Population` is an aggregate cohort definition; `Person` covers investigators,
  authors, and reviewers.
- Participant-level content is rejected without an attested de-identification
  status.
- Evidence level and confidence are distinct fields; no built-in converts one
  into the other.

## Surface

- Types: `MedicalNodeType`, `MedicalRelationshipType`, `StudyDesign`,
  `EffectEstimate`, `MedicalNodeRecord`.
- Validation: `validate_medical_node` with `MedicalValidationPolicy`.
- Built-ins: `medical_study_design`, `medical_evidence_level` (scale-aware),
  `medical_interval_contains_null`, `medical_deidentification_status`.
- Export: `EvidenceTableExporter`, deterministic and byte-stable per snapshot
  and exporter version, with strict and permissive modes.
- Provider: `medical_provider_api_v1` for Rust callers;
  `corrobore_domain_provider_get_api_v1` is the `dlsym` entry point. Both expose
  `node.validate` with `domain: medical` over the domain provider ABI v1.

## Artifacts

The pack ships two crates:

- `domain-medical` is a pure `rlib` holding the types, validation, built-ins,
  exporter, and provider table. Rust callers use `medical_provider_api_v1`.
- `domain-medical-provider` is the `cdylib` a host loads with `dlopen`. It
  contains only the `corrobore_domain_provider_get_api_v1` entry point,
  delegating to the accessor above.

The split is deliberate. Every pack must export that entry point under the same
symbol, because the host resolves it per loaded library. Keeping the symbol out
of the `rlib` is what lets several packs be linked into one binary; with it in
the `rlib`, doing so is a duplicate-symbol link error.

See the product requirements in
`Estance-Labs/project-documents/product/medical-domain-product-requirements.md`.

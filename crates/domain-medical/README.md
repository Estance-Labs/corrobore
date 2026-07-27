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

## Linking several packs

Every pack exports the same `dlsym` entry point,
`corrobore_domain_provider_get_api_v1`, because the host resolves it per loaded
library. Rust callers that link more than one pack as an `rlib` must use the
uniquely-named accessor (`medical_provider_api_v1`,
`research_provider_api_v1`), since the shared symbol collapses onto a single
definition at link time.

See the product requirements in
`Estance-Labs/project-documents/product/medical-domain-product-requirements.md`.

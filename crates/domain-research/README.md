# domain-research

MIT-licensed RESEARCH domain pack for scholarly production, method,
reproducibility, and epistemic state in any discipline, over Corrobore agentic
memory.

The pack supplies node and relationship types, fail-closed validation,
deterministic built-ins, and a reproducible bibliography exporter. It ships as
an optional workspace crate; an installation without it keeps every base memory
capability.

## Boundaries

- No bibliometric ranking, authority score, prestige score, or quality score.
  A contract test asserts the absence rather than trusting convention.
- `research_reproducibility_signals` reports observations. Absent artifacts are
  reported as absent, never as a low score.
- No plagiarism or misconduct detection, no statistical meta-analysis, and no
  full-text redistribution: the pack stores references, structure, and locators.
- No `Author` node type. Authorship, review, and investigation are roles carried
  by relationships; `Person` is the type.

## Surface

- Types: `ResearchNodeType`, `ResearchRelationshipType`, `CitationStance`,
  `ResearchNodeRecord`, `CitationRecord`, `ReplicationAttemptRecord`.
- Validation: `validate_research_node`, `validate_research_citation`.
- Built-ins: `research_claim_attribution`, `research_citation_stance`,
  `research_support_count`, `research_refutation_count`,
  `research_contradiction_count`, `research_replication_status`,
  `research_retraction_status`, `research_supersession_chain`,
  `research_reproducibility_signals`, `research_identifier_normalize`,
  `research_identifier_is_valid`.
- Export: `BibliographyExporter`, deterministic and byte-stable per snapshot and
  exporter version, with strict and permissive modes.
- Provider: `research_provider_api_v1` for Rust callers;
  `corrobore_domain_provider_get_api_v1` is the `dlsym` entry point.

## Identifier normalization

DOI, arXiv, ORCID, ROR, ISSN, and PubMed. Normalization is deterministic and
offline: it canonicalizes shape and verifies self-contained check digits, so it
proves a string is well-formed but never that the work it names exists. ORCID
uses ISO 7064 MOD 11-2 and ISSN uses its mod-11 check digit, so a transposed
digit is rejected rather than stored.

## Artifacts

The pack ships two crates:

- `domain-research` is a pure `rlib` holding the types, validation, built-ins,
  exporter, and provider table. Rust callers use `research_provider_api_v1`.
- `domain-research-provider` is the `cdylib` a host loads with `dlopen`. It
  contains only the `corrobore_domain_provider_get_api_v1` entry point,
  delegating to the accessor above.

The split is deliberate. Every pack must export that entry point under the same
symbol, because the host resolves it per loaded library. Keeping the symbol out
of the `rlib` is what lets several packs be linked into one binary; with it in
the `rlib`, doing so is a duplicate-symbol link error.

See the product requirements in
`Estance-Labs/project-documents/product/research-domain-product-requirements.md`.

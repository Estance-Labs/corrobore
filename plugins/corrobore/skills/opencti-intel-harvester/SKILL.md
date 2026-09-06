---
name: opencti-intel-harvester
description: Extract grounded cyber threat intelligence from supplied reports with Corrobore and return exactly one deterministic, validated STIX 2.1 bundle.
license: MIT
metadata:
  author: Estance-Labs
  version: "1.1.0"
---

# OpenCTI Intelligence Harvester for Corrobore

You extract grounded Cyber Threat Intelligence from supplied documents, use Corrobore as structured working memory and validation substrate, and return exactly one valid STIX 2.1 bundle.

Use the [general Corrobore skill](../corrobore/SKILL.md) when the task is not specifically an end-to-end CTI report extraction.

## Final response contract

- Return only the final JSON payload.
- Return one STIX 2.1 bundle, even for multiple input files.
- Do not add Markdown, prose, comments, or extraction metadata outside the bundle.
- Never create unsupported facts merely to make the graph complete.

## Corrobore workflow

1. Check `GET /health` and start a durable session.
2. Extract candidate entities, observables, and relationships grounded in source spans.
3. Search semantic seeds if graph ids are unknown.
4. Read the existing subgraph before writing.
5. Follow [candidate ingestion and targeted repair](../corrobore/references/candidate-ingestion.md): submit, read the failing constraint, re-extract that field, resubmit.
6. Diagnose orphans, weak links, contradictions, and missing bridges with focused reads.
7. Re-read implicated spans, retain repair lineage, and explicitly promote source-reviewed candidates within task authorization.
8. Validate the explicit STIX bundle or graph projection.
9. Export the deterministic bundle and stop the session.

Relevant HTTP routes:

- `POST /v1/sessions/start`
- `POST /v1/seed/search`
- `POST /v1/cypher/read`
- `POST /v1/import/candidates`
- `GET /v1/import/candidates/{id}`
- `POST /v1/import/candidates/{id}/repairs`
- `POST /v1/import/candidates/{id}/promote`
- `POST /v1/stix/validate`
- `GET /v1/export/stix`
- `GET /v1/sessions/{session_id}/health`
- `GET /v1/sessions/{session_id}/logs`
- `POST /v1/sessions/{session_id}/stop`

## Extraction scope

Extract only when explicit or strongly supported:

- threat actors, intrusion sets, and campaigns;
- malware, tools, vulnerabilities, and ATT&CK techniques;
- indicators, observables, and infrastructure;
- identities, victims, organizations, sectors, and locations;
- relationships supported by the source.

## Evidence and uncertainty

Never invent names, aliases, dates, confidence, motives, sectors, countries, ATT&CK mappings, or attribution. Preserve language such as suspected, possible, likely, and unconfirmed. Do not overwrite stronger evidence with a weaker statement. Treat semantic ranking as navigation guidance, not proof.

## Multi-file output

- Produce one bundle containing one report per input file or logical document.
- Do not emit one bundle per file.
- Do not emit one global report unless requested.
- Do not use STIX grouping objects.

## Quality gates

Before returning the JSON bundle, verify:

1. the JSON parses and contains a single bundle;
2. every reference resolves;
3. key assertions are grounded in source evidence;
4. uncertain facts remain low-confidence or candidate;
5. required STIX fields pass validation;
6. no grouping object is present.

If an object cannot be repaired without invention, retain its unresolved candidate
and omit it and unsupported references from the final export. Do not delete raw
proposals or fabricate a record to satisfy the output shape.

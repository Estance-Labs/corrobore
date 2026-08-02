# Report-to-STIX acceptance

Status: release-gating corpus for issue #120.

This acceptance starts with **already-extracted** structured candidates. PDF parsing,
OCR, and LLM extraction remain outside Corrobore; the gate neither invokes nor
simulates those systems. It verifies the supported boundary between evidence
ingestion, native CTI metadata, graph validation, and deterministic STIX export.

These surfaces must stay distinct:

- evidence ingestion stores source digests and page, paragraph, or table-cell locators;
- generic graph memory stores application-owned entities and relationships but does not
  turn arbitrary nodes into CTI;
- the licensed CTI path validates eligible imported records through the native provider;
- the OpenCTI provider and OpenCTI-specific endpoints remain integration surfaces, not
  aliases for extraction or generic memory.

## Reproduce the gate

```sh
scripts/report-to-stix-acceptance.sh
```

The test compiles the committed provider fixture, verifies its manifest digest at server
startup, imports the corpus through `/v1/import/stix`, checks exact candidate metadata,
validates named issues, applies only the corrections below through `/v1/cypher/write`,
exports strict STIX, and compares a persistent restart byte for byte.

## Supported corrections

Raise the explicitly named low-confidence candidate:

```cypher
MATCH (n:OpenCtiType_malware) WHERE n.confidence = 0.4 SET n.confidence = 0.95, n.status = 'exportable' RETURN n.confidence, n.evidence_refs, n.status
```

Attach existing evidence to the explicitly named missing-evidence candidate:

```cypher
MATCH (n) WHERE n.confidence = 0.9 SET n.evidence_refs = ['evidence--report-table-6-1-2-3'], n.confidence = 0.95, n.status = 'exportable' RETURN n.confidence, n.evidence_refs, n.status
```

Promote the remaining evidence-backed nodes only after the named corrections succeed:

```cypher
MATCH (n) SET n.status = 'exportable'
```

Promote the evidence-backed directed relationships:

```cypher
MATCH (source)-[r]->(target) SET r.status = 'exportable'
```

Every `cypher` block in this page is discovered and executed by
`report_to_stix_acceptance`; adding an untested published query fails the gate.

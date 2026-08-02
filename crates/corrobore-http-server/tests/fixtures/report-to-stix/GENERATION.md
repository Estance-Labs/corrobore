# Deterministic generation recipe

The committed `input.json` starts at Corrobore's supported **already-extracted**
candidate boundary. PDF parsing, OCR, and LLM extraction happen outside Corrobore and
are deliberately neither invoked nor simulated by this fixture.

Regenerate the JSON oracle and checksums from the repository root:

```sh
node crates/corrobore-http-server/tests/fixtures/report-to-stix/generate.mjs
sha256sum -c crates/corrobore-http-server/tests/fixtures/report-to-stix/checksums.sha256
```

The generator uses fixed identifiers, timestamps, source text, object ordering, and
relationship construction. Review changes to generated JSON like source changes.

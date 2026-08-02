// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';

const read = (path) => readFile(new URL(`../${path}`, import.meta.url), 'utf8');

const fixtureRoot = 'crates/corrobore-http-server/tests/fixtures/report-to-stix';

test('the report-to-STIX corpus is redistributable, reproducible, and bounded', async () => {
  const [license, provenance, recipe, checksums, input, expected, provider] = await Promise.all([
    read(`${fixtureRoot}/LICENSE`),
    read(`${fixtureRoot}/PROVENANCE.md`),
    read(`${fixtureRoot}/GENERATION.md`),
    read(`${fixtureRoot}/checksums.sha256`),
    read(`${fixtureRoot}/input.json`),
    read(`${fixtureRoot}/expected.json`),
    read(`${fixtureRoot}/provider.c`),
  ]);

  assert.match(license, /MIT License/);
  assert.match(provenance, /synthetic/i);
  assert.match(provenance, /redistribut/i);
  assert.match(recipe, /already-extracted/i);
  assert.match(recipe, /outside Corrobore/i);
  assert.match(recipe, /neither invoked nor simulated/i);
  assert.match(checksums, /input\.json/);
  assert.match(checksums, /expected\.json/);
  assert.match(provider, /node\.validate/);

  const corpus = JSON.parse(input);
  const oracle = JSON.parse(expected);
  assert.equal(corpus.schema_version, '1.0');
  assert.equal(corpus.bundle.type, 'bundle');
  assert.ok(corpus.bundle.objects.length >= 40);
  assert.ok(corpus.bundle.objects.filter(({ type }) => type === 'relationship').length >= 30);
  assert.equal(corpus.evidence.schema_version, '1.0');
  assert.ok(corpus.evidence.records.some(({ locator }) => locator.type === 'page'));
  assert.ok(corpus.evidence.records.some(({ locator }) => locator.type === 'paragraph'));
  assert.ok(corpus.evidence.records.some(({ locator }) => locator.type === 'table_cell'));
  assert.equal(oracle.expected_import.relationships, 30);
  assert.deepEqual(oracle.validation_issue_codes.sort(), [
    'CTI_CONFIDENCE_TOO_LOW',
    'CTI_EVIDENCE_REQUIRED',
  ]);
  assert.ok(oracle.negative_cases.some(({ id }) => id === 'dangling-reference'));
  assert.ok(oracle.negative_cases.some(({ id }) => id === 'contradictory-candidate'));
  assert.ok(oracle.negative_cases.some(({ id }) => id === 'unknown-extension'));
});

test('the acceptance harness is a bounded release gate and the docs state the boundary', async () => {
  const [workflow, harness, guide, releaseNotes] = await Promise.all([
    read('.github/workflows/standalone-acceptance.yml'),
    read('scripts/report-to-stix-acceptance.sh'),
    read('docs/acceptance/report-to-stix.md'),
    read('docs/release-notes/v0.3.0.md'),
  ]);

  assert.match(workflow, /report-to-stix-acceptance/);
  assert.match(workflow, /timeout-minutes:/);
  assert.match(harness, /cargo test/);
  assert.match(harness, /report_to_stix_acceptance/);
  assert.match(harness, /--no-default-features/);
  assert.match(workflow, /Run report-to-STIX release acceptance/);
  assert.match(guide, /already-extracted/i);
  assert.match(guide, /outside Corrobore/i);
  assert.match(guide, /evidence ingestion/i);
  assert.match(guide, /generic (graph )?memory/i);
  assert.match(guide, /OpenCTI provider/i);
  assert.match(releaseNotes, /does not parse PDF/i);
});

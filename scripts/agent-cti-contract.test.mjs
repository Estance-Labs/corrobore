// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const read = (name) => fs.readFileSync(path.join(root, name), 'utf8');

test('agent surfaces explain confidence scales at their owning boundaries', () => {
  const skill = read('docs/skills/corrobore/how-to-use.md');
  const llmGuide = read('docs/for-llms.md');
  const cypherGuide = read('docs/user-guide/cypher.md');
  const ingestionGuide = read('docs/user-guide/ingestion.md');
  const openapi = read('docs/api/openapi.yaml');

  for (const guide of [skill, llmGuide]) {
    assert.match(guide, /confidence boundary/i);
    assert.match(guide, /Cypher.*0\.\.=1/is);
    assert.match(guide, /STIX import annotations.*0\.\.=100/is);
    assert.match(guide, /90.*native 0\.9/is);
  }

  assert.match(cypherGuide, /native 0\.\.=1 scale/i);
  assert.match(cypherGuide, /use 0\.9 for 90% STIX confidence/i);
  assert.match(ingestionGuide, /STIX import annotations.*0\.\.=100/is);
  assert.match(ingestionGuide, /90.*native 0\.9/is);
  assert.match(openapi, /STIX import confidence uses 0\.\.=100.*90 is stored as native 0\.9/is);
});

test('packaged skill requires relationship-owned metadata and coverage', () => {
  const skill = read('docs/skills/corrobore/how-to-use.md');
  const llmGuide = read('docs/for-llms.md');

  for (const guide of [skill, llmGuide]) {
    assert.match(guide, /relationship(?: object| assertion) owns its own evidence and confidence/i);
    assert.match(guide, /Indicator.*Observed Data.*based-on/is);
    assert.match(guide, /Indicator.*CTI (?:domain object|SDO).*indicates/is);
    assert.match(guide, /relationship coverage/i);
    assert.match(guide, /do not fabricate/i);
  }

  assert.match(skill, /MATCH \(source\)-\[r\]->\(target\)/);
  assert.match(skill, /r\.confidence IS NULL/);
  assert.match(skill, /r\.evidence_refs IS NULL/);
});

test('agent export choreography preserves read-only strict correctness', () => {
  const skill = read('docs/skills/corrobore/how-to-use.md');
  const llmGuide = read('docs/for-llms.md');
  const exporters = read('docs/user-guide/exporters.md');
  const combined = `${skill}\n${llmGuide}\n${exporters}`;

  assert.match(combined, /late writes remain candidate/i);
  assert.match(combined, /new readiness and promotion pass/i);
  assert.match(combined, /strict.*default correctness gate/is);
  assert.match(combined, /permissive.*explicit.*diagnostic partial bundle/is);
  assert.match(combined, /force=true.*explicit operator decision.*never.*automatic LLM fallback/is);
  assert.match(exporters, /GET \/v1\/export\/stix.*read-only/is);
  assert.match(exporters, /does not promote|never promotes/i);
});

test('public agent and CTI guides do not claim the obsolete 0.1.x baseline', () => {
  for (const path of [
    'docs/architecture.md',
    'docs/for-llms.md',
    'docs/user-guide/cypher.md',
    'docs/user-guide/ingestion.md',
    'docs/user-guide/exporters.md',
    'docs/user-guide/embedded-engine.md',
    'docs/user-guide/working-set.md',
  ]) {
    assert.doesNotMatch(read(path), /current `0\.1\.x` runtime baseline/);
  }
});

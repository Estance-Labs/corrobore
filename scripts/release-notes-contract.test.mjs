import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { checkReleaseDocumentation } from './release-notes-contract.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

test('v0.3.3 version metadata and public release documentation stay aligned', () => {
  const result = checkReleaseDocumentation('0.3.3', '0.3.2');

  assert.equal(result.version, '0.3.3');
  assert.equal(result.tag, 'v0.3.3');
  assert.equal(result.previousTag, 'v0.3.2');
  assert.ok(result.releaseNoteSectionCount >= 5);

  const releaseNote = fs.readFileSync(path.join(root, 'docs/release-notes/v0.3.3.md'), 'utf8');
  assert.match(releaseNote, /Cypher.*`0\.\.=1`.*STIX.*`0\.\.=100`/is);
  assert.match(releaseNote, /relationship.*own.*evidence.*confidence/is);
  assert.match(releaseNote, /based-on.*indicates/is);
  assert.match(releaseNote, /GET \/v1\/export\/stix.*read-only.*never promotes/is);
  assert.match(releaseNote, /tag push.*completion\s+boundary/is);
});

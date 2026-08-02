import test from 'node:test';
import assert from 'node:assert/strict';

import { checkReleaseDocumentation } from './release-notes-contract.mjs';

test('v0.3.2 version metadata and public release documentation stay aligned', () => {
  const result = checkReleaseDocumentation('0.3.2', '0.3.1');

  assert.equal(result.version, '0.3.2');
  assert.equal(result.tag, 'v0.3.2');
  assert.equal(result.previousTag, 'v0.3.1');
  assert.ok(result.releaseNoteSectionCount >= 5);
});

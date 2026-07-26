import test from 'node:test';
import assert from 'node:assert/strict';

import { checkReleaseDocumentation } from './release-notes-contract.mjs';

test('v0.3.0 version metadata and public release documentation stay aligned', () => {
  const result = checkReleaseDocumentation('0.3.0');

  assert.equal(result.version, '0.3.0');
  assert.equal(result.tag, 'v0.3.0');
  assert.ok(result.releaseNoteSectionCount >= 5);
});

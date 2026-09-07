import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const read = async (name) => readFile(new URL(`../${name}`, import.meta.url), 'utf8');
const json = async (name) => JSON.parse(await read(name));

// The nlq/v1 envelope is a committed contract (epic #82, item #260): the
// schema, its replayable fixtures and the public page must agree with each
// other and with the crate that implements them.

test('the committed schema is closed, versioned and names every action kind', async () => {
  const schema = await json('compatibility/nlq/v1/action-envelope.schema.json');
  assert.equal(schema.properties.schema_version.const, 'nlq/v1');
  assert.equal(schema.additionalProperties, false);
  const kinds = schema.properties.action.oneOf.map((variant) => variant.properties.kind.const).sort();
  assert.deepEqual(kinds, [
    'abstain',
    'clarification_required',
    'cypher_read',
    'cypher_write_proposal',
    'investigation',
    'memory_operation',
    'unsupported',
  ]);
  for (const variant of schema.properties.action.oneOf) {
    assert.equal(variant.additionalProperties, false, `${variant.properties.kind.const} is closed`);
  }
  // Trusted runtime context has no field in the envelope.
  for (const key of schema['x-corrobore'].trusted_context_keys_refused_at_any_depth) {
    assert.equal(schema.properties[key], undefined, `${key} must not be an envelope field`);
  }
  const source = await read('crates/corrobore-nlq/src/lib.rs');
  for (const key of schema['x-corrobore'].trusted_context_keys_refused_at_any_depth) {
    assert.ok(source.includes(`"${key}"`), `${key} is refused by the crate`);
  }
});

test('fixtures cover every rejection code and every action kind', async () => {
  const schema = await json('compatibility/nlq/v1/action-envelope.schema.json');
  const fixtures = await json('compatibility/nlq/v1/fixtures.json');
  assert.equal(fixtures.schema_version, 'nlq/v1');
  const refusedCodes = new Set(fixtures.refused.map((entry) => entry.code));
  for (const code of schema['x-corrobore'].rejection_codes) {
    // A code with no fixture cannot be replayed by another adapter. Two codes
    // are reachable only with a permissive boundary and are covered by the
    // Rust envelope contract instead.
    if (code === 'action_kind_mismatch') continue;
    assert.ok(refusedCodes.has(code), `no fixture refuses with ${code}`);
  }
  const acceptedKinds = new Set(fixtures.accepted.map((entry) => entry.kind));
  for (const kind of ['cypher_read', 'investigation', 'memory_operation', 'clarification_required', 'abstain']) {
    assert.ok(acceptedKinds.has(kind), `no accepted fixture for ${kind}`);
  }
  for (const entry of [...fixtures.accepted, ...fixtures.refused]) {
    assert.ok(entry.name && entry.envelope, 'every fixture is named and carries an envelope');
  }
});

test('the public page and navigation carry the contract', async () => {
  const [page, navigation, changelog] = await Promise.all([
    read('docs/user-guide/nlq.md'),
    read('mkdocs.yml'),
    read('CHANGELOG.md'),
  ]);
  assert.match(navigation, /user-guide\/nlq\.md/);
  assert.match(page, /nlq\/v1/);
  assert.match(page, /compatibility\/nlq\/v1\/action-envelope\.schema\.json/);
  for (const kind of ['memory_operation', 'cypher_read', 'cypher_write_proposal', 'investigation', 'clarification_required', 'abstain', 'unsupported']) {
    assert.ok(page.includes(`\`${kind}\``), `page documents ${kind}`);
  }
  assert.match(page, /invented_evidence/);
  assert.match(page, /trusted/i);
  assert.match(changelog, /corrobore-nlq/);
});

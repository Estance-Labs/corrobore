import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';

const root = path.resolve(import.meta.dirname, '..');
const read = (name) => fs.readFileSync(path.join(root, name), 'utf8');
const skills = ['plugins/corrobore/skills/corrobore/SKILL.md', 'plugins/corrobore/skills/opencti-intel-harvester/SKILL.md'];
const reference = 'plugins/corrobore/skills/corrobore/references/candidate-ingestion.md';

test('both discovered skills reach the packaged candidate loop', () => {
  for (const skill of skills) {
    const links = [...read(skill).matchAll(/\]\(([^)]+candidate-ingestion\.md)\)/g)];
    assert.ok(links.some((link) => path.resolve(root, path.dirname(skill), link[1]) === path.join(root, reference)), `${skill} must reach the candidate contract`);
  }
});
test('candidate examples use supported routes and retain repair lineage', () => {
  const guide = read(reference);
  const examples = [...guide.matchAll(/```json\n([\s\S]*?)\n```/g)].map((m) => JSON.parse(m[1]));
  const [submission, repair] = examples;
  assert.equal(submission.tier, 'Shadow');
  assert.ok(submission.extraction_run_id);
  const original = JSON.parse(submission.raw_payload);
  const corrected = JSON.parse(repair.raw_payload);
  assert.notEqual(repair.id, submission.id);
  assert.deepEqual(repair.caused_by, [submission.constraints[0].id]);
  assert.equal(original.evidence_ref, corrected.evidence_ref);
  assert.equal(original.confidence, corrected.confidence);
  assert.equal(submission.constraints[0].field, '/name');
  assert.equal(typeof corrected.name, 'string');
  for (const route of ['/v1/import/candidates', '/v1/import/candidates/{id}/repairs', '/v1/import/candidates/{id}/promote']) {
    assert.ok(guide.includes(route));
    assert.ok(read('docs/api/openapi.yaml').includes(`  ${route}:`));
  }
});
test('agent extraction recipes do not bypass the candidate tier', () => {
  const files = [...skills, ...fs.readdirSync(path.join(root, 'plugins/corrobore/skills/corrobore/references')).filter((f) => f.endsWith('.md')).map((f) => `plugins/corrobore/skills/corrobore/references/${f}`)];
  for (const file of files) {
    const text = read(file);
    for (const code of text.matchAll(/```cypher\n([\s\S]*?)\n```/g)) {
      assert.doesNotMatch(code[1], /\b(CREATE|MERGE|SET|DELETE|REMOVE)\b/i, file);
    }
    assert.doesNotMatch(text, /^\s*(?:\d+\.|-)\s.*(?:`MERGE`|POST \/v1\/cypher\/write|Materialize .*graph)/m, file);
  }
});

import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
const root = path.resolve(import.meta.dirname, '..');
const read = (file) => fs.readFileSync(path.join(root, file), 'utf8');
const reference = 'plugins/corrobore/skills/corrobore/references/claim-audit.md';
test('both packaged skills require the audit path before asserting a verdict', () => {
  for (const skill of ['plugins/corrobore/skills/corrobore/SKILL.md', 'plugins/corrobore/skills/opencti-intel-harvester/SKILL.md']) {
    const text = read(skill);
    assert.match(text, /Before asserting a verdict/);
    assert.match(text, /GET \/v1\/claims\/\{id\}\/audit/);
    const links = [...text.matchAll(/\]\(([^)]+claim-audit\.md)\)/g)];
    assert.ok(links.some((link) => path.resolve(root, path.dirname(skill), link[1]) === path.join(root, reference)));
  }
});
test('audit guidance preserves uncertainty and uses supported reversible decision requests', () => {
  const guide = read(reference);
  for (const token of ['current_verdict', 'contradictions', 'state_transitions', 'unverified_steps', 'semantically_judged', 'mechanically_checked', 'unchecked', 'failing', 'verifier_id', 'verifier_version', 'link_membership', 'dimensions']) assert.ok(guide.includes(token), token);
  assert.match(guide, /does not re-run/);
  assert.match(guide, /do not\s+assert a verdict/i);
  const examples = [...guide.matchAll(/```json\n([\s\S]*?)\n```/g)].map((m) => JSON.parse(m[1]));
  assert.deepEqual(examples.map((p) => p.action.kind), ['override', 'reversal']);
  assert.equal(examples[1].action.decision_id, examples[0].id);
  for (const example of examples) {
    assert.ok(example.actor && example.id && Number.isFinite(Date.parse(example.recorded_at)));
    assert.ok(example.action.rationale);
    assert.equal(example.verdict, undefined);
  }
  assert.ok(read('docs/user-guide/claim-audit.md').includes('WS-F acceptance evidence'));
  assert.ok(read('mkdocs.yml').includes('user-guide/claim-audit.md'));
});

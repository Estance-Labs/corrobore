// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const read = (name) => fs.readFileSync(path.join(root, name), 'utf8');

test('engine contract owns seven domain-neutral operations and trusted context', () => {
  const source = read('crates/corrobore-engine/src/memory.rs');
  for (const operation of [
    'Remember',
    'Relate',
    'Recall',
    'Update',
    'Forget',
    'Consolidate',
    'Trace',
  ]) {
    assert.match(source, new RegExp(`\\b${operation}\\(`));
  }
  assert.match(source, /pub struct MemoryServiceContext/);
  assert.match(source, /pub struct MemoryPermissions/);
  assert.match(source, /persist_graph_transition/);
  assert.match(source, /pub struct MutationReceipt/);
  assert.match(source, /pub enum RecallOutcome/);
  for (const outcome of ['PartialPageIn', 'Cancelled', 'Overloaded']) {
    assert.match(source, new RegExp(`\\b${outcome},`));
  }
  assert.doesNotMatch(source, /pub query:/);
});

test('standalone route, OpenAPI, guide, and trusted configuration stay aligned', () => {
  const app = read('crates/corrobore-http-server/src/app.rs');
  const openapi = read('docs/api/openapi.yaml');
  const guide = read('docs/user-guide/memory-operations.md');
  const httpGuide = read('docs/user-guide/http-server.md');
  const navigation = read('mkdocs.yml');
  assert.match(app, /"\/v1\/memory\/operations"/);
  assert.match(openapi, /^  \/v1\/memory\/operations:/m);
  assert.match(openapi, /MemoryOperationRequest/);
  assert.match(guide, /remember → relate → recall → update → trace → forget/);
  assert.match(guide, /regulatory erasure/i);
  assert.match(guide, /compatibility/i);
  assert.match(navigation, /user-guide\/memory-operations\.md/);
  for (const variable of [
    'CORROBORE_MEMORY_WORKSPACE_ID',
    'CORROBORE_MEMORY_ACTOR_ID',
    'CORROBORE_MEMORY_AGENT_ID',
    'CORROBORE_MEMORY_SESSION_ID',
    'CORROBORE_MEMORY_PERMISSIONS',
  ]) {
    assert.match(httpGuide, new RegExp(variable));
  }
});

test('agent lifecycle guide keeps memory governance explicit and navigable', () => {
  const guide = read('docs/user-guide/agent-memory-lifecycle.md');
  const navigation = read('mkdocs.yml');

  assert.match(navigation, /Agent Memory Lifecycle: user-guide\/agent-memory-lifecycle\.md/);
  for (const operation of [
    'remember',
    'relate',
    'recall',
    'update',
    'forget',
    'consolidate',
    'trace',
  ]) {
    assert.match(guide, new RegExp(`\\b${operation}\\b`));
  }
  for (const kind of ['working_state', 'episode', 'claim', 'fact', 'procedure', 'source']) {
    assert.match(guide, new RegExp(`\\b${kind}\\b`));
  }
  for (const status of ['candidate', 'validated', 'contested', 'rejected']) {
    assert.match(guide, new RegExp(`\\b${status}\\b`));
  }
  assert.match(guide, /application-owned conventions/i);
  assert.match(guide, /confidence is not proof/i);
  assert.match(guide, /never silently (?:delete|overwrite)/i);
  assert.match(guide, /hot.*warm.*cold/is);
  assert.match(guide, /canonical.*shadow.*quarantine.*hypothesis/is);
  assert.match(guide, /\[High-level Memory Operations\]\(memory-operations\.md\)/);
  assert.match(guide, /\[For LLM Agents\]\(\.\.\/for-llms\.md\)/);
});

test('shared v1 conformance corpus covers every operation without Cypher', () => {
  const corpus = JSON.parse(read('compatibility/memory/v1/conformance.json'));
  assert.equal(corpus.contract_version, 'v1');
  assert.deepEqual(
    new Set(corpus.operations.map((entry) => entry.operation)),
    new Set(['remember', 'relate', 'recall', 'update', 'forget', 'consolidate', 'trace']),
  );
  assert.equal(JSON.stringify(corpus).toLowerCase().includes('cypher'), false);
});

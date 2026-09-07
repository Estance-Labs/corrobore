import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile, readdir} from 'node:fs/promises';

const read = (name) => readFile(new URL(`../${name}`, import.meta.url), 'utf8');

test('no agent runtime object exists in the evidence graph', async () => {
  const directory = new URL('../crates/graph-core/src/', import.meta.url);
  const sources = (await readdir(directory, {recursive: true})).filter((name) => name.endsWith('.rs'));
  assert.ok(sources.length > 40, 'the graph-core source tree must be scanned');

  for (const name of sources) {
    const source = await readFile(new URL(name, directory), 'utf8');
    // ADR-0019: AgentDefinition, Session, Run and ToolCall belong to the control
    // plane. The graph stores links, never runtime objects.
    assert.doesNotMatch(
      source,
      /\b(AgentDefinition|ToolCall|AgentRun|RuntimeRef|MutationAuditChain|AgentWritePolicy)\b/,
      `${name} must not carry an agent runtime object`,
    );
  }
});

test('the evidence graph does not depend on the runtime that calls it', async () => {
  const manifest = await read('crates/graph-core/Cargo.toml');
  // The dependency direction is the structural guarantee: shared-runtime and
  // the engine depend on graph-core, never the reverse, so a runtime object
  // cannot become reachable from a graph record.
  assert.doesNotMatch(manifest, /shared-runtime|corrobore-engine|cypher-/);
});

test('a protocol revision cannot reach the graph through the capability catalogue', async () => {
  const catalogue = JSON.parse(await read('compatibility/capabilities/v1/catalogue.json'));
  const runtimeOperations = new Set(catalogue.capabilities.map((capability) => capability.runtime_operation));

  for (const operation of runtimeOperations) {
    assert.doesNotMatch(operation, /jsonrpc|mcp|a2a|webmcp|\d{4}-\d{2}-\d{2}/i, `${operation} names a runtime operation, not a protocol`);
  }
});

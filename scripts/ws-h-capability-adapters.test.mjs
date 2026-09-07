import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {listTools} from '../plugins/corrobore/mcp-server/lib.mjs';

const read = async (name) => JSON.parse(await readFile(new URL(`../${name}`, import.meta.url), 'utf8'));

// The MCP adapter names its own surface. The convention lives here, in the
// adapter's contract, and never in the capability definition.
const toolName = (id) => `corrobore_${id.replace(/^(memory|runtime)\./, '').replace(/\./g, '_')}`;

test('every MCP tool projects one catalogued capability and nothing else', async () => {
  const catalogue = await read('compatibility/capabilities/v1/catalogue.json');
  const exposed = catalogue.capabilities.filter(
    (capability) => !capability.restricted_to || capability.restricted_to.includes('mcp'),
  );
  const tools = listTools();

  assert.deepEqual(
    tools.map((tool) => tool.name).sort(),
    exposed.map((capability) => toolName(capability.id)).sort(),
    'the MCP surface covers exactly the capabilities exposed to it',
  );
});

test('an adapter cannot disagree with the definition about writing', async () => {
  const catalogue = await read('compatibility/capabilities/v1/catalogue.json');
  const byTool = new Map(
    catalogue.capabilities.map((capability) => [toolName(capability.id), capability]),
  );

  for (const tool of listTools()) {
    const capability = byTool.get(tool.name);
    assert.ok(capability, `${tool.name} must project a catalogued capability`);
    assert.equal(
      tool.annotations.readOnlyHint,
      capability.effect === 'read',
      `${tool.name} must advertise the effect the catalogue defines`,
    );
    if (capability.authorization === 'operator_approval') {
      assert.equal(
        tool.annotations.destructiveHint,
        true,
        `${tool.name} needs an operator approval and must be advertised as destructive`,
      );
    }
  }
});

test('the catalogue carries no protocol shape and no protocol version', async () => {
  const catalogue = await read('compatibility/capabilities/v1/catalogue.json');
  const source = await readFile(new URL('../crates/shared-runtime/src/capabilities.rs', import.meta.url), 'utf8');

  // A protocol version bump touches an adapter, never the definition: there is
  // no field here to record one in.
  for (const capability of catalogue.capabilities) {
    assert.deepEqual(
      Object.keys(capability).filter((key) => !['id', 'summary', 'effect', 'authorization', 'runtime_operation', 'restricted_to'].includes(key)),
      [],
      `${capability.id} carries only definition fields`,
    );
    assert.doesNotMatch(capability.runtime_operation, /^\/|jsonrpc|mcp|a2a/i);
  }
  assert.doesNotMatch(source, /jsonrpc|json-rpc|tools\/list|2026-07-28/i);
});

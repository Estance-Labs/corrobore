// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

import assert from 'node:assert/strict';
import { EventEmitter, once } from 'node:events';
import fs from 'node:fs';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import { spawn } from 'node:child_process';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const serverPath = path.join(root, 'plugins', 'corrobore', 'mcp-server', 'server.mjs');
const protocolVersion = '2025-06-18';
const expectedTools = [
  'corrobore_consolidate',
  'corrobore_forget',
  'corrobore_ready',
  'corrobore_recall',
  'corrobore_relate',
  'corrobore_remember',
  'corrobore_stix_export',
  'corrobore_stix_import',
  'corrobore_stix_validate',
  'corrobore_trace',
  'corrobore_update',
];

async function listen(handler) {
  const requests = [];
  const server = http.createServer(async (request, response) => {
    const chunks = [];
    for await (const chunk of request) chunks.push(chunk);
    const bodyText = Buffer.concat(chunks).toString('utf8');
    const record = {
      method: request.method,
      url: request.url,
      authorization: request.headers.authorization,
      body: bodyText === '' ? undefined : JSON.parse(bodyText),
    };
    requests.push(record);
    await handler(request, response, record);
  });
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  const address = server.address();
  return {
    baseUrl: `http://127.0.0.1:${address.port}`,
    requests,
    close: () => new Promise((resolve, reject) => {
      server.close((error) => error ? reject(error) : resolve());
    }),
  };
}

function startMcp(environment = {}) {
  const child = spawn(process.execPath, [serverPath], {
    env: {
      ...process.env,
      CORROBORE_MCP_BASE_URL: 'http://127.0.0.1:8080',
      ...environment,
    },
    stdio: ['pipe', 'pipe', 'pipe'],
  });
  const events = new EventEmitter();
  const messages = [];
  let stdoutBuffer = '';
  let stderr = '';

  child.stdout.setEncoding('utf8');
  child.stdout.on('data', (chunk) => {
    stdoutBuffer += chunk;
    while (stdoutBuffer.includes('\n')) {
      const index = stdoutBuffer.indexOf('\n');
      const line = stdoutBuffer.slice(0, index);
      stdoutBuffer = stdoutBuffer.slice(index + 1);
      if (line === '') continue;
      messages.push(JSON.parse(line));
      events.emit('message');
    }
  });
  child.stderr.setEncoding('utf8');
  child.stderr.on('data', (chunk) => { stderr += chunk; });

  async function receive(predicate, timeoutMs = 3_000) {
    const deadline = Date.now() + timeoutMs;
    while (true) {
      const index = messages.findIndex(predicate);
      if (index >= 0) return messages.splice(index, 1)[0];
      const remaining = deadline - Date.now();
      if (remaining <= 0) {
        throw new Error(`timed out waiting for MCP message; stderr=${stderr}`);
      }
      await Promise.race([
        once(events, 'message'),
        new Promise((_, reject) => setTimeout(() => reject(new Error('poll timeout')), remaining)),
      ]).catch((error) => {
        if (error.message !== 'poll timeout') throw error;
      });
    }
  }

  return {
    child,
    send(message) {
      const line = typeof message === 'string' ? message : JSON.stringify(message);
      child.stdin.write(`${line}\n`);
    },
    receive,
    getStderr: () => stderr,
    getStdoutRemainder: () => stdoutBuffer,
    async stop() {
      if (child.exitCode !== null || child.signalCode !== null) return;
      child.stdin.end();
      const exit = once(child, 'exit');
      const timer = setTimeout(() => child.kill('SIGKILL'), 2_000);
      await exit;
      clearTimeout(timer);
    },
  };
}

async function initialize(client) {
  client.send({
    jsonrpc: '2.0',
    id: 1,
    method: 'initialize',
    params: {
      protocolVersion,
      capabilities: {},
      clientInfo: { name: 'corrobore-contract-test', version: '1.0.0' },
    },
  });
  const response = await client.receive((message) => message.id === 1);
  assert.equal(response.result.protocolVersion, protocolVersion);
  assert.equal(response.result.serverInfo.name, 'corrobore-portable-mcp');
  assert.deepEqual(response.result.capabilities, { tools: { listChanged: false } });
  client.send({ jsonrpc: '2.0', method: 'notifications/initialized' });
}

async function callTool(client, id, name, args = {}) {
  client.send({
    jsonrpc: '2.0',
    id,
    method: 'tools/call',
    params: { name, arguments: args },
  });
  return client.receive((message) => message.id === id);
}

test('stdio server negotiates MCP, lists the complete tool surface, and keeps stdout protocol-only', async () => {
  const client = startMcp();
  try {
    await initialize(client);
    client.send({ jsonrpc: '2.0', id: 2, method: 'ping' });
    assert.deepEqual(await client.receive((message) => message.id === 2), {
      jsonrpc: '2.0', id: 2, result: {},
    });

    client.send({ jsonrpc: '2.0', id: 3, method: 'tools/list', params: {} });
    const listed = await client.receive((message) => message.id === 3);
    const tools = listed.result.tools;
    assert.deepEqual(tools.map((tool) => tool.name).sort(), expectedTools);
    for (const tool of tools) {
      assert.equal(tool.inputSchema.type, 'object');
      assert.equal(tool.annotations.openWorldHint, true);
      assert.match(tool.description, /Corrobore/);
    }
    assert.equal(client.getStdoutRemainder(), '');
    assert.equal(client.getStderr(), '');
  } finally {
    await client.stop();
  }
});

test('tools map to the documented HTTP methods, paths, bodies, query, and bearer boundary', async () => {
  const upstream = await listen(async (_request, response, record) => {
    response.writeHead(200, { 'content-type': 'application/json' });
    response.end(JSON.stringify({ ok: true, result: record.url }));
  });
  const client = startMcp({
    CORROBORE_MCP_BASE_URL: upstream.baseUrl,
    CORROBORE_MCP_AUTH_TOKEN: 'contract-test-token',
  });
  try {
    await initialize(client);
    let id = 10;
    await callTool(client, id++, 'corrobore_ready');
    const operations = ['remember', 'relate', 'recall', 'update', 'forget', 'consolidate', 'trace'];
    for (const operation of operations) {
      const args = { input: { marker: operation } };
      if (['remember', 'relate', 'update', 'forget'].includes(operation)) {
        args.idempotency_key = `idem-${operation}`;
      }
      const response = await callTool(client, id++, `corrobore_${operation}`, args);
      assert.equal(response.result.isError, undefined);
      assert.equal(response.result.structuredContent.ok, true);
    }
    await callTool(client, id++, 'corrobore_stix_import', {
      bundle: { type: 'bundle', id: 'bundle--test', objects: [] },
    });
    await callTool(client, id++, 'corrobore_stix_validate', {
      source: 'graph', snapshot_id: 'snapshot--test',
    });
    await callTool(client, id++, 'corrobore_stix_export', {
      snapshot_id: 'snapshot--test',
      transaction_id: 'transaction--test',
      exporter_version: 'contract-test',
      mode: 'permissive',
      profile: 'stix-mvp',
      force: true,
    });

    assert.deepEqual(upstream.requests[0], {
      method: 'GET', url: '/health/ready', authorization: 'Bearer contract-test-token', body: undefined,
    });
    for (const [index, operation] of operations.entries()) {
      const expected = {
        contract_version: 'v1',
        operation,
        input: { marker: operation },
      };
      if (['remember', 'relate', 'update', 'forget'].includes(operation)) {
        expected.idempotency_key = `idem-${operation}`;
      }
      assert.deepEqual(upstream.requests[index + 1], {
        method: 'POST',
        url: '/v1/memory/operations',
        authorization: 'Bearer contract-test-token',
        body: expected,
      });
    }
    assert.equal(upstream.requests[8].method, 'POST');
    assert.equal(upstream.requests[8].url, '/v1/import/stix');
    assert.equal(upstream.requests[8].body.bundle.type, 'bundle');
    assert.deepEqual(upstream.requests[9].body, { source: 'graph', snapshot_id: 'snapshot--test' });
    assert.equal(upstream.requests[9].url, '/v1/stix/validate');
    assert.equal(
      upstream.requests[10].url,
      '/v1/export/stix?snapshot_id=snapshot--test&transaction_id=transaction--test&exporter_version=contract-test&mode=permissive&profile=stix-mvp&force=true',
    );
  } finally {
    await client.stop();
    await upstream.close();
  }
});

test('token files are supported without placing credentials in mcp.json', async () => {
  const temporaryRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'corrobore-mcp-token-'));
  const tokenPath = path.join(temporaryRoot, 'token');
  fs.writeFileSync(tokenPath, 'token-from-file\n', { mode: 0o600 });
  const upstream = await listen(async (_request, response) => {
    response.writeHead(200, { 'content-type': 'application/json' });
    response.end('{"ok":true}');
  });
  const client = startMcp({
    CORROBORE_MCP_BASE_URL: upstream.baseUrl,
    CORROBORE_MCP_AUTH_TOKEN: '',
    CORROBORE_MCP_AUTH_TOKEN_FILE: tokenPath,
  });
  try {
    await initialize(client);
    await callTool(client, 10, 'corrobore_ready');
    assert.equal(upstream.requests[0].authorization, 'Bearer token-from-file');
  } finally {
    await client.stop();
    await upstream.close();
    fs.rmSync(temporaryRoot, { recursive: true });
  }
});

test('protocol and upstream failures are bounded and do not crash the server', async () => {
  const upstream = await listen(async (_request, response) => {
    response.writeHead(503, { 'content-type': 'application/json' });
    response.end('{"ok":false,"error":{"code":"not_ready"}}');
  });
  const client = startMcp({ CORROBORE_MCP_BASE_URL: upstream.baseUrl });
  try {
    await initialize(client);
    const failure = await callTool(client, 10, 'corrobore_ready');
    assert.equal(failure.result.isError, true);
    assert.match(failure.result.content[0].text, /503/);

    const unknown = await callTool(client, 11, 'corrobore_unknown');
    assert.equal(unknown.error.code, -32602);

    client.send('{not-json');
    const malformed = await client.receive((message) => message.error?.code === -32700);
    assert.equal(malformed.id, null);

    client.send({ jsonrpc: '2.0', id: 12, method: 'ping' });
    assert.deepEqual((await client.receive((message) => message.id === 12)).result, {});
  } finally {
    await client.stop();
    await upstream.close();
  }
});

test('timeouts, network failures, oversized messages, and unsafe configuration fail closed', async () => {
  const slowUpstream = await listen(async (_request, response) => {
    await new Promise((resolve) => setTimeout(resolve, 200));
    response.writeHead(200, { 'content-type': 'application/json' });
    response.end('{"ok":true}');
  });
  const timed = startMcp({
    CORROBORE_MCP_BASE_URL: slowUpstream.baseUrl,
    CORROBORE_MCP_TIMEOUT_MS: '50',
  });
  try {
    await initialize(timed);
    const response = await callTool(timed, 10, 'corrobore_ready');
    assert.equal(response.result.isError, true);
    assert.match(response.result.content[0].text, /timed out/i);
  } finally {
    await timed.stop();
    await slowUpstream.close();
  }

  const unavailable = await listen(async (_request, response) => response.end());
  const closedBaseUrl = unavailable.baseUrl;
  await unavailable.close();
  const networked = startMcp({ CORROBORE_MCP_BASE_URL: closedBaseUrl });
  try {
    await initialize(networked);
    const response = await callTool(networked, 10, 'corrobore_ready');
    assert.equal(response.result.isError, true);
    assert.match(response.result.content[0].text, /request failed/i);
  } finally {
    await networked.stop();
  }

  const bounded = startMcp({ CORROBORE_MCP_MAX_MESSAGE_BYTES: '256' });
  try {
    bounded.send(JSON.stringify({ jsonrpc: '2.0', id: 20, method: 'ping', padding: 'x'.repeat(512) }));
    const response = await bounded.receive((message) => message.error?.code === -32600);
    assert.match(response.error.message, /maximum size/i);
    bounded.send({ jsonrpc: '2.0', id: 21, method: 'ping' });
    assert.deepEqual((await bounded.receive((message) => message.id === 21)).result, {});
  } finally {
    await bounded.stop();
  }

  const unsafe = startMcp({ CORROBORE_MCP_BASE_URL: 'file:///tmp/corrobore.sock' });
  const [exitCode] = await once(unsafe.child, 'exit');
  assert.equal(exitCode, 1);
  assert.equal(unsafe.getStdoutRemainder(), '');
  assert.match(unsafe.getStderr(), /http or https/i);
});

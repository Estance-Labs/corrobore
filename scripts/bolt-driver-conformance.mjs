#!/usr/bin/env node
// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//
// Driver conformance for the Bolt listener (epic #12, item #258).
//
// This script runs the official `neo4j-driver` for Node.js against a live
// Corrobore Bolt listener and checks the behaviour a driver user sees:
// connection and authentication, `executeQuery`, managed read and write
// transactions, typed values, node and relationship structures, streaming of a
// larger result, and the failure path. It is not a unit test: it needs a
// running server and the driver package.
//
//   CORROBORE_BOLT_URL=bolt://127.0.0.1:7687 \
//   CORROBORE_BOLT_TOKEN=<bearer token> \
//   node scripts/bolt-driver-conformance.mjs
//
// The driver is resolved with CommonJS rules from the current working
// directory, so either `npm install --no-save neo4j-driver` where you run the
// script or point NODE_PATH at a directory that holds it. The repository itself
// carries no npm dependency.

import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import path from 'node:path';

const url = process.env.CORROBORE_BOLT_URL ?? 'bolt://127.0.0.1:7687';
const token = process.env.CORROBORE_BOLT_TOKEN;
if (!token) {
  console.error('CORROBORE_BOLT_TOKEN is required');
  process.exit(2);
}

let neo4j;
try {
  const require = createRequire(path.join(process.cwd(), 'package.json'));
  neo4j = require('neo4j-driver');
} catch (error) {
  console.error(`neo4j-driver is not installed: ${error.message}`);
  process.exit(2);
}

const results = [];
async function check(name, run) {
  try {
    await run();
    results.push({ name, ok: true });
    console.log(`ok   ${name}`);
  } catch (error) {
    results.push({ name, ok: false, error: error.message });
    console.log(`FAIL ${name}\n     ${error.message}`);
  }
}

const marker = `conformance-${Date.now()}`;
const driver = neo4j.driver(url, neo4j.auth.basic('analyst', token), {
  disableLosslessIntegers: true,
});

await check('connectivity and server agent', async () => {
  const info = await driver.getServerInfo();
  assert.match(info.agent, /^Corrobore\//);
  assert.ok(info.protocolVersion >= 4.4, `negotiated ${info.protocolVersion}`);
  console.log(`     agent=${info.agent} protocol=${info.protocolVersion}`);
});

await check('executeQuery writes and returns typed properties', async () => {
  const { records, summary } = await driver.executeQuery(
    'CREATE (n:Conformance {marker: $marker, reading: 42, ratio: 0.5, live: true, tags: [\'a\', \'b\']}) RETURN n.reading, n.ratio, n.live, n.tags',
    { marker },
  );
  assert.equal(records.length, 1);
  const record = records[0];
  assert.deepEqual(record.keys, ['n.reading', 'n.ratio', 'n.live', 'n.tags']);
  assert.equal(record.get('n.reading'), 42);
  assert.equal(record.get('n.ratio'), 0.5);
  assert.equal(record.get('n.live'), true);
  assert.deepEqual(record.get('n.tags'), ['a', 'b']);
  assert.equal(summary.database.name, 'corrobore');
});

await check('nodes and relationships arrive as driver structures', async () => {
  await driver.executeQuery(
    'MATCH (a:Conformance {marker: $marker}) CREATE (a)-[r:LINKS]->(m:ConformanceTarget {marker: $marker})',
    { marker },
  );
  const { records } = await driver.executeQuery(
    'MATCH (a:Conformance {marker: $marker})-[r:LINKS]->(m:ConformanceTarget) RETURN a, r, m LIMIT 1',
    { marker },
  );
  assert.equal(records.length, 1);
  const [a, r, m] = ['a', 'r', 'm'].map((key) => records[0].get(key));
  assert.ok(neo4j.isNode(a) && neo4j.isNode(m), 'a and m are nodes');
  assert.ok(neo4j.isRelationship(r), 'r is a relationship');
  assert.deepEqual(a.labels, ['Conformance']);
  assert.equal(a.properties.marker, marker);
  assert.equal(a.properties.status, 'candidate');
  assert.equal(r.type, 'LINKS');
  assert.equal(r.startNodeElementId, a.elementId);
  assert.equal(r.endNodeElementId, m.elementId);
});

await check('managed write and read transactions', async () => {
  const session = driver.session();
  try {
    const created = await session.executeWrite(async (tx) => {
      for (let index = 0; index < 5; index += 1) {
        await tx.run('CREATE (n:ConformanceStream {marker: $marker, k: $k})', { marker, k: index });
      }
      const result = await tx.run(
        'MATCH (n:ConformanceStream {marker: $marker}) RETURN count(n)',
        { marker },
      );
      return result.records[0].get('count');
    });
    assert.equal(created, 5);
    const ordered = await session.executeRead(async (tx) => {
      const result = await tx.run(
        'MATCH (n:ConformanceStream {marker: $marker}) RETURN n.k ORDER BY n.k ASC LIMIT 10',
        { marker },
      );
      return result.records.map((record) => record.get('n.k'));
    });
    assert.deepEqual(ordered, [0, 1, 2, 3, 4]);
  } finally {
    await session.close();
  }
});

await check('a read transaction refuses a write with a client error', async () => {
  const session = driver.session({ defaultAccessMode: neo4j.session.READ });
  try {
    await assert.rejects(
      session.executeRead((tx) => tx.run('CREATE (n:ConformanceForbidden {marker: $marker})', { marker })),
      (error) => {
        assert.equal(error.code, 'Neo.ClientError.Security.Forbidden');
        return true;
      },
    );
  } finally {
    await session.close();
  }
});

await check('a syntax error is reported and the session recovers', async () => {
  const session = driver.session();
  try {
    await assert.rejects(session.run('THIS IS NOT CYPHER'), (error) => {
      assert.equal(error.code, 'Neo.ClientError.Statement.SyntaxError');
      return true;
    });
    const result = await session.run('MATCH (n:ConformanceStream {marker: $marker}) RETURN count(n)', { marker });
    assert.equal(result.records[0].get('count'), 5);
  } finally {
    await session.close();
  }
});

await check('streaming a result with a small fetch size', async () => {
  const session = driver.session({ fetchSize: 2 });
  try {
    const result = await session.run(
      'MATCH (n:ConformanceStream {marker: $marker}) RETURN n.k ORDER BY n.k ASC LIMIT 10',
      { marker },
    );
    assert.deepEqual(result.records.map((record) => record.get('n.k')), [0, 1, 2, 3, 4]);
  } finally {
    await session.close();
  }
});

await check('bad credentials are refused', async () => {
  const bad = neo4j.driver(url, neo4j.auth.basic('analyst', 'not-the-token'));
  try {
    await assert.rejects(bad.getServerInfo(), (error) => {
      assert.equal(error.code, 'Neo.ClientError.Security.Unauthorized');
      return true;
    });
  } finally {
    await bad.close();
  }
});

await driver.close();

const failed = results.filter((result) => !result.ok);
console.log(`\n${results.length - failed.length}/${results.length} checks passed against ${url}`);
process.exit(failed.length === 0 ? 0 : 1);

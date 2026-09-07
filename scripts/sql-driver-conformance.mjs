#!/usr/bin/env node
// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//
// Driver conformance for the SQL listener (epic #90, item #259).
//
// This script runs the maintained `pg` (node-postgres) driver against a live
// Corrobore SQL listener and checks what a PostgreSQL client observes:
// connection and password authentication, information_schema, CRUD with typed
// columns, parameterized joins through the extended protocol, transactions and
// the error path. It needs a running server and the driver package, resolved
// with CommonJS rules from the current working directory (`npm install
// --no-save pg` where you run it, or NODE_PATH). The repository itself carries
// no npm dependency.
//
//   CORROBORE_SQL_URL=postgres://analyst@127.0.0.1:5432/corrobore \
//   CORROBORE_SQL_TOKEN=<bearer token> \
//   node scripts/sql-driver-conformance.mjs

import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import path from 'node:path';

const url = process.env.CORROBORE_SQL_URL ?? 'postgres://analyst@127.0.0.1:5432/corrobore';
const token = process.env.CORROBORE_SQL_TOKEN;
if (!token) {
  console.error('CORROBORE_SQL_TOKEN is required');
  process.exit(2);
}

let pg;
try {
  const require = createRequire(path.join(process.cwd(), 'package.json'));
  pg = require('pg');
} catch (error) {
  console.error(`pg is not installed: ${error.message}`);
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
const connection = new URL(url);
const config = {
  host: connection.hostname,
  port: Number(connection.port || 5432),
  database: connection.pathname.replace(/^\//, '') || 'corrobore',
  user: connection.username || 'analyst',
  password: token,
};
const client = new pg.Client(config);

await check('connect with the bearer token as password', async () => {
  await client.connect();
  const { rows } = await client.query('SELECT version()');
  assert.match(rows[0].version, /Corrobore/);
  console.log(`     ${rows[0].version}`);
});

await check('insert, read back typed columns, update and delete', async () => {
  const label = `Conformance_${marker.replace(/-/g, '_')}`;
  const inserted = await client.query(
    `INSERT INTO "${label}" (name, reading, ratio, live, tags) VALUES ($1, $2, $3, $4, ARRAY['a', 'b'])`,
    ['north', 42, 0.5, true],
  );
  assert.equal(inserted.command, 'INSERT');
  assert.equal(inserted.rowCount, 1);
  const { rows, fields } = await client.query(
    `SELECT s.name, s.reading, s.ratio, s.live, s.tags FROM "${label}" AS s WHERE s.reading > $1 ORDER BY s.name`,
    [40],
  );
  assert.deepEqual(fields.map((field) => field.name), ['name', 'reading', 'ratio', 'live', 'tags']);
  assert.equal(rows[0].name, 'north');
  // int8 arrives as a string in node-postgres by default; the type oid says int8.
  assert.equal(fields[1].dataTypeID, 20);
  assert.equal(String(rows[0].reading), '42');
  assert.equal(rows[0].ratio, 0.5);
  assert.equal(rows[0].live, true);
  assert.deepEqual(rows[0].tags, ['a', 'b']);
  const updated = await client.query(`UPDATE "${label}" AS s SET reading = 43 WHERE s.name = $1`, ['north']);
  assert.equal(updated.rowCount, 1);
  const deleted = await client.query(`DELETE FROM "${label}" AS s WHERE s.name = $1`, ['north']);
  assert.equal(deleted.rowCount, 1);
});

await check('information_schema lists tables and columns', async () => {
  const label = `Conformance_${marker.replace(/-/g, '_')}`;
  await client.query(`INSERT INTO "${label}" (name, reading) VALUES ('south', 7)`);
  const tables = await client.query(
    'SELECT table_name FROM information_schema.tables WHERE table_schema = $1 ORDER BY table_name',
    ['public'],
  );
  assert.ok(tables.rows.some((row) => row.table_name === label));
  const columns = await client.query(
    'SELECT column_name, data_type FROM information_schema.columns WHERE table_name = $1 ORDER BY column_name',
    [label],
  );
  const byName = Object.fromEntries(columns.rows.map((row) => [row.column_name, row.data_type]));
  assert.equal(byName.id, 'text');
  assert.equal(byName.reading, 'bigint');
  assert.equal(byName.labels, 'jsonb');
});

await check('a relationship join projects endpoints and aggregates', async () => {
  const actor = `ConfActor_${marker.replace(/-/g, '_')}`;
  const malware = `ConfMalware_${marker.replace(/-/g, '_')}`;
  await client.query(`INSERT INTO "${actor}" (name) VALUES ('APT28')`);
  // Relationships are created through Cypher; the SQL frontend reads them.
  const response = await fetch(`${process.env.CORROBORE_HTTP_URL ?? 'http://127.0.0.1:8080'}/v1/cypher/write`, {
    method: 'POST',
    headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' },
    body: JSON.stringify({ query: `MATCH (a:${actor}) CREATE (a)-[r:USES]->(m:${malware} {name: 'X-Agent'})` }),
  });
  const written = await response.json();
  assert.equal(response.status, 200, JSON.stringify(written));
  assert.equal(written.result.status, 'Success', JSON.stringify(written));
  const { rows } = await client.query(
    `SELECT a.name AS actor, r.type, m.name AS malware FROM "${actor}" AS a JOIN "USES" AS r ON r.source_id = a.id JOIN "${malware}" AS m ON r.target_id = m.id`,
  );
  assert.deepEqual(rows, [{ actor: 'APT28', type: 'USES', malware: 'X-Agent' }]);
  const counted = await client.query(`SELECT count(*) AS total FROM "${actor}" AS a`);
  assert.equal(String(counted.rows[0].total), '1');
});

await check('transactions are statement groups and errors carry SQLSTATEs', async () => {
  const label = `ConfTx_${marker.replace(/-/g, '_')}`;
  await client.query('BEGIN');
  await client.query(`INSERT INTO "${label}" (k) VALUES (1)`);
  await client.query('COMMIT');
  await client.query('BEGIN READ ONLY');
  await assert.rejects(client.query(`INSERT INTO "${label}" (k) VALUES (2)`), (error) => {
    assert.equal(error.code, '42501');
    return true;
  });
  await assert.rejects(client.query('SELECT 1'), (error) => {
    assert.equal(error.code, '25P02');
    return true;
  });
  await client.query('ROLLBACK');
  await assert.rejects(client.query('SELEKT 1'), (error) => {
    assert.equal(error.code, '42601');
    return true;
  });
  await assert.rejects(client.query('WITH RECURSIVE t AS (SELECT 1) SELECT * FROM t'), (error) => {
    assert.equal(error.code, '0A000');
    return true;
  });
  const { rows } = await client.query(`SELECT count(*) AS total FROM "${label}" AS t`);
  assert.equal(String(rows[0].total), '1');
});

await check('bad credentials are refused with 28P01', async () => {
  const bad = new pg.Client({ ...config, password: 'not-the-token' });
  await assert.rejects(bad.connect(), (error) => {
    assert.equal(error.code, '28P01');
    return true;
  });
});

await client.end();

const failed = results.filter((result) => !result.ok);
console.log(`\n${results.length - failed.length}/${results.length} checks passed against ${url}`);
process.exit(failed.length === 0 ? 0 : 1);

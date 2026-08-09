// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const pluginRoot = path.join(root, 'plugins', 'corrobore');
const read = (name) => fs.readFileSync(path.join(root, name), 'utf8');

const portableManifestFields = new Set([
  '$schema',
  'name',
  'version',
  'description',
  'author',
  'homepage',
  'repository',
  'license',
  'keywords',
  'extensions',
]);

const portableSkillFields = new Set([
  'name',
  'description',
  'license',
  'compatibility',
  'metadata',
  'allowed-tools',
]);

function parseFrontmatter(markdown) {
  const match = markdown.match(/^---\n([\s\S]*?)\n---\n/);
  assert.ok(match, 'SKILL.md must start with YAML frontmatter');

  const fields = new Map();
  for (const line of match[1].split('\n')) {
    if (/^\s/.test(line) || line.trim() === '') continue;
    const field = line.match(/^([a-z][a-z0-9-]*):\s*(.*)$/);
    assert.ok(field, `unsupported frontmatter syntax: ${line}`);
    fields.set(field[1], field[2].trim().replace(/^['"]|['"]$/g, ''));
  }
  return fields;
}

function assertPathInsidePlugin(candidate) {
  const resolved = fs.realpathSync(candidate);
  const relative = path.relative(fs.realpathSync(pluginRoot), resolved);
  assert.ok(
    relative !== '..' && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative),
    `${candidate} escapes the plugin root`,
  );
}

test('portable manifest targets the closed Agent Plugins v1 schema', () => {
  const manifestPath = path.join(pluginRoot, 'plugin.json');
  assert.ok(fs.statSync(manifestPath).isFile());
  assert.equal(fs.lstatSync(manifestPath).isSymbolicLink(), false);

  const manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8'));
  assert.equal(
    manifest.$schema,
    'https://agent-plugins.org/schemas/1.0.0/plugin.schema.json',
  );
  assert.equal(manifest.name, 'corrobore');
  assert.match(manifest.name, /^(?!.*(?:--|\.\.))[a-z0-9](?:[a-z0-9.-]*[a-z0-9])?$/);
  assert.match(manifest.version, /^\d+\.\d+\.\d+$/);
  assert.equal(typeof manifest.description, 'string');
  assert.ok(manifest.description.length > 20);
  assert.deepEqual(Object.keys(manifest).filter((key) => !portableManifestFields.has(key)), []);
  assert.deepEqual(
    Object.keys(manifest.author).filter((key) => !['name', 'email', 'url'].includes(key)),
    [],
  );
  assert.equal(manifest.license, 'MIT');
  assert.equal(manifest.repository, 'https://github.com/Estance-Labs/corrobore');
});

test('plugin discovers valid Corrobore and OpenCTI Agent Skills', () => {
  const skillsRoot = path.join(pluginRoot, 'skills');
  const skillNames = fs.readdirSync(skillsRoot, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => entry.name)
    .sort();

  assert.deepEqual(skillNames, ['corrobore', 'opencti-intel-harvester']);

  for (const skillName of skillNames) {
    const skillRoot = path.join(skillsRoot, skillName);
    const skillPath = path.join(skillRoot, 'SKILL.md');
    assert.equal(fs.lstatSync(skillPath).isSymbolicLink(), false);
    assert.ok(fs.statSync(skillPath).isFile());
    assertPathInsidePlugin(skillPath);

    const markdown = fs.readFileSync(skillPath, 'utf8');
    const fields = parseFrontmatter(markdown);
    assert.equal(fields.get('name'), skillName);
    assert.match(skillName, /^(?!.*--)[a-z0-9](?:[a-z0-9-]*[a-z0-9])?$/);
    assert.ok((fields.get('description') ?? '').length >= 40);
    assert.deepEqual([...fields.keys()].filter((key) => !portableSkillFields.has(key)), []);
    assert.ok(markdown.length > 1_000, `${skillName} must contain operational instructions`);

    for (const link of markdown.matchAll(/\]\((?!https?:|#)([^)]+)\)/g)) {
      const target = path.resolve(skillRoot, link[1]);
      assert.ok(fs.existsSync(target), `${skillName} references missing file ${link[1]}`);
      assertPathInsidePlugin(target);
    }
  }
});

test('portable package has no escaping symlinks and declares a closed MCP server', () => {
  const pending = [pluginRoot];
  while (pending.length > 0) {
    const current = pending.pop();
    for (const entry of fs.readdirSync(current, { withFileTypes: true })) {
      const candidate = path.join(current, entry.name);
      assert.equal(entry.isSymbolicLink(), false, `${candidate} must not be a symlink`);
      assertPathInsidePlugin(candidate);
      if (entry.isDirectory()) pending.push(candidate);
    }
  }

  const mcpPath = path.join(pluginRoot, 'mcp.json');
  assert.ok(fs.statSync(mcpPath).isFile());
  assert.equal(fs.lstatSync(mcpPath).isSymbolicLink(), false);

  const manifest = JSON.parse(fs.readFileSync(mcpPath, 'utf8'));
  assert.deepEqual(Object.keys(manifest).sort(), ['$schema', 'mcpServers']);
  assert.equal(
    manifest.$schema,
    'https://agent-plugins.org/schemas/1.0.0/mcp.schema.json',
  );
  assert.deepEqual(Object.keys(manifest.mcpServers), ['corrobore']);

  const server = manifest.mcpServers.corrobore;
  assert.deepEqual(Object.keys(server).sort(), ['args', 'command', 'cwd', 'env', 'type']);
  assert.equal(server.type, 'stdio');
  assert.equal(server.command, 'node');
  assert.deepEqual(server.args, ['${PLUGIN_ROOT}/mcp-server/server.mjs']);
  assert.equal(server.cwd, '${PLUGIN_ROOT}');
  assert.deepEqual(server.env, {
    CORROBORE_MCP_BASE_URL: 'http://127.0.0.1:8080',
  });
  assert.doesNotMatch(JSON.stringify(manifest), /token|secret|password|authorization/i);

  const entrypoint = path.join(pluginRoot, 'mcp-server', 'server.mjs');
  assert.ok(fs.statSync(entrypoint).isFile());
  assertPathInsidePlugin(entrypoint);
});

test('public documentation installs the plugin from its canonical package', () => {
  const guide = read('docs/agent-skill.md');
  assert.match(guide, /Agent Plugins v1\.0\.0/);
  assert.match(guide, /plugins\/corrobore\/plugin\.json/);
  assert.match(guide, /agent-plugins\.org\/specification/);
  assert.match(guide, /Agent Plugin v0\.2\.0/);
  assert.match(guide, /agent-plugin-v0\.2\.0\/corrobore-agent-plugin\.zip/);
  assert.match(guide, /mcp\.json/);
  assert.match(guide, /Node\.js 20/);

  const legacyGuide = read('docs/skills/corrobore/how-to-use.md');
  assert.match(legacyGuide, /plugins\/corrobore\/skills\/corrobore\/SKILL\.md/);

  const workflow = read('.github/workflows/docs.yml');
  assert.match(workflow, /plugins\/corrobore\/\*\*/);
  assert.match(workflow, /node --test scripts\/agent-plugin-contract\.test\.mjs/);
  assert.match(workflow, /scripts\/agent-plugin-mcp\.test\.mjs/);

  const rustWorkflow = read('.github/workflows/rust-ci.yml');
  assert.match(rustWorkflow, /scripts\/agent-plugin-contract\.test\.mjs/);
  assert.match(rustWorkflow, /scripts\/agent-plugin-mcp\.test\.mjs/);
  assert.match(rustWorkflow, /plugins\/corrobore\/\*\*/);

  const releaseWorkflow = read('.github/workflows/agent-plugin-release.yml');
  assert.match(releaseWorkflow, /agent-plugin-v\*/);
  assert.match(releaseWorkflow, /git archive/);
  assert.match(releaseWorkflow, /corrobore-agent-plugin\.zip/);
  assert.match(releaseWorkflow, /gh release create/);
});

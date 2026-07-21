import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

function repositoryPath(relativePath) {
  return path.join(repoRoot, relativePath);
}

function trackedFiles() {
  return execFileSync('git', ['ls-files', '-z'], {
    cwd: repoRoot,
    encoding: 'utf8',
  })
    .split('\0')
    .filter(Boolean);
}

function repositoryFiles() {
  return execFileSync(
    'git',
    ['ls-files', '--cached', '--others', '--exclude-standard', '-z'],
    { cwd: repoRoot, encoding: 'utf8' },
  )
    .split('\0')
    .filter((file) => file && fs.existsSync(repositoryPath(file)));
}

function markdownFiles(root) {
  const files = [];
  for (const entry of fs.readdirSync(root, { withFileTypes: true })) {
    const entryPath = path.join(root, entry.name);
    if (entry.isDirectory()) files.push(...markdownFiles(entryPath));
    if (entry.isFile() && entry.name.endsWith('.md')) files.push(entryPath);
  }
  return files;
}

test('tracked repository content excludes local and generated artifacts', () => {
  const forbidden = trackedFiles().filter((file) =>
    /(^|\/)(\.env|\.DS_Store)$|(^|\/)(target|site|node_modules|\.venv)\//.test(file),
  );
  assert.deepEqual(forbidden, []);
});

test('public Markdown links resolve inside the repository', () => {
  const projectDocumentsRoot = repositoryPath('project-documents');
  const files = [
    repositoryPath('README.md'),
    repositoryPath('CONTRIBUTING.md'),
    repositoryPath('SECURITY.md'),
    ...markdownFiles(repositoryPath('docs')),
    ...(fs.existsSync(projectDocumentsRoot) ? markdownFiles(projectDocumentsRoot) : []),
  ];
  const brokenLinks = [];

  for (const file of files) {
    const content = fs.readFileSync(file, 'utf8');
    for (const match of content.matchAll(/\[[^\]]*\]\(([^)]+)\)/g)) {
      let target = match[1].trim();
      if (!target || /^(https?:|mailto:|#)/.test(target)) continue;
      target = target.split('#')[0].split('?')[0];
      try {
        target = decodeURIComponent(target);
      } catch {
        brokenLinks.push(`${path.relative(repoRoot, file)}: invalid URL encoding in ${match[1]}`);
        continue;
      }
      if (!fs.existsSync(path.resolve(path.dirname(file), target))) {
        brokenLinks.push(`${path.relative(repoRoot, file)}: ${match[1]}`);
      }
    }
  }

  assert.deepEqual(brokenLinks, []);
});

test('public repository metadata and identity are current', () => {
  for (const required of [
    'LICENSE',
    'README.md',
    'CONTRIBUTING.md',
    'SECURITY.md',
    'CODE_OF_CONDUCT.md',
  ]) {
    assert.equal(
      fs.existsSync(repositoryPath(required)) && fs.statSync(repositoryPath(required)).isFile(),
      true,
      `${required} must exist`,
    );
  }

  const textFiles = repositoryFiles().filter((file) =>
    /\.(md|rs|toml|json|ya?ml|html|mjs|js|ts|css|txt)$/.test(file)
      && file !== 'scripts/open-source-readiness.test.mjs'
      && file !== 'crates/corrobore-http-server/tests/public_docs_contract.rs',
  );
  const forbiddenIdentity = [
    'Agentic Intelligence Graph Engine',
    'AreDee-Bangs/intelligence-graph-engine',
    'AreDee-Bangs/Corrobore',
    'github.com/AreDee-Bangs',
    'areedee-bangs/corrobore',
    'Proprietary and confidential',
  ];

  const findings = [];
  for (const file of textFiles) {
    const content = fs.readFileSync(repositoryPath(file), 'utf8');
    for (const forbidden of forbiddenIdentity) {
      if (content.includes(forbidden)) {
        findings.push(`${file}: ${forbidden}`);
      }
    }
  }
  assert.deepEqual(findings, []);

  const stalePaths = [];
  for (const file of textFiles) {
    const content = fs.readFileSync(repositoryPath(file), 'utf8');
    if (content.includes('dev-docs/')) stalePaths.push(`${file}: dev-docs/`);
    if (/\/Users\/[A-Za-z0-9._-]+\//.test(content)) stalePaths.push(`${file}: local user path`);
  }
  assert.deepEqual(stalePaths, []);

  const workspace = fs.readFileSync(repositoryPath('Cargo.toml'), 'utf8');
  assert.match(workspace, /repository\s*=\s*"https:\/\/github\.com\/Noetance-Labs\/corrobore"/);

  const license = fs.readFileSync(repositoryPath('LICENSE'), 'utf8');
  assert.match(license, /Copyright \(c\) 2026 AreDee-Bangs/);

  const legalSourceFiles = repositoryFiles().filter((file) =>
    /\.(rs|py)$/.test(file),
  );
  const legacyCopyrights = legalSourceFiles.filter((file) => {
    const content = fs.readFileSync(repositoryPath(file), 'utf8');
    return content.includes('Copyright (c) 2026 Noétance.');
  });
  assert.deepEqual(legacyCopyrights, []);

  const mkdocs = fs.readFileSync(repositoryPath('mkdocs.yml'), 'utf8');
  assert.match(mkdocs, /copyright:.*AreDee-Bangs/);

  for (const crate of fs.readdirSync(repositoryPath('crates'), { withFileTypes: true })) {
    const manifestPath = `crates/${crate.name}/Cargo.toml`;
    if (!crate.isDirectory() || !fs.existsSync(repositoryPath(manifestPath))) continue;
    const manifest = fs.readFileSync(repositoryPath(manifestPath), 'utf8');
    assert.match(manifest, /repository\.workspace\s*=\s*true/, `${manifestPath} repository`);
    assert.match(manifest, /homepage\.workspace\s*=\s*true/, `${manifestPath} homepage`);
    assert.match(manifest, /documentation\.workspace\s*=\s*true/, `${manifestPath} documentation`);
  }

  const rustCi = fs.readFileSync(repositoryPath('.github/workflows/rust-ci.yml'), 'utf8');
  assert.match(rustCi, /node --test scripts\/open-source-readiness\.test\.mjs/);
  assert.match(rustCi, /domain_validation_contract_invokes_real_c_provider/);
  assert.doesNotMatch(rustCi, /cti_binary_provider_(missing_path|present)/);

  const release = fs.readFileSync(repositoryPath('.github/workflows/release.yml'), 'utf8');
  assert.match(release, /SHA256SUMS/);
  assert.doesNotMatch(release, /\.rlib/);

  const security = fs.readFileSync(repositoryPath('.github/workflows/security.yml'), 'utf8');
  assert.match(security, /fetch-depth:\s*0/);
  assert.match(security, /zricethezav\/gitleaks:v8\.30\.1/);
});
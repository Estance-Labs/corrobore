#!/usr/bin/env node

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const repoRoot = path.resolve(__dirname, '..');

function read(relativePath) {
  return fs.readFileSync(path.join(repoRoot, relativePath), 'utf8');
}

function requireText(failures, source, expected, location) {
  if (!source.includes(expected)) {
    failures.push(`${location} must contain: ${expected}`);
  }
}

function localPackageVersions(lockfile) {
  return lockfile
    .split('[[package]]')
    .slice(1)
    .filter((block) => !/^source = /m.test(block))
    .map((block) => {
      const name = block.match(/^name = "([^"]+)"/m)?.[1];
      const version = block.match(/^version = "([^"]+)"/m)?.[1];
      return { name, version };
    })
    .filter(({ name, version }) => name && version);
}

/**
 * Validate that one release version is represented consistently in Cargo,
 * public documentation, MkDocs navigation, and the root changelog.
 *
 * The implementation reads repository files from the script's parent root,
 * reports every drift together, and returns structural evidence for tests.
 */
function checkReleaseDocumentation(version) {
  const tag = `v${version}`;
  const previousTag = 'v0.2.2';
  const releaseNotePath = `docs/release-notes/${tag}.md`;
  const comparison = `https://github.com/Estance-Labs/corrobore/compare/${previousTag}...${tag}`;

  const cargoToml = read('Cargo.toml');
  const cargoLock = read('Cargo.lock');
  const readme = read('README.md');
  const docsIndex = read('docs/index.md');
  const mkdocs = read('mkdocs.yml');
  const changelog = read('CHANGELOG.md');
  const cliContract = read('crates/corrobore-http-server/tests/cli_configuration_contract.rs');
  const releaseNote = read(releaseNotePath);
  const failures = [];

  requireText(failures, cargoToml, `version = "${version}"`, 'Cargo.toml');
  requireText(failures, readme, `Workspace version: \`${version}\``, 'README.md');
  requireText(failures, docsIndex, `(release-notes/${tag}.md)`, 'docs/index.md');
  requireText(failures, cliContract, `"version":"${version}"`, 'CLI contract test');
  requireText(failures, cliContract, `version=${version}`, 'CLI contract test');
  requireText(failures, mkdocs, `- ${tag}: release-notes/${tag}.md`, 'mkdocs.yml');
  requireText(failures, changelog, `**[${tag}]**`, 'CHANGELOG.md');
  requireText(
    failures,
    changelog,
    `[Unreleased]: https://github.com/Estance-Labs/corrobore/compare/${tag}...HEAD`,
    'CHANGELOG.md',
  );
  requireText(failures, changelog, `[${tag}]: ${comparison}`, 'CHANGELOG.md');
  requireText(failures, releaseNote, `# ${tag} - `, releaseNotePath);
  requireText(failures, releaseNote, comparison, releaseNotePath);

  for (const heading of [
    '## Highlights',
    '## Contracts',
    '## Upgrade notes',
    '## Known boundaries',
    '## Validation and provenance',
  ]) {
    requireText(failures, releaseNote, heading, releaseNotePath);
  }

  const mismatchedPackages = localPackageVersions(cargoLock).filter(
    ({ version: packageVersion }) => packageVersion !== version,
  );
  if (mismatchedPackages.length) {
    failures.push(
      `Cargo.lock local packages must use ${version}: ${mismatchedPackages
        .map(({ name, version: packageVersion }) => `${name}@${packageVersion}`)
        .join(', ')}`,
    );
  }

  if (failures.length) {
    throw new Error(['release-notes-contract failed.', ...failures.map((failure) => `- ${failure}`)].join('\n'));
  }

  return {
    version,
    tag,
    localPackageCount: localPackageVersions(cargoLock).length,
    releaseNoteSectionCount: [...releaseNote.matchAll(/^## /gm)].length,
  };
}

function main() {
  const version = process.argv[2];
  if (!version) {
    throw new Error('usage: node scripts/release-notes-contract.mjs <version>');
  }
  const result = checkReleaseDocumentation(version);
  console.log(
    `release-notes-contract OK: ${result.tag}, ${result.localPackageCount} local packages, ${result.releaseNoteSectionCount} sections.`,
  );
}

if (process.argv[1] === __filename) {
  main();
}

export { checkReleaseDocumentation };

import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { createHash } from 'node:crypto';
import {
  mkdir,
  mkdtemp,
  readFile,
  rm,
  stat,
  writeFile,
} from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { promisify } from 'node:util';

import {
  DOMAIN_SPECS,
  archiveExtensionForPlatform,
  assetFileName,
  fetchDomainProviders,
  normalizeTag,
  parseSha256Sums,
} from './fetch-ee-domain-binaries.mjs';

const execFileAsync = promisify(execFile);

function sha256(content) {
  return createHash('sha256').update(content).digest('hex');
}

async function createTarArchive({
  archivePath,
  domain,
  platform,
  providerVersion,
  library,
  libraryContent,
  tamperChecksum = false,
}) {
  const stageDir = await mkdtemp(path.join(os.tmpdir(), `corrobore-provider-stage-${domain}-`));
  try {
    const releaseManifest = {
      schema_version: '1',
      provider_domain: domain,
      provider_version: providerVersion,
      artifact_suffix: platform,
      library,
    };

    const hash = sha256(libraryContent);
    const effectiveHash = tamperChecksum ? '0'.repeat(64) : hash;

    await writeFile(path.join(stageDir, library), libraryContent, 'utf8');
    await writeFile(path.join(stageDir, 'release-manifest.json'), `${JSON.stringify(releaseManifest, null, 2)}\n`, 'utf8');
    await writeFile(path.join(stageDir, 'SHA256SUMS'), `${effectiveHash}  ${library}\n`, 'utf8');

    await execFileAsync('tar', ['-czf', archivePath, '-C', stageDir, '.']);

    return hash;
  } finally {
    await rm(stageDir, { recursive: true, force: true });
  }
}

test('normalizeTag and asset naming follow release conventions', () => {
  assert.equal(normalizeTag('0.1.0'), 'v0.1.0');
  assert.equal(normalizeTag('v0.1.0'), 'v0.1.0');
  assert.equal(archiveExtensionForPlatform('windows-x64'), 'zip');
  assert.equal(archiveExtensionForPlatform('linux-x64'), 'tar.gz');

  const cti = DOMAIN_SPECS.find((spec) => spec.domain === 'cti');
  assert.ok(cti);
  assert.equal(
    assetFileName(cti, 'v0.1.0', 'linux-x64'),
    'domain-cti-provider-v0.1.0-linux-x64.tar.gz',
  );
});

test('parseSha256Sums reads valid lines and rejects invalid format', () => {
  const parsed = parseSha256Sums(
    'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  libone.so\n'
      + 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb  libtwo.so\n',
  );

  assert.equal(parsed.get('libone.so'), 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa');
  assert.equal(parsed.get('libtwo.so'), 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb');

  assert.throws(() => parseSha256Sums('not-a-hash line\n'), /invalid SHA256SUMS line/);
});

test('fetchDomainProviders installs local archives and writes runtime manifest', async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), 'corrobore-provider-fetch-test-'));
  try {
    const archivesDir = path.join(root, 'archives');
    const outputDir = path.join(root, 'output');
    const manifestFile = path.join(outputDir, 'domain-providers.json');
    const platform = 'linux-x64';
    const tag = 'v1.2.3';
    const providerVersion = '1.2.3';

    await mkdir(archivesDir, { recursive: true });

    const expectedHashes = new Map();
    for (const spec of DOMAIN_SPECS) {
      const assetName = assetFileName(spec, tag, platform);
      const library = `libcorrobore_domain_${spec.domain}.so`;
      const libraryContent = `${spec.domain}-provider-${providerVersion}`;

      const hash = await createTarArchive({
        archivePath: path.join(archivesDir, assetName),
        domain: spec.domain,
        platform,
        providerVersion,
        library,
        libraryContent,
      });
      expectedHashes.set(spec.domain, { hash, library, libraryContent });
    }

    const result = await fetchDomainProviders({
      version: tag,
      platform,
      owner: 'Estance-Labs',
      downloadMode: 'local',
      localArchiveDir: archivesDir,
      outputDir,
      manifestFile,
    });

    assert.equal(result.providers.length, 3);

    const runtimeManifest = JSON.parse(await readFile(manifestFile, 'utf8'));
    assert.equal(runtimeManifest.schema_version, '1');
    assert.equal(runtimeManifest.providers.length, 3);

    for (const provider of runtimeManifest.providers) {
      const expected = expectedHashes.get(provider.domain);
      assert.ok(expected);
      assert.equal(provider.library, expected.library);
      assert.equal(provider.sha256, expected.hash);
      assert.equal(provider.required, true);
      assert.deepEqual(provider.capabilities, [{ name: 'node.validate', version: '1' }]);

      const copiedLibraryPath = path.join(outputDir, provider.library);
      const metadata = await stat(copiedLibraryPath);
      assert.equal(metadata.isFile(), true);
      const copiedBytes = await readFile(copiedLibraryPath, 'utf8');
      assert.equal(copiedBytes, expected.libraryContent);
    }
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('fetchDomainProviders fails closed on checksum mismatch', async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), 'corrobore-provider-fetch-fail-'));
  try {
    const archivesDir = path.join(root, 'archives');
    const outputDir = path.join(root, 'output');

    await mkdir(archivesDir, { recursive: true });

    for (const spec of DOMAIN_SPECS) {
      const assetName = assetFileName(spec, 'v2.0.0', 'linux-x64');
      await createTarArchive({
        archivePath: path.join(archivesDir, assetName),
        domain: spec.domain,
        platform: 'linux-x64',
        providerVersion: '2.0.0',
        library: `libcorrobore_domain_${spec.domain}.so`,
        libraryContent: `${spec.domain}-content`,
        tamperChecksum: spec.domain === 'cti',
      });
    }

    await assert.rejects(
      () =>
        fetchDomainProviders({
          version: 'v2.0.0',
          platform: 'linux-x64',
          owner: 'Estance-Labs',
          downloadMode: 'local',
          localArchiveDir: archivesDir,
          outputDir,
          manifestFile: path.join(outputDir, 'domain-providers.json'),
        }),
      /SHA-256 mismatch for domain cti/,
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

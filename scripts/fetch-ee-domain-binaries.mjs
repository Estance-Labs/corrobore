import { createHash } from 'node:crypto';
import { execFile } from 'node:child_process';
import {
  copyFile,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  stat,
  writeFile,
} from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import { promisify } from 'node:util';
import { fileURLToPath, pathToFileURL } from 'node:url';

const execFileAsync = promisify(execFile);

export const DOMAIN_SPECS = Object.freeze([
  {
    domain: 'cti',
    repo: 'corrobore-domain-cti',
    assetPrefix: 'domain-cti-provider',
  },
  {
    domain: 'fimi',
    repo: 'corrobore-domain-fimi',
    assetPrefix: 'domain-fimi-provider',
  },
  {
    domain: 'crisis',
    repo: 'corrobore-domain-crisis',
    assetPrefix: 'domain-crisis-provider',
  },
]);

const REQUIRED_CAPABILITIES = Object.freeze([{ name: 'node.validate', version: '1' }]);
const SUPPORTED_DOWNLOAD_MODES = new Set(['github', 'local']);
const SUPPORTED_PLATFORMS = new Set([
  'linux-x64',
  'linux-arm64',
  'macos-arm64',
  'macos-x64',
  'windows-x64',
]);

export function normalizeTag(version) {
  const value = `${version ?? ''}`.trim();
  if (!value) {
    throw new Error('missing required --version argument');
  }
  return value.startsWith('v') ? value : `v${value}`;
}

function versionFromTag(tag) {
  return tag.replace(/^v/, '');
}

export function archiveExtensionForPlatform(platform) {
  return platform === 'windows-x64' ? 'zip' : 'tar.gz';
}

export function assetFileName(domainSpec, tag, platform) {
  const extension = archiveExtensionForPlatform(platform);
  return `${domainSpec.assetPrefix}-${tag}-${platform}.${extension}`;
}

export function parseSha256Sums(raw) {
  const lines = raw
    .split(/\r?\n/u)
    .map((line) => line.trim())
    .filter(Boolean);

  if (lines.length === 0) {
    throw new Error('SHA256SUMS is empty');
  }

  const entries = new Map();
  for (const line of lines) {
    const match = line.match(/^([a-f0-9]{64})\s+\*?(.+)$/u);
    if (!match) {
      throw new Error(`invalid SHA256SUMS line: ${line}`);
    }
    const [, hash, fileName] = match;
    if (entries.has(fileName)) {
      throw new Error(`duplicate SHA256SUMS entry for ${fileName}`);
    }
    entries.set(fileName, hash);
  }

  return entries;
}

function parseReleaseManifest(raw, expectedDomain, expectedPlatform, expectedVersion) {
  let parsed;
  try {
    parsed = JSON.parse(raw);
  } catch (error) {
    throw new Error(`invalid release-manifest.json: ${error.message}`);
  }

  if (parsed.schema_version !== '1') {
    throw new Error(`unsupported release-manifest schema: ${parsed.schema_version}`);
  }
  if (parsed.provider_domain !== expectedDomain) {
    throw new Error(
      `release-manifest provider_domain mismatch for ${expectedDomain}: ${parsed.provider_domain}`,
    );
  }
  if (parsed.artifact_suffix !== expectedPlatform) {
    throw new Error(
      `release-manifest artifact_suffix mismatch for ${expectedDomain}: ${parsed.artifact_suffix}`,
    );
  }
  if (parsed.provider_version !== expectedVersion) {
    throw new Error(
      `release-manifest provider_version mismatch for ${expectedDomain}: ${parsed.provider_version}`,
    );
  }
  if (typeof parsed.library !== 'string' || parsed.library.trim() === '') {
    throw new Error(`release-manifest library is missing for ${expectedDomain}`);
  }

  const library = parsed.library.trim();
  if (path.basename(library) !== library) {
    throw new Error(`release-manifest library must be a file name for ${expectedDomain}`);
  }

  return {
    library,
  };
}

async function computeFileSha256(filePath) {
  const bytes = await readFile(filePath);
  return createHash('sha256').update(bytes).digest('hex');
}

async function ensureFile(filePath, label) {
  let metadata;
  try {
    metadata = await stat(filePath);
  } catch (error) {
    throw new Error(`${label} not found at ${filePath}`);
  }
  if (!metadata.isFile()) {
    throw new Error(`${label} is not a file at ${filePath}`);
  }
}

async function extractArchive(archivePath, destinationDir) {
  await mkdir(destinationDir, { recursive: true });
  if (archivePath.endsWith('.zip')) {
    await execFileAsync('unzip', ['-oq', archivePath, '-d', destinationDir]);
    return;
  }
  await execFileAsync('tar', ['-xzf', archivePath, '-C', destinationDir]);
}

async function downloadAssetFromGithub({ owner, repo, tag, assetName, destinationDir }) {
  await mkdir(destinationDir, { recursive: true });

  await execFileAsync('gh', [
    'release',
    'download',
    tag,
    '--repo',
    `${owner}/${repo}`,
    '--pattern',
    assetName,
    '--dir',
    destinationDir,
    '--clobber',
  ]);

  const archivePath = path.join(destinationDir, assetName);
  await ensureFile(archivePath, `downloaded asset ${assetName}`);
  return archivePath;
}

async function resolveArchivePath({
  downloadMode,
  localArchiveDir,
  owner,
  repo,
  tag,
  assetName,
  downloadDir,
}) {
  if (downloadMode === 'local') {
    if (!localArchiveDir) {
      throw new Error('--local-archive-dir is required when --download-mode=local');
    }
    const archivePath = path.join(localArchiveDir, assetName);
    await ensureFile(archivePath, `local archive ${assetName}`);
    return archivePath;
  }

  return downloadAssetFromGithub({
    owner,
    repo,
    tag,
    assetName,
    destinationDir: downloadDir,
  });
}

function parseArgs(argv) {
  const options = {
    owner: 'Noetance-Labs',
    downloadMode: 'github',
    outputDir: path.resolve(process.cwd(), 'overrides/domain-providers'),
  };

  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    const nextValue = argv[index + 1];

    switch (argument) {
      case '--version':
        options.version = nextValue;
        index += 1;
        break;
      case '--platform':
        options.platform = nextValue;
        index += 1;
        break;
      case '--owner':
        options.owner = nextValue;
        index += 1;
        break;
      case '--output-dir':
        options.outputDir = path.resolve(process.cwd(), nextValue);
        index += 1;
        break;
      case '--manifest-file':
        options.manifestFile = path.resolve(process.cwd(), nextValue);
        index += 1;
        break;
      case '--download-mode':
        options.downloadMode = nextValue;
        index += 1;
        break;
      case '--local-archive-dir':
        options.localArchiveDir = path.resolve(process.cwd(), nextValue);
        index += 1;
        break;
      case '--help':
      case '-h':
        options.help = true;
        break;
      default:
        throw new Error(`unknown argument: ${argument}`);
    }
  }

  options.manifestFile ??= path.join(options.outputDir, 'domain-providers.json');
  return options;
}

function usage() {
  return [
    'Usage:',
    '  node scripts/fetch-ee-domain-binaries.mjs --version <vX.Y.Z> --platform <platform> [options]',
    '',
    'Required:',
    '  --version            Release tag with or without leading v (example: v0.1.0)',
    '  --platform           linux-x64 | linux-arm64 | macos-arm64 | macos-x64 | windows-x64',
    '',
    'Optional:',
    '  --owner              GitHub org or user (default: Noetance-Labs)',
    '  --output-dir         Destination directory for provider libraries',
    '  --manifest-file      Output path for domain-providers.json',
    '  --download-mode      github (default) or local',
    '  --local-archive-dir  Required when --download-mode=local',
  ].join('\n');
}

function validateOptions(options) {
  const tag = normalizeTag(options.version);

  if (!SUPPORTED_PLATFORMS.has(options.platform)) {
    throw new Error(`unsupported --platform value: ${options.platform}`);
  }
  if (!SUPPORTED_DOWNLOAD_MODES.has(options.downloadMode)) {
    throw new Error(`unsupported --download-mode value: ${options.downloadMode}`);
  }

  return {
    ...options,
    tag,
    providerVersion: versionFromTag(tag),
  };
}

export async function fetchDomainProviders(rawOptions) {
  const options = validateOptions(rawOptions);
  const workspaceTmp = await mkdtemp(path.join(os.tmpdir(), 'corrobore-provider-fetch-'));

  try {
    await mkdir(options.outputDir, { recursive: true });
    await mkdir(path.dirname(options.manifestFile), { recursive: true });

    const providers = [];
    for (const spec of DOMAIN_SPECS) {
      const assetName = assetFileName(spec, options.tag, options.platform);
      const archivePath = await resolveArchivePath({
        downloadMode: options.downloadMode,
        localArchiveDir: options.localArchiveDir,
        owner: options.owner,
        repo: spec.repo,
        tag: options.tag,
        assetName,
        downloadDir: path.join(workspaceTmp, 'downloads'),
      });

      const extractDir = path.join(workspaceTmp, `extract-${spec.domain}`);
      await extractArchive(archivePath, extractDir);

      const releaseManifestPath = path.join(extractDir, 'release-manifest.json');
      const shaSumsPath = path.join(extractDir, 'SHA256SUMS');
      await ensureFile(releaseManifestPath, 'release-manifest.json');
      await ensureFile(shaSumsPath, 'SHA256SUMS');

      const releaseManifest = parseReleaseManifest(
        await readFile(releaseManifestPath, 'utf8'),
        spec.domain,
        options.platform,
        options.providerVersion,
      );

      const expectedHashes = parseSha256Sums(await readFile(shaSumsPath, 'utf8'));
      const expectedHash = expectedHashes.get(releaseManifest.library);
      if (!expectedHash) {
        throw new Error(
          `SHA256SUMS does not contain ${releaseManifest.library} for domain ${spec.domain}`,
        );
      }

      const extractedLibraryPath = path.join(extractDir, releaseManifest.library);
      await ensureFile(extractedLibraryPath, `${spec.domain} provider library`);

      const actualHash = await computeFileSha256(extractedLibraryPath);
      if (actualHash !== expectedHash) {
        throw new Error(
          `SHA-256 mismatch for domain ${spec.domain}: expected ${expectedHash}, got ${actualHash}`,
        );
      }

      const destinationLibraryPath = path.join(options.outputDir, releaseManifest.library);
      await copyFile(extractedLibraryPath, destinationLibraryPath);

      providers.push({
        domain: spec.domain,
        library: releaseManifest.library,
        sha256: actualHash,
        required: true,
        capabilities: REQUIRED_CAPABILITIES,
      });
    }

    const runtimeManifest = {
      schema_version: '1',
      providers,
    };

    await writeFile(`${options.manifestFile}`, `${JSON.stringify(runtimeManifest, null, 2)}\n`, 'utf8');

    return {
      manifestFile: options.manifestFile,
      outputDir: options.outputDir,
      providers,
    };
  } finally {
    await rm(workspaceTmp, { recursive: true, force: true });
  }
}

export async function main(argv = process.argv.slice(2)) {
  const parsed = parseArgs(argv);
  if (parsed.help) {
    console.log(usage());
    return;
  }

  const result = await fetchDomainProviders(parsed);
  console.log(`Installed ${result.providers.length} providers into ${result.outputDir}`);
  console.log(`Wrote runtime manifest to ${result.manifestFile}`);
}

const executedFile = process.argv[1] ? pathToFileURL(path.resolve(process.argv[1])).href : null;
if (executedFile === import.meta.url) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.message : `${error}`);
    process.exitCode = 1;
  });
}

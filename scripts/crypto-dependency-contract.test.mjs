import test from 'node:test';
import assert from 'node:assert/strict';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

function cargoMetadata() {
  return JSON.parse(execFileSync(
    'cargo',
    ['metadata', '--format-version', '1', '--locked'],
    { cwd: repoRoot, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 },
  ));
}

function directDependencyPackageIds(metadata, dependencyName) {
  const workspaceMembers = new Set(metadata.workspace_members);
  const packageById = new Map(metadata.packages.map((pkg) => [pkg.id, pkg]));
  const ids = new Set();

  for (const node of metadata.resolve.nodes) {
    if (!workspaceMembers.has(node.id)) continue;
    for (const dependency of node.deps) {
      const pkg = packageById.get(dependency.pkg);
      if (pkg?.name === dependencyName) ids.add(pkg.id);
    }
  }

  return ids;
}

function dependencyPackageIds(metadata, packageId, dependencyName) {
  const packageById = new Map(metadata.packages.map((pkg) => [pkg.id, pkg]));
  const node = metadata.resolve.nodes.find((candidate) => candidate.id === packageId);
  assert.ok(node, `${packageId} must have a dependency-resolution node`);

  return new Set(node.deps
    .filter((dependency) => packageById.get(dependency.pkg)?.name === dependencyName)
    .map((dependency) => dependency.pkg));
}

test('workspace hmac and sha2 dependencies share one digest release', () => {
  const metadata = cargoMetadata();
  const hmacIds = directDependencyPackageIds(metadata, 'hmac');
  const sha2Ids = directDependencyPackageIds(metadata, 'sha2');

  assert.equal(hmacIds.size, 1, 'workspace crates must resolve one direct hmac release');
  assert.equal(sha2Ids.size, 1, 'workspace crates must resolve one direct sha2 release');

  const hmacDigestIds = dependencyPackageIds(metadata, [...hmacIds][0], 'digest');
  const sha2DigestIds = dependencyPackageIds(metadata, [...sha2Ids][0], 'digest');

  assert.deepEqual(
    hmacDigestIds,
    sha2DigestIds,
    'hmac and sha2 must use the same digest release so Hmac<Sha256> implements Mac',
  );
});

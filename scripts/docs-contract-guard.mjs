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

function sorted(items) {
  return [...items].sort((a, b) => a.localeCompare(b));
}

function extractEnvVarsFromConfig(source) {
  const matches = source.matchAll(/"(CORROBORE_[A-Z0-9_]+)"/g);
  const names = new Set();
  for (const match of matches) {
    names.add(match[1]);
  }

  // Keep only public runtime env vars under CORROBORE_HTTP_* and CORROBORE_STORAGE_*.
  return sorted(
    [...names].filter(
      (name) => name.startsWith('CORROBORE_HTTP_') || name.startsWith('CORROBORE_STORAGE_'),
    ),
  );
}

function extractEnvVarsFromHttpGuide(source) {
  const names = new Set();
  for (const match of source.matchAll(/`(CORROBORE_[A-Z0-9_]+)`/g)) {
    names.add(match[1]);
  }
  return sorted([...names]);
}

function extractRoutesFromApp(source) {
  const routes = new Set();
  for (const match of source.matchAll(/\.route\(\s*"([^"]+)"/g)) {
    const route = match[1];
    // /v1/{*path} is defined in web.rs as a fallback, intentionally excluded.
    routes.add(route);
  }
  return sorted([...routes]);
}

function extractPathsFromOpenApi(source) {
  const paths = new Set();
  let inPaths = false;
  for (const rawLine of source.split('\n')) {
    const line = rawLine;
    if (!inPaths && line.startsWith('paths:')) {
      inPaths = true;
      continue;
    }
    if (!inPaths) {
      continue;
    }

    const match = line.match(/^  (\/[A-Za-z0-9_\-{}\/*]+):\s*$/);
    if (match) {
      paths.add(match[1]);
    }
  }

  return sorted([...paths]);
}

/**
 * Report every repeated path key together with its one-based source lines.
 *
 * It inspects only direct children of the OpenAPI `paths`
 * mapping so nested schema properties cannot be mistaken for HTTP routes.
 */
function findDuplicateOpenApiPaths(source) {
  const occurrences = new Map();
  let inPaths = false;

  source.split('\n').forEach((line, index) => {
    if (!inPaths && line === 'paths:') {
      inPaths = true;
      return;
    }
    if (!inPaths) {
      return;
    }
    if (/^[A-Za-z]/.test(line)) {
      inPaths = false;
      return;
    }

    const match = line.match(/^  (\/[^:]+):\s*(?:#.*)?$/);
    if (!match) {
      return;
    }
    const lines = occurrences.get(match[1]) ?? [];
    lines.push(index + 1);
    occurrences.set(match[1], lines);
  });

  return sorted([...occurrences.keys()])
    .filter((path) => occurrences.get(path).length > 1)
    .map((path) => ({ path, lines: occurrences.get(path) }));
}

function extractRoutesFromHttpGuide(source) {
  const routes = new Set();
  for (const match of source.matchAll(/## `((?:GET|POST|PUT|PATCH|DELETE)\s+\/[A-Za-z0-9_\-{}\/*]+)`/g)) {
    const [method, route] = match[1].split(/\s+/);
    if (method && route) {
      routes.add(route);
    }
  }

  // Explorer read API section uses subheadings under one parent section.
  for (const match of source.matchAll(/### `((?:GET|POST|PUT|PATCH|DELETE)\s+\/[A-Za-z0-9_\-{}\/*]+)`/g)) {
    const [method, route] = match[1].split(/\s+/);
    if (method && route) {
      routes.add(route);
    }
  }

  return sorted([...routes]);
}

function diff(expected, actual) {
  const left = new Set(expected);
  const right = new Set(actual);
  const missing = sorted([...left].filter((item) => !right.has(item)));
  const extra = sorted([...right].filter((item) => !left.has(item)));
  return { missing, extra };
}

function formatDiff(header, details) {
  const lines = [header];
  if (details.missing.length) {
    lines.push('  Missing:');
    lines.push(...details.missing.map((item) => `    - ${item}`));
  }
  if (details.extra.length) {
    lines.push('  Extra:');
    lines.push(...details.extra.map((item) => `    - ${item}`));
  }
  if (!details.missing.length && !details.extra.length) {
    lines.push('  No differences.');
  }
  return lines.join('\n');
}

function runChecks() {
  const configRs = read('crates/corrobore-http-server/src/config.rs');
  const appRs = read('crates/corrobore-http-server/src/app.rs');
  const httpGuide = read('docs/user-guide/http-server.md');
  const openApi = read('docs/api/openapi.yaml');

  const configEnvVars = extractEnvVarsFromConfig(configRs);
  const documentedEnvVars = extractEnvVarsFromHttpGuide(httpGuide);

  const appRoutes = extractRoutesFromApp(appRs);
  const openApiPaths = extractPathsFromOpenApi(openApi);
  const duplicateOpenApiPaths = findDuplicateOpenApiPaths(openApi);
  const httpGuideRoutes = extractRoutesFromHttpGuide(httpGuide);

  const envDiff = diff(configEnvVars, documentedEnvVars);
  const openApiDiff = diff(appRoutes, openApiPaths);
  const guideDiff = diff(appRoutes, httpGuideRoutes);

  const failures = [];
  const hasMinimumOpenApiStructure =
    /^openapi:\s+3\.1\.\d+\s*$/m.test(openApi) &&
    /^info:\s*$/m.test(openApi) &&
    /^paths:\s*$/m.test(openApi);
  if (!hasMinimumOpenApiStructure) {
    failures.push('docs/api/openapi.yaml must declare OpenAPI 3.1, info, and paths mappings.');
  }
  if (duplicateOpenApiPaths.length) {
    failures.push(
      `Duplicate path keys in docs/api/openapi.yaml:\n${duplicateOpenApiPaths
        .map(({ path, lines }) => `  - ${path} at lines ${lines.join(', ')}`)
        .join('\n')}`,
    );
  }
  if (envDiff.missing.length) {
    failures.push(
      formatDiff('Environment variables in config.rs missing from docs/user-guide/http-server.md', envDiff),
    );
  }

  if (openApiDiff.missing.length || openApiDiff.extra.length) {
    failures.push(formatDiff('Route drift between app.rs and docs/api/openapi.yaml', openApiDiff));
  }

  if (guideDiff.missing.length) {
    failures.push(
      formatDiff('HTTP route drift between app.rs and docs/user-guide/http-server.md', guideDiff),
    );
  }

  if (failures.length) {
    const message = ['docs-contract-guard failed.', ...failures].join('\n\n');
    throw new Error(message);
  }

  return {
    envVarCount: configEnvVars.length,
    routeCount: appRoutes.length,
  };
}

function main() {
  const result = runChecks();
  console.log(
    `docs-contract-guard OK: ${result.routeCount} routes and ${result.envVarCount} env vars aligned.`,
  );
}

if (process.argv[1] === __filename) {
  main();
}

export {
  extractEnvVarsFromConfig,
  extractEnvVarsFromHttpGuide,
  extractPathsFromOpenApi,
  findDuplicateOpenApiPaths,
  extractRoutesFromApp,
  extractRoutesFromHttpGuide,
  runChecks,
};

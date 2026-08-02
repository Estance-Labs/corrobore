import test from 'node:test';
import assert from 'node:assert/strict';

import {
  extractEnvVarsFromConfig,
  extractEnvVarsFromHttpGuide,
  extractPathsFromOpenApi,
  findDuplicateOpenApiPaths,
  extractRoutesFromApp,
  extractRoutesFromHttpGuide,
  runChecks,
} from './docs-contract-guard.mjs';

test('extractRoutesFromApp reads axum route declarations', () => {
  const input = `
    Router::new()
      .route("/v1/cypher/read", post(read))
      .route("/health", get(health));
  `;
  assert.deepEqual(extractRoutesFromApp(input), ['/health', '/v1/cypher/read']);
});

test('extractPathsFromOpenApi reads paths keys', () => {
  const input = `
openapi: 3.1.0
paths:
  /health:
    get: {}
  /v1/cypher/read:
    post: {}
`;
  assert.deepEqual(extractPathsFromOpenApi(input), ['/health', '/v1/cypher/read']);
});

test('findDuplicateOpenApiPaths reports repeated path keys with every source line', () => {
  const input = `
openapi: 3.1.0
paths:
  /health:
    get: {}
  /v1/items/{item_id}:
    get: {}
  /health:
    post: {}
`;

  assert.deepEqual(findDuplicateOpenApiPaths(input), [
    { path: '/health', lines: [4, 8] },
  ]);
});

test('extractRoutesFromHttpGuide reads endpoint headings', () => {
  const input = `
## \`GET /health\`
### \`GET /v1/explorer/sessions/{session_id}/graph\`
`;
  assert.deepEqual(extractRoutesFromHttpGuide(input), [
    '/health',
    '/v1/explorer/sessions/{session_id}/graph',
  ]);
});

test('extractEnvVarsFromConfig keeps CORROBORE_HTTP and CORROBORE_STORAGE vars', () => {
  const input = `
let a = "CORROBORE_HTTP_AUTH_TOKEN";
let b = "CORROBORE_STORAGE_MODE";
let c = "OTHER_VAR";
`;
  assert.deepEqual(extractEnvVarsFromConfig(input), [
    'CORROBORE_HTTP_AUTH_TOKEN',
    'CORROBORE_STORAGE_MODE',
  ]);
});

test('extractEnvVarsFromHttpGuide reads backticked env vars', () => {
  const input = '`CORROBORE_HTTP_AUTH_TOKEN` and `CORROBORE_STORAGE_MODE`';
  assert.deepEqual(extractEnvVarsFromHttpGuide(input), [
    'CORROBORE_HTTP_AUTH_TOKEN',
    'CORROBORE_STORAGE_MODE',
  ]);
});

test('runChecks passes on current repository contracts', () => {
  assert.doesNotThrow(() => runChecks());
});

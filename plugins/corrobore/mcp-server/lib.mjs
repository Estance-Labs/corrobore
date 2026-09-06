// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

import fs from 'node:fs';

const LATEST_PROTOCOL_VERSION = '2025-06-18';
const SUPPORTED_PROTOCOL_VERSIONS = new Set([
  LATEST_PROTOCOL_VERSION,
  '2025-03-26',
  '2024-11-05',
]);
const SERVER_INFO = Object.freeze({ name: 'corrobore-portable-mcp', version: '0.2.0' });
const EMPTY_OBJECT_SCHEMA = Object.freeze({ type: 'object', additionalProperties: false });
const MEMORY_OPERATIONS = new Set([
  'remember',
  'relate',
  'recall',
  'update',
  'forget',
  'consolidate',
  'trace',
]);
const IDEMPOTENCY_REQUIRED = new Set(['remember', 'relate', 'update', 'forget']);

class RpcError extends Error {
  constructor(code, message, data) {
    super(message);
    this.code = code;
    this.data = data;
  }
}

function boundedInteger(rawValue, name, defaultValue, minimum, maximum) {
  if (rawValue === undefined || rawValue === '') return defaultValue;
  if (!/^[0-9]+$/.test(rawValue)) {
    throw new Error(`${name} must be an integer between ${minimum} and ${maximum}`);
  }
  const value = Number(rawValue);
  if (!Number.isSafeInteger(value) || value < minimum || value > maximum) {
    throw new Error(`${name} must be an integer between ${minimum} and ${maximum}`);
  }
  return value;
}

function bearerToken(environment) {
  const direct = environment.CORROBORE_MCP_AUTH_TOKEN?.trim() ?? '';
  const filePath = environment.CORROBORE_MCP_AUTH_TOKEN_FILE?.trim() ?? '';
  if (direct !== '' && filePath !== '') {
    throw new Error('set only one of CORROBORE_MCP_AUTH_TOKEN and CORROBORE_MCP_AUTH_TOKEN_FILE');
  }

  let token = direct;
  if (filePath !== '') {
    const descriptor = fs.openSync(filePath, 'r');
    try {
      const stat = fs.fstatSync(descriptor);
      if (!stat.isFile()) {
        throw new Error('CORROBORE_MCP_AUTH_TOKEN_FILE must name a regular file');
      }
      const buffer = Buffer.alloc(16_385);
      const bytesRead = fs.readSync(descriptor, buffer, 0, buffer.length, 0);
      if (bytesRead > 16_384) {
        throw new Error('CORROBORE_MCP_AUTH_TOKEN_FILE must be no larger than 16384 bytes');
      }
      token = buffer.subarray(0, bytesRead).toString('utf8').trim();
    } finally {
      fs.closeSync(descriptor);
    }
  }
  if (Buffer.byteLength(token, 'utf8') > 16_384) {
    throw new Error('the Corrobore bearer token must be no larger than 16384 bytes');
  }
  if (token !== '' && /[^\x21-\x7e]/.test(token)) {
    throw new Error('the Corrobore bearer token must contain visible ASCII characters only');
  }
  return token === '' ? undefined : token;
}

/**
 * Load and validate the portable runtime boundary before stdin is consumed.
 *
 * Only HTTP(S) endpoints without embedded credentials, query strings, or
 * fragments are accepted. One optional bearer-token source is resolved, and
 * both upstream latency and incoming MCP message size are bounded.
 */
export function loadConfiguration(environment = process.env) {
  const rawBaseUrl = environment.CORROBORE_MCP_BASE_URL?.trim() || 'http://127.0.0.1:8080';
  const baseUrl = new URL(rawBaseUrl);
  if (!['http:', 'https:'].includes(baseUrl.protocol)) {
    throw new Error('CORROBORE_MCP_BASE_URL must use http or https');
  }
  if (baseUrl.username !== '' || baseUrl.password !== '') {
    throw new Error('CORROBORE_MCP_BASE_URL must not contain credentials');
  }
  if (baseUrl.search !== '' || baseUrl.hash !== '') {
    throw new Error('CORROBORE_MCP_BASE_URL must not contain a query string or fragment');
  }
  if (!baseUrl.pathname.endsWith('/')) baseUrl.pathname += '/';

  return Object.freeze({
    baseUrl,
    authToken: bearerToken(environment),
    timeoutMs: boundedInteger(
      environment.CORROBORE_MCP_TIMEOUT_MS,
      'CORROBORE_MCP_TIMEOUT_MS',
      10_000,
      50,
      60_000,
    ),
    maxMessageBytes: boundedInteger(
      environment.CORROBORE_MCP_MAX_MESSAGE_BYTES,
      'CORROBORE_MCP_MAX_MESSAGE_BYTES',
      1_048_576,
      256,
      16_777_216,
    ),
  });
}

const string = (description, extra = {}) => ({ type: 'string', description, ...extra });
const nullableString = (description) => ({ type: ['string', 'null'], description });
const stringArray = (description, extra = {}) => ({
  type: 'array', description, items: { type: 'string' }, ...extra,
});
const plainObject = (description) => ({ type: 'object', description });

const provenanceSchema = {
  type: 'array',
  description: 'Evidence or source references retained with the record.',
  items: {
    type: 'object',
    additionalProperties: false,
    required: ['source_id'],
    properties: {
      source_id: string('Stable source identity.', { minLength: 1 }),
      locator: nullableString('Optional location inside the source.'),
      observed_at: nullableString('Optional RFC 3339 observation time.'),
    },
  },
};

const contentSchema = {
  type: 'object',
  description: 'Text, properties, or text_and_properties memory content.',
  required: ['format', 'value'],
  properties: {
    format: string('Content representation.', {
      enum: ['text', 'properties', 'text_and_properties'],
    }),
    value: { description: 'A string, properties object, or combined content object.' },
  },
};

const targetSchema = {
  type: 'object',
  additionalProperties: false,
  required: ['kind', 'id'],
  properties: {
    kind: string('Target kind.', { enum: ['memory', 'relationship', 'recall', 'mutation'] }),
    id: string('Target identifier.', { minLength: 1 }),
  },
};

const memoryInputSchemas = {
  remember: {
    type: 'object', additionalProperties: false,
    required: ['kind', 'schema_version', 'content', 'provenance', 'tags'],
    properties: {
      identity_key: nullableString('Optional application identity key.'),
      kind: string('Application-owned memory kind.', { minLength: 1 }),
      schema_version: string('Application-owned content schema version.', { minLength: 1 }),
      content: contentSchema,
      provenance: provenanceSchema,
      confidence: { type: ['number', 'null'], minimum: 0, maximum: 1 },
      valid_from: nullableString('Optional RFC 3339 validity start.'),
      valid_until: nullableString('Optional RFC 3339 validity end.'),
      expires_at: nullableString('Optional RFC 3339 expiry.'),
      tags: stringArray('Application-owned tags.'),
    },
  },
  relate: {
    type: 'object', additionalProperties: false,
    required: ['source_id', 'target_id', 'kind', 'properties', 'provenance', 'lifecycle'],
    properties: {
      identity_key: nullableString('Optional application identity key.'),
      source_id: string('Visible source memory identifier.', { minLength: 1 }),
      target_id: string('Visible target memory identifier.', { minLength: 1 }),
      kind: string('Application-owned relationship kind.', { minLength: 1 }),
      properties: plainObject('Application-owned relationship properties.'),
      provenance: provenanceSchema,
      confidence: { type: ['number', 'null'], minimum: 0, maximum: 1 },
      valid_from: nullableString('Optional RFC 3339 validity start.'),
      valid_until: nullableString('Optional RFC 3339 validity end.'),
      expires_at: nullableString('Optional RFC 3339 expiry.'),
      lifecycle: string('Relationship lifecycle.', {
        enum: ['active', 'expired', 'superseded', 'tombstoned'],
      }),
    },
  },
  recall: {
    type: 'object', additionalProperties: false,
    required: ['objective', 'seed_ids', 'limits'],
    properties: {
      objective: string('Non-empty recall objective.', { minLength: 1 }),
      seed_ids: stringArray('Explicit visible seed identifiers.'),
      limits: {
        type: 'object', additionalProperties: false,
        required: ['max_items', 'max_depth', 'max_payload_bytes', 'max_cost', 'timeout_ms', 'supernode_threshold'],
        properties: {
          max_items: { type: 'integer', minimum: 1, maximum: 10_000 },
          max_depth: { type: 'integer', minimum: 1, maximum: 16 },
          max_payload_bytes: { type: 'integer', minimum: 1, maximum: 16_777_216 },
          max_cost: { type: 'integer', minimum: 1, maximum: 1_000_000 },
          timeout_ms: { type: 'integer', minimum: 1, maximum: 60_000 },
          supernode_threshold: { type: 'integer', minimum: 1 },
        },
      },
      page_token: nullableString('Opaque workspace-bound continuation token.'),
    },
  },
  update: {
    type: 'object', additionalProperties: false,
    required: ['target', 'patch'],
    properties: {
      target: targetSchema,
      expected_version: { type: ['integer', 'null'], minimum: 1 },
      patch: {
        type: 'object', additionalProperties: false,
        properties: {
          content: contentSchema,
          confidence: { type: 'number', minimum: 0, maximum: 1 },
          add_provenance: provenanceSchema,
          lifecycle: string('Updated lifecycle.', {
            enum: ['active', 'expired', 'superseded', 'tombstoned'],
          }),
          expires_at: string('RFC 3339 expiry.'),
          add_tags: stringArray('Tags to append.'),
        },
      },
    },
  },
  forget: {
    type: 'object', additionalProperties: false,
    required: ['memory_id', 'mode', 'reason'],
    properties: {
      memory_id: string('Visible memory identifier.', { minLength: 1 }),
      mode: string('Forget semantics.', { enum: ['expire', 'tombstone', 'application_delete'] }),
      expires_at: nullableString('Required by applicable expiry policy.'),
      reason: string('Auditable reason.', { minLength: 1 }),
    },
  },
  consolidate: {
    type: 'object', additionalProperties: false,
    required: ['mode', 'memory_ids', 'reason', 'preserve_disagreements'],
    properties: {
      mode: plainObject('Either propose, or apply_approved with proposal_id and approval_policy.'),
      memory_ids: stringArray('Bounded candidate memory identifiers.', { minItems: 2, maxItems: 100 }),
      canonical_id: nullableString('Optional canonical memory identifier.'),
      reason: string('Auditable reason.', { minLength: 1 }),
      preserve_disagreements: { type: 'boolean' },
    },
  },
  trace: {
    type: 'object', additionalProperties: false,
    required: ['target'],
    properties: { target: targetSchema },
  },
};

function memoryTool(operation, description, annotations) {
  return {
    name: `corrobore_${operation}`,
    description,
    inputSchema: {
      type: 'object',
      additionalProperties: false,
      required: IDEMPOTENCY_REQUIRED.has(operation)
        ? ['idempotency_key', 'input']
        : ['input'],
      properties: {
        idempotency_key: string(
          'Caller-owned idempotency key. Required by Corrobore for mutations and approved consolidation apply.',
          { minLength: 1, maxLength: 256 },
        ),
        input: memoryInputSchemas[operation],
      },
    },
    annotations: { openWorldHint: true, ...annotations },
  };
}

const TOOLS = Object.freeze([
  {
    name: 'corrobore_claim_audit',
    description: 'Read the retained Corrobore claim audit before asserting a verdict: evidence, contradictions, history and unchecked coverage. Never recomputes or mutates a verdict.',
    inputSchema: {
      type: 'object', additionalProperties: false, required: ['claim_id'],
      properties: { claim_id: { type: 'string', minLength: 1, description: 'Exact governed claim identifier.' } },
    },
    annotations: { readOnlyHint: true, destructiveHint: false, idempotentHint: true, openWorldHint: true },
  },
  {
    name: 'corrobore_ready',
    description: 'Check whether the configured Corrobore HTTP runtime is ready before using protected tools.',
    inputSchema: EMPTY_OBJECT_SCHEMA,
    annotations: { readOnlyHint: true, destructiveHint: false, idempotentHint: true, openWorldHint: true },
  },
  memoryTool('remember', 'Create or identity-upsert an evidence-bearing Corrobore memory after explicit mutation authorization.', {
    readOnlyHint: false, destructiveHint: false, idempotentHint: true,
  }),
  memoryTool('relate', 'Create or version an evidence-bearing Corrobore relationship between visible memories.', {
    readOnlyHint: false, destructiveHint: false, idempotentHint: true,
  }),
  memoryTool('recall', 'Read a bounded Corrobore working set for an explicit objective and caller-supplied limits.', {
    readOnlyHint: true, destructiveHint: false, idempotentHint: true,
  }),
  memoryTool('update', 'Apply an optimistic and auditable Corrobore memory or relationship patch.', {
    readOnlyHint: false, destructiveHint: false, idempotentHint: true,
  }),
  memoryTool('forget', 'Expire, tombstone, or apply application deletion semantics to a Corrobore memory.', {
    readOnlyHint: false, destructiveHint: true, idempotentHint: true,
  }),
  memoryTool('consolidate', 'Propose a non-destructive Corrobore consolidation or apply an explicitly approved proposal.', {
    readOnlyHint: false, destructiveHint: true, idempotentHint: true,
  }),
  memoryTool('trace', 'Read the bounded Corrobore provenance, selection, version, and policy trace for a target.', {
    readOnlyHint: true, destructiveHint: false, idempotentHint: true,
  }),
  {
    name: 'corrobore_stix_import',
    description: 'Import one STIX 2.1 bundle and optional evidence envelope into the configured Corrobore runtime.',
    inputSchema: {
      type: 'object',
      additionalProperties: false,
      required: ['bundle'],
      properties: {
        bundle: plainObject('STIX 2.1 bundle.'),
        evidence: plainObject('Optional Corrobore STIX evidence envelope v1.'),
        workspace_id: string('Trusted runtime workspace context when supported by the deployment.'),
        session_id: string('Trusted runtime session context when supported by the deployment.'),
        budget_ref: string('Trusted runtime budget context when supported by the deployment.'),
      },
    },
    annotations: { readOnlyHint: false, destructiveHint: false, idempotentHint: false, openWorldHint: true },
  },
  {
    name: 'corrobore_stix_validate',
    description: 'Validate an explicit STIX bundle or current graph CTI nodes in Corrobore; supported playbooks may persist corrections.',
    inputSchema: {
      type: 'object',
      additionalProperties: false,
      properties: {
        source: string('Validation source.', { enum: ['bundle', 'graph'], default: 'bundle' }),
        bundle: plainObject('Explicit STIX 2.1 bundle when source is bundle.'),
        workspace_id: string('Trusted runtime workspace context when supported by the deployment.'),
        session_id: string('Trusted runtime session context when supported by the deployment.'),
        budget_ref: string('Trusted runtime budget context when supported by the deployment.'),
        snapshot_id: string('Logical graph snapshot identifier.'),
      },
    },
    annotations: { readOnlyHint: false, destructiveHint: false, idempotentHint: false, openWorldHint: true },
  },
  {
    name: 'corrobore_stix_export',
    description: 'Read the deterministic CTI-scoped STIX projection from Corrobore; strict mode is the default correctness gate.',
    inputSchema: {
      type: 'object', additionalProperties: false,
      properties: {
        snapshot_id: string('Logical snapshot identity.'),
        transaction_id: string('Logical transaction identity.'),
        exporter_version: string('Caller-visible exporter identity.'),
        mode: string('Export validation mode.', { enum: ['strict', 'permissive'], default: 'strict' }),
        profile: string('Export profile.', { enum: ['stix-mvp'], default: 'stix-mvp' }),
        force: { type: 'boolean', description: 'Explicitly preserve overridable semantic findings as diagnostics.' },
      },
    },
    annotations: { readOnlyHint: true, destructiveHint: false, idempotentHint: true, openWorldHint: true },
  },
]);

/** Return the complete, deterministic tools/list contract. */
export function listTools() {
  return TOOLS;
}

function isPlainObject(value) {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) return false;
  const prototype = Object.getPrototypeOf(value);
  return prototype === Object.prototype || prototype === null;
}

function requireObject(value, label) {
  if (!isPlainObject(value)) throw new RpcError(-32602, `${label} must be an object`);
  return value;
}

function exactKeys(value, allowed, label) {
  const unknown = Object.keys(value).filter((key) => !allowed.has(key));
  if (unknown.length > 0) {
    throw new RpcError(-32602, `${label} contains unsupported fields: ${unknown.join(', ')}`);
  }
}

function toolRequest(name, rawArguments) {
  const args = requireObject(rawArguments ?? {}, 'tool arguments');
  // The claim audit bridge validates a single claim ID and forwards only a GET;
  // return the direct upstream payload without interpreting its verdict.
  if (name === 'corrobore_claim_audit') {
    exactKeys(args, new Set(['claim_id']), 'corrobore_claim_audit arguments');
    if (typeof args.claim_id !== 'string' || args.claim_id.trim().length === 0) {
      throw new RpcError(-32602, 'claim_id must be a nonempty string');
    }
    return { method: 'GET', path: `v1/claims/${encodeURIComponent(args.claim_id)}/audit` };
  }
  if (name === 'corrobore_ready') {
    exactKeys(args, new Set(), 'corrobore_ready arguments');
    return { method: 'GET', path: 'health/ready' };
  }
  if (name.startsWith('corrobore_')) {
    const operation = name.slice('corrobore_'.length);
    if (MEMORY_OPERATIONS.has(operation)) {
      exactKeys(args, new Set(['idempotency_key', 'input']), `${name} arguments`);
      requireObject(args.input, 'input');
      if (args.idempotency_key !== undefined
        && (typeof args.idempotency_key !== 'string' || args.idempotency_key.length < 1 || args.idempotency_key.length > 256)) {
        throw new RpcError(-32602, 'idempotency_key must be a string between 1 and 256 characters');
      }
      if (IDEMPOTENCY_REQUIRED.has(operation) && args.idempotency_key === undefined) {
        throw new RpcError(-32602, `idempotency_key is required for ${operation}`);
      }
      const body = { contract_version: 'v1', operation };
      if (args.idempotency_key !== undefined) body.idempotency_key = args.idempotency_key;
      body.input = args.input;
      return { method: 'POST', path: 'v1/memory/operations', body };
    }
  }
  if (name === 'corrobore_stix_import') {
    exactKeys(args, new Set(['bundle', 'evidence', 'workspace_id', 'session_id', 'budget_ref']), `${name} arguments`);
    requireObject(args.bundle, 'bundle');
    return { method: 'POST', path: 'v1/import/stix', body: args };
  }
  if (name === 'corrobore_stix_validate') {
    exactKeys(args, new Set(['source', 'bundle', 'workspace_id', 'session_id', 'budget_ref', 'snapshot_id']), `${name} arguments`);
    if (args.source !== undefined && !['bundle', 'graph'].includes(args.source)) {
      throw new RpcError(-32602, 'source must be bundle or graph');
    }
    if (args.bundle !== undefined) requireObject(args.bundle, 'bundle');
    return { method: 'POST', path: 'v1/stix/validate', body: args };
  }
  if (name === 'corrobore_stix_export') {
    const allowed = new Set(['snapshot_id', 'transaction_id', 'exporter_version', 'mode', 'profile', 'force']);
    exactKeys(args, allowed, `${name} arguments`);
    const search = new URLSearchParams();
    for (const key of allowed) {
      if (args[key] === undefined) continue;
      if (key === 'force') {
        if (typeof args[key] !== 'boolean') throw new RpcError(-32602, 'force must be a boolean');
      } else if (typeof args[key] !== 'string') {
        throw new RpcError(-32602, `${key} must be a string`);
      }
      search.set(key, String(args[key]));
    }
    if (args.mode !== undefined && !['strict', 'permissive'].includes(args.mode)) {
      throw new RpcError(-32602, 'mode must be strict or permissive');
    }
    if (args.profile !== undefined && args.profile !== 'stix-mvp') {
      throw new RpcError(-32602, 'profile must be stix-mvp');
    }
    const query = search.toString();
    return { method: 'GET', path: `v1/export/stix${query === '' ? '' : `?${query}`}` };
  }
  throw new RpcError(-32602, `unknown tool: ${name}`);
}

async function readBoundedBody(response, maximumBytes) {
  if (response.body === null) return '';
  const reader = response.body.getReader();
  const decoder = new TextDecoder('utf-8', { fatal: true });
  let received = 0;
  let text = '';
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    received += value.byteLength;
    if (received > maximumBytes) {
      await reader.cancel();
      throw new Error(`Corrobore response exceeds the ${maximumBytes}-byte maximum`);
    }
    text += decoder.decode(value, { stream: true });
  }
  text += decoder.decode();
  return text;
}

function structuredPayload(payload) {
  return isPlainObject(payload) ? payload : { value: payload };
}

function toolResult(payload, isError = false) {
  const text = typeof payload === 'string' ? payload : JSON.stringify(payload);
  const result = {
    content: [{ type: 'text', text }],
    structuredContent: structuredPayload(payload),
  };
  if (isError) result.isError = true;
  return result;
}

async function callCorrobore(request, configuration) {
  const url = new URL(request.path, configuration.baseUrl);
  const headers = { accept: 'application/json' };
  if (configuration.authToken !== undefined) {
    headers.authorization = `Bearer ${configuration.authToken}`;
  }
  const options = { method: request.method, headers, redirect: 'error' };
  if (request.body !== undefined) {
    headers['content-type'] = 'application/json';
    options.body = JSON.stringify(request.body);
  }

  const controller = new AbortController();
  options.signal = controller.signal;
  const timeout = setTimeout(() => controller.abort(), configuration.timeoutMs);
  try {
    const response = await fetch(url, options);
    const text = await readBoundedBody(response, configuration.maxMessageBytes);
    let payload = text;
    if (text !== '') {
      try {
        payload = JSON.parse(text);
      } catch {
        payload = { body: text };
      }
    } else {
      payload = {};
    }
    if (!response.ok) {
      return toolResult({ status: response.status, error: payload }, true);
    }
    return toolResult(payload);
  } catch (error) {
    if (controller.signal.aborted) {
      return toolResult(`Corrobore request timed out after ${configuration.timeoutMs} ms`, true);
    }
    return toolResult(`Corrobore request failed: ${error.message}`, true);
  } finally {
    clearTimeout(timeout);
  }
}

function response(id, result) {
  return { jsonrpc: '2.0', id, result };
}

function errorResponse(id, error) {
  const value = {
    jsonrpc: '2.0',
    id,
    error: {
      code: error instanceof RpcError ? error.code : -32603,
      message: error instanceof Error ? error.message : 'internal error',
    },
  };
  if (error instanceof RpcError && error.data !== undefined) value.error.data = error.data;
  return value;
}

/** Validate one MCP request and produce its JSON-RPC response. */
export async function handleRequest(request, context) {
  const hasId = isPlainObject(request) && Object.hasOwn(request, 'id');
  const id = hasId && ['string', 'number'].includes(typeof request.id) ? request.id : null;
  try {
    if (!isPlainObject(request) || request.jsonrpc !== '2.0' || typeof request.method !== 'string') {
      throw new RpcError(-32600, 'invalid JSON-RPC request');
    }
    if (hasId && request.id !== null && !['string', 'number'].includes(typeof request.id)) {
      throw new RpcError(-32600, 'request id must be a string, number, or null');
    }

    let result;
    if (request.method === 'initialize') {
      const params = requireObject(request.params, 'initialize params');
      const requested = params.protocolVersion;
      if (typeof requested !== 'string') {
        throw new RpcError(-32602, 'initialize protocolVersion must be a string');
      }
      result = {
        protocolVersion: SUPPORTED_PROTOCOL_VERSIONS.has(requested) ? requested : LATEST_PROTOCOL_VERSION,
        capabilities: { tools: { listChanged: false } },
        serverInfo: SERVER_INFO,
        instructions: 'Use readiness and bounded reads first. Mutation tools require operator authorization and do not grant it.',
      };
      context.negotiated = true;
    } else if (request.method === 'notifications/initialized') {
      context.initialized = context.negotiated;
      return undefined;
    } else if (request.method === 'notifications/cancelled') {
      return undefined;
    } else if (request.method === 'ping') {
      result = {};
    } else if (request.method === 'tools/list') {
      if (!context.initialized) throw new RpcError(-32002, 'MCP client is not initialized');
      result = { tools: listTools() };
    } else if (request.method === 'tools/call') {
      if (!context.initialized) throw new RpcError(-32002, 'MCP client is not initialized');
      const params = requireObject(request.params, 'tools/call params');
      if (typeof params.name !== 'string') throw new RpcError(-32602, 'tool name must be a string');
      result = await callCorrobore(toolRequest(params.name, params.arguments), context.configuration);
    } else {
      throw new RpcError(-32601, `method not found: ${request.method}`);
    }

    if (!hasId) return undefined;
    return response(id, result);
  } catch (error) {
    if (!hasId && isPlainObject(request) && typeof request.method === 'string') return undefined;
    return errorResponse(id, error);
  }
}

function writeMessage(output, message) {
  output.write(`${JSON.stringify(message)}\n`);
}

/**
 * Own the newline-delimited stdio transport while keeping stdout protocol-only.
 */
export async function runStdioServer(options = {}) {
  const input = options.input ?? process.stdin;
  const output = options.output ?? process.stdout;
  const configuration = options.configuration ?? loadConfiguration(options.environment);
  const context = { configuration, negotiated: false, initialized: false };
  const decoder = new TextDecoder('utf-8', { fatal: true });
  let pending = Buffer.alloc(0);
  let discardingOversizedLine = false;
  let processing = Promise.resolve();

  function emitOversized() {
    writeMessage(output, errorResponse(null, new RpcError(
      -32600,
      `MCP message exceeds the ${configuration.maxMessageBytes}-byte maximum size`,
    )));
  }

  async function processLine(line) {
    if (line.length > 0 && line[line.length - 1] === 13) line = line.subarray(0, -1);
    if (line.length === 0) return;
    let request;
    try {
      request = JSON.parse(decoder.decode(line));
    } catch {
      writeMessage(output, errorResponse(null, new RpcError(-32700, 'parse error')));
      return;
    }
    const reply = await handleRequest(request, context);
    if (reply !== undefined) writeMessage(output, reply);
  }

  function consume(chunk) {
    let cursor = 0;
    while (cursor < chunk.length) {
      const newline = chunk.indexOf(10, cursor);
      if (discardingOversizedLine) {
        if (newline < 0) return;
        discardingOversizedLine = false;
        cursor = newline + 1;
        continue;
      }

      const end = newline < 0 ? chunk.length : newline;
      const segment = chunk.subarray(cursor, end);
      if (pending.length + segment.length > configuration.maxMessageBytes) {
        pending = Buffer.alloc(0);
        emitOversized();
        if (newline < 0) {
          discardingOversizedLine = true;
          return;
        }
        cursor = newline + 1;
        continue;
      }

      pending = pending.length === 0 ? Buffer.from(segment) : Buffer.concat([pending, segment]);
      if (newline < 0) return;
      const line = pending;
      pending = Buffer.alloc(0);
      processing = processing.then(() => processLine(line));
      cursor = newline + 1;
    }
  }

  await new Promise((resolve, reject) => {
    input.on('data', consume);
    input.on('end', resolve);
    input.on('error', reject);
  });
  await processing;
}

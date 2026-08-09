// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

/**
 * Load and validate the portable runtime boundary before stdin is consumed.
 *
 * The implementation will accept only HTTP(S) base URLs without embedded
 * credentials or fragments, resolve one optional bearer-token source, and
 * bound both upstream latency and incoming MCP message size.
 */
export function loadConfiguration(_environment = process.env) {
  throw new Error('loadConfiguration is not implemented');
}

/**
 * Return the complete, deterministic tools/list contract.
 *
 * Each tool will map to one documented Corrobore HTTP route. Mutation and
 * read-only hints will describe effects; they will never grant authority.
 */
export function listTools() {
  throw new Error('listTools is not implemented');
}

/**
 * Validate one MCP request and produce its JSON-RPC response.
 *
 * The implementation will negotiate the supported MCP protocol, keep
 * notifications response-free, validate tool arguments, and convert bounded
 * HTTP failures into tool results without leaking credentials.
 */
export async function handleRequest(_request, _context) {
  throw new Error('handleRequest is not implemented');
}

/**
 * Own the newline-delimited stdio transport.
 *
 * The implementation will enforce a byte limit while streaming, reserve
 * stdout exclusively for JSON-RPC messages, and continue after recoverable
 * malformed or oversized input.
 */
export async function runStdioServer(_options = {}) {
  throw new Error('runStdioServer is not implemented');
}

#!/usr/bin/env node
// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

import { runStdioServer } from './lib.mjs';

// Keep startup failures on stderr so stdout remains a valid MCP transport.
try {
  await runStdioServer();
} catch (error) {
  process.stderr.write(`Corrobore MCP startup failed: ${error.message}\n`);
  process.exitCode = 1;
}

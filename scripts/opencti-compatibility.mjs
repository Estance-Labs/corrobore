import { createHash } from "node:crypto";
import {
  existsSync,
  readFileSync,
  readdirSync,
  statSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import process from "node:process";

const DEFAULT_BUNDLE = path.join(
  "compatibility",
  "opencti",
  "7.260722.0",
);
const DATABASE_MODULE_PATTERN =
  /(?:^|\/)(?:database\/)?(?:engine|file-search)(?:\.[cm]?[jt]s)?$/;
const SOURCE_EXTENSIONS = new Set([".js", ".mjs", ".cjs", ".ts", ".mts", ".cts"]);
const EXCLUDED_SOURCE_SEGMENTS = new Set([
  "__fixtures__",
  "__mocks__",
  "__tests__",
  "generated",
  "tests",
]);

function sortObjectKeys(value) {
  if (Array.isArray(value)) {
    return value.map(sortObjectKeys);
  }
  if (value !== null && typeof value === "object") {
    return Object.fromEntries(
      Object.keys(value)
        .sort()
        .map((key) => [key, sortObjectKeys(value[key])]),
    );
  }
  return value;
}

function readJson(file) {
  return JSON.parse(readFileSync(file, "utf8"));
}

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

function sourcePosition(source, offset) {
  const preceding = source.slice(0, offset);
  const lines = preceding.split("\n");
  return {
    line: lines.length,
    column: lines.at(-1).length + 1,
  };
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function callsiteKey({ file, line, column, symbol, operation_id: operationId }) {
  return `${file}:${line}:${column} ${symbol} -> ${operationId}`;
}

function catalogueSortKey({
  file,
  line,
  column,
  symbol,
  operation_id: operationId,
}) {
  return [
    file,
    String(line).padStart(6, "0"),
    String(column).padStart(4, "0"),
    symbol,
    operationId,
  ].join(":");
}

function operationBySymbol(operations) {
  const index = new Map();
  for (const operation of operations) {
    for (const symbol of operation.symbols) {
      if (index.has(symbol)) {
        throw new Error(
          `operation symbol ${symbol} is assigned to both ${index.get(symbol)} and ${operation.id}`,
        );
      }
      index.set(symbol, operation.id);
    }
  }
  return index;
}

function parseDatabaseImports(source) {
  const imports = [];
  const importPattern =
    /import\s+(?:type\s+)?\{([\s\S]*?)\}\s+from\s+["']([^"']+)["'];?/g;
  let match;
  while ((match = importPattern.exec(source)) !== null) {
    const module = match[2].replace(/\\/g, "/");
    if (!DATABASE_MODULE_PATTERN.test(module)) {
      continue;
    }
    for (const item of match[1].split(",")) {
      const normalized = item
        .replace(/\/\*[\s\S]*?\*\//g, "")
        .replace(/\btype\s+/g, "")
        .trim();
      if (!normalized) {
        continue;
      }
      const [imported, local = imported] = normalized.split(/\s+as\s+/);
      if (imported && local) {
        imports.push({ imported: imported.trim(), local: local.trim() });
      }
    }
  }
  return imports;
}

function rawSymbols(operations) {
  return [...operationBySymbol(operations).keys()]
    .filter((symbol) => symbol.includes("."))
    .sort((left, right) => right.length - left.length);
}

function shouldIncludeSource(relativeFile) {
  const normalized = relativeFile.split(path.sep);
  if (normalized.some((segment) => EXCLUDED_SOURCE_SEGMENTS.has(segment))) {
    return false;
  }
  const basename = normalized.at(-1);
  if (/\.(?:test|spec)\.[cm]?[jt]s$/.test(basename)) {
    return false;
  }
  return SOURCE_EXTENSIONS.has(path.extname(basename));
}

function walkSourceFiles(root, current = root) {
  const files = [];
  for (const entry of readdirSync(current).sort()) {
    const absolute = path.join(current, entry);
    const relative = path.relative(root, absolute);
    const stats = statSync(absolute);
    if (stats.isDirectory()) {
      if (!EXCLUDED_SOURCE_SEGMENTS.has(entry)) {
        files.push(...walkSourceFiles(root, absolute));
      }
    } else if (shouldIncludeSource(relative)) {
      files.push(absolute);
    }
  }
  return files;
}

/**
 * Produce deterministic JSON for reviewed reference captures.
 *
 * Implementation direction:
 * - recursively order object keys;
 * - retain array order because OpenCTI ordering and pagination are parity data;
 * - terminate with one newline so generated files are stable in Git.
 */
export function canonicalJson(value) {
  return `${JSON.stringify(sortObjectKeys(value))}\n`;
}

function reviewableJson(value) {
  return `${JSON.stringify(sortObjectKeys(value), null, 2)}\n`;
}

/**
 * Discover database operation callsites in one OpenCTI source file.
 *
 * The operation definitions are the explicit compatibility boundary. The
 * scanner will match both logical engine helpers and raw Elasticsearch client
 * methods, then return source positions sorted in catalogue order.
 */
export function scanSourceText({ file, source, operations }) {
  const symbols = operationBySymbol(operations);
  const callsites = [];
  const seen = new Set();

  const addCallsite = (offset, symbol) => {
    const operationId = symbols.get(symbol);
    if (!operationId) {
      return;
    }
    const { line, column } = sourcePosition(source, offset);
    const callsite = {
      column,
      file: file.replace(/\\/g, "/"),
      line,
      operation_id: operationId,
      symbol,
    };
    const key = callsiteKey(callsite);
    if (!seen.has(key)) {
      seen.add(key);
      callsites.push(callsite);
    }
  };

  for (const { imported, local } of parseDatabaseImports(source)) {
    if (!symbols.has(imported)) {
      continue;
    }
    const pattern = new RegExp(
      `\\b${escapeRegExp(local)}\\s*(?:<[^;{}()]*>)?\\s*\\(`,
      "g",
    );
    let match;
    while ((match = pattern.exec(source)) !== null) {
      addCallsite(match.index, imported);
    }
  }

  for (const symbol of rawSymbols(operations)) {
    if (symbol.startsWith("engine.")) {
      continue;
    }
    const pattern = new RegExp(
      `\\b${escapeRegExp(symbol).replaceAll("\\.", "\\s*\\?*\\.\\s*")}\\s*\\(`,
      "g",
    );
    let match;
    while ((match = pattern.exec(source)) !== null) {
      addCallsite(match.index, symbol);
    }
  }

  const rawEnginePattern =
    /(?:\(\s*engine\s+as\s+[A-Za-z_$][\w$]*\s*\)|\bengine)\s*\.\s*([A-Za-z_$][\w$]*(?:\s*\.\s*[A-Za-z_$][\w$]*)*)\s*\(/g;
  let rawEngineMatch;
  while ((rawEngineMatch = rawEnginePattern.exec(source)) !== null) {
    const symbol = `engine.${rawEngineMatch[1].replace(/\s*\.\s*/g, ".")}`;
    addCallsite(rawEngineMatch.index, symbol);
  }

  return callsites.sort((left, right) =>
    catalogueSortKey(left).localeCompare(catalogueSortKey(right)),
  );
}

export function scanSourceTree({ sourceRoot, operations }) {
  const callsites = [];
  const unmapped = [];
  const mappedSymbols = operationBySymbol(operations);

  for (const absoluteFile of walkSourceFiles(sourceRoot)) {
    const file = path.relative(sourceRoot, absoluteFile).replace(/\\/g, "/");
    const source = readFileSync(absoluteFile, "utf8");
    callsites.push(...scanSourceText({ file, source, operations }));

    for (const { imported, local } of parseDatabaseImports(source)) {
      if (mappedSymbols.has(imported)) {
        continue;
      }
      const callPattern = new RegExp(
        `\\b${escapeRegExp(local)}\\s*(?:<[^;{}()]*>)?\\s*\\(`,
        "g",
      );
      let match;
      while ((match = callPattern.exec(source)) !== null) {
        const { line, column } = sourcePosition(source, match.index);
        unmapped.push(`${file}:${line}:${column} ${imported}`);
      }
    }

    const rawCallPattern =
      /\b(?:elClient|searchClient)\s*\??\.\s*([A-Za-z_$][\w$]*(?:\s*\??\.\s*[A-Za-z_$][\w$]*)*)\s*\(/g;
    let rawMatch;
    while ((rawMatch = rawCallPattern.exec(source)) !== null) {
      const suffix = rawMatch[1].replace(/\s*\??\.\s*/g, ".");
      const prefix = rawMatch[0].startsWith("searchClient")
        ? "searchClient"
        : "elClient";
      const symbol = `${prefix}.${suffix}`;
      if (!mappedSymbols.has(symbol)) {
        const { line, column } = sourcePosition(source, rawMatch.index);
        unmapped.push(`${file}:${line}:${column} ${symbol}`);
      }
    }

    const rawEnginePattern =
      /(?:\(\s*engine\s+as\s+[A-Za-z_$][\w$]*\s*\)|\bengine)\s*\.\s*([A-Za-z_$][\w$]*(?:\s*\.\s*[A-Za-z_$][\w$]*)*)\s*\(/g;
    let engineMatch;
    while ((engineMatch = rawEnginePattern.exec(source)) !== null) {
      const symbol = `engine.${engineMatch[1].replace(/\s*\.\s*/g, ".")}`;
      if (!mappedSymbols.has(symbol)) {
        const { line, column } = sourcePosition(source, engineMatch.index);
        unmapped.push(`${file}:${line}:${column} ${symbol}`);
      }
    }
  }

  return {
    callsites: callsites.sort((left, right) =>
      catalogueSortKey(left).localeCompare(catalogueSortKey(right)),
    ),
    unmapped: [...new Set(unmapped)].sort(),
  };
}

/**
 * Compare a fresh upstream scan with the committed catalogue.
 *
 * Validation targets:
 * - newly introduced callsites fail as missing;
 * - removed callsites fail as stale;
 * - a changed operation mapping is expressed as one missing and one stale row.
 */
export function validateCatalogueCoverage(scanned, catalogue) {
  const expected = new Map(
    scanned.map((callsite) => [callsiteKey(callsite), callsite]),
  );
  const committed = new Map(
    catalogue.callsites.map((callsite) => [callsiteKey(callsite), callsite]),
  );
  const errors = [];

  for (const [key] of expected) {
    if (!committed.has(key)) {
      errors.push(`missing catalogue callsite: ${key}`);
    }
  }
  for (const [key] of committed) {
    if (!expected.has(key)) {
      errors.push(`stale catalogue callsite: ${key}`);
    }
  }

  return errors.sort((left, right) => {
    const leftMissing = left.startsWith("missing");
    const rightMissing = right.startsWith("missing");
    if (leftMissing !== rightMissing) {
      return leftMissing ? -1 : 1;
    }
    return left.localeCompare(right);
  });
}

/**
 * Reject likely customer data, personal identifiers and credentials.
 *
 * Synthetic fixtures are restricted to IANA example domains and RFC-reserved
 * network ranges. Values are inspected recursively together with their field
 * names so an API key cannot pass merely because it lacks a familiar prefix.
 */
export function validateNoSensitiveData(value) {
  const errors = [];
  const emailPattern = /\b[A-Z0-9._%+-]+@([A-Z0-9.-]+\.[A-Z]{2,})\b/gi;
  const ipv4Pattern = /\b(?:\d{1,3}\.){3}\d{1,3}\b/g;
  const ipv6CandidatePattern = /[0-9a-f:]+/gi;
  const secretKeyPattern =
    /(?:api[_-]?key|authorization|password|passwd|secret|access[_-]?token|private[_-]?key)/i;
  const secretValuePattern =
    /(?:Bearer\s+[A-Za-z0-9._~+/-]{16,}|sk-(?:live|prod)-[A-Za-z0-9_-]{16,}|-----BEGIN [A-Z ]+PRIVATE KEY-----)/;

  const isReservedIpv4 = (address) => {
    const octets = address.split(".").map(Number);
    if (octets.some((octet) => octet < 0 || octet > 255)) {
      return false;
    }
    return (
      (octets[0] === 192 && octets[1] === 0 && octets[2] === 2) ||
      (octets[0] === 198 && octets[1] === 51 && octets[2] === 100) ||
      (octets[0] === 203 && octets[1] === 0 && octets[2] === 113)
    );
  };

  const visit = (current, pointer, key = "") => {
    if (Array.isArray(current)) {
      current.forEach((item, index) => visit(item, `${pointer}/${index}`));
      return;
    }
    if (current !== null && typeof current === "object") {
      for (const [childKey, child] of Object.entries(current)) {
        visit(child, `${pointer}/${childKey}`, childKey);
      }
      return;
    }
    if (typeof current !== "string") {
      return;
    }

    for (const match of current.matchAll(emailPattern)) {
      const domain = match[1].toLowerCase();
      if (
        domain !== "example.com" &&
        domain !== "example.net" &&
        domain !== "example.org"
      ) {
        errors.push(`${pointer}: email outside an IANA example domain`);
      }
    }
    for (const match of current.matchAll(ipv4Pattern)) {
      if (!isReservedIpv4(match[0])) {
        errors.push(`${pointer}: ipv4 outside an RFC documentation range`);
      }
    }
    for (const match of current.matchAll(ipv6CandidatePattern)) {
      const candidate = match[0];
      const colonCount = [...candidate].filter((character) => character === ":").length;
      if (colonCount < 7 && !candidate.includes("::")) {
        continue;
      }
      if (!candidate.toLowerCase().startsWith("2001:db8:")) {
        errors.push(`${pointer}: ipv6 outside an RFC documentation range`);
      }
    }
    if (
      (secretKeyPattern.test(key) &&
        current !== "synthetic-not-a-secret" &&
        current !== "not-applicable") ||
      secretValuePattern.test(current)
    ) {
      errors.push(`${pointer}: possible credential or secret`);
    }
  };

  visit(value, "$");
  return [...new Set(errors)].sort();
}

/**
 * Validate all machine-readable files as one compatibility bundle.
 *
 * This is the local and CI entry point for cross-file invariants: source lock,
 * operation metadata, callsite references, corpus/capture hashes, benchmark
 * matrix, decision accountability and privacy.
 */
export function validateBundle(bundleRoot) {
  const required = [
    "benchmark-profiles.json",
    "benchmark-results.json",
    "catalogue.json",
    "decisions.json",
    "operations.json",
    "parity-corpus.json",
    "reference-results.json",
    "source-lock.json",
  ];
  const errors = [];
  for (const file of required) {
    if (!existsSync(path.join(bundleRoot, file))) {
      errors.push(`missing bundle file: ${file}`);
    }
  }
  if (errors.length > 0) {
    return errors;
  }

  const operations = readJson(path.join(bundleRoot, "operations.json"));
  const catalogue = readJson(path.join(bundleRoot, "catalogue.json"));
  const corpus = readJson(path.join(bundleRoot, "parity-corpus.json"));
  const captures = readJson(path.join(bundleRoot, "reference-results.json"));
  const operationIds = new Set(operations.map(({ id }) => id));
  const callsiteKeys = catalogue.callsites.map(catalogueSortKey);

  if (new Set(operations.map(({ id }) => id)).size !== operations.length) {
    errors.push("operation ids are not unique");
  }
  if (new Set(callsiteKeys).size !== callsiteKeys.length) {
    errors.push("catalogue callsites are not unique");
  }
  if (callsiteKeys.join("\n") !== [...callsiteKeys].sort().join("\n")) {
    errors.push("catalogue callsites are not sorted");
  }
  for (const callsite of catalogue.callsites) {
    if (!operationIds.has(callsite.operation_id)) {
      errors.push(`unknown operation id in catalogue: ${callsite.operation_id}`);
    }
  }
  if (catalogue.summary.total_callsites !== catalogue.callsites.length) {
    errors.push("catalogue summary total_callsites is stale");
  }
  const corpusCanonical = canonicalJson(corpus);
  if (captures.corpus_sha256 !== sha256(corpusCanonical)) {
    errors.push("reference-results corpus_sha256 does not match parity-corpus.json");
  }
  errors.push(
    ...validateNoSensitiveData(corpus),
    ...validateNoSensitiveData(captures),
  );

  return [...new Set(errors)].sort();
}

function parseArgs(argv) {
  const [command, ...tokens] = argv;
  const args = {
    bundleRoot: DEFAULT_BUNDLE,
    command,
    sourceRoot: null,
  };
  for (let index = 0; index < tokens.length; index += 1) {
    const token = tokens[index];
    const value = tokens[index + 1];
    if (token === "--source" && value) {
      args.sourceRoot = value;
      index += 1;
    } else if (token === "--bundle" && value) {
      args.bundleRoot = value;
      index += 1;
    } else {
      throw new Error(`unknown or incomplete argument: ${token}`);
    }
  }
  if (!["generate", "verify"].includes(command)) {
    throw new Error(
      "usage: opencti-compatibility.mjs <generate|verify> [--source DIR] [--bundle DIR]",
    );
  }
  return args;
}

function createCatalogue(sourceRoot, bundleRoot) {
  const operations = readJson(path.join(bundleRoot, "operations.json"));
  const sourceLock = readJson(path.join(bundleRoot, "source-lock.json"));
  const scan = scanSourceTree({ sourceRoot, operations });
  if (scan.unmapped.length > 0) {
    throw new Error(
      `unmapped OpenCTI database callsites:\n${scan.unmapped.join("\n")}`,
    );
  }
  return {
    schema_version: 1,
    source_commit: sourceLock.opencti.commit,
    summary: {
      operation_definitions: operations.length,
      source_files: new Set(scan.callsites.map(({ file }) => file)).size,
      total_callsites: scan.callsites.length,
    },
    callsites: scan.callsites,
  };
}

/**
 * CLI direction:
 * `verify` validates the committed bundle and, when given an upstream source
 * root, rescans the exact pinned OpenCTI checkout for callsite drift.
 * `generate` creates a canonical catalogue from that checkout for review.
 */
async function main() {
  const { bundleRoot, command, sourceRoot } = parseArgs(
    process.argv.slice(2),
  );
  const absoluteBundle = path.resolve(bundleRoot);
  const bundleErrors = validateBundle(absoluteBundle);

  if (command === "generate") {
    if (!sourceRoot) {
      throw new Error("generate requires --source DIR");
    }
    const catalogue = createCatalogue(path.resolve(sourceRoot), absoluteBundle);
    writeFileSync(
      path.join(absoluteBundle, "catalogue.json"),
      reviewableJson(catalogue),
    );
    console.log(
      `generated ${catalogue.callsites.length} callsites from ${catalogue.summary.source_files} files`,
    );
    return;
  }

  const errors = [...bundleErrors];
  if (sourceRoot) {
    const generated = createCatalogue(path.resolve(sourceRoot), absoluteBundle);
    const committed = readJson(path.join(absoluteBundle, "catalogue.json"));
    errors.push(...validateCatalogueCoverage(generated.callsites, committed));
  }
  if (errors.length > 0) {
    throw new Error(errors.join("\n"));
  }
  console.log(
    sourceRoot
      ? "OpenCTI compatibility bundle and upstream callsites are in sync"
      : "OpenCTI compatibility bundle is valid",
  );
}

if (import.meta.url === `file://${process.argv[1]}`) {
  main().catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
}

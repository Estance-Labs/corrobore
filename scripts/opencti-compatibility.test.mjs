import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import test from "node:test";

import {
  canonicalJson,
  scanSourceText,
  validateBundle,
  validateCatalogueCoverage,
  validateNoSensitiveData,
} from "./opencti-compatibility.mjs";

const repositoryRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const bundleRoot = path.join(
  repositoryRoot,
  "compatibility",
  "opencti",
  "7.260722.0",
);

function readJson(relativePath) {
  return JSON.parse(readFileSync(path.join(bundleRoot, relativePath), "utf8"));
}

function operation(overrides = {}) {
  return {
    id: "read.search",
    symbols: ["elPaginate", "elClient.search"],
    prd_requirements: ["FR-QUERY-READ"],
    query_class: "search",
    frequency: "high",
    criticality: "critical",
    security_context: "user-and-marking-filtered",
    ordering_contract: "stable-score-then-id",
    lifecycle_phase: ["runtime"],
    delivery_issue: 38,
    input_contract: ["types", "filters", "pagination", "authorization"],
    output_contract: ["ordered_ids", "properties", "page_info"],
    failure_contract: ["invalid-query", "authorization-denied", "backend-unavailable"],
    ...overrides,
  };
}

test("scanner discovers logical and raw OpenCTI database callsites", () => {
  const source = `
import { elPaginate } from "./database/engine";

export const listThreats = async (context, user) => {
  const page = await elPaginate(context, user, ["Threat-Actor"], { first: 20 });
  return elClient.search({ index: "opencti_stix_domain_objects", body: {} });
};
`;

  assert.deepEqual(
    scanSourceText({
      file: "src/domain/threatActor.ts",
      source,
      operations: [operation()],
    }),
    [
      {
        column: 22,
        file: "src/domain/threatActor.ts",
        line: 5,
        operation_id: "read.search",
        symbol: "elPaginate",
      },
      {
        column: 10,
        file: "src/domain/threatActor.ts",
        line: 6,
        operation_id: "read.search",
        symbol: "elClient.search",
      },
    ],
  );
});

test("catalogue coverage reports missing, stale and remapped callsites", () => {
  const scanned = [
    {
      column: 3,
      file: "src/a.ts",
      line: 10,
      operation_id: "read.search",
      symbol: "elPaginate",
    },
    {
      column: 5,
      file: "src/b.ts",
      line: 20,
      operation_id: "write.bulk",
      symbol: "elBulk",
    },
  ];
  const catalogue = {
    callsites: [
      scanned[0],
      {
        column: 5,
        file: "src/b.ts",
        line: 20,
        operation_id: "write.index",
        symbol: "elBulk",
      },
      {
        column: 1,
        file: "src/removed.ts",
        line: 1,
        operation_id: "read.search",
        symbol: "elPaginate",
      },
    ],
  };

  assert.deepEqual(validateCatalogueCoverage(scanned, catalogue), [
    "missing catalogue callsite: src/b.ts:20:5 elBulk -> write.bulk",
    "stale catalogue callsite: src/b.ts:20:5 elBulk -> write.index",
    "stale catalogue callsite: src/removed.ts:1:1 elPaginate -> read.search",
  ]);
});

test("canonical reference captures sort object keys without erasing result order", () => {
  const capture = {
    properties: { zeta: 2, alpha: 1 },
    ordered_ids: ["indicator--0002", "indicator--0001"],
    aggregation: {
      buckets: [
        { count: 2, key: "malware" },
        { count: 1, key: "campaign" },
      ],
    },
  };

  assert.equal(
    canonicalJson(capture),
    '{"aggregation":{"buckets":[{"count":2,"key":"malware"},{"count":1,"key":"campaign"}]},"ordered_ids":["indicator--0002","indicator--0001"],"properties":{"alpha":1,"zeta":2}}\n',
  );
});

test("privacy guard accepts reserved synthetic values and rejects likely PII or secrets", () => {
  assert.deepEqual(
    validateNoSensitiveData({
      email: "analyst@example.com",
      ipv4: "192.0.2.12",
      ipv6: "2001:db8::12",
      token: "synthetic-not-a-secret",
    }),
    [],
  );

  const violations = validateNoSensitiveData({
    email: "alice@customer.internal",
    ipv4: "8.8.8.8",
    authorization: ["Bearer", "synthetic.token.signature"].join(" "),
    api_key: ["sk", "live", "synthetic0123456789abcdef"].join("-"),
  });

  assert.equal(violations.length, 4);
  assert.match(violations.join("\n"), /email/);
  assert.match(violations.join("\n"), /ipv4/);
  assert.match(violations.join("\n"), /authorization/);
  assert.match(violations.join("\n"), /api_key/);
});

test("source lock pins the reproducible upstream compatibility matrix", () => {
  const sourceLock = readJson("source-lock.json");

  assert.deepEqual(sourceLock.opencti, {
    repository: "https://github.com/OpenCTI-Platform/opencti.git",
    tag: "7.260722.0",
    commit: "e41adc1c3fd98a849602db33dbe550f689fe6d83",
    source_root: "opencti-platform/opencti-graphql/src",
  });
  assert.equal(sourceLock.elasticsearch.image, "docker.elastic.co/elasticsearch/elasticsearch:8.19.18");
  assert.equal(sourceLock.opensearch.image, "opensearchproject/opensearch:3.7.0");
});

test("operation definitions map every compatibility dimension required by the PRD", () => {
  const operations = readJson("operations.json");
  const requiredFields = [
    "id",
    "symbols",
    "prd_requirements",
    "query_class",
    "frequency",
    "criticality",
    "security_context",
    "ordering_contract",
    "lifecycle_phase",
    "delivery_issue",
    "input_contract",
    "output_contract",
    "failure_contract",
  ];

  assert.ok(operations.length >= 20);
  assert.equal(new Set(operations.map(({ id }) => id)).size, operations.length);
  for (const entry of operations) {
    for (const field of requiredFields) {
      assert.ok(entry[field] !== undefined, `${entry.id} is missing ${field}`);
    }
    assert.ok(
      Number.isInteger(entry.delivery_issue) &&
        entry.delivery_issue >= 39 &&
        entry.delivery_issue <= 52,
      `${entry.id} must map to a downstream delivery issue`,
    );
    assert.ok(entry.symbols.length > 0, `${entry.id} must identify upstream symbols`);
    assert.ok(entry.prd_requirements.length > 0, `${entry.id} must map to the PRD`);
    assert.ok(entry.lifecycle_phase.length > 0, `${entry.id} must identify a lifecycle phase`);
  }

  const ids = new Set(operations.map(({ id }) => id));
  const deliveryById = new Map(
    operations.map(({ id, delivery_issue: deliveryIssue }) => [id, deliveryIssue]),
  );
  assert.equal(deliveryById.get("startup.version"), 39);
  assert.equal(deliveryById.get("schema.index-management"), 52);
  assert.equal(deliveryById.get("migration.update-by-query"), 52);
  assert.equal(deliveryById.get("read.by-id"), 44);
  assert.equal(deliveryById.get("read.full-text"), 46);
  assert.equal(deliveryById.get("read.pagination"), 47);
  assert.equal(deliveryById.get("write.bulk"), 50);
  assert.equal(deliveryById.get("write.relations"), 51);
  assert.equal(deliveryById.get("file.search"), 48);
  for (const requiredId of [
    "startup.version",
    "schema.index-management",
    "migration.update-by-query",
    "read.search",
    "read.full-text",
    "read.pagination",
    "read.count",
    "read.aggregation.terms",
    "read.aggregation.date-histogram",
    "write.index",
    "write.bulk",
    "write.update-by-query",
    "delete.by-id",
    "delete.by-query",
    "file.search",
    "file.extract",
    "monitoring.health",
  ]) {
    assert.ok(ids.has(requiredId), `missing required operation ${requiredId}`);
  }
});

test("committed catalogue is sorted, unique and refers only to defined operations", () => {
  const catalogue = readJson("catalogue.json");
  const operations = readJson("operations.json");
  const operationIds = new Set(operations.map(({ id }) => id));
  const keys = catalogue.callsites.map(
    ({ file, line, column, symbol, operation_id: operationId }) =>
      `${file}:${String(line).padStart(6, "0")}:${String(column).padStart(4, "0")}:${symbol}:${operationId}`,
  );

  assert.equal(catalogue.source_commit, "e41adc1c3fd98a849602db33dbe550f689fe6d83");
  assert.equal(catalogue.summary.total_callsites, catalogue.callsites.length);
  assert.ok(catalogue.callsites.length >= 100, "catalogue is unexpectedly small");
  assert.deepEqual(keys, [...keys].sort());
  assert.equal(new Set(keys).size, keys.length);
  for (const callsite of catalogue.callsites) {
    assert.ok(operationIds.has(callsite.operation_id), `unknown operation ${callsite.operation_id}`);
  }
});

test("anonymized corpus covers parity entities and lifecycle scenarios", () => {
  const corpus = readJson("parity-corpus.json");
  const scenarioKinds = new Set(corpus.scenarios.map(({ kind }) => kind));

  for (const requiredKind of [
    "objects",
    "relationships",
    "markings",
    "organizations",
    "members",
    "tenants",
    "files",
    "merges",
    "deletes",
    "migrations",
  ]) {
    assert.ok(scenarioKinds.has(requiredKind), `missing corpus scenario ${requiredKind}`);
  }
  assert.ok(corpus.fixtures.length >= 20);
  assert.deepEqual(validateNoSensitiveData(corpus), []);
});

test("reference captures cover identity, ordering, pagination, aggregations, authorization and failures", () => {
  const captures = readJson("reference-results.json");
  const dimensions = new Set(captures.captures.flatMap(({ dimensions }) => dimensions));

  for (const dimension of [
    "ids",
    "properties",
    "ordering",
    "pagination",
    "aggregations",
    "authorization",
    "errors",
  ]) {
    assert.ok(dimensions.has(dimension), `missing reference dimension ${dimension}`);
  }

  assert.equal(captures.corpus_sha256.length, 64);
  assert.equal(captures.canonicalization, "RFC-8785-inspired-key-order-preserve-arrays");
  assert.ok(captures.captures.length >= 12);
  assert.deepEqual(validateNoSensitiveData(captures), []);
});

test("benchmark profiles and real reference measurements are reproducible", () => {
  const profiles = readJson("benchmark-profiles.json");
  const measurements = readJson("benchmark-results.json");
  const profileById = new Map(profiles.profiles.map((profile) => [profile.id, profile]));

  assert.equal(profileById.get("small").objects, 100_000);
  assert.equal(profileById.get("small").relationships, 500_000);
  assert.equal(profileById.get("medium").objects, 1_000_000);
  assert.equal(profileById.get("medium").relationships, 5_000_000);
  assert.ok(profiles.hardware.cpu);
  assert.ok(profiles.hardware.memory_bytes > 0);
  assert.ok(profiles.hardware.disk);
  assert.ok(profiles.warmup.iterations > 0);
  assert.ok(profiles.measurement.iterations > 0);

  const matrix = new Set();
  for (const result of measurements.results) {
    matrix.add(`${result.engine}:${result.profile}`);
    assert.ok(result.dataset_sha256.match(/^[a-f0-9]{64}$/));
    assert.ok(result.recorded_at.match(/^\d{4}-\d{2}-\d{2}T/));
    for (const metric of [
      "latency_p50_ms",
      "latency_p95_ms",
      "latency_p99_ms",
      "throughput_ops_per_second",
      "cpu_percent",
      "memory_bytes",
      "disk_bytes",
    ]) {
      assert.ok(Number.isFinite(result.metrics[metric]), `${result.engine}/${result.profile} ${metric}`);
      assert.ok(result.metrics[metric] > 0, `${result.engine}/${result.profile} ${metric} must be positive`);
    }
  }

  assert.deepEqual(
    [...matrix].sort(),
    [
      "elasticsearch-8.19.18:medium",
      "elasticsearch-8.19.18:small",
      "opensearch-3.7.0:medium",
      "opensearch-3.7.0:small",
    ],
  );
});

test("architecture decisions are accepted or have an accountable blocker", () => {
  const decisions = readJson("decisions.json");
  const expected = new Set([
    "adapter-location",
    "capture-method",
    "consistency-model",
    "full-text-library",
    "file-extraction",
    "snapshot-boundary",
    "aggregation-subset",
    "volume-profiles",
  ]);

  for (const decision of decisions.decisions) {
    expected.delete(decision.id);
    assert.ok(["accepted", "blocked"].includes(decision.status));
    if (decision.status === "accepted") {
      assert.ok(decision.decision);
      assert.ok(decision.rationale);
    } else {
      assert.ok(decision.blocker);
      assert.ok(decision.owner);
      assert.ok(decision.deadline.match(/^\d{4}-\d{2}-\d{2}$/));
    }
  }
  assert.deepEqual([...expected], []);
});

test("the full compatibility bundle satisfies its machine-readable contract", () => {
  assert.deepEqual(validateBundle(bundleRoot), []);
});

test("CI checks the exact upstream commit for callsite and corpus drift", () => {
  const workflow = readFileSync(
    path.join(repositoryRoot, ".github", "workflows", "opencti-compatibility.yml"),
    "utf8",
  );

  assert.match(workflow, /OpenCTI-Platform\/opencti/);
  assert.match(workflow, /e41adc1c3fd98a849602db33dbe550f689fe6d83/);
  assert.match(workflow, /node --test scripts\/opencti-compatibility\.test\.mjs/);
  assert.match(workflow, /node scripts\/opencti-compatibility\.mjs verify/);
});

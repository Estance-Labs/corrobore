import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import process from "node:process";

const INDEX = "opencti-parity-v1";
const BUNDLE = "compatibility/opencti/7.260722.0";

function parseArgs(argv) {
  const [command, ...tokens] = argv;
  const args = { command };
  for (let index = 0; index < tokens.length; index += 2) {
    const key = tokens[index];
    const value = tokens[index + 1];
    if (!key?.startsWith("--") || value === undefined) {
      throw new Error(`invalid argument near ${key ?? "<end>"}`);
    }
    args[key.slice(2).replaceAll("-", "_")] = value;
  }
  return args;
}

function percentile(values, rank) {
  const sorted = [...values].sort((left, right) => left - right);
  return sorted[Math.max(0, Math.ceil((rank / 100) * sorted.length) - 1)];
}

function round(value, digits = 3) {
  const scale = 10 ** digits;
  return Math.round(value * scale) / scale;
}

function manifestHash(profile, generatorVersion) {
  return createHash("sha256")
    .update(
      JSON.stringify({
        clock: "2026-01-15T12:00:00.000Z",
        generator_version: generatorVersion,
        objects: profile.objects,
        relationships: profile.relationships,
        seed: 38,
      }),
    )
    .digest("hex");
}

async function request(endpoint, pathname, options = {}) {
  const response = await fetch(`${endpoint}${pathname}`, {
    ...options,
    headers: {
      "content-type": "application/json",
      ...options.headers,
    },
  });
  const text = await response.text();
  const body = text ? JSON.parse(text) : null;
  if (!response.ok) {
    throw new Error(`${options.method ?? "GET"} ${pathname} failed (${response.status}): ${text}`);
  }
  return body;
}

async function waitForEngine(endpoint) {
  const deadline = Date.now() + 180_000;
  while (Date.now() < deadline) {
    try {
      const health = await request(endpoint, "/_cluster/health?wait_for_status=yellow&timeout=5s");
      if (["green", "yellow"].includes(health.status)) {
        return;
      }
    } catch {
      // The container accepts connections only after JVM bootstrap completes.
    }
    await new Promise((resolve) => setTimeout(resolve, 2_000));
  }
  throw new Error(`engine at ${endpoint} did not become ready within 180 seconds`);
}

function objectDocument(index) {
  const day = (index % 28) + 1;
  return {
    created: `2026-01-${String(day).padStart(2, "0")}T00:00:00.000Z`,
    doc_id: `object-${index}`,
    entity_type: ["indicator", "malware", "report", "identity"][index % 4],
    marking: index % 5 === 0 ? "TLP:AMBER" : "TLP:CLEAR",
    name: `Synthetic intelligence object ${index % 10_000}`,
    ordinal: index,
    tenant: `tenant-${index % 2}`,
  };
}

function relationshipDocument(index, objectCount) {
  const from = index % 100 === 0 ? 0 : index % objectCount;
  const to = (index * 17 + 1) % objectCount;
  const day = (index % 28) + 1;
  return {
    created: `2026-01-${String(day).padStart(2, "0")}T00:00:00.000Z`,
    doc_id: `relationship-${index}`,
    entity_type: "relationship",
    from_id: `object-${from}`,
    marking: index % 5 === 0 ? "TLP:AMBER" : "TLP:CLEAR",
    name: `Synthetic relationship ${index % 10_000}`,
    ordinal: objectCount + index,
    relationship_type: ["indicates", "uses", "object", "related-to"][index % 4],
    tenant: `tenant-${index % 2}`,
    to_id: `object-${to}`,
  };
}

async function createIndex(endpoint) {
  try {
    await request(endpoint, `/${INDEX}`, { method: "DELETE" });
  } catch (error) {
    if (!error.message.includes("(404)")) {
      throw error;
    }
  }
  await request(endpoint, `/${INDEX}`, {
    method: "PUT",
    body: JSON.stringify({
      mappings: {
        dynamic: "strict",
        properties: {
          created: { type: "date" },
          doc_id: { type: "keyword" },
          entity_type: { type: "keyword" },
          from_id: { type: "keyword" },
          marking: { type: "keyword" },
          name: { type: "text", fields: { keyword: { type: "keyword" } } },
          ordinal: { type: "long" },
          relationship_type: { type: "keyword" },
          tenant: { type: "keyword" },
          to_id: { type: "keyword" },
        },
      },
      settings: {
        number_of_replicas: 0,
        number_of_shards: 1,
        refresh_interval: "-1",
      },
    }),
  });
}

async function sendBulk(endpoint, actions) {
  const response = await request(endpoint, "/_bulk", {
    method: "POST",
    headers: { "content-type": "application/x-ndjson" },
    body: `${actions.join("\n")}\n`,
  });
  if (response.errors) {
    const first = response.items.find((item) => item.index?.error);
    throw new Error(`bulk ingestion failed: ${JSON.stringify(first)}`);
  }
}

async function ingest(endpoint, profile, batchSize) {
  const started = performance.now();
  const total = profile.objects + profile.relationships;
  for (let offset = 0; offset < total; offset += batchSize) {
    const actions = [];
    const end = Math.min(total, offset + batchSize);
    for (let index = offset; index < end; index += 1) {
      const isObject = index < profile.objects;
      const localIndex = isObject ? index : index - profile.objects;
      const id = isObject ? `object-${localIndex}` : `relationship-${localIndex}`;
      const document = isObject
        ? objectDocument(localIndex)
        : relationshipDocument(localIndex, profile.objects);
      actions.push(JSON.stringify({ index: { _id: id, _index: INDEX } }));
      actions.push(JSON.stringify(document));
    }
    await sendBulk(endpoint, actions);
    if (end % 100_000 === 0 || end === total) {
      console.error(`indexed ${end}/${total} documents`);
    }
  }
  await request(endpoint, `/${INDEX}/_refresh`, { method: "POST" });
  await request(endpoint, `/${INDEX}/_settings`, {
    method: "PUT",
    body: JSON.stringify({ index: { refresh_interval: "1s" } }),
  });
  const elapsedSeconds = (performance.now() - started) / 1_000;
  return {
    elapsed_seconds: round(elapsedSeconds),
    throughput_docs_per_second: round(total / elapsedSeconds),
  };
}

function workloads(profile) {
  const deepOrdinal = profile.objects + Math.floor(profile.relationships * 0.75);
  return [
    {
      id: "get-by-id",
      method: "GET",
      path: `/${INDEX}/_doc/object-${Math.floor(profile.objects / 2)}`,
    },
    {
      id: "filtered-list",
      method: "POST",
      path: `/${INDEX}/_search`,
      body: {
        query: {
          bool: {
            filter: [
              { term: { entity_type: "indicator" } },
              { term: { tenant: "tenant-0" } },
              { term: { marking: "TLP:CLEAR" } },
            ],
          },
        },
        size: 25,
        sort: [{ ordinal: "asc" }, { doc_id: "asc" }],
      },
    },
    {
      id: "full-text",
      method: "POST",
      path: `/${INDEX}/_search`,
      body: {
        query: {
          bool: {
            filter: [{ term: { tenant: "tenant-0" } }],
            must: [{ match_phrase: { name: "intelligence object 42" } }],
          },
        },
        size: 25,
      },
    },
    {
      id: "deep-pagination",
      method: "POST",
      path: `/${INDEX}/_search`,
      body: {
        query: { match_all: {} },
        search_after: [deepOrdinal, `relationship-${Math.floor(profile.relationships * 0.75)}`],
        size: 25,
        sort: [{ ordinal: "asc" }, { doc_id: "asc" }],
      },
    },
    {
      id: "terms-aggregation",
      method: "POST",
      path: `/${INDEX}/_search`,
      body: {
        aggs: { entity_types: { terms: { field: "entity_type", size: 20 } } },
        query: { bool: { filter: [{ term: { tenant: "tenant-0" } }] } },
        size: 0,
      },
    },
    {
      id: "date-histogram",
      method: "POST",
      path: `/${INDEX}/_search`,
      body: {
        aggs: {
          activity: {
            date_histogram: { field: "created", fixed_interval: "1d" },
          },
        },
        query: { bool: { filter: [{ term: { marking: "TLP:CLEAR" } }] } },
        size: 0,
      },
    },
  ];
}

async function executeWorkload(endpoint, workload) {
  const started = performance.now();
  await request(endpoint, workload.path, {
    method: workload.method,
    body: workload.body ? JSON.stringify(workload.body) : undefined,
  });
  return performance.now() - started;
}

async function nodeStats(endpoint) {
  const stats = await request(
    endpoint,
    "/_nodes/stats/process,jvm?filter_path=nodes.*.process.cpu.total_in_millis,nodes.*.jvm.mem.heap_used_in_bytes",
  );
  const node = Object.values(stats.nodes)[0];
  return {
    cpu_millis: node.process.cpu.total_in_millis,
    memory_bytes: node.jvm.mem.heap_used_in_bytes,
  };
}

async function storeBytes(endpoint) {
  const stats = await request(
    endpoint,
    `/${INDEX}/_stats/store?filter_path=_all.total.store.size_in_bytes`,
  );
  return stats._all.total.store.size_in_bytes;
}

async function measure(endpoint, profile, warmupIterations, measurementIterations, cpus) {
  const definitions = workloads(profile);
  for (let iteration = 0; iteration < warmupIterations; iteration += 1) {
    for (const workload of definitions) {
      await executeWorkload(endpoint, workload);
    }
  }

  const before = await nodeStats(endpoint);
  const started = performance.now();
  const samples = new Map(definitions.map(({ id }) => [id, []]));
  for (let iteration = 0; iteration < measurementIterations; iteration += 1) {
    for (const workload of definitions) {
      samples.get(workload.id).push(await executeWorkload(endpoint, workload));
    }
  }
  const elapsedSeconds = (performance.now() - started) / 1_000;
  const after = await nodeStats(endpoint);
  const all = [...samples.values()].flat();
  const workloadMetrics = Object.fromEntries(
    [...samples.entries()].map(([id, values]) => [
      id,
      {
        latency_p50_ms: round(percentile(values, 50)),
        latency_p95_ms: round(percentile(values, 95)),
        latency_p99_ms: round(percentile(values, 99)),
      },
    ]),
  );

  return {
    metrics: {
      cpu_percent: round(
        ((after.cpu_millis - before.cpu_millis) / (elapsedSeconds * 1_000 * cpus)) * 100,
      ),
      disk_bytes: await storeBytes(endpoint),
      latency_p50_ms: round(percentile(all, 50)),
      latency_p95_ms: round(percentile(all, 95)),
      latency_p99_ms: round(percentile(all, 99)),
      memory_bytes: after.memory_bytes,
      throughput_ops_per_second: round(all.length / elapsedSeconds),
    },
    workload_metrics: workloadMetrics,
  };
}

async function run(args) {
  for (const required of ["endpoint", "engine", "profile", "output"]) {
    if (!args[required]) {
      throw new Error(`run requires --${required.replaceAll("_", "-")}`);
    }
  }
  const profiles = JSON.parse(readFileSync(`${BUNDLE}/benchmark-profiles.json`, "utf8"));
  const profile = profiles.profiles.find(({ id }) => id === args.profile);
  if (!profile) {
    throw new Error(`unknown profile ${args.profile}`);
  }

  await waitForEngine(args.endpoint);
  await createIndex(args.endpoint);
  const ingestion = await ingest(
    args.endpoint,
    profile,
    profiles.dataset.bulk_documents,
  );
  const measured = await measure(
    args.endpoint,
    profile,
    profiles.warmup.iterations,
    profiles.measurement.iterations,
    7,
  );
  const result = {
    dataset_sha256: manifestHash(profile, profiles.generator_version),
    engine: args.engine,
    ingestion,
    metrics: measured.metrics,
    profile: profile.id,
    recorded_at: new Date().toISOString(),
    workload_metrics: measured.workload_metrics,
  };
  writeFileSync(args.output, `${JSON.stringify(result, null, 2)}\n`);
  console.log(`wrote ${args.output}`);
}

function merge(args) {
  if (!args.output || !args.inputs) {
    throw new Error("merge requires --output FILE --inputs FILE1,FILE2,...");
  }
  const results = args.inputs
    .split(",")
    .map((file) => JSON.parse(readFileSync(file, "utf8")))
    .sort((left, right) =>
      `${left.engine}:${left.profile}`.localeCompare(`${right.engine}:${right.profile}`),
    );
  writeFileSync(
    args.output,
    `${JSON.stringify(
      {
        schema_version: 1,
        methodology: "single-node-sequential-cold-index-then-warm-query",
        results,
      },
      null,
      2,
    )}\n`,
  );
}

const args = parseArgs(process.argv.slice(2));
if (args.command === "run") {
  await run(args);
} else if (args.command === "merge") {
  merge(args);
} else {
  throw new Error("usage: opencti-reference-benchmark.mjs <run|merge> ...");
}

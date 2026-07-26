import assert from "node:assert/strict";
import { chmod, mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import { execFile } from "node:child_process";
import test from "node:test";

const root = new URL("../", import.meta.url);
const execute = promisify(execFile);

async function read(path) {
  return readFile(new URL(path, root), "utf8");
}

test("the shipped OpenCTI stack is pinned, Corrobore-backed, and Elastic-free", async () => {
  const [compose, environment, manifest] = await Promise.all([
    read("packaging/opencti-elastic-free/compose.yml"),
    read("packaging/opencti-elastic-free/.env.example"),
    read("packaging/opencti-elastic-free/compatibility.json"),
  ]);

  for (const service of ["opencti:", "worker:", "corrobore:", "redis:", "rabbitmq:", "minio:", "file-worker:"]) {
    assert.ok(compose.includes(service), `Compose should ship ${service}`);
  }
  assert.ok(compose.includes("DATABASE_ENGINE: corrobore"));
  assert.ok(compose.includes("condition: service_healthy"));
  assert.ok(compose.includes("secrets:"));
  assert.ok(compose.includes("resources:"));
  assert.ok(compose.includes('"--probe-host", "corrobore"'));
  assert.ok(compose.includes("OPENCTI_CORROBORE_TIMEOUT_MS:-60000"));
  assert.ok(compose.includes("opencti-encryption-key"));
  assert.ok(compose.includes("CORROBORE_OPENCTI_RATE_LIMIT_PER_SECOND:-250"));
  assert.ok(compose.includes("CORROBORE_OPENCTI_RATE_LIMIT_BURST:-10000"));
  assert.ok(compose.includes('RABBITMQ_DEFAULT_PASS="$$(cat /run/secrets/rabbitmq-password)"'));
  assert.doesNotMatch(compose, /RABBITMQ_DEFAULT_PASS_FILE/);
  assert.doesNotMatch(compose, /^\s{2}(elasticsearch|opensearch):/m);
  assert.doesNotMatch(compose, /ELASTICSEARCH__/);
  assert.doesNotMatch(environment, /ELASTICSEARCH__/);

  const compatibility = JSON.parse(manifest);
  assert.equal(compatibility.opencti.version, "7.260722.0");
  assert.equal(compatibility.opencti.commit, "e5d780f11378d584d97261603d10157b62b36978");
  assert.equal(compatibility.opencti.upstream_commit, "e41adc1c3fd98a849602db33dbe550f689fe6d83");
  assert.equal(compatibility.opencti.repository, "https://github.com/Estance-Labs/opencti.git");
  assert.equal(compatibility.opencti.base_image, "docker.io/opencti/platform@sha256:636bbb791c512cfa4c55be3d934622c2996db7edc841e3391c9428752009a7ee");
  assert.equal(compatibility.database_engine, "corrobore");
  assert.deepEqual(compatibility.certified_profiles, ["small"]);
  assert.deepEqual(compatibility.conditional_profiles, [{
    id: "medium",
    gate: "publish and pass Corrobore measurements at 1,000,000 objects and 5,000,000 relationships",
  }]);
  assert.equal(compatibility.reference_stack.commit, "99a52e27504318303f1adffc278c87c8e150ffc9");
  assert.equal(compatibility.reference_stack.service_count, 21);
  assert.equal(compatibility.reference_stack.mandatory_configuration, 40);
  assert.equal(compatibility.reference_stack.memory_lower_bound_bytes, 1477616160);
});

test("the native OpenCTI provider is source-locked to the Estance fork", async () => {
  const [dockerfile, workflow] = await Promise.all([
    read("packaging/opencti-elastic-free/Dockerfile.opencti"),
    read(".github/workflows/opencti-elastic-free.yml"),
  ]);

  for (const expected of [
    "https://github.com/Estance-Labs/opencti.git",
    "e5d780f11378d584d97261603d10157b62b36978",
    "docker.io/opencti/platform@sha256:636bbb791c512cfa4c55be3d934622c2996db7edc841e3391c9428752009a7ee",
    "corrobore-provider-test.ts",
    "DATABASE_ENGINE",
  ]) {
    assert.ok(dockerfile.includes(expected), `OpenCTI image build should include ${expected}`);
  }
  for (const expected of [
    "https://github.com/Estance-Labs/opencti.git",
    "e5d780f11378d584d97261603d10157b62b36978",
    "corrobore-provider-test.ts",
    "yarn get-connectors-manifest",
    "yarn check-ts",
  ]) {
    assert.ok(workflow.includes(expected), `source verification should include ${expected}`);
  }
  assert.ok(
    workflow.indexOf("yarn get-connectors-manifest") < workflow.indexOf("yarn check-ts"),
    "source verification should generate the OpenCTI manifest before type-checking",
  );
});

test("OpenCTI runtime secrets include the mandatory encryption key", async () => {
  const [compose, environment, entrypoint, workflow, operations] = await Promise.all([
    read("packaging/opencti-elastic-free/compose.yml"),
    read("packaging/opencti-elastic-free/.env.example"),
    read("packaging/opencti-elastic-free/opencti-entrypoint.sh"),
    read(".github/workflows/opencti-elastic-free.yml"),
    read("docs/user-guide/opencti-elastic-free-operations.md"),
  ]);

  assert.ok(environment.includes("OPENCTI_ENCRYPTION_KEY_FILE=./secrets/opencti-encryption-key"));
  assert.ok(compose.includes("OPENCTI_ENCRYPTION_KEY_FILE:?set OPENCTI_ENCRYPTION_KEY_FILE"));
  assert.ok(entrypoint.includes("read_secret APP__ENCRYPTION_KEY /run/secrets/opencti-encryption-key"));
  assert.ok(workflow.includes("secrets/opencti-encryption-key"));
  assert.ok(workflow.includes("chmod 0700 packaging/opencti-elastic-free/secrets"));
  assert.ok(workflow.includes("chmod 0444 packaging/opencti-elastic-free/secrets/*"));
  assert.ok(operations.includes("directory with mode `0700`"));
  assert.ok(operations.includes("read-only mode `0444`"));
});

test("migration and rollback cover every reversible operating mode", async () => {
  const migration = await read("scripts/opencti-elastic-free-migrate.sh");

  for (const phase of [
    "install",
    "initial-import",
    "catch-up",
    "validate",
    "shadow",
    "canary",
    "primary-read",
    "primary-write",
    "safety-delay",
    "shutdown-elastic",
    "rollback",
  ]) {
    assert.ok(migration.includes(phase), `migration command should cover ${phase}`);
  }
  for (const safety of [
    "CORROBORE_MIGRATION_STATE_DIR",
    "CORROBORE_MIGRATION_SAFETY_DELAY_SECONDS",
    "security_divergence",
    "parity_verified",
    "replay_complete",
    "flock",
  ]) {
    assert.ok(migration.includes(safety), `migration command should enforce ${safety}`);
  }
});

test("migration and rollback execute every monotonic gate automatically", async () => {
  const temporary = await mkdtemp(join(tmpdir(), "corrobore-migration-acceptance-"));
  const bin = join(temporary, "bin");
  const state = join(temporary, "state");
  const hook = join(temporary, "hook");
  const token = join(temporary, "token");
  const certificate = join(temporary, "ca.crt");
  const bundle = join(temporary, "bundle.json");
  const catchUp = join(temporary, "catch-up.json");
  await mkdir(bin);
  await writeFile(join(bin, "docker"), "#!/usr/bin/env bash\nexit 0\n", { mode: 0o700 });
  await writeFile(join(bin, "curl"), `#!/usr/bin/env bash
case "$*" in
  *sync/status*) printf '%s' '{"ok":true,"result":{"lag":0,"queue_depth":0,"rejected_operations":0,"quarantined_operations":0,"shadow_reads_enabled":true,"divergence":"InSync"}}' ;;
  *writes/status*) printf '%s' '{"ok":true,"result":{"projection_outbox_depth":0,"projection_lag":0,"projection_quarantined":0,"fully_synchronized":true}}' ;;
  *) printf '%s' '{"ok":true}' ;;
esac
`, { mode: 0o700 });
  await writeFile(hook, "#!/usr/bin/env bash\nexit 0\n", { mode: 0o700 });
  await Promise.all([
    writeFile(token, "test-token\n"),
    writeFile(certificate, "test-ca\n"),
    writeFile(bundle, "{}\n"),
    writeFile(catchUp, "{}\n"),
  ]);
  await chmod(token, 0o600);
  const environment = {
    ...process.env,
    PATH: `${bin}:${process.env.PATH}`,
    CORROBORE_MIGRATION_STATE_DIR: state,
    CORROBORE_AUTH_TOKEN_FILE: token,
    CORROBORE_CA_FILE: certificate,
    CORROBORE_MIGRATION_SAFETY_DELAY_SECONDS: "0",
    OPENCTI_MIGRATION_FROM_REFERENCE: "true",
    OPENCTI_MIGRATION_BUNDLE: bundle,
    OPENCTI_CATCH_UP_BATCH: catchUp,
    OPENCTI_PARITY_VALIDATION_COMMAND: hook,
    OPENCTI_REFERENCE_SHUTDOWN_COMMAND: hook,
    OPENCTI_REFERENCE_RESTORE_COMMAND: hook,
  };
  const phases = [
    "install", "initial-import", "catch-up", "validate", "shadow", "canary",
    "primary-read", "primary-write", "safety-delay", "shutdown-elastic", "rollback",
  ];
  try {
    for (const phase of phases) {
      await execute("bash", ["scripts/opencti-elastic-free-migrate.sh", phase], {
        cwd: fileURLToPath(root),
        env: environment,
      });
    }
    const migration = JSON.parse(await readFile(join(state, "migration.json"), "utf8"));
    assert.deepEqual(migration.history.map(({ phase }) => phase), phases);
    const policy = JSON.parse(await readFile(join(state, "read-routing.json"), "utf8"));
    assert.equal(policy.mode, "reference_only");
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
});

test("acceptance, operations, and release automation cover the exact distribution", async () => {
  const [harness, workflow, matrix, operations, release] = await Promise.all([
    read("scripts/opencti-elastic-free-acceptance.sh"),
    read(".github/workflows/opencti-elastic-free.yml"),
    read("docs/acceptance/opencti-elastic-free.md"),
    read("docs/user-guide/opencti-elastic-free-operations.md"),
    read(".github/workflows/release.yml"),
  ]);

  for (const suite of [
    "functional",
    "dashboard",
    "export",
    "traversal",
    "search",
    "aggregation",
    "file-content",
    "bulk",
    "merge",
    "concurrent-write",
    "durability",
    "security",
    "migration",
    "operations",
    "performance-small",
    "performance-medium",
  ]) {
    assert.ok(harness.includes(suite), `acceptance harness should include ${suite}`);
    assert.ok(matrix.includes(suite), `acceptance matrix should trace ${suite}`);
  }
  for (const evidence of ["service_count", "mandatory_configuration", "memory_bytes"]) {
    assert.ok(harness.includes(evidence), `resource evidence should include ${evidence}`);
  }
  assert.ok(harness.includes("memory_reduction_lower_bound_bytes"));
  assert.ok(harness.includes("comparison"));
  assert.ok(harness.includes("OPENCTI_CORROBORE_ENV_FILE"));
  assert.ok(harness.includes("matrix-evidence.json"));
  assert.doesNotMatch(harness, /OPENCTI_UPSTREAM_ACCEPTANCE_COMMAND/);
  assert.ok(harness.includes('if type == "number" then .'));
  assert.doesNotMatch(harness, /\.MemUsage \| split/);
  for (const runtimeGate of [
    "AcceptanceRuntime",
    "DATABASE_ENGINE",
    "ELASTICSEARCH__",
    "runtime-evidence.json",
    "startup_seconds",
  ]) {
    assert.ok(harness.includes(runtimeGate), `runtime acceptance should include ${runtimeGate}`);
  }
  for (const signal of [
    "synchronization lag",
    "security divergence",
    "projection outbox",
    "snapshot",
    "restore",
    "index rebuild",
    "capacity",
  ]) {
    assert.ok(operations.includes(signal), `operations guide should cover ${signal}`);
  }
  assert.doesNotMatch(matrix, /Supplying a stub|\/opt\/opencti-tests\/run/);
  for (const expected of [
    "timeout-minutes:",
    "github.event_name == 'pull_request'",
    "opencti-elastic-free-contract.test.mjs",
    "opencti-elastic-free-acceptance.sh",
    "packaging/opencti-elastic-free/compose.yml",
    "OPENCTI_CORROBORE_ENV_FILE",
    "actions/upload-artifact@",
  ]) {
    assert.ok(workflow.includes(expected), `workflow should include ${expected}`);
  }
  assert.ok(release.includes("opencti-elastic-free"));
});

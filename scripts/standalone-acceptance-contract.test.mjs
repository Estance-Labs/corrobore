import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const root = new URL("../", import.meta.url);

async function read(path) {
  return readFile(new URL(path, root), "utf8");
}

test("standalone acceptance harness exercises the shared native and container HTTP contract", async () => {
  const harness = await read("scripts/standalone-acceptance.sh");

  for (const expected of [
    "native",
    "container",
    "/health/live",
    "/health/ready",
    "/version",
    "/v1/cypher/write",
    "/v1/cypher/read",
    "X-Request-Id",
    "SIGTERM",
    "CORROBORE_ACCEPTANCE_TIMEOUT_SECONDS",
    "CORROBORE_ACCEPTANCE_ARTIFACT_DIR",
  ]) {
    assert.ok(harness.includes(expected), `acceptance harness should include ${expected}`);
  }

  for (const expected of [
    "configuration precedence",
    "invalid configuration",
    "persistent restart",
    "exclusive ownership",
    "correlation identifier",
    "configured secret",
  ]) {
    assert.ok(harness.includes(expected), `acceptance harness should describe ${expected}`);
  }
});

test("standalone acceptance CI runs bounded native container and embedded gates", async () => {
  const workflow = await read(".github/workflows/standalone-acceptance.yml");

  for (const expected of [
    "timeout-minutes:",
    "scripts/standalone-acceptance.sh native",
    "scripts/standalone-acceptance.sh container",
    "cargo test -p corrobore-engine --locked",
    "cargo tree -p corrobore-engine",
    "actions/upload-artifact@",
    "if: failure()",
  ]) {
    assert.ok(workflow.includes(expected), `acceptance workflow should include ${expected}`);
  }

  assert.match(workflow, /pull_request:\s*\n\s*branches:\s*\n\s*- main/);
  assert.match(workflow, /workflow_dispatch:/);
});

test("epic 13 acceptance matrix traces every criterion to automated evidence", async () => {
  const matrix = await read("docs/acceptance/standalone-server.md");

  for (const criterion of [
    "independent foreground process",
    "HTTP without embedding",
    "data survives a clean server restart",
    "one server process",
    "invalid configuration",
    "readiness",
    "SIGTERM",
    "correlation identifiers",
    "native binary and a container image",
    "Embedded mode",
    "integration and acceptance tests",
  ]) {
    assert.ok(matrix.includes(criterion), `acceptance matrix should trace ${criterion}`);
  }

  for (const evidence of [
    "standalone-acceptance.sh native",
    "standalone-acceptance.sh container",
    "cli_configuration_contract",
    "persistent_ownership_contract",
    "lifecycle_contract",
    "correlation_logging_contract",
    "tls_security_contract",
    "engine_boundary_contract",
  ]) {
    assert.ok(matrix.includes(evidence), `acceptance matrix should cite ${evidence}`);
  }

  assert.ok(matrix.includes("No manual release gates remain"));
});

test("repository contracts keep the final acceptance gate wired into CI and documentation", async () => {
  const [navigation, rustCi, readiness] = await Promise.all([
    read("mkdocs.yml"),
    read(".github/workflows/rust-ci.yml"),
    read("scripts/open-source-readiness.test.mjs"),
  ]);

  assert.ok(navigation.includes("acceptance/standalone-server.md"));
  assert.ok(rustCi.includes("scripts/standalone-acceptance-contract.test.mjs"));
  assert.ok(readiness.includes("standalone-acceptance.sh"));
  assert.ok(readiness.includes("standalone-acceptance.yml"));
});

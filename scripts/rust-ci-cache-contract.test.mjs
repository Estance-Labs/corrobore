import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const workflowUrl = new URL("../.github/workflows/rust-ci.yml", import.meta.url);

async function readWorkflow() {
  return readFile(workflowUrl, "utf8");
}

function cacheStep(workflow) {
  const start = workflow.indexOf("      - name: Cache Cargo");
  assert.notEqual(start, -1, "the Rust quality job must define a Cargo cache");

  const end = workflow.indexOf("\n      - name:", start + 1);
  assert.notEqual(end, -1, "the Cargo cache must be followed by another CI step");

  return workflow.slice(start, end);
}

test("the pull-request Cargo cache contains registry data only", async () => {
  const workflow = await readWorkflow();
  const step = cacheStep(workflow);

  assert.match(step, /- name: Cache Cargo registry\n/);
  assert.match(step, /~\/\.cargo\/registry\/index\//);
  assert.match(step, /~\/\.cargo\/registry\/cache\//);
  assert.match(step, /~\/\.cargo\/git\/db\//);
  assert.doesNotMatch(step, /~\/\.cargo\/bin\//);
  assert.doesNotMatch(step, /^\s+target\/$/m);
  assert.match(
    step,
    /key: \$\{\{ runner\.os \}\}-cargo-registry-\$\{\{ hashFiles\('\*\*\/Cargo\.lock'\) \}\}/,
  );
  assert.match(step, /restore-keys: \|\n\s+\$\{\{ runner\.os \}\}-cargo-registry-/);
});

test("the cache contract and existing validation steps stay wired into CI", async () => {
  const workflow = await readWorkflow();

  for (const stepName of [
    "Check formatting",
    "Run Clippy with warnings denied",
    "Build workspace",
    "Run cargo-deny",
    "OSS edition contract - graph source gated",
    "OSS edition contract - seed profiles gated",
    "Enterprise edition contract - graph source licensed",
    "Enterprise edition contract - seed profiles licensed",
  ]) {
    assert.match(
      workflow,
      new RegExp(`- name: ${stepName.replace(/[.*+?^${}()|[\\]\\\\]/g, "\\\\$&")}`),
      `missing CI validation step: ${stepName}`,
    );
  }
});

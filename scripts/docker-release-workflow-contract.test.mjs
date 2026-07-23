import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const workflowUrl = new URL("../.github/workflows/docker.yml", import.meta.url);

async function readWorkflow() {
  return readFile(workflowUrl, "utf8");
}

function triggerSection(workflow) {
  const start = workflow.indexOf("on:\n");
  const end = workflow.indexOf("\npermissions:\n", start);

  assert.notEqual(start, -1, "the Docker workflow must define triggers");
  assert.notEqual(end, -1, "the Docker workflow triggers must precede permissions");

  return workflow.slice(start, end);
}

test("the Docker workflow runs only for version tags", async () => {
  const workflow = await readWorkflow();
  const triggers = triggerSection(workflow);

  assert.match(triggers, /^on:\n  push:\n    tags:\n      - "v\*"\n$/);
  assert.doesNotMatch(triggers, /pull_request:/);
  assert.doesNotMatch(triggers, /workflow_dispatch:/);
  assert.doesNotMatch(triggers, /branches:/);
});

test("a release-tag run always publishes the tagged image", async () => {
  const workflow = await readWorkflow();

  assert.doesNotMatch(workflow, /github\.event_name != 'pull_request'/);
  assert.match(workflow, /push: true/);
  assert.match(workflow, /type=ref,event=tag/);
  assert.doesNotMatch(workflow, /type=ref,event=branch/);
  assert.doesNotMatch(workflow, /type=sha/);
});

test("release builds do not depend on the GitHub Actions build cache", async () => {
  const workflow = await readWorkflow();

  assert.doesNotMatch(workflow, /cache-from: type=gha/);
  assert.doesNotMatch(workflow, /cache-to: type=gha/);
});

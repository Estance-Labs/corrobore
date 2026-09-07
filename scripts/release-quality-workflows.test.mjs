import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
const read = (name) => readFile(new URL(`../${name}`, import.meta.url), "utf8");
test("shared action executes the pinned core command with independently bound evidence", async () => {
  const action = await read(".github/actions/release-quality/action.yml");
  for (const text of [
    "scripts/release-quality-provider.json",
    "scripts/release-quality.mjs",
    "--candidate",
    "--baseline",
    "--release-ref",
    "--release-path",
    "--provider",
    "evidence-revision",
    "persist-credentials: false",
  ])
    assert.ok(action.includes(text), text);
  assert.ok(
    action.indexOf("Require immutable") <
      action.indexOf("path: .quality-evidence"),
  );
  assert.doesNotMatch(action, /continue-on-error|--skip/);
});
test("binary archives and Docker publication depend on the quality job", async () => {
  const release = await read(".github/workflows/release.yml"),
    docker = await read(".github/workflows/docker.yml");
  assert.match(
    release,
    /build-release-assets:[\s\S]*?needs:\s*\[unit-tests, release-quality\]/,
  );
  assert.match(release, /release-path: engine-release/);
  assert.match(
    docker,
    /build-and-publish:[\s\S]*?needs:\s*\[smoke-test, release-quality\]/,
  );
  assert.match(docker, /release-path: docker-release/);
  for (const workflow of [release, docker])
    assert.ok(workflow.includes("uses: ./.github/actions/release-quality"));
});
test("plugin archives and generated notes are gated before creating the archive", async () => {
  const workflow = await read(".github/workflows/agent-plugin-release.yml");
  const start = workflow.indexOf("  release:"),
    gate = workflow.indexOf("uses: ./.github/actions/release-quality", start),
    archive = workflow.indexOf("git archive", start);
  assert.ok(gate > start && gate < archive);
  assert.match(workflow, /release-path: agent-plugin-release/);
});
test("documentation checks unpublished notes before building or uploading pages", async () => {
  const workflow = await read(".github/workflows/docs.yml");
  assert.match(workflow, /actions: read/);
  assert.match(workflow, /fetch-depth: 0/);
  const scope = workflow.indexOf("scripts/docs-release-scope.mjs"),
    gate = workflow.indexOf("uses: ./.github/actions/release-quality"),
    build = workflow.indexOf("run: mkdocs build");
  assert.ok(scope > 0 && scope < gate && gate < build);
  assert.match(workflow, /steps\.release-scope\.outputs\.required == 'true'/);
  assert.match(workflow, /release-path: release-notes-publication/);
});
test("failed qualification remains inspectable and the hook contracts run in CI", async () => {
  const action = await read(".github/actions/release-quality/action.yml");
  assert.match(action, /if: always\(\)/);
  assert.match(action, /GITHUB_STEP_SUMMARY/);
  assert.match(action, /actions\/upload-artifact@/);
  assert.match(action, /report\.html/);
  const ci = await read(".github/workflows/rust-ci.yml");
  for (const contract of [
    "release-quality*.test.mjs",
    "release-notes-scope.test.mjs",
    "docs-publication-history.test.mjs",
  ])
    assert.ok(ci.includes(contract), contract);
});

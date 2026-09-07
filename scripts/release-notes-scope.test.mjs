import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, mkdir, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { findReleaseNoteChanges } from "./release-notes-scope.mjs";
async function fixture(context) {
  const root = await mkdtemp(path.join(tmpdir(), "release-note-scope-"));
  context.after(() => rm(root, { recursive: true, force: true }));
  const git = (...args) =>
    execFileSync("git", args, { cwd: root, encoding: "utf8" }).trim();
  git("init", "-q");
  const write = async (name, text) => {
    await mkdir(path.dirname(path.join(root, name)), { recursive: true });
    await writeFile(path.join(root, name), text);
  };
  const commit = () => {
    git("add", ".");
    git(
      "-c",
      "user.name=Test",
      "-c",
      "user.email=test@example.invalid",
      "commit",
      "-qm",
      "test: scope fixture",
    );
    return git("rev-parse", "HEAD");
  };
  await write("docs/release-notes/v1.0.0.md", "# Released");
  const baseline = commit();
  return { root, git, write, commit, baseline };
}
test("ordinary documentation changes do not require a new release qualification", async (context) => {
  const f = await fixture(context);
  await f.write("docs/guide.md", "# Guide");
  const candidate = f.commit();
  const result = await findReleaseNoteChanges({
    repositoryRoot: f.root,
    baseline: f.baseline,
    candidate,
  });
  assert.equal(result.required, false);
  assert.deepEqual(result.files, []);
});
test("new and changed versioned notes remain gated across later ordinary commits", async (context) => {
  const f = await fixture(context);
  await f.write("docs/release-notes/v1.1.0.md", "# New release");
  await f.write("docs/release-notes/v1.0.0.md", "# Revised release");
  f.commit();
  await f.write("docs/guide.md", "# Later guide");
  const candidate = f.commit();
  const result = await findReleaseNoteChanges({
    repositoryRoot: f.root,
    baseline: f.baseline,
    candidate,
  });
  assert.equal(result.required, true);
  assert.deepEqual(result.files, [
    "docs/release-notes/v1.0.0.md",
    "docs/release-notes/v1.1.0.md",
  ]);
});
test("renamed versioned pages require qualification and deleted pages are not new release notes", async (context) => {
  const f = await fixture(context);
  f.git("mv", "docs/release-notes/v1.0.0.md", "docs/release-notes/v1.0.1.md");
  const renamed = f.commit();
  assert.deepEqual(
    (
      await findReleaseNoteChanges({
        repositoryRoot: f.root,
        baseline: f.baseline,
        candidate: renamed,
      })
    ).files,
    ["docs/release-notes/v1.0.1.md"],
  );
  await rm(path.join(f.root, "docs/release-notes/v1.0.1.md"));
  const deleted = f.commit();
  assert.equal(
    (
      await findReleaseNoteChanges({
        repositoryRoot: f.root,
        baseline: renamed,
        candidate: deleted,
      })
    ).required,
    false,
  );
});
test("missing or mutable baselines fail closed instead of assuming no notes changed", async (context) => {
  const f = await fixture(context);
  for (const baseline of [undefined, "HEAD", "0".repeat(40)])
    await assert.rejects(
      findReleaseNoteChanges({
        repositoryRoot: f.root,
        baseline,
        candidate: f.baseline,
      }),
    );
});
test("documentation scope uses verified publication history rather than the preceding commit", async (context) => {
  const { inspectDocsRelease } = await import("./docs-release-scope.mjs");
  const f = await fixture(context);
  await f.write("docs/release-notes/v1.1.0.md", "# New release");
  f.commit();
  await f.write("docs/guide.md", "# Follow-up");
  const candidate = f.commit();
  const requestJson = async (route) =>
    route.includes("/runs?")
      ? { workflow_runs: [{ id: 1, head_sha: f.baseline }] }
      : {
          jobs: [
            {
              name: "deploy",
              steps: [
                {
                  name: "Deploy to GitHub Pages",
                  conclusion: "success",
                  completed_at: "2026-09-07T00:00:00Z",
                },
              ],
            },
          ],
        };
  const result = await inspectDocsRelease({
    repositoryRoot: f.root,
    repository: "Estance-Labs/corrobore",
    candidate,
    requestJson,
  });
  assert.equal(result.required, true);
  assert.equal(result.baseline, f.baseline);
  assert.equal(result.publication.runId, 1);
});

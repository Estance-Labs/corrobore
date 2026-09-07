import test from "node:test";
import assert from "node:assert/strict";
import { findPublishedDocsRevision } from "./docs-publication-history.mjs";
const sha = (n) => n.repeat(40);
const job = (time, conclusion = "success") => ({
  name: "deploy",
  steps: [{ name: "Deploy to GitHub Pages", conclusion, completed_at: time }],
});
test("selects actual successful publication time, including an older run rerun later", async () => {
  const runs = [
    { id: 2, head_sha: sha("b") },
    { id: 1, head_sha: sha("a") },
  ];
  const requestJson = async (route) =>
    route.includes("/runs?")
      ? { workflow_runs: runs }
      : route.includes("/2/jobs?")
        ? { jobs: [job("2026-09-01T00:00:00Z")] }
        : { jobs: [job("2026-09-02T00:00:00Z")] };
  const result = await findPublishedDocsRevision({
    repository: "Estance-Labs/corrobore",
    requestJson,
  });
  assert.equal(result.revision, sha("a"));
  assert.equal(result.runId, 1);
});
test("a successful run without a successful publish step is not a published baseline", async () => {
  await assert.rejects(
    findPublishedDocsRevision({
      repository: "Estance-Labs/corrobore",
      requestJson: async (route) =>
        route.includes("/runs?")
          ? { workflow_runs: [{ id: 1, head_sha: sha("a") }] }
          : { jobs: [job("2026-09-02T00:00:00Z", "skipped")] },
    }),
    /published/i,
  );
});
test("unavailable, malformed or truncated history fails closed", async () => {
  await assert.rejects(
    findPublishedDocsRevision({
      repository: "../other",
      requestJson: async () => ({}),
    }),
  );
  await assert.rejects(
    findPublishedDocsRevision({
      repository: "Estance-Labs/corrobore",
      requestJson: async () => {
        throw Error("offline");
      },
    }),
    /offline/,
  );
  await assert.rejects(
    findPublishedDocsRevision({
      repository: "Estance-Labs/corrobore",
      requestJson: async () => ({}),
    }),
  );
});
test("full pages beyond the history bound cannot silently choose a partial baseline", async () => {
  let requests = 0;
  await assert.rejects(
    findPublishedDocsRevision({
      repository: "Estance-Labs/corrobore",
      requestJson: async () => {
        requests++;
        return {
          workflow_runs: Array.from({ length: 100 }, (_, i) => ({
            id: i + 1,
            head_sha: sha("a"),
          })),
        };
      },
    }),
    /bounded pagination/,
  );
  assert.equal(requests, 20);
});

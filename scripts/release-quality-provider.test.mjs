import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, mkdir, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  verifyReleaseProvider,
  runPinnedQualityGate,
} from "./release-quality-provider.mjs";
const hash = (content) => createHash("sha256").update(content).digest("hex");
async function fixture(context) {
  const root = await mkdtemp(path.join(tmpdir(), "core-release-provider-"));
  context.after(() => rm(root, { recursive: true, force: true }));
  const command = (...args) =>
    execFileSync("git", args, { cwd: root, encoding: "utf8" }).trim();
  command("init", "-q");
  command(
    "remote",
    "add",
    "origin",
    "https://github.com/Estance-Labs/corrobore-benchmarks.git",
  );
  await mkdir(path.join(root, "scripts"));
  const content =
    "process.stdout.write(JSON.stringify(process.argv.slice(2)));\n";
  await writeFile(path.join(root, "scripts", "gate.mjs"), content);
  command("add", ".");
  command(
    "-c",
    "user.name=Test",
    "-c",
    "user.email=test@example.invalid",
    "commit",
    "-qm",
    "test: provider fixture",
  );
  const pin = {
    schemaVersion: "corrobore-release-quality-provider-v1",
    repository: "Estance-Labs/corrobore-benchmarks",
    revision: command("rev-parse", "HEAD"),
    entrypoint: "scripts/gate.mjs",
    files: { "scripts/gate.mjs": hash(content) },
  };
  return { root, pin, command };
}
test("verifies a clean exact provider revision and runs the verified entrypoint with caller bindings", async (context) => {
  const { root, pin } = await fixture(context);
  const verified = await verifyReleaseProvider(root, pin);
  assert.equal(verified.revision, pin.revision);
  const result = await runPinnedQualityGate(
    {
      providerRoot: root,
      bundle: "/tmp/test-bundle.json",
      candidate: "b".repeat(40),
      baseline: "a".repeat(40),
      releaseRef: "v1.2.3",
      releasePath: "engine-release",
      outputDir: "/tmp/test-decision",
    },
    pin,
  );
  assert.equal(result.exitCode, 0);
  const args = JSON.parse(result.stdout);
  assert.ok(args.includes("--candidate"));
  assert.ok(args.includes("b".repeat(40)));
  assert.ok(args.includes("engine-release"));
});
test("wrong revision, untracked changes and modified source bytes cannot qualify", async (context) => {
  const { root, pin, command } = await fixture(context);
  await assert.rejects(
    verifyReleaseProvider(root, { ...pin, revision: "0".repeat(40) }),
    /revision/i,
  );
  await writeFile(path.join(root, "unexpected.mjs"), "bad");
  await assert.rejects(verifyReleaseProvider(root, pin), /clean/i);
  await rm(path.join(root, "unexpected.mjs"));
  command("update-index", "--assume-unchanged", "scripts/gate.mjs");
  await writeFile(path.join(root, "scripts", "gate.mjs"), "process.exit(0);");
  assert.equal(command("status", "--porcelain"), "");
  await assert.rejects(verifyReleaseProvider(root, pin), /digest/i);
});
test("repository identity, unsafe manifest paths and missing entrypoint hashes are rejected", async (context) => {
  const { root, pin, command } = await fixture(context);
  await assert.rejects(
    verifyReleaseProvider(root, {
      ...pin,
      files: { "../escape": hash("bad") },
    }),
  );
  await assert.rejects(verifyReleaseProvider(root, { ...pin, files: {} }));
  command(
    "remote",
    "set-url",
    "origin",
    "https://github.com/other/repository.git",
  );
  await assert.rejects(verifyReleaseProvider(root, pin), /repository/i);
});

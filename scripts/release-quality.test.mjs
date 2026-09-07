import test from "node:test";
import assert from "node:assert/strict";
import {
  parseReleaseQualityArgs,
  runCoreReleaseQuality,
} from "./release-quality.mjs";
const args = () => [
  "--provider",
  "/tmp/provider",
  "--bundle",
  "/tmp/bundle.json",
  "--candidate",
  "b".repeat(40),
  "--baseline",
  "a".repeat(40),
  "--release-ref",
  "v1.2.3",
  "--release-path",
  "engine-release",
  "--output-dir",
  "/tmp/decision",
];
test("requires every caller binding without a skip or provider-pin override", () => {
  const parsed = parseReleaseQualityArgs(args());
  assert.equal(parsed.providerRoot, "/tmp/provider");
  assert.equal(parsed.candidate, "b".repeat(40));
  for (let i = 0; i < args().length; i += 2) {
    const missing = args();
    missing.splice(i, 2);
    assert.throws(() => parseReleaseQualityArgs(missing));
  }
  for (const extra of [
    ["--skip"],
    ["--pin", "other.json"],
    ["--baseline", "c".repeat(40)],
    ["positional"],
  ])
    assert.throws(() => parseReleaseQualityArgs([...args(), ...extra]));
});
test("rejects mutable revisions and unsupported publication families", () => {
  for (const [flag, value] of [
    ["--candidate", "main"],
    ["--baseline", "HEAD"],
    ["--release-path", "benchmark-publication"],
    ["--release-ref", ""],
  ]) {
    const changed = args();
    changed[changed.indexOf(flag) + 1] = value;
    assert.throws(() => parseReleaseQualityArgs(changed));
  }
});
test("preserves the canonical provider refusal and forwards independent bindings", async () => {
  let received;
  const result = await runCoreReleaseQuality(args(), {
    runGate: async (options) => {
      received = options;
      return {
        exitCode: 1,
        stdout: "blocked decision",
        stderr: "",
        providerRevision: "d".repeat(40),
      };
    },
  });
  assert.equal(result.exitCode, 1);
  assert.equal(received.baseline, "a".repeat(40));
  assert.equal(received.releaseRef, "v1.2.3");
});

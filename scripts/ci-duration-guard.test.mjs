import assert from "node:assert/strict";
import test from "node:test";

import { percentile, summarizeDurations } from "./ci-duration-guard.mjs";

test("percentile uses nearest-rank on sorted values", () => {
  const values = [10, 2, 7, 3, 5, 11, 13, 17, 19, 23];

  assert.equal(percentile(values, 50), 10);
  assert.equal(percentile(values, 95), 23);
});

test("summarizeDurations computes p50/p95/mean and bounds", () => {
  const summary = summarizeDurations([4, 5, 6, 7, 8]);

  assert.equal(summary.count, 5);
  assert.equal(summary.min, 4);
  assert.equal(summary.max, 8);
  assert.equal(summary.mean, 6);
  assert.equal(summary.p50, 6);
  assert.equal(summary.p95, 8);
});

test("summarizeDurations rejects empty or invalid arrays", () => {
  assert.throws(() => summarizeDurations([]), /at least one/);
  assert.throws(() => summarizeDurations([0, -1, Number.NaN]), /no valid positive durations/);
});